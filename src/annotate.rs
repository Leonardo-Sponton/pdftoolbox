//! Anotações e assinatura visual: textos, imagens (assinatura escaneada),
//! destaques e molduras posicionados pelo usuário numa página.
//!
//! Tudo vira a mesma camada sobreposta pelo qpdf usada nos carimbos: o
//! conteúdo original não é rasterizado nem reescrito. A assinatura é apenas
//! uma imagem; não tem validade criptográfica (isso é a Fase 3).

use std::path::{Path, PathBuf};

use crate::error::{OpError, OpResult};
use crate::pdfwriter::{ImageMark, OverlayImage, OverlayPage, ShapeMark, TextMark};
use crate::stamp;

/// Distância da linha de base ao topo da caixa do texto, em fração do corpo
/// (cobre maiúsculas acentuadas da Helvetica).
pub const ASCENT: f32 = 0.93;
/// Abaixo da linha de base (descendentes como “g” e “p”).
pub const DESCENT: f32 = 0.22;
/// Distância entre linhas, em fração do corpo.
pub const LEADING: f32 = 1.2;

pub type Rgb = (f32, f32, f32);

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Texto em Helvetica; várias linhas separadas por quebra de linha.
    Text { text: String, size: f32, bold: bool, color: Rgb },
    /// Imagem esticada na caixa (a interface mantém a proporção).
    Image { path: PathBuf, remove_white: bool },
    /// Marca-texto: retângulo colorido com mistura “multiplicar”.
    Highlight { color: Rgb },
    /// Moldura (retângulo só com contorno).
    Frame { color: Rgb, line_width: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub kind: Kind,
    /// Página (1 = primeira) em que a anotação foi posicionada.
    pub page: u32,
    /// Repetir em todas as páginas (rubrica, carimbo).
    pub all_pages: bool,
    /// Canto superior esquerdo, em fração da página exibida (0 a 1).
    pub x: f32,
    pub y: f32,
    /// Tamanho da caixa em pontos. Para texto, é calculado pela fonte
    /// (`text_box`) e estes valores são ignorados.
    pub w: f32,
    pub h: f32,
    pub opacity: f32,
}

impl Annotation {
    pub fn on_page(&self, page: u32) -> bool {
        self.all_pages || self.page == page
    }

    /// Tamanho da caixa em pontos.
    pub fn size(&self) -> (f32, f32) {
        match &self.kind {
            Kind::Text { text, size, bold, .. } => text_box(text, *size, *bold),
            _ => (self.w, self.h),
        }
    }

    /// Caixa na página `page_size`, em pontos a partir do canto superior
    /// esquerdo: (x, y, largura, altura).
    pub fn rect_on(&self, page_size: (f32, f32)) -> (f32, f32, f32, f32) {
        let (w, h) = self.size();
        (self.x.clamp(0.0, 1.0) * page_size.0, self.y.clamp(0.0, 1.0) * page_size.1, w, h)
    }
}

fn lines(text: &str) -> Vec<&str> {
    let trimmed = text.trim_end();
    if trimmed.is_empty() { Vec::new() } else { trimmed.lines().collect() }
}

/// Largura e altura, em pontos, de um texto com várias linhas.
pub fn text_box(text: &str, size: f32, bold: bool) -> (f32, f32) {
    let lines = lines(text);
    if lines.is_empty() {
        return (0.0, 0.0);
    }
    let width = lines.iter().map(|l| stamp::width(&stamp::encode(l).0, bold, size)).fold(0.0, f32::max);
    let height = (lines.len() - 1) as f32 * LEADING * size + (ASCENT + DESCENT) * size;
    (width, height)
}

/// Confere as anotações antes de executar: texto vazio, caracteres sem
/// representação nas fontes padrão e páginas inexistentes.
pub fn validate(items: &[Annotation], total: u32) -> OpResult<()> {
    if items.is_empty() {
        return Err(OpError::InvalidRange("Adicione pelo menos uma anotação.".into()));
    }
    for item in items {
        if item.page == 0 || item.page > total {
            return Err(OpError::InvalidRange(format!(
                "Uma anotação está na página {}, mas o documento tem {total}.",
                item.page
            )));
        }
        match &item.kind {
            Kind::Text { text, .. } => {
                if text.trim().is_empty() {
                    return Err(OpError::InvalidRange("Há uma anotação de texto vazia.".into()));
                }
                let (_, missing) = stamp::encode(text);
                if !missing.is_empty() {
                    return Err(OpError::InvalidRange(crate::operations::unsupported_chars(&missing)));
                }
            }
            Kind::Image { path, .. } if !path.is_file() => {
                return Err(OpError::InvalidRange(format!(
                    "A imagem “{}” não foi encontrada.",
                    crate::error::display_name(path)
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Imagens distintas das anotações (caminho + remoção de fundo), na ordem em
/// que aparecem. O índice nesta lista é o usado em `ImageMark::image`.
pub fn image_sources(items: &[Annotation]) -> Vec<(PathBuf, bool)> {
    let mut list: Vec<(PathBuf, bool)> = Vec::new();
    for item in items {
        if let Kind::Image { path, remove_white } = &item.kind {
            let key = (path.clone(), *remove_white);
            if !list.contains(&key) {
                list.push(key);
            }
        }
    }
    list
}

pub fn load_images(sources: &[(PathBuf, bool)]) -> OpResult<Vec<OverlayImage>> {
    sources.iter().map(|(path, remove_white)| OverlayImage::load(path, *remove_white)).collect()
}

fn image_index(sources: &[(PathBuf, bool)], path: &Path, remove_white: bool) -> Option<usize> {
    sources.iter().position(|(p, r)| p == path && *r == remove_white)
}

/// Camada de uma página (`page`, 1 = primeira) com tamanho exibido `size`.
pub fn page_layer(items: &[Annotation], page: u32, size: (f32, f32), sources: &[(PathBuf, bool)]) -> OverlayPage {
    let (pw, ph) = size;
    let mut layer = OverlayPage::empty(pw, ph);
    for item in items.iter().filter(|a| a.on_page(page)) {
        let (left, top, w, h) = item.rect_on(size);
        // O PDF conta de baixo para cima.
        let bottom = ph - top - h;
        let opacity = item.opacity.clamp(0.05, 1.0);
        match &item.kind {
            Kind::Text { text, size, bold, color } => {
                for (i, line) in lines(text).iter().enumerate() {
                    layer.marks.push(TextMark {
                        text: stamp::encode(line).0,
                        bold: *bold,
                        size: *size,
                        x: left,
                        y: ph - top - ASCENT * size - i as f32 * LEADING * size,
                        angle: 0.0,
                        color: *color,
                        opacity,
                    });
                }
            }
            Kind::Image { path, remove_white } => {
                if let Some(image) = image_index(sources, path, *remove_white) {
                    layer.images.push(ImageMark { image, x: left, y: bottom, w, h, opacity });
                }
            }
            Kind::Highlight { color } => layer.shapes.push(ShapeMark {
                x: left,
                y: bottom,
                w,
                h,
                fill: Some(*color),
                stroke: None,
                line_width: 0.0,
                opacity,
                multiply: true,
            }),
            Kind::Frame { color, line_width } => layer.shapes.push(ShapeMark {
                x: left,
                y: bottom,
                w,
                h,
                fill: None,
                stroke: Some(*color),
                line_width: *line_width,
                opacity,
                multiply: false,
            }),
        }
    }
    layer
}

/// Camada do documento inteiro: uma página de camada por página.
pub fn layer(items: &[Annotation], sizes: &[(f32, f32)], sources: &[(PathBuf, bool)]) -> Vec<OverlayPage> {
    sizes.iter().enumerate().map(|(i, &size)| page_layer(items, i as u32 + 1, size, sources)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: (f32, f32) = (595.0, 842.0);

    fn text(t: &str, page: u32) -> Annotation {
        Annotation {
            kind: Kind::Text { text: t.into(), size: 12.0, bold: false, color: (0.0, 0.0, 0.0) },
            page,
            all_pages: false,
            x: 0.1,
            y: 0.2,
            w: 0.0,
            h: 0.0,
            opacity: 1.0,
        }
    }

    #[test]
    fn text_lines_start_below_the_top_of_the_box() {
        let layer = page_layer(&[text("Aprovado\nem 14/10", 1)], 1, A4, &[]);
        assert_eq!(layer.marks.len(), 2);
        let (first, second) = (&layer.marks[0], &layer.marks[1]);
        assert!((first.x - 59.5).abs() < 0.01);
        assert!((first.y - (842.0 - 168.4 - ASCENT * 12.0)).abs() < 0.01);
        assert!((first.y - second.y - LEADING * 12.0).abs() < 0.01);
        let (w, h) = text_box("Aprovado\nem 14/10", 12.0, false);
        assert!((w - stamp::width(b"Aprovado", false, 12.0)).abs() < 0.01);
        assert!((h - (LEADING + ASCENT + DESCENT) * 12.0).abs() < 0.01);
    }

    #[test]
    fn items_only_appear_on_their_page_unless_repeated() {
        let mut stamp = text("Rubrica", 2);
        let single = layer(&[stamp.clone()], &[A4; 3], &[]);
        assert!(single[0].marks.is_empty() && !single[1].marks.is_empty() && single[2].marks.is_empty());
        stamp.all_pages = true;
        assert!(layer(&[stamp], &[A4; 3], &[]).iter().all(|p| p.marks.len() == 1));
    }

    #[test]
    fn boxes_are_converted_to_pdf_coordinates() {
        let sources = vec![(PathBuf::from("/x/ass.png"), true)];
        let items = vec![
            Annotation {
                kind: Kind::Image { path: "/x/ass.png".into(), remove_white: true },
                page: 1,
                all_pages: false,
                x: 0.5,
                y: 0.5,
                w: 100.0,
                h: 40.0,
                opacity: 1.0,
            },
            Annotation {
                kind: Kind::Highlight { color: (1.0, 0.9, 0.0) },
                page: 1,
                all_pages: false,
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 20.0,
                opacity: 0.5,
            },
        ];
        let page = page_layer(&items, 1, A4, &sources);
        let img = page.images[0];
        assert_eq!((img.image, img.x, img.y, img.w, img.h), (0, 297.5, 421.0 - 40.0, 100.0, 40.0));
        let hl = page.shapes[0];
        assert_eq!((hl.x, hl.y, hl.h), (0.0, 822.0, 20.0));
        assert!(hl.multiply && hl.fill.is_some());
    }

    #[test]
    fn same_image_is_loaded_once() {
        let mut a = text("", 1);
        a.kind = Kind::Image { path: "/a.png".into(), remove_white: false };
        let mut b = a.clone();
        b.page = 2;
        let mut c = a.clone();
        c.kind = Kind::Image { path: "/a.png".into(), remove_white: true };
        assert_eq!(image_sources(&[a, b, c]).len(), 2);
    }

    #[test]
    fn validation_explains_problems() {
        assert!(validate(&[], 3).is_err());
        assert!(validate(&[text("ok", 4)], 3).is_err());
        assert!(validate(&[text("  ", 1)], 3).is_err());
        let e = validate(&[text("✓ feito", 1)], 3).unwrap_err();
        assert!(e.to_string().contains('✓') || format!("{e:?}").contains('✓'));
        assert!(validate(&[text("Pago — João", 1)], 3).is_ok());
    }
}
