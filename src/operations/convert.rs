//! Imagens → PDF (gerador próprio) e PDF → imagens (Poppler).

use std::path::{Path, PathBuf};

use super::{Destination, Outcome, batch, batch_output_name, output_name, parallel_map, verify_pdf};
use crate::engines::poppler::{self, ImageFormat};
use crate::engines::qpdf;
use crate::error::{OpError, OpResult};
use crate::jobs::JobCtx;
use crate::pdfwriter::{self, ImageLayout};
use crate::storage::{self, JobDir, stem};

fn write_images(
    images: &[PathBuf],
    layout: ImageLayout,
    tmp: &Path,
    ctx: &JobCtx,
    base: f64,
    span: f64,
) -> OpResult<()> {
    let n = images.len();
    pdfwriter::images_to_pdf(images, layout, tmp, &ctx.cancel, |done| {
        ctx.report(base + span * done as f64 / n as f64, &tr!("Imagem {done} de {n}", done, n));
    })?;
    qpdf::check(tmp, &ctx.cancel)?;
    verify_pdf(tmp, n as u32, ctx)
}

pub fn images_to_pdf(
    inputs: &[PathBuf],
    layout: ImageLayout,
    one_file: bool,
    dest: &Destination,
    ctx: &JobCtx,
) -> OpResult<Outcome> {
    if inputs.is_empty() {
        return Err(OpError::NoInput);
    }
    if one_file {
        let job = JobDir::new()?;
        let tmp = job.file("imagens.pdf");
        write_images(inputs, layout, &tmp, ctx, 0.0, 0.95)?;
        let mut outcome = Outcome::default();
        let suggested = if inputs.len() == 1 { stem(&inputs[0]) } else { "Imagens".to_string() };
        outcome.push(storage::commit(&tmp, &dest.dir, &output_name(dest, &suggested, "pdf"))?);
        return Ok(outcome);
    }
    let n = inputs.len() as f64;
    batch(inputs, |i, input, outcome| {
        let job = JobDir::new()?;
        let tmp = job.file("imagem.pdf");
        write_images(std::slice::from_ref(&input.to_path_buf()), layout, &tmp, ctx, i as f64 / n, 1.0 / n)?;
        let name = storage::sanitize_file_name(&stem(input), "pdf");
        outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
        Ok(())
    })
}

/// Até quantos `pdftoppm` rodam ao mesmo tempo (cada um usa um núcleo).
const MAX_RENDERERS: usize = 4;

pub fn pdf_to_images(
    inputs: &[PathBuf],
    format: ImageFormat,
    dpi: u32,
    quality: u8,
    dest: &Destination,
    ctx: &JobCtx,
) -> OpResult<Outcome> {
    let dpi = dpi.clamp(36, 1200);
    let n = inputs.len() as f64;
    let workers = std::thread::available_parallelism().map_or(1, |c| (c.get() / 2).max(1)).min(MAX_RENDERERS);
    batch(inputs, |i, input, outcome| {
        let total = qpdf::page_count(input, &ctx.cancel)?;
        let job = JobDir::new()?;
        let digits = total.to_string().len();
        let pages: Vec<u32> = (1..=total).collect();
        let rendered = parallel_map(
            &pages,
            workers,
            &ctx.cancel,
            |&page| {
                let base = job.file(&format!("p{page:0digits$}"));
                poppler::render_page(input, page, format, dpi, quality, &base, &ctx.cancel)
            },
            |done| {
                let fraction = (i as f64 + done as f64 / total as f64) / n;
                ctx.report(
                    fraction,
                    &trn!(
                        total as usize,
                        "{file}: {done} de {total} página",
                        "{file}: {done} de {total} páginas",
                        file = stem(input),
                        done,
                        total
                    ),
                );
            },
        )?;
        let dir = if total > 1 {
            let folder = storage::create_unique_dir(
                &dest.dir,
                &format!("{} ({})", stem(input), format.extension().to_uppercase()),
            )?;
            if outcome.folder.is_none() {
                outcome.folder = Some(folder.clone());
            }
            folder
        } else {
            dest.dir.clone()
        };
        for (page, file) in pages.into_iter().zip(rendered) {
            let name = format!("{} - página {page:0digits$}.{}", stem(input), format.extension());
            outcome.push(storage::commit(&file, &dir, &storage::sanitize_file_name(&name, format.extension()))?);
        }
        Ok(())
    })
}

/// Extrai o texto selecionável de cada PDF. Com `ocr`, as páginas sem texto
/// (digitalizadas) são reconhecidas antes, numa cópia temporária com a
/// camada de texto — só o .txt é gravado.
pub fn extract_text(
    inputs: &[PathBuf],
    layout: bool,
    ocr: Option<&super::OcrOptions>,
    dest: &Destination,
    ctx: &JobCtx,
) -> OpResult<Outcome> {
    let n = inputs.len();
    // Confere idiomas e fonte uma vez, antes do lote.
    let engine = ocr.map(|o| super::ocr::engine(o, ctx)).transpose()?;
    let mut used_ocr = false;
    let mut outcome = batch(inputs, |i, input, outcome| {
        ctx.report(i as f64 / n as f64, &tr!("Extraindo texto de {file}", file = stem(input)));
        qpdf::page_count(input, &ctx.cancel)?;
        let job = JobDir::new()?;
        let mut source = input.to_path_buf();
        if let (Some(opts), Some(engine)) = (ocr, &engine) {
            let opts = super::OcrOptions { skip_text_pages: true, straighten: false, ..opts.clone() };
            if let Some(done) = super::ocr::searchable(input, &opts, engine, &job, ctx, (i, n))? {
                outcome.notes.push(super::ocr::summary(input, &done, true));
                source = done.pdf;
                used_ocr = true;
            }
        }
        let tmp = job.file("texto.txt");
        poppler::extract_text(&source, &tmp, layout, &ctx.cancel)?;
        let text = std::fs::read(&tmp).map_err(|e| OpError::io(tr!("ler o texto extraído"), e))?;
        // pdftotext separa páginas com form feed; sem nada além disso, não há texto.
        if text.iter().all(|b| b.is_ascii_whitespace() || *b == 0x0C) {
            outcome.notes.push(tr!(
                "“{file}” não tem texto selecionável — provavelmente é uma digitalização (imagem). \
                 Nenhum arquivo foi criado; ligue “Reconhecer texto das páginas digitalizadas (OCR)”.",
                file = crate::human::file_name(input)
            ));
            return Ok(());
        }
        let name = batch_output_name(dest, n, &stem(input), "txt");
        outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
        Ok(())
    })?;
    if used_ocr && !outcome.outputs.is_empty() {
        outcome.notes.push(super::ocr::caveat().into());
    }
    Ok(outcome)
}

/// Extensões aceitas pela conversão de documentos de escritório.
pub const OFFICE_EXTENSIONS: &[&str] =
    &["doc", "docx", "odt", "rtf", "txt", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp"];

/// Converte documentos, planilhas e apresentações em PDF com o LibreOffice.
pub fn office_to_pdf(inputs: &[PathBuf], dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    let n = inputs.len();
    let mut isolated_all = true;
    let mut outcome = batch(inputs, |i, input, outcome| {
        ctx.report(
            (i as f64 + 0.1) / n as f64,
            &tr!(
                "Convertendo {file} ({index} de {n}) — a primeira conversão pode levar alguns segundos",
                file = stem(input),
                index = i + 1,
                n
            ),
        );
        std::fs::File::open(input).map_err(|e| OpError::io_path(tr!("abrir “{file}”"), input, e))?;
        let job = JobDir::new()?;
        let profile = job.subdir("perfil")?;
        let out_dir = job.subdir("saida")?;
        // Cópia com nome simples: o LibreOffice deriva o nome de saída do de
        // entrada e não lida bem com alguns caracteres.
        let ext = input.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let source = job.file(&format!("documento.{ext}"));
        std::fs::copy(input, &source).map_err(|e| OpError::io_path(tr!("ler “{file}”"), input, e))?;
        let (pdf, isolated) = crate::engines::libreoffice::to_pdf(&source, &out_dir, &profile, &ctx.cancel)?;
        isolated_all &= isolated;
        let pages = qpdf::page_count(&pdf, &ctx.cancel).map_err(|e| match e {
            OpError::Cancelled => e,
            other => {
                OpError::OutputInvalid(tr!("o PDF gerado não pôde ser lido ({detail})", detail = other.technical()))
            }
        })?;
        if pages == 0 {
            return Err(OpError::OutputInvalid(tr!("o PDF gerado não tem páginas").into()));
        }
        let name = batch_output_name(dest, n, &stem(input), "pdf");
        outcome.push(storage::commit(&pdf, &dest.dir, &name)?);
        Ok(())
    })?;
    if !isolated_all && !outcome.outputs.is_empty() {
        outcome.notes.push(
            tr!("Este sistema não permite isolar a conversão da rede (namespaces sem privilégios desativados). \
                 O LibreOffice não costuma acessar a internet ao converter, mas o isolamento não pôde ser \
                 garantido.")
            .into(),
        );
    }
    Ok(outcome)
}
