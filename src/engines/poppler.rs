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
    let p = page.to_string();
    let mut args: Vec<OsString> = vec!["-f".into(), p.clone().into(), "-l".into(), p.into()];
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
    let produced = out_base.with_extension(format.extension());
    if !produced.is_file() {
        return Err(OpError::OutputInvalid(format!("o pdftoppm não gerou a página {page}")));
    }
    Ok(produced)
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
    let prefix = dir.join(format!("t{first}"));
    let args: Vec<OsString> = vec![
        "-f".into(),
        first.to_string().into(),
        "-l".into(),
        last.to_string().into(),
        "-scale-to".into(),
        size.to_string().into(),
        "-png".into(),
        pdf.into(),
        prefix.clone().into(),
    ];
    run_ok(Tool::Pdftoppm, args, &[0], cancel)?;
    // O pdftoppm numera como prefixo-N.png com zeros à esquerda conforme o
    // total de páginas do documento; aceitamos qualquer largura de padding.
    let stem = prefix.file_name().unwrap().to_string_lossy().into_owned();
    let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| OpError::io("ler miniaturas", e))?
        .filter_map(|e| e.ok())
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
    let p = page.to_string();
    let args: Vec<OsString> = vec!["-f".into(), p.clone().into(), "-l".into(), p.into(), pdf.into()];
    let out = run_ok(Tool::Pdfinfo, args, &[0], cancel)?;
    parse_page_size(&String::from_utf8_lossy(&out.stdout), page)
        .ok_or_else(|| OpError::OutputInvalid(format!("o pdfinfo não informou o tamanho da página {page}")))
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
    use super::parse_page_size;

    #[test]
    fn reads_size_and_rotation() {
        let text = "Pages:           3\nPage    2 size: 595.276 x 841.89 pts (A4)\nPage    2 rot:  90\n";
        assert_eq!(parse_page_size(text, 2), Some((841.89, 595.276)));
        assert_eq!(parse_page_size("Page    1 size: 612 x 792 pts (letter)\n", 1), Some((612.0, 792.0)));
        assert_eq!(parse_page_size("nada", 1), None);
    }
}
