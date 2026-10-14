//! Renderização com Poppler (`pdftoppm`) e leitura de tamanhos (`pdfinfo`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{Tool, run_ok};
use crate::error::{OpError, OpResult};
use crate::jobs::CancelToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Tiff,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Tiff => "tif",
        }
    }
}

/// `-f first -l last`: o intervalo de páginas, como pdftoppm, pdfinfo e
/// pdftotext esperam.
fn page_range(first: u32, last: u32) -> [OsString; 4] {
    ["-f".into(), first.to_string().into(), "-l".into(), last.to_string().into()]
}

/// O arquivo que o pdftoppm deveria ter criado para `page`.
fn produced(path: PathBuf, page: u32) -> OpResult<PathBuf> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(OpError::OutputInvalid(format!("o pdftoppm não gerou a página {page}")))
    }
}

/// Renderiza uma única página para `out_base` + extensão (o pdftoppm acrescenta
/// a extensão). Devolve o caminho criado.
pub fn render_page(
    pdf: &Path,
    page: u32,
    format: ImageFormat,
    dpi: u32,
    jpeg_quality: u8,
    out_base: &Path,
    cancel: &CancelToken,
) -> OpResult<PathBuf> {
    let mut args: Vec<OsString> = page_range(page, page).into();
    args.extend(["-r".into(), dpi.to_string().into(), "-singlefile".into()]);
    match format {
        ImageFormat::Png => args.push("-png".into()),
        ImageFormat::Jpeg => {
            args.push("-jpeg".into());
            args.push("-jpegopt".into());
            args.push(format!("quality={jpeg_quality}").into());
        }
        ImageFormat::Tiff => {
            args.push("-tiff".into());
            args.push("-tiffcompression".into());
            args.push("lzw".into());
        }
    }
    args.push(pdf.into());
    args.push(out_base.into());
    run_ok(Tool::Pdftoppm, args, &[0], cancel)?;
    produced(out_base.with_extension(format.extension()), page)
}

/// Renderiza uma página em tons de cinza, sem compressão (PGM), para o OCR:
/// bem mais rápido que PNG em digitalizações com ruído. Devolve o caminho.
pub fn render_gray(pdf: &Path, page: u32, dpi: u32, out_base: &Path, cancel: &CancelToken) -> OpResult<PathBuf> {
    let mut args: Vec<OsString> = page_range(page, page).into();
    args.extend(["-r".into(), dpi.to_string().into(), "-gray".into(), "-singlefile".into(), pdf.into()]);
    args.push(out_base.into());
    run_ok(Tool::Pdftoppm, args, &[0], cancel)?;
    produced(out_base.with_extension("pgm"), page)
}

/// Gera miniaturas PNG (lado maior = `size` px) das páginas `first..=last`
/// em `dir`. Devolve os arquivos na ordem das páginas.
pub fn thumbnails(
    pdf: &Path,
    first: u32,
    last: u32,
    size: u32,
    dir: &Path,
    cancel: &CancelToken,
) -> OpResult<Vec<PathBuf>> {
    let stem = format!("t{first}");
    let mut args: Vec<OsString> = page_range(first, last).into();
    args.extend(["-scale-to".into(), size.to_string().into(), "-png".into(), pdf.into(), dir.join(&stem).into()]);
    run_ok(Tool::Pdftoppm, args, &[0], cancel)?;
    // O pdftoppm numera como prefixo-N.png com zeros à esquerda conforme o
    // total de páginas do documento; aceitamos qualquer largura de padding.
    let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| OpError::io("ler miniaturas", e))?
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix(&stem)?.strip_prefix('-')?.strip_suffix(".png")?.parse().ok()?;
            Some((n, e.path()))
        })
        .filter(|(n, _)| (first..=last).contains(n))
        .collect();
    found.sort();
    Ok(found.into_iter().map(|(_, p)| p).collect())
}

/// Tamanho exibido (pontos, rotação aplicada) de uma página, via `pdfinfo`.
pub fn page_size(pdf: &Path, page: u32, cancel: &CancelToken) -> OpResult<(f32, f32)> {
    let mut args: Vec<OsString> = page_range(page, page).into();
    args.push(pdf.into());
    let out = run_ok(Tool::Pdfinfo, args, &[0], cancel)?;
    parse_page_size(&String::from_utf8_lossy(&out.stdout), page)
        .ok_or_else(|| OpError::OutputInvalid(format!("o pdfinfo não informou o tamanho da página {page}")))
}

/// Tamanho exibido de todas as páginas, em uma única chamada ao `pdfinfo`.
pub fn page_sizes(pdf: &Path, total: u32, cancel: &CancelToken) -> OpResult<Vec<(f32, f32)>> {
    let mut args: Vec<OsString> = page_range(1, total).into();
    args.push(pdf.into());
    let out = run_ok(Tool::Pdfinfo, args, &[0], cancel)?;
    let text = String::from_utf8_lossy(&out.stdout);
    (1..=total)
        .map(|page| {
            parse_page_size(&text, page)
                .ok_or_else(|| OpError::OutputInvalid(format!("o pdfinfo não informou o tamanho da página {page}")))
        })
        .collect()
}

/// Extrai o texto para `out` (UTF-8). `layout` mantém colunas e espaçamento
/// aproximados da página; sem ele, o texto segue a ordem de leitura.
pub fn extract_text(pdf: &Path, out: &Path, layout: bool, cancel: &CancelToken) -> OpResult<()> {
    let mut args: Vec<OsString> = vec!["-enc".into(), "UTF-8".into()];
    if layout {
        args.push("-layout".into());
    }
    args.push(pdf.into());
    args.push(out.into());
    run_ok(Tool::Pdftotext, args, &[0], cancel)?;
    Ok(())
}

/// Texto de uma página (para pré-visualizar), via saída padrão.
pub fn page_text(pdf: &Path, page: u32, layout: bool, cancel: &CancelToken) -> OpResult<String> {
    let mut args: Vec<OsString> = vec!["-enc".into(), "UTF-8".into()];
    args.extend(page_range(page, page));
    if layout {
        args.push("-layout".into());
    }
    args.push(pdf.into());
    args.push("-".into());
    let out = run_ok(Tool::Pdftotext, args, &[0], cancel)?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Texto de cada página, em ordem (o `pdftotext` separa páginas com form feed).
pub fn texts_per_page(pdf: &Path, total: u32, cancel: &CancelToken) -> OpResult<Vec<String>> {
    let args: Vec<OsString> = vec!["-enc".into(), "UTF-8".into(), pdf.into(), "-".into()];
    let out = run_ok(Tool::Pdftotext, args, &[0], cancel)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pages: Vec<String> = text.split('\x0C').take(total as usize).map(str::to_string).collect();
    pages.resize(total as usize, String::new());
    Ok(pages)
}

/// Metadados do dicionário de informações do documento.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocInfo {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
    /// Datas em ISO 8601, como o `pdfinfo -isodates` informa.
    pub created: String,
    pub modified: String,
}

pub fn doc_info(pdf: &Path, cancel: &CancelToken) -> OpResult<DocInfo> {
    let args: Vec<OsString> = vec!["-isodates".into(), "-enc".into(), "UTF-8".into(), pdf.into()];
    let out = run_ok(Tool::Pdfinfo, args, &[0], cancel)?;
    Ok(parse_doc_info(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_doc_info(text: &str) -> DocInfo {
    let mut info = DocInfo::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().to_string();
        match key {
            "Title" => info.title = value,
            "Author" => info.author = value,
            "Subject" => info.subject = value,
            "Keywords" => info.keywords = value,
            "Creator" => info.creator = value,
            "Producer" => info.producer = value,
            "CreationDate" => info.created = value,
            "ModDate" => info.modified = value,
            _ => {}
        }
    }
    info
}

fn parse_page_size(text: &str, page: u32) -> Option<(f32, f32)> {
    let prefix = format!("Page {page:>4} ");
    let mut size = None;
    let mut rot = 0;
    for line in text.lines().filter_map(|l| l.strip_prefix(&prefix)) {
        if let Some(rest) = line.strip_prefix("size:") {
            let mut nums = rest.split_whitespace().filter_map(|t| t.parse::<f32>().ok());
            size = Some((nums.next()?, nums.next()?));
        } else if let Some(rest) = line.strip_prefix("rot:") {
            rot = rest.trim().parse::<i32>().unwrap_or(0);
        }
    }
    let (w, h) = size?;
    Some(if rot.rem_euclid(180) == 90 { (h, w) } else { (w, h) })
}

#[cfg(test)]
mod tests {
    use super::{DocInfo, parse_doc_info, parse_page_size};

    #[test]
    fn reads_document_info() {
        let text = "Title:           Relatório: 2026\nAuthor:          Ana\nCreationDate:    2026-10-14T09:00:00-03\nPages:           3\n";
        let info = parse_doc_info(text);
        assert_eq!(info.title, "Relatório: 2026");
        assert_eq!(info.author, "Ana");
        assert_eq!(info.created, "2026-10-14T09:00:00-03");
        assert_eq!(parse_doc_info("Pages: 1\n"), DocInfo::default());
    }

    #[test]
    fn reads_size_and_rotation() {
        let text = "Pages:           3\nPage    2 size: 595.276 x 841.89 pts (A4)\nPage    2 rot:  90\n";
        assert_eq!(parse_page_size(text, 2), Some((841.89, 595.276)));
        assert_eq!(parse_page_size("Page    1 size: 612 x 792 pts (letter)\n", 1), Some((612.0, 792.0)));
        assert_eq!(parse_page_size("nada", 1), None);
    }
}
