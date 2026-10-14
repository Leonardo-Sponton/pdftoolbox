//! Operações de documento. Cada operação recebe uma `Request` já validada pela
//! interface, trabalha em um `JobDir` temporário, verifica o resultado e só
//! então grava no destino com `storage::commit`.

mod compress;
mod convert;
mod edit;
mod ocr;
mod organize;
mod security;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

pub use crate::annotate::{Annotation, Kind as AnnotationKind};
pub use crate::engines::poppler::ImageFormat;
pub use crate::pdfwriter::{ImageLayout, PageSize};
pub use crate::secret::Secret;
pub use crate::stamp::{NumberFormat, NumberStyle, Position, WatermarkStyle};
pub use compress::{CUSTOM_DPI, CompressLevel, Estimate, estimate as estimate_compression};
pub use convert::OFFICE_EXTENSIONS;
pub use edit::{DocFields, MetadataChange, unsupported_chars};
pub use ocr::{OcrOptions, installed_languages, language_install, language_name, pages_with_text, text_pages};
pub use organize::PageItem;
pub use security::{Restrictions, UnlockedCopy, unlocked_copy};

use crate::engines::qpdf;
use crate::error::{OpError, OpResult};
use crate::jobs::{CancelToken, JobCtx};
use crate::storage;

/// Destino escolhido pelo usuário.
#[derive(Debug, Clone)]
pub struct Destination {
    pub dir: PathBuf,
    /// Nome do arquivo para operações com uma única saída; `None` = sugerido.
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SplitMode {
    /// Cada item da seleção (separado por vírgula) vira um arquivo.
    Ranges(String),
    /// Blocos de N páginas.
    Every(u32),
    /// Um arquivo por página.
    EachPage,
}

#[derive(Debug, Clone)]
pub enum Request {
    Merge {
        inputs: Vec<PathBuf>,
    },
    Split {
        input: PathBuf,
        mode: SplitMode,
    },
    Extract {
        input: PathBuf,
        spec: String,
        separate: bool,
    },
    Remove {
        input: PathBuf,
        spec: String,
    },
    Rotate {
        inputs: Vec<PathBuf>,
        spec: Option<String>,
        angle: i32,
    },
    Organize {
        input: PathBuf,
        pages: Vec<PageItem>,
    },
    ImagesToPdf {
        inputs: Vec<PathBuf>,
        layout: ImageLayout,
        one_file: bool,
    },
    PdfToImages {
        inputs: Vec<PathBuf>,
        format: ImageFormat,
        dpi: u32,
        quality: u8,
    },
    Compress {
        inputs: Vec<PathBuf>,
        level: CompressLevel,
    },
    NumberPages {
        inputs: Vec<PathBuf>,
        spec: Option<String>,
        style: NumberStyle,
    },
    Watermark {
        inputs: Vec<PathBuf>,
        spec: Option<String>,
        style: WatermarkStyle,
    },
    Metadata {
        input: PathBuf,
        change: MetadataChange,
    },
    Protect {
        inputs: Vec<PathBuf>,
        password: Secret,
        restrictions: Option<Restrictions>,
    },
    Unlock {
        inputs: Vec<PathBuf>,
        password: Secret,
    },
    /// `ocr`: reconhecer antes as páginas digitalizadas (sem texto).
    ExtractText {
        inputs: Vec<PathBuf>,
        layout: bool,
        ocr: Option<OcrOptions>,
    },
    OfficeToPdf {
        inputs: Vec<PathBuf>,
    },
    Annotate {
        input: PathBuf,
        items: Vec<Annotation>,
    },
    Ocr {
        inputs: Vec<PathBuf>,
        options: OcrOptions,
    },
}

#[derive(Debug, Clone)]
pub struct OutputFile {
    pub path: PathBuf,
    pub size: u64,
}

/// Resultado de uma tarefa. Em lotes, falhas individuais não interrompem os
/// demais arquivos e ficam em `failures`.
#[derive(Debug, Default)]
pub struct Outcome {
    pub outputs: Vec<OutputFile>,
    /// Pasta criada para agrupar várias saídas, se houver.
    pub folder: Option<PathBuf>,
    pub notes: Vec<String>,
    pub failures: Vec<(PathBuf, OpError)>,
    /// Lote interrompido pelo usuário depois de algumas saídas já gravadas.
    pub cancelled: bool,
}

impl Outcome {
    fn push(&mut self, path: PathBuf) {
        let size = std::fs::metadata(&path).map_or(0, |m| m.len());
        self.outputs.push(OutputFile { path, size });
    }
}

pub fn execute(req: Request, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    storage::check_writable(&dest.dir)?;
    match req {
        Request::Merge { inputs } => organize::merge(&inputs, dest, ctx),
        Request::Split { input, mode } => organize::split(&input, &mode, dest, ctx),
        Request::Extract { input, spec, separate } => organize::extract(&input, &spec, separate, dest, ctx),
        Request::Remove { input, spec } => organize::remove(&input, &spec, dest, ctx),
        Request::Rotate { inputs, spec, angle } => organize::rotate(&inputs, spec.as_deref(), angle, dest, ctx),
        Request::Organize { input, pages } => organize::organize(&input, &pages, dest, ctx),
        Request::ImagesToPdf { inputs, layout, one_file } => {
            convert::images_to_pdf(&inputs, layout, one_file, dest, ctx)
        }
        Request::PdfToImages { inputs, format, dpi, quality } => {
            convert::pdf_to_images(&inputs, format, dpi, quality, dest, ctx)
        }
        Request::Compress { inputs, level } => compress::compress(&inputs, level, dest, ctx),
        Request::NumberPages { inputs, spec, style } => edit::number_pages(&inputs, spec.as_deref(), &style, dest, ctx),
        Request::Watermark { inputs, spec, style } => edit::watermark(&inputs, spec.as_deref(), &style, dest, ctx),
        Request::Metadata { input, change } => edit::metadata(&input, &change, dest, ctx),
        Request::Protect { inputs, password, restrictions } => {
            security::protect(&inputs, &password, restrictions.as_ref(), dest, ctx)
        }
        Request::Unlock { inputs, password } => security::unlock(&inputs, &password, dest, ctx),
        Request::ExtractText { inputs, layout, ocr } => convert::extract_text(&inputs, layout, ocr.as_ref(), dest, ctx),
        Request::OfficeToPdf { inputs } => convert::office_to_pdf(&inputs, dest, ctx),
        Request::Annotate { input, items } => edit::annotate(&input, &items, dest, ctx),
        Request::Ocr { inputs, options } => ocr::ocr(&inputs, options, dest, ctx),
    }
}

/// Nome final: o escolhido pelo usuário ou o sugerido.
fn output_name(dest: &Destination, suggested: &str, ext: &str) -> String {
    let name = dest.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(suggested);
    storage::sanitize_file_name(name, ext)
}

/// Nome de uma saída de lote: o nome digitado pelo usuário só vale quando
/// o lote tem um único arquivo; nos demais, cada saída usa o sugerido.
fn batch_output_name(dest: &Destination, batch_size: usize, suggested: &str, ext: &str) -> String {
    if batch_size == 1 { output_name(dest, suggested, ext) } else { storage::sanitize_file_name(suggested, ext) }
}

/// Confere se o PDF gerado abre e tem o número de páginas esperado.
fn verify_pdf(path: &Path, expected_pages: u32, ctx: &JobCtx) -> OpResult<()> {
    let got = qpdf::page_count(path, &ctx.cancel).map_err(|e| match e {
        OpError::Cancelled => e,
        other => OpError::OutputInvalid(format!("o PDF gerado não pôde ser lido ({})", other.technical())),
    })?;
    if got != expected_pages {
        return Err(OpError::OutputInvalid(format!("esperava {expected_pages} página(s), mas o arquivo tem {got}")));
    }
    Ok(())
}

/// Executa `per_file` para cada entrada de um lote, acumulando falhas.
/// Só devolve erro se todos falharem (ou se nada foi feito antes do cancelamento).
fn batch(
    inputs: &[PathBuf],
    mut per_file: impl FnMut(usize, &Path, &mut Outcome) -> OpResult<()>,
) -> OpResult<Outcome> {
    if inputs.is_empty() {
        return Err(OpError::NoInput);
    }
    let mut outcome = Outcome::default();
    for (i, input) in inputs.iter().enumerate() {
        match per_file(i, input, &mut outcome) {
            Ok(()) => {}
            Err(OpError::Cancelled) => {
                if outcome.outputs.is_empty() {
                    return Err(OpError::Cancelled);
                }
                outcome.cancelled = true;
                return Ok(outcome);
            }
            Err(e) => outcome.failures.push((input.clone(), e)),
        }
    }
    if outcome.outputs.is_empty() && !outcome.failures.is_empty() && outcome.notes.is_empty() {
        return Err(outcome.failures.remove(0).1);
    }
    Ok(outcome)
}

/// Executa `work` para cada item em até `workers` threads (cada uma pega o
/// próximo item livre). Para no primeiro erro ou no cancelamento;
/// `on_done(n)` recebe quantos itens já terminaram, para o progresso.
/// Devolve os resultados na ordem de `items`.
fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    cancel: &CancelToken,
    work: impl Fn(&T) -> OpResult<R> + Sync,
    on_done: impl Fn(usize) + Sync,
) -> OpResult<Vec<R>> {
    fn lock<V>(m: &Mutex<V>) -> MutexGuard<'_, V> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    let first_error: Mutex<Option<OpError>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(items.len()) {
            scope.spawn(|| {
                while !cancel.is_cancelled() && lock(&first_error).is_none() {
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(k) else { break };
                    match work(item) {
                        Ok(result) => {
                            lock(&results)[k] = Some(result);
                            on_done(done.fetch_add(1, Ordering::SeqCst) + 1);
                        }
                        Err(e) => {
                            lock(&first_error).get_or_insert(e);
                            break;
                        }
                    }
                }
            });
        }
    });
    cancel.check()?;
    if let Some(e) = first_error.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(e);
    }
    results
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .into_iter()
        .collect::<Option<Vec<R>>>()
        .ok_or_else(|| OpError::OutputInvalid("faltou o resultado de alguma página".into()))
}
