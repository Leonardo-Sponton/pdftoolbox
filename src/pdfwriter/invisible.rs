//! Camada de texto invisível do OCR: selecionável e pesquisável, sem
//! desenhar nada por cima da página digitalizada.

use std::io;
use std::path::Path;

use super::{Writer, deflate, fmt_num};

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

    fn content(&self) -> String {
        let mut content = match self.upright_matrix() {
            Some(m) => format!("q {m} cm\nBT 3 Tr\n"),
            None => String::from("q\nBT 3 Tr\n"),
        };
        for word in &self.words {
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
        content
    }
}

/// Largura de cada caractere da fonte sem desenho, em milésimos do corpo.
const GLYPHLESS_WIDTH: f32 = 500.0;

/// Texto em UTF-16BE hexadecimal (os CIDs da fonte são as unidades UTF-16).
fn utf16_hex(text: &str) -> (String, usize) {
    let units: Vec<u16> = text.encode_utf16().collect();
    (units.iter().map(|u| format!("{u:04X}")).collect(), units.len())
}

/// ToUnicode identidade: cada CID é a própria unidade UTF-16.
fn identity_cmap() -> String {
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
    cmap
}

/// Grava a fonte sem desenho do Tesseract como fonte composta (Type0) e
/// devolve o número do objeto da fonte.
fn write_glyphless_font(w: &mut Writer, font_program: &[u8]) -> io::Result<u32> {
    let font_file = w.alloc();
    w.stream(font_file, &format!("/Length1 {}", font_program.len()), font_program)?;
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
    let to_unicode = w.alloc();
    w.stream(to_unicode, "/Filter /FlateDecode", &deflate(identity_cmap().as_bytes())?)?;
    let font = w.alloc();
    w.obj(
        font,
        &format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /GlyphLessFont /Encoding /Identity-H \
             /DescendantFonts [{cid_font} 0 R] /ToUnicode {to_unicode} 0 R >>"
        ),
    )?;
    Ok(font)
}

/// PDF de camada com texto invisível (modo de renderização 3). Usa a fonte
/// sem desenho do Tesseract (`glyphless_font`), com CID = unidade UTF-16 e
/// ToUnicode identidade, de modo que qualquer caractere reconhecido é
/// preservado. Cada palavra é esticada (`Tz`) para cobrir a largura dela na
/// imagem.
pub fn invisible_text_pdf(out: &Path, pages: &[InvisiblePage], glyphless_font: &[u8]) -> io::Result<()> {
    let mut w = Writer::create(out)?;
    let catalog = w.alloc();
    let tree = w.alloc();
    let font = write_glyphless_font(&mut w, glyphless_font)?;
    let resources = format!("/Font << /F1 {font} 0 R >>");
    let mut ids = Vec::new();
    for page in pages {
        let content = deflate(page.content().as_bytes())?;
        ids.push(w.page(tree, (page.width, page.height), &resources, "/Filter /FlateDecode", &content)?);
    }
    w.finish(catalog, tree, &ids)
}
