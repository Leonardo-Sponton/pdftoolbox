//! Carimbos de texto sobre as páginas: numeração e marca d’água.
//!
//! Gera a camada (`pdfwriter::overlay_pdf`) que o qpdf sobrepõe a cada página
//! (`--overlay`), sem rasterizar o documento. O texto usa as fontes padrão
//! Helvetica do PDF (sem embutir fonte), codificado em WinAnsi: cobre
//! português e as demais línguas da Europa Ocidental. Caracteres fora disso
//! viram “?” e são informados à interface antes de executar.

use crate::pdfwriter::{MM, OverlayPage, TextMark};

/// Altura das maiúsculas da Helvetica, em fração do corpo.
const CAP_HEIGHT: f32 = 0.718;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberFormat {
    /// “3”
    Number,
    /// “3 / 10”
    NumberOfTotal,
    /// “Página 3 de 10”
    PageOfTotal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NumberStyle {
    pub position: Position,
    pub format: NumberFormat,
    /// Número mostrado na primeira página numerada.
    pub first_number: u32,
    pub size: f32,
    pub margin_mm: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatermarkStyle {
    pub text: String,
    /// Fração da largura (ou da diagonal) ocupada pelo texto, de 0,1 a 1.
    pub scale: f32,
    pub opacity: f32,
    pub diagonal: bool,
    pub color: (f32, f32, f32),
}

/// Texto da numeração de uma página.
pub fn number_text(format: NumberFormat, n: u32, last: u32) -> String {
    match format {
        NumberFormat::Number => n.to_string(),
        NumberFormat::NumberOfTotal => format!("{n} / {last}"),
        NumberFormat::PageOfTotal => format!("Página {n} de {last}"),
    }
}

/// Bytes 0x80–0x9F da WinAnsiEncoding (os demais coincidem com o Latin-1).
const WIN_ANSI_HIGH: [(u8, char); 27] = [
    (0x80, '€'),
    (0x82, '‚'),
    (0x83, 'ƒ'),
    (0x84, '„'),
    (0x85, '…'),
    (0x86, '†'),
    (0x87, '‡'),
    (0x88, 'ˆ'),
    (0x89, '‰'),
    (0x8A, 'Š'),
    (0x8B, '‹'),
    (0x8C, 'Œ'),
    (0x8E, 'Ž'),
    (0x91, '‘'),
    (0x92, '’'),
    (0x93, '“'),
    (0x94, '”'),
    (0x95, '•'),
    (0x96, '–'),
    (0x97, '—'),
    (0x98, '˜'),
    (0x99, '™'),
    (0x9A, 'š'),
    (0x9B, '›'),
    (0x9C, 'œ'),
    (0x9E, 'ž'),
    (0x9F, 'Ÿ'),
];

/// Codifica em WinAnsi. Devolve os bytes e os caracteres sem representação
/// (substituídos por “?”), sem repetição.
pub fn encode(text: &str) -> (Vec<u8>, Vec<char>) {
    let mut bytes = Vec::with_capacity(text.len());
    let mut missing = Vec::new();
    for c in text.chars() {
        let code = c as u32;
        let byte = match c {
            '\t' | '\n' | '\r' => Some(b' '),
            _ if (0x20..=0x7E).contains(&code) || (0xA0..=0xFF).contains(&code) => Some(code as u8),
            _ => WIN_ANSI_HIGH.iter().find(|(_, ch)| *ch == c).map(|(b, _)| *b),
        };
        if let Some(b) = byte {
            bytes.push(b)
        } else {
            bytes.push(b'?');
            if !missing.contains(&c) {
                missing.push(c);
            }
        }
    }
    (bytes, missing)
}

/// Largura do texto codificado, em pontos.
pub fn width(bytes: &[u8], bold: bool, size: f32) -> f32 {
    let table = if bold { &HELVETICA_BOLD } else { &HELVETICA };
    let units: u32 = bytes.iter().map(|&b| if b >= 32 { table[(b - 32) as usize] as u32 } else { 0 }).sum();
    units as f32 * size / 1000.0
}

/// Camada de uma única página com o número `n` (de `last`).
pub fn number_page(size: (f32, f32), n: u32, last: u32, style: &NumberStyle) -> OverlayPage {
    let (w, h) = size;
    let margin = style.margin_mm * MM;
    let (text, _) = encode(&number_text(style.format, n, last));
    let tw = width(&text, false, style.size);
    let x = match style.position {
        Position::TopLeft | Position::BottomLeft => margin,
        Position::TopCenter | Position::BottomCenter => (w - tw) / 2.0,
        Position::TopRight | Position::BottomRight => w - margin - tw,
    };
    let y = match style.position {
        Position::TopLeft | Position::TopCenter | Position::TopRight => h - margin - CAP_HEIGHT * style.size,
        _ => margin,
    };
    let mark = TextMark { text, bold: false, size: style.size, x, y, angle: 0.0, color: (0.0, 0.0, 0.0), opacity: 1.0 };
    OverlayPage { marks: vec![mark], ..OverlayPage::empty(w, h) }
}

/// Camada de numeração. `pages` são as páginas (1 = primeira) que recebem
/// número, em ordem; a primeira delas mostra `first_number`.
pub fn number_pages(sizes: &[(f32, f32)], pages: &[u32], style: &NumberStyle) -> Vec<OverlayPage> {
    let last = style.first_number + pages.len().saturating_sub(1) as u32;
    sizes
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| match pages.iter().position(|&p| p as usize == i + 1) {
            Some(k) => number_page((w, h), style.first_number + k as u32, last, style),
            None => OverlayPage::empty(w, h),
        })
        .collect()
}

/// Marca d’água centralizada, horizontal ou na diagonal (do canto inferior
/// esquerdo ao superior direito), nas páginas indicadas.
pub fn watermark(sizes: &[(f32, f32)], pages: &[u32], style: &WatermarkStyle) -> Vec<OverlayPage> {
    let (text, _) = encode(style.text.trim());
    sizes
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| {
            let mut marks = Vec::new();
            if !text.is_empty() && pages.contains(&(i as u32 + 1)) {
                let angle = if style.diagonal { h.atan2(w).to_degrees() } else { 0.0 };
                let span = if style.diagonal { w.hypot(h) } else { w };
                let unit = width(&text, true, 1.0).max(0.01);
                // Texto curto não deve passar da altura da página.
                let size = (style.scale.clamp(0.1, 1.0) * span / unit).min(w.min(h) * 0.5);
                let (tw, ch) = (unit * size, CAP_HEIGHT * size);
                let (sin, cos) = angle.to_radians().sin_cos();
                let (cx, cy) = (w / 2.0, h / 2.0);
                marks.push(TextMark {
                    text: text.clone(),
                    bold: true,
                    size,
                    x: cx - (cos * tw / 2.0 - sin * ch / 2.0),
                    y: cy - (sin * tw / 2.0 + cos * ch / 2.0),
                    angle,
                    color: style.color,
                    opacity: style.opacity.clamp(0.05, 1.0),
                });
            }
            OverlayPage { marks, ..OverlayPage::empty(w, h) }
        })
        .collect()
}

// Larguras (1/1000 do corpo) dos códigos 32–255 em WinAnsi, das fontes
// Nimbus Sans, métricas idênticas às da Helvetica.
const HELVETICA: [u16; 224] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556,
    556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334,
    260, 334, 584, 0, 556, 0, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, 0, 222, 222, 333,
    333, 350, 556, 1000, 333, 1000, 500, 333, 944, 0, 500, 667, 278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370,
    556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611, 667, 667,
    667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778,
    722, 722, 722, 722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];
const HELVETICA_BOLD: [u16; 224] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611,
    556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389,
    280, 389, 584, 0, 556, 0, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, 0, 278, 278, 500,
    500, 350, 556, 1000, 333, 1000, 556, 333, 944, 0, 500, 667, 278, 333, 556, 556, 556, 556, 280, 556, 333, 737, 370,
    556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 611, 556, 278, 333, 333, 365, 556, 834, 834, 834, 611, 722, 722,
    722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778,
    722, 722, 722, 722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278, 278,
    611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556,
];

#[cfg(test)]
mod tests {
    use super::*;

    const A4: (f32, f32) = (595.0, 842.0);

    #[test]
    fn encodes_portuguese_and_reports_the_rest() {
        let (bytes, missing) = encode("Página “ação” €");
        assert_eq!(bytes, b"P\xE1gina \x93a\xE7\xE3o\x94 \x80");
        assert!(missing.is_empty());
        let (bytes, missing) = encode("a漢b漢✓");
        assert_eq!(bytes, b"a?b??");
        assert_eq!(missing, vec!['漢', '✓']);
    }

    #[test]
    fn widths_follow_helvetica_metrics() {
        assert!((width(b"A", false, 10.0) - 6.67).abs() < 0.01);
        assert!((width(b"A", true, 10.0) - 7.22).abs() < 0.01);
        assert_eq!(width(b"", false, 10.0), 0.0);
    }

    #[test]
    fn numbers_only_chosen_pages_counting_from_first_number() {
        let style = NumberStyle {
            position: Position::BottomRight,
            format: NumberFormat::PageOfTotal,
            first_number: 1,
            size: 10.0,
            margin_mm: 10.0,
        };
        let layer = number_pages(&[A4; 4], &[2, 3, 4], &style);
        assert_eq!(layer.len(), 4);
        assert!(layer[0].marks.is_empty());
        assert_eq!(layer[1].marks[0].text, b"P\xE1gina 1 de 3");
        assert_eq!(layer[3].marks[0].text, b"P\xE1gina 3 de 3");
        let m = &layer[3].marks[0];
        let right_edge = m.x + width(&m.text, false, 10.0);
        assert!((right_edge - (595.0 - 10.0 * MM)).abs() < 0.01);
        assert!((m.y - 10.0 * MM).abs() < 0.01);
    }

    #[test]
    fn top_center_numbers_sit_inside_the_page() {
        let style = NumberStyle {
            position: Position::TopCenter,
            format: NumberFormat::NumberOfTotal,
            first_number: 5,
            size: 12.0,
            margin_mm: 8.0,
        };
        let layer = number_pages(&[(842.0, 595.0)], &[1], &style);
        let m = &layer[0].marks[0];
        assert_eq!(m.text, b"5 / 5");
        let tw = width(&m.text, false, 12.0);
        assert!((m.x + tw / 2.0 - 421.0).abs() < 0.01);
        assert!(m.y + CAP_HEIGHT * 12.0 <= 595.0 - 8.0 * MM + 0.01);
    }

    #[test]
    fn diagonal_watermark_is_centered_and_follows_the_diagonal() {
        let style = WatermarkStyle {
            text: "CONFIDENCIAL".into(),
            scale: 0.6,
            opacity: 0.3,
            diagonal: true,
            color: (0.5, 0.5, 0.5),
        };
        let layer = watermark(&[A4, A4], &[2], &style);
        assert!(layer[0].marks.is_empty());
        let m = &layer[1].marks[0];
        assert!((m.angle - 842f32.atan2(595.0).to_degrees()).abs() < 0.01);
        // O centro do texto (meia largura, meia altura de maiúscula) cai no centro da página.
        let (sin, cos) = m.angle.to_radians().sin_cos();
        let (tw, ch) = (width(&m.text, true, m.size), CAP_HEIGHT * m.size);
        let cx = m.x + cos * tw / 2.0 - sin * ch / 2.0;
        let cy = m.y + sin * tw / 2.0 + cos * ch / 2.0;
        assert!((cx - 297.5).abs() < 0.01 && (cy - 421.0).abs() < 0.01);
        assert!((tw - 0.6 * 595f32.hypot(842.0)).abs() < 0.5);
    }

    #[test]
    fn short_watermark_does_not_exceed_the_page() {
        let style =
            WatermarkStyle { text: "X".into(), scale: 1.0, opacity: 0.5, diagonal: false, color: (0.0, 0.0, 0.0) };
        let layer = watermark(&[A4], &[1], &style);
        assert!(layer[0].marks[0].size <= 595.0 * 0.5);
        let empty = WatermarkStyle { text: "   ".into(), ..style };
        assert!(watermark(&[A4], &[1], &empty)[0].marks.is_empty());
    }
}
