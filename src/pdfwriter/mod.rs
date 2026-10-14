//! Gerador mínimo de PDF: imagens → PDF, camadas sobrepostas (carimbos e
//! anotações), a camada de texto invisível do OCR e fixtures de teste.
//!
//! - `images`: imagens → PDF; JPEGs entram sem recompressão (`DCTDecode`).
//! - `overlay`: camadas transparentes aplicadas com `qpdf --overlay`.
//! - `invisible`: texto invisível e pesquisável sobre páginas digitalizadas.

mod images;
mod invisible;
mod overlay;

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::error::OpError;

pub use images::{ImageLayout, PageSize, Placement, decode_rgba, display_dimensions, images_to_pdf, place};
pub use invisible::{InvisiblePage, InvisibleWord, invisible_text_pdf};
pub use overlay::{ImageMark, OverlayImage, OverlayPage, ShapeMark, TextMark, overlay_pdf, overlay_pdf_with_images};

/// Pontos por milímetro.
pub const MM: f32 = 72.0 / 25.4;

pub const A4_POINTS: (f32, f32) = (595.276, 841.89);

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

    /// Grava o fluxo de conteúdo e o objeto Page que o usa. `resources` é o
    /// miolo de `/Resources << … >>`; `content_dict`, o do fluxo (filtro).
    /// Devolve o número do objeto Page.
    fn page(
        &mut self,
        tree: u32,
        (width, height): (f32, f32),
        resources: &str,
        content_dict: &str,
        content: &[u8],
    ) -> io::Result<u32> {
        let content_id = self.alloc();
        self.stream(content_id, content_dict, content)?;
        let page_id = self.alloc();
        self.obj(
            page_id,
            &format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] \
                 /Resources << {resources} >> /Contents {content_id} 0 R >>",
                fmt_num(width),
                fmt_num(height)
            ),
        )?;
        Ok(page_id)
    }

    /// Catálogo, árvore de páginas, Info e tabela de referências.
    /// `pages` são os ids dos objetos Page, na ordem.
    fn finish(mut self, catalog: u32, tree: u32, pages: &[u32]) -> io::Result<()> {
        let kids: Vec<String> = pages.iter().map(|p| format!("{p} 0 R")).collect();
        self.obj(tree, &format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()))?;
        self.obj(catalog, &format!("<< /Type /Catalog /Pages {tree} 0 R >>"))?;
        let info = self.alloc();
        self.obj(info, "<< /Producer (PDFToolbox) >>")?;

        let xref = self.pos;
        let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", self.offsets.len());
        for off in &self.offsets[1..] {
            table.push_str(&format!("{off:010} 00000 n \n"));
        }
        table.push_str(&format!(
            "trailer\n<< /Size {} /Root {catalog} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            self.offsets.len()
        ));
        self.raw(table.as_bytes())?;
        self.out.flush()?;
        self.out.get_ref().sync_all()
    }
}

fn deflate(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::with_capacity(data.len() / 2), Compression::default());
    enc.write_all(data)?;
    enc.finish()
}

fn write_err(e: io::Error) -> OpError {
    OpError::io(tr!("gravar o PDF temporário"), e)
}

/// Número compacto para o conteúdo do PDF: “1”, “0.5”, nunca “-0”.
fn fmt_num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
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
        ids.push(w.page(tree, size, &format!("/Font << /F1 {font} 0 R >>"), "", content.as_bytes())?);
    }
    w.finish(catalog, tree, &ids)
}

#[cfg(test)]
mod tests {
    use super::fmt_num;

    #[test]
    fn numbers_are_compact() {
        assert_eq!(fmt_num(1.0), "1");
        assert_eq!(fmt_num(0.5), "0.5");
        assert_eq!(fmt_num(-0.0), "0");
    }
}
