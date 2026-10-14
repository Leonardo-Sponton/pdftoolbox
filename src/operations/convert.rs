//! Imagens → PDF (gerador próprio) e PDF → imagens (Poppler).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use super::{Destination, Outcome, batch, output_name, verify_pdf};
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
        ctx.report(base + span * done as f64 / n as f64, &format!("Imagem {done} de {n}"));
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
    batch(inputs, |i, input, outcome| {
        let total = qpdf::page_count(input, &ctx.cancel)?;
        let job = JobDir::new()?;
        let digits = total.to_string().len();
        let rendered = render_pages(input, total, digits, format, dpi, quality, &job, ctx, i, n)?;
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
        for (page, file) in rendered {
            let name = format!("{} - página {page:0digits$}.{}", stem(input), format.extension());
            outcome.push(storage::commit(&file, &dir, &storage::sanitize_file_name(&name, format.extension()))?);
        }
        Ok(())
    })
}

/// Renderiza todas as páginas com até `MAX_RENDERERS` processos simultâneos
/// (o pdftoppm usa um núcleo por processo). Devolve (página, arquivo) em ordem.
#[allow(clippy::too_many_arguments)]
fn render_pages(
    input: &Path,
    total: u32,
    digits: usize,
    format: ImageFormat,
    dpi: u32,
    quality: u8,
    job: &JobDir,
    ctx: &JobCtx,
    file_index: usize,
    file_count: f64,
) -> OpResult<Vec<(u32, PathBuf)>> {
    const MAX_RENDERERS: usize = 4;
    let workers = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1)).min(MAX_RENDERERS);
    let next = AtomicU32::new(1);
    let done = AtomicU32::new(0);
    let results: Mutex<Vec<(u32, PathBuf)>> = Mutex::new(Vec::with_capacity(total as usize));
    let first_error: Mutex<Option<OpError>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(total as usize) {
            scope.spawn(|| {
                loop {
                    if ctx.cancel.is_cancelled() || first_error.lock().unwrap().is_some() {
                        break;
                    }
                    let page = next.fetch_add(1, Ordering::SeqCst);
                    if page > total {
                        break;
                    }
                    let base = job.file(&format!("p{page:0digits$}"));
                    match poppler::render_page(input, page, format, dpi, quality, &base, &ctx.cancel) {
                        Ok(file) => {
                            results.lock().unwrap().push((page, file));
                            let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                            ctx.report(
                                (file_index as f64 + d as f64 / total as f64) / file_count,
                                &format!("{}: {d} de {total} páginas", stem(input)),
                            );
                        }
                        Err(e) => {
                            first_error.lock().unwrap().get_or_insert(e);
                            break;
                        }
                    }
                }
            });
        }
    });
    ctx.cancel.check()?;
    if let Some(e) = first_error.into_inner().unwrap() {
        return Err(e);
    }
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(p, _)| *p);
    Ok(results)
}
