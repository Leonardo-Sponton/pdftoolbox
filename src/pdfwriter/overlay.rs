//! Camadas sobrepostas (`qpdf --overlay`): numeração, marca d’água,
//! anotações e assinaturas. Uma página da camada por página do documento,
//! do mesmo tamanho exibido, só com as marcas (fundo transparente).

use std::io;
use std::path::Path;

use image::metadata::Orientation;

use super::images::{PdfImage, load_image, orientation_cm, split_alpha};
use super::{Writer, deflate, fmt_num, write_err};
use crate::error::OpResult;

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
        let rgba = super::decode_rgba(path, KNOCKOUT_MAX_SIDE, true)?;
        let (width, height) = rgba.dimensions();
        let (rgb, alpha) = split_alpha(rgba.as_raw());
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
        self.img.aspect()
    }
}

/// Gera o PDF de camada usado com `qpdf --overlay`.
pub fn overlay_pdf(out: &Path, pages: &[OverlayPage]) -> io::Result<()> {
    overlay_pdf_with_images(out, pages, &[])
}

/// Estados gráficos (opacidade, mistura) usados por uma página.
#[derive(Default)]
struct GraphicStates(Vec<String>);

impl GraphicStates {
    /// Operador que ativa o estado pedido (vazio quando é o padrão).
    fn select(&mut self, opacity: f32, multiply: bool) -> String {
        if opacity >= 1.0 && !multiply {
            return String::new();
        }
        let i = self.0.len();
        let blend = if multiply { " /BM /Multiply" } else { "" };
        self.0.push(format!("/G{i} << /Type /ExtGState /ca {0} /CA {0}{blend} >>", fmt_num(opacity.min(1.0))));
        format!("/G{i} gs ")
    }

    fn resources(&self) -> String {
        if self.0.is_empty() { String::new() } else { format!(" /ExtGState << {} >>", self.0.join(" ")) }
    }
}

fn shape_ops(s: &ShapeMark, gs: &str) -> String {
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
    format!(
        "q {gs}{ops}{} {} {} {} re {paint} Q\n",
        fmt_num(s.x + inset),
        fmt_num(s.y + inset),
        fmt_num((s.w - 2.0 * inset).max(0.0)),
        fmt_num((s.h - 2.0 * inset).max(0.0)),
    )
}

fn text_ops(m: &TextMark, gs: &str) -> String {
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
    format!(
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
    )
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
    let image_ids = images.iter().map(|i| w.image(&i.img)).collect::<io::Result<Vec<u32>>>()?;
    let mut ids = Vec::new();
    for page in pages {
        let mut content = String::new();
        let mut states = GraphicStates::default();
        for s in &page.shapes {
            let gs = states.select(s.opacity, s.multiply);
            content.push_str(&shape_ops(s, &gs));
        }
        let mut used_images = Vec::new();
        for m in &page.images {
            let Some(OverlayImage { img }) = images.get(m.image) else { continue };
            if !used_images.contains(&m.image) {
                used_images.push(m.image);
            }
            let gs = states.select(m.opacity, false);
            content.push_str(&format!(
                "q {gs}{} 0 0 {} {} {} cm {} cm /Im{} Do Q\n",
                fmt_num(m.w),
                fmt_num(m.h),
                fmt_num(m.x),
                fmt_num(m.y),
                orientation_cm(img.orientation),
                m.image
            ));
        }
        for m in &page.marks {
            let gs = states.select(m.opacity, false);
            content.push_str(&text_ops(m, &gs));
        }
        let xobjects = if used_images.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = used_images.iter().map(|&i| format!("/Im{i} {} 0 R", image_ids[i])).collect();
            format!(" /XObject << {} >>", list.join(" "))
        };
        let resources = format!("/Font << /F1 {regular} 0 R /F2 {bold} 0 R >>{}{xobjects}", states.resources());
        ids.push(w.page(tree, (page.width, page.height), &resources, "", content.as_bytes())?);
    }
    w.finish(catalog, tree, &ids)
}
