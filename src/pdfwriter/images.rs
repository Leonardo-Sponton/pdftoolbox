//! Imagens → PDF, uma imagem por página.
//!
//! JPEGs de 8 bits em cinza ou RGB são embutidos sem recompressão
//! (`DCTDecode`); a orientação EXIF é aplicada pela matriz de transformação,
//! também sem perda. Os demais formatos são decodificados e gravados com
//! Flate; a transparência vira `SMask`.

use std::io;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageDecoder, ImageReader, metadata::Orientation};

use super::{MM, Writer, deflate, fmt_num, write_err};
use crate::error::{OpError, OpResult};
use crate::jobs::CancelToken;

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

/// Imagem pronta para ser embutida.
pub(super) struct PdfImage {
    pub width: u32,
    pub height: u32,
    pub color_space: &'static str,
    pub filter: &'static str,
    pub data: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
    pub orientation: Orientation,
}

impl PdfImage {
    /// Largura ÷ altura da imagem exibida.
    pub fn aspect(&self) -> f32 {
        let (w, h) = (self.width as f32, self.height as f32);
        if orientation_matrix(self.orientation).1 { h / w } else { w / h }
    }
}

impl Writer {
    /// Grava a imagem (e a máscara de transparência, se houver) como
    /// XObject. Devolve o número do objeto da imagem.
    pub(super) fn image(&mut self, img: &PdfImage) -> io::Result<u32> {
        let smask = match &img.alpha {
            Some(alpha) => {
                let id = self.alloc();
                let dict = format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray \
                     /BitsPerComponent 8 /Filter /FlateDecode",
                    img.width, img.height
                );
                self.stream(id, &dict, alpha)?;
                format!(" /SMask {id} 0 R")
            }
            None => String::new(),
        };
        let id = self.alloc();
        let dict = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent 8 /Filter {}{smask}",
            img.width, img.height, img.color_space, img.filter
        );
        self.stream(id, &dict, &img.data)?;
        Ok(id)
    }
}

/// Erro “imagem não suportada” de `path`, com o detalhe do decodificador.
fn unsupported<E: std::fmt::Display>(path: &Path) -> impl Fn(E) -> OpError + '_ {
    |e| OpError::UnsupportedImage { file: path.to_path_buf(), detail: e.to_string() }
}

/// Decodificador da imagem em `path` (formato pelo conteúdo, não pela extensão).
fn open_decoder(path: &Path) -> OpResult<impl ImageDecoder> {
    let reader = ImageReader::open(path)
        .map_err(|e| OpError::io_path("ler", path, e))?
        .with_guessed_format()
        .map_err(unsupported(path))?;
    reader.into_decoder().map_err(unsupported(path))
}

pub(super) fn load_image(path: &Path) -> OpResult<PdfImage> {
    let bytes = std::fs::read(path).map_err(|e| OpError::io_path("ler", path, e))?;
    let reader = ImageReader::new(io::Cursor::new(&bytes)).with_guessed_format().map_err(unsupported(path))?;
    let format = reader.format();
    let mut decoder = reader.into_decoder().map_err(unsupported(path))?;
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

    let img = DynamicImage::from_decoder(decoder).map_err(unsupported(path))?;
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
            let (rgb, a) = split_alpha(&img.into_rgba8().into_raw());
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

/// RGBA intercalado → (RGB, alfa).
pub(super) fn split_alpha(rgba: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    let mut alpha = Vec::with_capacity(rgba.len() / 4);
    for p in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
        alpha.push(p[3]);
    }
    (rgb, alpha)
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
/// exibida, segundo a orientação EXIF, e se largura e altura se trocam.
pub(super) fn orientation_matrix(o: Orientation) -> ([f32; 6], bool) {
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

/// Matriz de orientação no formato do operador `cm`.
pub(super) fn orientation_cm(o: Orientation) -> String {
    let matrix: Vec<String> = orientation_matrix(o).0.iter().map(|&v| fmt_num(v)).collect();
    matrix.join(" ")
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
    let mut decoder = open_decoder(path)?;
    let (w, h) = decoder.dimensions();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    Ok(if orientation_matrix(orientation).1 { (h, w) } else { (w, h) })
}

/// Decodifica a imagem já na orientação exibida, com lado maior de no máximo
/// `max_side` px e, se pedido, com o fundo claro removido (`knockout_alpha`).
/// Também usada pela interface para desenhar a prévia da assinatura.
pub fn decode_rgba(path: &Path, max_side: u32, remove_white: bool) -> OpResult<image::RgbaImage> {
    let mut decoder = open_decoder(path)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(unsupported(path))?;
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
        let swapped = orientation_matrix(img.orientation).1;
        let (dw, dh) = if swapped { (img.height, img.width) } else { (img.width, img.height) };
        let Placement { page_w, page_h, x, y, w: w_img, h: h_img } = place(layout, dw as f32, dh as f32);
        let image_id = w.image(&img).map_err(write_err)?;
        let content = format!(
            "q {} 0 0 {} {} {} cm {} cm /Im0 Do Q",
            fmt_num(w_img),
            fmt_num(h_img),
            fmt_num(x),
            fmt_num(y),
            orientation_cm(img.orientation)
        );
        drop(img);
        let resources = format!("/XObject << /Im0 {image_id} 0 R >>");
        pages.push(w.page(tree, (page_w, page_h), &resources, "", content.as_bytes()).map_err(write_err)?);
        on_page(index + 1);
    }
    w.finish(catalog, tree, &pages).map_err(write_err)
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
    fn landscape_images_get_landscape_pages() {
        let layout = ImageLayout { size: PageSize::A4, margin_mm: 10.0 };
        let p = place(layout, 4000.0, 3000.0);
        assert!(p.page_w > p.page_h);
        assert!((p.x - 10.0 * MM).abs() < 0.5 || (p.y - 10.0 * MM).abs() < 0.5);
        let fit = place(ImageLayout { size: PageSize::FitImage, margin_mm: 10.0 }, 800.0, 600.0);
        assert_eq!((fit.page_w, fit.page_h, fit.x, fit.w), (600.0, 450.0, 0.0, 600.0));
    }
}
