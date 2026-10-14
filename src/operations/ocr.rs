//! OCR: torna pesquisável o texto de documentos digitalizados.
//!
//! Cada página sem texto é renderizada (`pdftoppm`), o Tesseract gera só a
//! camada de texto invisível dela, e o qpdf sobrepõe essas camadas às páginas
//! originais. A imagem digitalizada não é recomprimida nem rasterizada de
//! novo: o arquivo cresce poucos KB por página.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Destination, Outcome, batch, output_name, verify_pdf};
use crate::engines::{install_line, poppler, qpdf, tesseract};
use crate::error::{OpError, OpResult, display_name};
use crate::jobs::JobCtx;
use crate::pdfwriter::{self, InvisiblePage, InvisibleWord};
use crate::storage::{self, JobDir, stem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrLanguage {
    PortugueseEnglish,
    Portuguese,
    English,
}

impl OcrLanguage {
    fn codes(self) -> &'static [&'static str] {
        match self {
            OcrLanguage::PortugueseEnglish => &["por", "eng"],
            OcrLanguage::Portuguese => &["por"],
            OcrLanguage::English => &["eng"],
        }
    }
}

fn language_name(code: &str) -> &'static str {
    match code {
        "por" => "português",
        "eng" => "inglês",
        _ => "outro idioma",
    }
}

fn language_install(code: &str) -> String {
    install_line(
        &format!("tesseract-ocr-{code}"),
        &format!("tesseract-data-{code}"),
        &format!("tesseract-langpack-{code}"),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrOptions {
    pub language: OcrLanguage,
    /// Não reconhecer páginas que já têm texto selecionável (evita texto
    /// duplicado na busca e na cópia).
    pub skip_text_pages: bool,
    /// Girar (sem perda, como “Girar PDF”) as páginas que o reconhecimento
    /// encontrou deitadas ou de cabeça para baixo.
    pub straighten: bool,
}

/// Resolução do reconhecimento: 300 ppp, menos em páginas enormes (plantas,
/// cartazes) para a imagem não passar de `MAX_SIDE` pixels.
const DPI: u32 = 300;
const MAX_SIDE: f32 = 7000.0;

fn dpi_for((w, h): (f32, f32)) -> u32 {
    let inches = w.max(h) / 72.0;
    if inches <= 0.0 { DPI } else { ((MAX_SIDE / inches).floor() as u32).clamp(72, DPI) }
}

/// Uma página “tem texto” se o `pdftotext` encontra letras ou números nela.
fn has_text(page: &str) -> bool {
    page.chars().any(char::is_alphanumeric)
}

/// Quantas páginas já têm texto selecionável, e o total (para a interface
/// mostrar o que será reconhecido antes de executar).
pub fn text_pages(pdf: &Path, cancel: &crate::jobs::CancelToken) -> OpResult<(u32, u32)> {
    let flags = pages_with_text(pdf, cancel)?;
    Ok((flags.iter().filter(|&&t| t).count() as u32, flags.len() as u32))
}

/// Para cada página (na ordem), se ela já tem texto selecionável.
pub fn pages_with_text(pdf: &Path, cancel: &crate::jobs::CancelToken) -> OpResult<Vec<bool>> {
    let total = qpdf::page_count(pdf, cancel)?;
    Ok(poppler::texts_per_page(pdf, total, cancel)?.iter().map(|t| has_text(t)).collect())
}

/// Páginas a reconhecer (1 = primeira).
fn targets(texts: &[String], skip_text_pages: bool) -> Vec<u32> {
    texts.iter().enumerate().filter(|(_, t)| !(skip_text_pages && has_text(t))).map(|(i, _)| i as u32 + 1).collect()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn ocr(inputs: &[PathBuf], opts: OcrOptions, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    if inputs.is_empty() {
        return Err(OpError::NoInput);
    }
    let installed = tesseract::languages(&ctx.cancel)?;
    if let Some(code) = opts.language.codes().iter().find(|c| !installed.iter().any(|i| i == *c)) {
        return Err(OpError::MissingOcrLanguage { language: language_name(code), install: language_install(code) });
    }
    let langs = opts.language.codes().join("+");
    let font = tesseract::glyphless_font(&ctx.cancel)?;
    let n = inputs.len();
    let mut outcome = batch(inputs, |i, input, outcome| {
        ctx.report(i as f64 / n as f64, &format!("Analisando {}", stem(input)));
        let total = qpdf::page_count(input, &ctx.cancel)?;
        let sizes = poppler::page_sizes(input, total, &ctx.cancel)?;
        let texts = poppler::texts_per_page(input, total, &ctx.cancel)?;
        let pages = targets(&texts, opts.skip_text_pages);
        let already = texts.iter().filter(|t| has_text(t)).count();
        if pages.is_empty() {
            outcome.notes.push(format!(
                "“{}” já tem texto selecionável em todas as páginas; não havia o que reconhecer e nenhum arquivo \
                 foi criado.",
                display_name(input)
            ));
            return Ok(());
        }
        let job = JobDir::new()?;
        let layers = recognize(input, &pages, &sizes, &langs, &job, ctx, i, n)?;

        ctx.report((i as f64 + 0.97) / n as f64, &format!("Montando {}", stem(input)));
        // O qpdf encaixa cada página da camada na página de destino, então o
        // arredondamento de pixels da renderização não desloca o texto.
        let layer_pdf = job.file("camada-texto.pdf");
        pdfwriter::invisible_text_pdf(&layer_pdf, &layers, &font)
            .map_err(|e| OpError::io("gravar a camada de texto temporária", e))?;
        let mut tmp = job.file("ocr.pdf");
        qpdf::overlay_pages(input, &layer_pdf, &pages, &tmp, &ctx.cancel)?;
        let mut turned: Vec<(i32, Vec<u32>)> = Vec::new();
        if opts.straighten {
            for (layer, &page) in layers.iter().zip(&pages).filter(|(l, _)| l.rotation != 0) {
                match turned.iter_mut().find(|(a, _)| *a == layer.rotation as i32) {
                    Some((_, list)) => list.push(page),
                    None => turned.push((layer.rotation as i32, vec![page])),
                }
            }
        }
        if !turned.is_empty() {
            let straight = job.file("ocr-endireitado.pdf");
            qpdf::rotate(&tmp, &turned, &straight, &ctx.cancel)?;
            tmp = straight;
        }
        verify_pdf(&tmp, total, ctx)?;

        let after = poppler::texts_per_page(&tmp, total, &ctx.cancel)?;
        let recognized = pages.iter().filter(|&&p| has_text(&after[p as usize - 1])).count();
        if recognized == 0 {
            return Err(OpError::OutputInvalid(format!(
                "nenhum texto foi reconhecido em “{}”. Confira se o idioma escolhido é o do documento e se a \
                 digitalização está legível",
                display_name(input)
            )));
        }
        let base = format!("{} - OCR", stem(input));
        let name = if n == 1 { output_name(dest, &base, "pdf") } else { storage::sanitize_file_name(&base, "pdf") };
        outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
        let mut note = format!(
            "“{}”: texto reconhecido em {recognized} de {}.",
            display_name(input),
            plural(pages.len(), "página analisada", "páginas analisadas")
        );
        let straightened: usize = turned.iter().map(|(_, p)| p.len()).sum();
        if straightened > 0 {
            note.push_str(&if straightened == 1 {
                " 1 página estava deitada ou de cabeça para baixo e foi girada.".to_string()
            } else {
                format!(" {straightened} páginas estavam deitadas ou de cabeça para baixo e foram giradas.")
            });
        }
        if opts.skip_text_pages && already > 0 {
            note.push_str(&if already == 1 {
                " 1 página já tinha texto e ficou como estava.".to_string()
            } else {
                format!(" {already} páginas já tinham texto e ficaram como estavam.")
            });
        }
        outcome.notes.push(note);
        Ok(())
    })?;
    if !outcome.outputs.is_empty() {
        outcome.notes.push(
            "O reconhecimento automático pode errar letras e números, principalmente em digitalizações tortas, \
             manchadas ou com letra pequena. Confira trechos importantes antes de copiar ou confiar na busca."
                .into(),
        );
    }
    Ok(outcome)
}

/// Página da camada a partir das linhas reconhecidas numa imagem de
/// `image_px` pixels em `dpi`. Cada linha vira texto horizontal numa linha de
/// base só (a mediana das palavras): o Poppler — e com ele a busca de vários
/// leitores — quebra linhas cujas palavras sobem ou descem, como acontece
/// em digitalizações levemente tortas.
fn layer_page(lines: &[Vec<tesseract::Word>], image_px: (u32, u32), dpi: u32, rotation: u32) -> InvisiblePage {
    let scale = 72.0 / dpi as f32;
    // Palavras no sistema da imagem endireitada.
    let upright = if rotation % 180 == 90 { (image_px.1, image_px.0) } else { image_px };
    let height = upright.1 as f32 * scale;
    let median = |mut v: Vec<f32>| -> f32 {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let mut words = Vec::new();
    for line in lines.iter().filter(|l| !l.is_empty()) {
        let baseline = median(line.iter().map(|w| w.top + w.height).collect());
        let size = median(line.iter().map(|w| w.height).collect()) * scale;
        for w in line {
            words.push(InvisibleWord {
                text: w.text.clone(),
                x: w.left * scale,
                baseline: height - baseline * scale,
                width: w.width * scale,
                size,
            });
        }
    }
    InvisiblePage { width: image_px.0 as f32 * scale, height: image_px.1 as f32 * scale, rotation, words }
}

/// Lê uma página renderizada. Se a leitura parecer de uma página deitada ou
/// de cabeça para baixo (linhas na vertical, confiança baixa), detecta a
/// orientação, endireita a imagem, lê de novo e fica com a melhor leitura.
fn read_page(image: &Path, page: u32, langs: &str, dpi: u32, job: &JobDir, ctx: &JobCtx) -> OpResult<InvisiblePage> {
    let unreadable = || OpError::OutputInvalid(format!("imagem da página {page} ilegível"));
    let dims = image::image_dimensions(image).map_err(|_| unreadable())?;
    let first = tesseract::recognize(image, &job.file(&format!("t{page}")), langs, dpi, &ctx.cancel)?;
    let vertical = first.mostly_vertical();
    let suspicious = first.words().next().is_some() && (vertical || first.mean_conf() < 60.0);
    let rotation = if suspicious {
        match tesseract::orientation(image, &ctx.cancel)? {
            Some(r @ (90 | 180 | 270)) => Some(r),
            // Pouco texto para a detecção: a direção das linhas resolve; com
            // linhas horizontais e leitura ruim, testa de cabeça para baixo.
            _ if vertical => Some(first.vertical_rotation()),
            _ => Some(180),
        }
    } else {
        None
    };
    if let Some(rotation) = rotation {
        let turned = job.file(&format!("p{page}-endireitada.pgm"));
        let img = image::open(image).map_err(|_| unreadable())?;
        let img = match rotation {
            90 => img.rotate90(),
            180 => img.rotate180(),
            _ => img.rotate270(),
        };
        img.save(&turned)
            .map_err(|e| OpError::OutputInvalid(format!("não foi possível girar a página {page}: {e}")))?;
        drop(img);
        let second = tesseract::recognize(&turned, &job.file(&format!("u{page}")), langs, dpi, &ctx.cancel);
        let _ = std::fs::remove_file(&turned);
        let second = second?;
        // Linhas verticais não podem ficar como estão (a camada é horizontal):
        // a leitura endireitada vale, a menos que seja muito pior.
        let better = if vertical { second.score() * 4 >= first.score() * 3 } else { second.score() > first.score() };
        if better {
            return Ok(layer_page(&second.lines, dims, dpi, rotation));
        }
    }
    Ok(layer_page(&first.lines, dims, dpi, 0))
}

/// Reconhece as páginas em paralelo (um processo do Tesseract por página).
/// Devolve as páginas da camada na ordem de `pages`.
#[allow(clippy::too_many_arguments)]
fn recognize(
    input: &Path,
    pages: &[u32],
    sizes: &[(f32, f32)],
    langs: &str,
    job: &JobDir,
    ctx: &JobCtx,
    file_index: usize,
    file_count: usize,
) -> OpResult<Vec<InvisiblePage>> {
    const MAX_WORKERS: usize = 4;
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1)).clamp(1, MAX_WORKERS);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let layers: Mutex<Vec<Option<InvisiblePage>>> = Mutex::new(vec![None; pages.len()]);
    let first_error: Mutex<Option<OpError>> = Mutex::new(None);
    let count = pages.len();
    ctx.report(
        file_index as f64 / file_count as f64,
        &format!("{}: reconhecendo 0 de {}", stem(input), plural(count, "página", "páginas")),
    );
    std::thread::scope(|scope| {
        for _ in 0..workers.min(count) {
            scope.spawn(|| {
                loop {
                    if ctx.cancel.is_cancelled() || first_error.lock().unwrap().is_some() {
                        break;
                    }
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    let Some(&page) = pages.get(k) else { break };
                    let dpi = dpi_for(sizes[page as usize - 1]);
                    let result = poppler::render_gray(input, page, dpi, &job.file(&format!("p{page}")), &ctx.cancel)
                        .and_then(|image| {
                            let result = read_page(&image, page, langs, dpi, job, ctx);
                            let _ = std::fs::remove_file(&image);
                            result
                        });
                    match result {
                        Ok(layer) => {
                            layers.lock().unwrap()[k] = Some(layer);
                            let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                            ctx.report(
                                (file_index as f64 + 0.95 * d as f64 / count as f64) / file_count as f64,
                                &format!("{}: reconhecendo {d} de {}", stem(input), plural(count, "página", "páginas")),
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
    layers
        .into_inner()
        .unwrap()
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| OpError::OutputInvalid("faltou a camada de texto de alguma página".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_pages_get_a_lower_resolution() {
        assert_eq!(dpi_for((595.0, 842.0)), 300);
        // A0 (841 × 1189 mm): 300 ppp passaria de 14 000 px.
        let a0 = (2384.0, 3370.0);
        let dpi = dpi_for(a0);
        assert!(dpi < 300 && (3370.0 / 72.0 * dpi as f32) <= MAX_SIDE);
        assert_eq!(dpi_for((0.0, 0.0)), 300);
    }

    #[test]
    fn skewed_lines_are_flattened_onto_one_baseline() {
        let word = |text: &str, left: f32, top: f32| tesseract::Word {
            text: text.into(),
            left,
            top,
            width: 100.0,
            height: 40.0,
            conf: 90.0,
        };
        // Linha subindo para a direita (digitalização torta), 300 ppp.
        let line = vec![word("um", 300.0, 1000.0), word("dois", 450.0, 990.0), word("três", 600.0, 980.0)];
        let page = layer_page(&[line], (2480, 3508), 300, 0);
        assert!((page.height - 3508.0 * 0.24).abs() < 0.01 && (page.width - 2480.0 * 0.24).abs() < 0.01);
        let bases: Vec<f32> = page.words.iter().map(|w| w.baseline).collect();
        assert!(bases.iter().all(|b| *b == bases[0]), "uma linha de base só: {bases:?}");
        // Mediana: a palavra do meio, base em 1030 px a partir do topo.
        assert!((bases[0] - (3508.0 - 1030.0) * 0.24).abs() < 0.01);
        assert!((page.words[1].x - 108.0).abs() < 0.01 && (page.words[1].width - 24.0).abs() < 0.01);
        assert!((page.words[0].size - 9.6).abs() < 0.01);
    }

    #[test]
    fn pages_with_text_are_skipped_only_when_asked() {
        let texts = vec!["Título\n".to_string(), "  \n ".to_string(), "".to_string(), "— ·".to_string()];
        assert_eq!(targets(&texts, true), [2, 3, 4]);
        assert_eq!(targets(&texts, false), [1, 2, 3, 4]);
    }
}
