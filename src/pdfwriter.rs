//! Gerador mínimo de PDF, usado para imagens → PDF e para fixtures de teste.
//!
//! JPEGs de 8 bits em cinza ou RGB são embutidos sem recompressão
//! (`DCTDecode`); a orientação EXIF é aplicada pela matriz de transformação,
//! também sem perda. Os demais formatos são decodificados e gravados com
//! Flate; a transparência vira `SMask`.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::ZlibEncoder;
use image::{DynamicImage, ImageDecoder, ImageReader, metadata::Orientation};

use crate::error::{OpError, OpResult};
use crate::jobs::CancelToken;

const MM: f32 = 72.0 / 25.4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageSize {
    /// Página do tamanho da imagem (considerando 96 ppp).
    FitImage,
    A4,
    Letter,
}

impl PageSize {
    fn portrait_points(self) -> Option<(f32, f32)> {
        match self {
            PageSize::FitImage => None,
            PageSize::A4 => Some((210.0 * MM, 297.0 * MM)),
            PageSize::Letter => Some((612.0, 792.0)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ImageLayout {
    pub size: PageSize,
    pub margin_mm: f32,
}

impl Default for ImageLayout {
    fn default() -> Self {
        ImageLayout { size: PageSize::A4, margin_mm: 10.0 }
    }
}

struct Writer {
    out: BufWriter<File>,
    pos: u64,
    /// Deslocamento de cada objeto; o índice é o número do objeto (0 não é usado).
    offsets: Vec<u64>,
}

impl Writer {
    fn create(path: &Path) -> io::Result<Self> {
        let file = File::create(path)?;
        let mut w = Writer { out: BufWriter::new(file), pos: 0, offsets: vec![0] };
        w.raw(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")?;
        Ok(w)
    }

    fn alloc(&mut self) -> u32 {
        self.offsets.push(0);
        (self.offsets.len() - 1) as u32
    }

    fn raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn obj(&mut self, id: u32, body: &str) -> io::Result<()> {
        self.offsets[id as usize] = self.pos;
        self.raw(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes())
    }

    fn stream(&mut self, id: u32, dict: &str, data: &[u8]) -> io::Result<()> {
        self.offsets[id as usize] = self.pos;
        self.raw(format!("{id} 0 obj\n<< {dict} /Length {} >>\nstream\n", data.len()).as_bytes())?;
        self.raw(data)?;
        self.raw(b"\nendstream\nendobj\n")
    }

    fn finish(mut self, root: u32, info: u32) -> io::Result<()> {
        let xref = self.pos;
        let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", self.offsets.len());
        for off in &self.offsets[1..] {
            table.push_str(&format!("{off:010} 00000 n \n"));
        }
        table.push_str(&format!(
            "trailer\n<< /Size {} /Root {root} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            self.offsets.len()
        ));
        self.raw(table.as_bytes())?;
        self.out.flush()?;
        self.out.get_ref().sync_all()
    }
}

/// Escreve catálogo, árvore de páginas e Info. `pages` são os ids dos objetos Page.
fn write_tree(w: &mut Writer, catalog: u32, tree: u32, pages: &[u32]) -> io::Result<u32> {
    let kids: Vec<String> = pages.iter().map(|p| format!("{p} 0 R")).collect();
    w.obj(tree, &format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()))?;
    w.obj(catalog, &format!("<< /Type /Catalog /Pages {tree} 0 R >>"))?;
    let info = w.alloc();
    w.obj(info, "<< /Producer (PDFToolbox) >>")?;
    Ok(info)
}

fn deflate(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::with_capacity(data.len() / 2), Compression::default());
    enc.write_all(data)?;
    enc.finish()
}

fn write_err(e: io::Error) -> OpError {
    OpError::io("gravar o PDF temporário", e)
}

/// Imagem pronta para ser embutida.
struct PdfImage {
    width: u32,
    height: u32,
    color_space: &'static str,
    filter: &'static str,
    data: Vec<u8>,
    alpha: Option<Vec<u8>>,
    orientation: Orientation,
}

fn load_image(path: &Path) -> OpResult<PdfImage> {
    let unsupported = |detail: String| OpError::UnsupportedImage { file: path.to_path_buf(), detail };
    let bytes = std::fs::read(path).map_err(|e| OpError::io_path("ler", path, e))?;

    let reader =
        ImageReader::new(io::Cursor::new(&bytes)).with_guessed_format().map_err(|e| unsupported(e.to_string()))?;
    let format = reader.format();
    let mut decoder = reader.into_decoder().map_err(|e| unsupported(e.to_string()))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);

    if format == Some(image::ImageFormat::Jpeg)
        && let Some((8, comps)) = jpeg_sof(&bytes)
        && (comps == 1 || comps == 3)
    {
        let (width, height) = decoder.dimensions();
        drop(decoder);
        return Ok(PdfImage {
            width,
            height,
            color_space: if comps == 1 { "/DeviceGray" } else { "/DeviceRGB" },
            filter: "/DCTDecode",
            data: bytes,
            alpha: None,
            orientation,
        });
    }

    let img = DynamicImage::from_decoder(decoder).map_err(|e| unsupported(e.to_string()))?;
    let (width, height) = (img.width(), img.height());
    let has_alpha = img.color().has_alpha();
    let gray = !img.color().has_color();
    let (pixels, color_space, alpha) = match (gray, has_alpha) {
        (true, false) => (img.into_luma8().into_raw(), "/DeviceGray", None),
        (false, false) => (img.into_rgb8().into_raw(), "/DeviceRGB", None),
        (true, true) => {
            let la = img.into_luma_alpha8().into_raw();
            let (l, a): (Vec<u8>, Vec<u8>) = la.chunks_exact(2).map(|p| (p[0], p[1])).unzip();
            (l, "/DeviceGray", Some(a))
        }
        (false, true) => {
            let rgba = img.into_rgba8().into_raw();
            let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
            let mut a = Vec::with_capacity(rgba.len() / 4);
            for p in rgba.chunks_exact(4) {
                rgb.extend_from_slice(&p[..3]);
                a.push(p[3]);
            }
            (rgb, "/DeviceRGB", Some(a))
        }
    };
    // Canal alfa totalmente opaco não precisa de máscara.
    let alpha = alpha.filter(|a| a.iter().any(|&v| v != 255));
    Ok(PdfImage {
        width,
        height,
        color_space,
        filter: "/FlateDecode",
        data: deflate(&pixels).map_err(write_err)?,
        alpha: alpha.map(|a| deflate(&a)).transpose().map_err(write_err)?,
        orientation,
    })
}

/// Lê o marcador SOF de um JPEG: (precisão em bits, número de componentes).
fn jpeg_sof(bytes: &[u8]) -> Option<(u8, u8)> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if matches!(marker, 0x01 | 0xD0..=0xD7) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return Some((*bytes.get(i + 4)?, *bytes.get(i + 9)?));
        }
        if marker == 0xDA || len < 2 {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// Matriz que leva o quadrado unitário da imagem armazenada para a imagem
/// exibida, segundo a orientação EXIF.
fn orientation_matrix(o: Orientation) -> ([f32; 6], bool) {
    match o {
        Orientation::NoTransforms => ([1., 0., 0., 1., 0., 0.], false),
        Orientation::FlipHorizontal => ([-1., 0., 0., 1., 1., 0.], false),
        Orientation::Rotate180 => ([-1., 0., 0., -1., 1., 1.], false),
        Orientation::FlipVertical => ([1., 0., 0., -1., 0., 1.], false),
        Orientation::Rotate90 => ([0., -1., 1., 0., 0., 1.], true),
        Orientation::Rotate270 => ([0., 1., -1., 0., 1., 0.], true),
        Orientation::Rotate90FlipH => ([0., -1., -1., 0., 1., 1.], true),
        Orientation::Rotate270FlipH => ([0., 1., 1., 0., 0., 0.], true),
    }
}

fn fmt_num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// Posição da imagem na página, em pontos (origem no canto inferior esquerdo).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub page_w: f32,
    pub page_h: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Calcula a página e a posição de uma imagem exibida com `dw` × `dh` px.
/// Imagens mais largas que altas ganham página em paisagem.
pub fn place(layout: ImageLayout, dw: f32, dh: f32) -> Placement {
    match layout.size.portrait_points() {
        None => {
            let (pw, ph) = (dw * 0.75, dh * 0.75); // 96 ppp → pontos
            Placement { page_w: pw, page_h: ph, x: 0.0, y: 0.0, w: pw, h: ph }
        }
        Some((pw, ph)) => {
            let (pw, ph) = if dw > dh { (ph, pw) } else { (pw, ph) };
            let m = (layout.margin_mm.max(0.0) * MM).min(pw.min(ph) / 3.0);
            let scale = ((pw - 2.0 * m) / dw).min((ph - 2.0 * m) / dh);
            let (w, h) = (dw * scale, dh * scale);
            Placement { page_w: pw, page_h: ph, x: (pw - w) / 2.0, y: (ph - h) / 2.0, w, h }
        }
    }
}

/// Dimensões da imagem como exibida (orientação EXIF aplicada), sem decodificá-la.
pub fn display_dimensions(path: &Path) -> OpResult<(u32, u32)> {
    let unsupported = |detail: String| OpError::UnsupportedImage { file: path.to_path_buf(), detail };
    let reader = ImageReader::open(path)
        .map_err(|e| OpError::io_path("ler", path, e))?
        .with_guessed_format()
        .map_err(|e| unsupported(e.to_string()))?;
    let mut decoder = reader.into_decoder().map_err(|e| unsupported(e.to_string()))?;
    let (w, h) = decoder.dimensions();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    Ok(if orientation_matrix(orientation).1 { (h, w) } else { (w, h) })
}

/// Converte imagens em um único PDF (uma imagem por página).
pub fn images_to_pdf(
    images: &[PathBuf],
    layout: ImageLayout,
    out: &Path,
    cancel: &CancelToken,
    mut on_page: impl FnMut(usize),
) -> OpResult<()> {
    if images.is_empty() {
        return Err(OpError::NoInput);
    }
    let mut w = Writer::create(out).map_err(write_err)?;
    let catalog = w.alloc();
    let tree = w.alloc();
    let mut pages = Vec::with_capacity(images.len());

    for (index, path) in images.iter().enumerate() {
        cancel.check()?;
        let img = load_image(path)?;
        let (matrix, swapped) = orientation_matrix(img.orientation);
        let (dw, dh) = if swapped { (img.height, img.width) } else { (img.width, img.height) };
        let (dw, dh) = (dw as f32, dh as f32);

        let Placement { page_w, page_h, x, y, w: w_img, h: h_img } = place(layout, dw, dh);

        let smask = match &img.alpha {
            Some(alpha) => {
                let id = w.alloc();
                let dict = format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray \
                     /BitsPerComponent 8 /Filter /FlateDecode",
                    img.width, img.height
                );
                w.stream(id, &dict, alpha).map_err(write_err)?;
                format!(" /SMask {id} 0 R")
            }
            None => String::new(),
        };
        let image_id = w.alloc();
        let dict = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent 8 /Filter {}{smask}",
            img.width, img.height, img.color_space, img.filter
        );
        w.stream(image_id, &dict, &img.data).map_err(write_err)?;
        drop(img);

        let m: Vec<String> = matrix.iter().map(|&v| fmt_num(v)).collect();
        let content = format!(
            "q {} 0 0 {} {} {} cm {} cm /Im0 Do Q",
            fmt_num(w_img),
            fmt_num(h_img),
            fmt_num(x),
            fmt_num(y),
            m.join(" ")
        );
        let content_id = w.alloc();
        w.stream(content_id, "", content.as_bytes()).map_err(write_err)?;
        let page_id = w.alloc();
        w.obj(
            page_id,
            &format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] \
                 /Resources << /XObject << /Im0 {image_id} 0 R >> >> /Contents {content_id} 0 R >>",
                fmt_num(page_w),
                fmt_num(page_h)
            ),
        )
        .map_err(write_err)?;
        pages.push(page_id);
        on_page(index + 1);
    }

    let info = write_tree(&mut w, catalog, tree, &pages).map_err(write_err)?;
    w.finish(catalog, info).map_err(write_err)
}

/// Gera um PDF de texto simples (Helvetica, ASCII) — usado nas fixtures de
/// teste e para páginas de amostra. Cada item de `pages` é o texto de uma página.
pub fn text_pdf(out: &Path, pages: &[&str], size: (f32, f32)) -> io::Result<()> {
    let mut w = Writer::create(out)?;
    let catalog = w.alloc();
    let tree = w.alloc();
    let font = w.alloc();
    w.obj(font, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")?;
    let mut ids = Vec::new();
    for text in pages {
        let escaped: String = text
            .chars()
            .filter(char::is_ascii)
            .flat_map(|c| match c {
                '(' | ')' | '\\' => vec!['\\', c],
                c => vec![c],
            })
            .collect();
        let content = format!("BT /F1 24 Tf 56 {} Td ({escaped}) Tj ET", fmt_num(size.1 - 80.0));
        let content_id = w.alloc();
        w.stream(content_id, "", content.as_bytes())?;
        let page = w.alloc();
        w.obj(
            page,
            &format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] \
                 /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content_id} 0 R >>",
                fmt_num(size.0),
                fmt_num(size.1)
            ),
        )?;
        ids.push(page);
    }
    let info = write_tree(&mut w, catalog, tree, &ids)?;
    w.finish(catalog, info)
}

pub const A4_POINTS: (f32, f32) = (595.276, 841.89);

/// Texto a desenhar numa página da camada sobreposta (carimbos).
#[derive(Debug, Clone, PartialEq)]
pub struct TextMark {
    /// Texto já codificado em WinAnsi (veja `stamp::encode`).
    pub text: Vec<u8>,
    pub bold: bool,
    pub size: f32,
    /// Ponto de origem da linha de base, em pontos.
    pub x: f32,
    pub y: f32,
    /// Rotação anti-horária em graus em torno de (x, y).
    pub angle: f32,
    pub color: (f32, f32, f32),
    pub opacity: f32,
}

/// Retângulo da camada (destaque ou moldura), origem no canto inferior esquerdo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeMark {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub fill: Option<(f32, f32, f32)>,
    pub stroke: Option<(f32, f32, f32)>,
    pub line_width: f32,
    pub opacity: f32,
    /// Mistura “multiplicar”: o texto embaixo continua legível (marca-texto).
    pub multiply: bool,
}

/// Imagem da camada: índice em `images` de `overlay_pdf_with_images` e a
/// caixa onde ela é desenhada, origem no canto inferior esquerdo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageMark {
    pub image: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub opacity: f32,
}

/// Página da camada: tamanho exibido e o que desenhar, nesta ordem:
/// formas, imagens e textos.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OverlayPage {
    pub width: f32,
    pub height: f32,
    pub shapes: Vec<ShapeMark>,
    pub images: Vec<ImageMark>,
    pub marks: Vec<TextMark>,
}

impl OverlayPage {
    pub fn empty(width: f32, height: f32) -> Self {
        OverlayPage { width, height, ..Default::default() }
    }
}

/// Imagem preparada para a camada (assinatura escaneada, logotipo).
pub struct OverlayImage {
    img: PdfImage,
}

/// Lado maior da imagem quando o fundo branco é removido (a imagem precisa
/// ser decodificada e recomprimida; assinaturas não precisam de mais).
const KNOCKOUT_MAX_SIDE: u32 = 2000;

impl OverlayImage {
    /// Carrega a imagem. Com `remove_white`, o papel claro de uma assinatura
    /// fotografada ou escaneada vira transparente (claro → transparente,
    /// escuro → opaco, com transição suave para não serrilhar o traço).
    pub fn load(path: &Path, remove_white: bool) -> OpResult<Self> {
        if !remove_white {
            return Ok(OverlayImage { img: load_image(path)? });
        }
        let rgba = decode_rgba(path, KNOCKOUT_MAX_SIDE, true)?;
        let (width, height) = rgba.dimensions();
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        let mut alpha = Vec::with_capacity((width * height) as usize);
        for p in rgba.pixels() {
            rgb.extend_from_slice(&p.0[..3]);
            alpha.push(p.0[3]);
        }
        Ok(OverlayImage {
            img: PdfImage {
                width,
                height,
                color_space: "/DeviceRGB",
                filter: "/FlateDecode",
                data: deflate(&rgb).map_err(write_err)?,
                alpha: Some(deflate(&alpha).map_err(write_err)?),
                orientation: Orientation::NoTransforms,
            },
        })
    }

    /// Largura ÷ altura da imagem exibida.
    pub fn aspect(&self) -> f32 {
        let (w, h) = (self.img.width as f32, self.img.height as f32);
        if orientation_matrix(self.img.orientation).1 { h / w } else { w / h }
    }
}

/// Decodifica a imagem já na orientação exibida, com lado maior de no máximo
/// `max_side` px e, se pedido, com o fundo claro removido (`knockout_alpha`).
/// Também usada pela interface para desenhar a prévia da assinatura.
pub fn decode_rgba(path: &Path, max_side: u32, remove_white: bool) -> OpResult<image::RgbaImage> {
    let unsupported = |detail: String| OpError::UnsupportedImage { file: path.to_path_buf(), detail };
    let reader = ImageReader::open(path)
        .map_err(|e| OpError::io_path("ler", path, e))?
        .with_guessed_format()
        .map_err(|e| unsupported(e.to_string()))?;
    let mut decoder = reader.into_decoder().map_err(|e| unsupported(e.to_string()))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| unsupported(e.to_string()))?;
    img.apply_orientation(orientation);
    if img.width().max(img.height()) > max_side {
        img = img.resize(max_side, max_side, image::imageops::FilterType::Triangle);
    }
    let mut rgba = img.into_rgba8();
    if remove_white {
        for p in rgba.pixels_mut() {
            p.0[3] = knockout_alpha(p.0);
        }
    }
    Ok(rgba)
}

/// Opacidade de um pixel com o fundo claro removido.
fn knockout_alpha([r, g, b, a]: [u8; 4]) -> u8 {
    const PAPER: f32 = 225.0;
    const INK: f32 = 160.0;
    let lum = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    let k = ((PAPER - lum) / (PAPER - INK)).clamp(0.0, 1.0);
    (k * a as f32).round() as u8
}

/// Gera o PDF de camada usado com `qpdf --overlay`: uma página por página do
/// documento, do mesmo tamanho exibido, só com as marcas (fundo transparente).
pub fn overlay_pdf(out: &Path, pages: &[OverlayPage]) -> io::Result<()> {
    overlay_pdf_with_images(out, pages, &[])
}

/// Como `overlay_pdf`, com as imagens citadas pelos `ImageMark` das páginas.
/// Cada imagem é gravada uma vez, mesmo que apareça em várias páginas.
pub fn overlay_pdf_with_images(out: &Path, pages: &[OverlayPage], images: &[OverlayImage]) -> io::Result<()> {
    let mut w = Writer::create(out)?;
    let catalog = w.alloc();
    let tree = w.alloc();
    let regular = w.alloc();
    let bold = w.alloc();
    w.obj(regular, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")?;
    w.obj(bold, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>")?;
    let mut image_ids = Vec::with_capacity(images.len());
    for OverlayImage { img } in images {
        let smask = match &img.alpha {
            Some(alpha) => {
                let id = w.alloc();
                let dict = format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray \
                     /BitsPerComponent 8 /Filter /FlateDecode",
                    img.width, img.height
                );
                w.stream(id, &dict, alpha)?;
                format!(" /SMask {id} 0 R")
            }
            None => String::new(),
        };
        let id = w.alloc();
        let dict = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent 8 /Filter {}{smask}",
            img.width, img.height, img.color_space, img.filter
        );
        w.stream(id, &dict, &img.data)?;
        image_ids.push(id);
    }
    let mut ids = Vec::new();
    for page in pages {
        let mut content = String::new();
        let mut states = Vec::new();
        let mut state = |opacity: f32, multiply: bool| -> String {
            if opacity >= 1.0 && !multiply {
                return String::new();
            }
            let i = states.len();
            let blend = if multiply { " /BM /Multiply" } else { "" };
            states.push(format!("/G{i} << /Type /ExtGState /ca {0} /CA {0}{blend} >>", fmt_num(opacity.min(1.0))));
            format!("/G{i} gs ")
        };
        for s in &page.shapes {
            let gs = state(s.opacity, s.multiply);
            let mut ops = String::new();
            if let Some((r, g, b)) = s.fill {
                ops.push_str(&format!("{} {} {} rg ", fmt_num(r), fmt_num(g), fmt_num(b)));
            }
            if let Some((r, g, b)) = s.stroke {
                ops.push_str(&format!("{} {} {} RG {} w ", fmt_num(r), fmt_num(g), fmt_num(b), fmt_num(s.line_width)));
            }
            let paint = match (s.fill.is_some(), s.stroke.is_some()) {
                (true, true) => "B",
                (true, false) => "f",
                (false, true) => "S",
                (false, false) => "n",
            };
            // A moldura fica do lado de dentro da caixa.
            let inset = if s.stroke.is_some() { s.line_width / 2.0 } else { 0.0 };
            content.push_str(&format!(
                "q {gs}{ops}{} {} {} {} re {paint} Q\n",
                fmt_num(s.x + inset),
                fmt_num(s.y + inset),
                fmt_num((s.w - 2.0 * inset).max(0.0)),
                fmt_num((s.h - 2.0 * inset).max(0.0)),
            ));
        }
        let mut used_images = Vec::new();
        for m in &page.images {
            let Some(OverlayImage { img }) = images.get(m.image) else { continue };
            if !used_images.contains(&m.image) {
                used_images.push(m.image);
            }
            let gs = state(m.opacity, false);
            let matrix: Vec<String> = orientation_matrix(img.orientation).0.iter().map(|&v| fmt_num(v)).collect();
            content.push_str(&format!(
                "q {gs}{} 0 0 {} {} {} cm {} cm /Im{} Do Q\n",
                fmt_num(m.w),
                fmt_num(m.h),
                fmt_num(m.x),
                fmt_num(m.y),
                matrix.join(" "),
                m.image
            ));
        }
        for m in &page.marks {
            let escaped: String = m
                .text
                .iter()
                .map(|&b| match b {
                    b'(' | b')' | b'\\' => format!("\\{}", b as char),
                    32..=126 => (b as char).to_string(),
                    _ => format!("\\{b:03o}"),
                })
                .collect();
            let (sin, cos) = m.angle.to_radians().sin_cos();
            let gs = state(m.opacity, false);
            content.push_str(&format!(
                "q {gs}{} {} {} rg BT /{} {} Tf {} {} {} {} {} {} Tm ({escaped}) Tj ET Q\n",
                fmt_num(m.color.0),
                fmt_num(m.color.1),
                fmt_num(m.color.2),
                if m.bold { "F2" } else { "F1" },
                fmt_num(m.size),
                fmt_num(cos),
                fmt_num(sin),
                fmt_num(-sin),
                fmt_num(cos),
                fmt_num(m.x),
                fmt_num(m.y),
            ));
        }
        let content_id = w.alloc();
        w.stream(content_id, "", content.as_bytes())?;
        let page_id = w.alloc();
        let ext = if states.is_empty() { String::new() } else { format!(" /ExtGState << {} >>", states.join(" ")) };
        let xobj = if used_images.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = used_images.iter().map(|&i| format!("/Im{i} {} 0 R", image_ids[i])).collect();
            format!(" /XObject << {} >>", list.join(" "))
        };
        w.obj(
            page_id,
            &format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] \
                 /Resources << /Font << /F1 {regular} 0 R /F2 {bold} 0 R >>{ext}{xobj} >> /Contents {content_id} 0 R >>",
                fmt_num(page.width),
                fmt_num(page.height)
            ),
        )?;
        ids.push(page_id);
    }
    let info = write_tree(&mut w, catalog, tree, &ids)?;
    w.finish(catalog, info)
}

/// Palavra da camada de texto invisível do OCR, em pontos (origem no canto
/// inferior esquerdo): começa em `x` sobre a linha de base `baseline` e ocupa
/// `width` na horizontal.
#[derive(Debug, Clone, PartialEq)]
pub struct InvisibleWord {
    pub text: String,
    pub x: f32,
    pub baseline: f32,
    pub width: f32,
    pub size: f32,
}

/// Página da camada do OCR. `width` × `height` é a página como exibida;
/// as palavras estão no sistema da imagem depois de girada `rotation` graus
/// (sentido horário) para o texto ficar de pé.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InvisiblePage {
    pub width: f32,
    pub height: f32,
    pub rotation: u32,
    pub words: Vec<InvisibleWord>,
}

impl InvisiblePage {
    /// Matriz que leva o sistema da imagem endireitada ao da página.
    fn upright_matrix(&self) -> Option<String> {
        let (w, h) = (fmt_num(self.width), fmt_num(self.height));
        match self.rotation {
            90 => Some(format!("0 1 -1 0 {w} 0")),
            180 => Some(format!("-1 0 0 -1 {w} {h}")),
            270 => Some(format!("0 -1 1 0 0 {h}")),
            _ => None,
        }
    }
}

/// Largura de cada caractere da fonte sem desenho, em milésimos do corpo.
const GLYPHLESS_WIDTH: f32 = 500.0;

/// Texto em UTF-16BE hexadecimal (os CIDs da fonte são as unidades UTF-16).
fn utf16_hex(text: &str) -> (String, usize) {
    let units: Vec<u16> = text.encode_utf16().collect();
    (units.iter().map(|u| format!("{u:04X}")).collect(), units.len())
}

/// PDF de camada com texto invisível (modo de renderização 3): selecionável
/// e pesquisável, sem desenhar nada. Usa a fonte sem desenho do Tesseract
/// (`glyphless_font`), com CID = unidade UTF-16 e ToUnicode identidade, de
/// modo que qualquer caractere reconhecido é preservado. Cada palavra é
/// esticada (`Tz`) para cobrir a largura dela na imagem.
pub fn invisible_text_pdf(out: &Path, pages: &[InvisiblePage], glyphless_font: &[u8]) -> io::Result<()> {
    let mut w = Writer::create(out)?;
    let catalog = w.alloc();
    let tree = w.alloc();
    let font_file = w.alloc();
    w.stream(font_file, &format!("/Length1 {}", glyphless_font.len()), glyphless_font)?;
    let descriptor = w.alloc();
    w.obj(
        descriptor,
        &format!(
            "<< /Type /FontDescriptor /FontName /GlyphLessFont /Flags 5 /FontBBox [0 0 {GLYPHLESS_WIDTH} 1000] \
             /ItalicAngle 0 /Ascent 1000 /Descent 0 /CapHeight 1000 /StemV 80 /FontFile2 {font_file} 0 R >>"
        ),
    )?;
    // Todos os CIDs apontam para o glifo 1 (vazio) da fonte.
    let gid_map: Vec<u8> = std::iter::repeat_n([0u8, 1u8], 65536).flatten().collect();
    let map = w.alloc();
    w.stream(map, "/Filter /FlateDecode", &deflate(&gid_map)?)?;
    let cid_font = w.alloc();
    w.obj(
        cid_font,
        &format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /GlyphLessFont \
             /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor {descriptor} 0 R /DW {GLYPHLESS_WIDTH} /CIDToGIDMap {map} 0 R >>"
        ),
    )?;
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    // Faixas de no máximo 256 códigos: o destino só incrementa o último byte.
    for chunk in (0..256u32).collect::<Vec<_>>().chunks(100) {
        cmap.push_str(&format!("{} beginbfrange\n", chunk.len()));
        for hi in chunk {
            cmap.push_str(&format!("<{hi:02X}00> <{hi:02X}FF> <{hi:02X}00>\n"));
        }
        cmap.push_str("endbfrange\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let to_unicode = w.alloc();
    w.stream(to_unicode, "/Filter /FlateDecode", &deflate(cmap.as_bytes())?)?;
    let font = w.alloc();
    w.obj(
        font,
        &format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /GlyphLessFont /Encoding /Identity-H \
             /DescendantFonts [{cid_font} 0 R] /ToUnicode {to_unicode} 0 R >>"
        ),
    )?;
    let mut ids = Vec::new();
    for page in pages {
        let mut content = match page.upright_matrix() {
            Some(m) => format!("q {m} cm\nBT 3 Tr\n"),
            None => String::from("q\nBT 3 Tr\n"),
        };
        for word in &page.words {
            let (hex, units) = utf16_hex(&word.text);
            if units == 0 || word.size <= 0.0 {
                continue;
            }
            let natural = units as f32 * GLYPHLESS_WIDTH / 1000.0 * word.size;
            let stretch = (100.0 * word.width / natural).clamp(1.0, 10_000.0);
            content.push_str(&format!(
                "/F1 {} Tf {} Tz 1 0 0 1 {} {} Tm <{hex}> Tj\n",
                fmt_num(word.size),
                fmt_num(stretch),
                fmt_num(word.x),
                fmt_num(word.baseline)
            ));
        }
        content.push_str("ET Q\n");
        let content_id = w.alloc();
        w.stream(content_id, "/Filter /FlateDecode", &deflate(content.as_bytes())?)?;
        let page_id = w.alloc();
        w.obj(
            page_id,
            &format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] \
                 /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content_id} 0 R >>",
                fmt_num(page.width),
                fmt_num(page.height)
            ),
        )?;
        ids.push(page_id);
    }
    let info = write_tree(&mut w, catalog, tree, &ids)?;
    w.finish(catalog, info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_sof_parses_header() {
        // SOI, APP0 (len 4), SOF0: len 17, precisão 8, 2x3, 3 componentes
        let data = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x02, 0x00, 0x03, 0x03,
        ];
        assert_eq!(jpeg_sof(&data), Some((8, 3)));
        assert_eq!(jpeg_sof(b"not a jpeg"), None);
    }

    #[test]
    fn numbers_are_compact() {
        assert_eq!(fmt_num(1.0), "1");
        assert_eq!(fmt_num(0.5), "0.5");
        assert_eq!(fmt_num(-0.0), "0");
    }

    #[test]
    fn landscape_images_get_landscape_pages() {
        let layout = ImageLayout { size: PageSize::A4, margin_mm: 10.0 };
        let p = place(layout, 4000.0, 3000.0);
        assert!(p.page_w > p.page_h);
        assert!((p.x - 10.0 * MM).abs() < 0.5 || (p.y - 10.0 * MM).abs() < 0.5);
        let fit = place(ImageLayout { size: PageSize::FitImage, margin_mm: 10.0 }, 800.0, 600.0);
        assert_eq!((fit.page_w, fit.page_h, fit.x, fit.w), (600.0, 450.0, 0.0, 600.0));
    }
}
