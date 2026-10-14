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

pub use crate::annotate::{Annotation, Kind as AnnotationKind};
pub use crate::engines::poppler::ImageFormat;
pub use crate::pdfwriter::{ImageLayout, PageSize};
pub use crate::secret::Secret;
pub use crate::stamp::{NumberFormat, NumberStyle, Position, WatermarkStyle};
pub use compress::{CUSTOM_DPI, CompressLevel, Estimate, estimate as estimate_compression};
pub use convert::OFFICE_EXTENSIONS;
pub use edit::{DocFields, MetadataChange, unsupported_chars};
pub use ocr::{OcrLanguage, OcrOptions, text_pages};
pub use organize::PageItem;
pub use security::Restrictions;

use crate::engines::qpdf;
use crate::error::{OpError, OpResult};
use crate::jobs::JobCtx;
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
    Merge { inputs: Vec<PathBuf> },
    Split { input: PathBuf, mode: SplitMode },
    Extract { input: PathBuf, spec: String, separate: bool },
    Remove { input: PathBuf, spec: String },
    Rotate { inputs: Vec<PathBuf>, spec: Option<String>, angle: i32 },
    Organize { input: PathBuf, pages: Vec<PageItem> },
    ImagesToPdf { inputs: Vec<PathBuf>, layout: ImageLayout, one_file: bool },
    PdfToImages { inputs: Vec<PathBuf>, format: ImageFormat, dpi: u32, quality: u8 },
    Compress { inputs: Vec<PathBuf>, level: CompressLevel },
    NumberPages { inputs: Vec<PathBuf>, spec: Option<String>, style: NumberStyle },
    Watermark { inputs: Vec<PathBuf>, spec: Option<String>, style: WatermarkStyle },
    Metadata { input: PathBuf, change: MetadataChange },
    Protect { inputs: Vec<PathBuf>, password: Secret, restrictions: Option<Restrictions> },
    Unlock { inputs: Vec<PathBuf>, password: Secret },
    ExtractText { inputs: Vec<PathBuf>, layout: bool },
    OfficeToPdf { inputs: Vec<PathBuf> },
    Annotate { input: PathBuf, items: Vec<Annotation> },
    Ocr { inputs: Vec<PathBuf>, options: OcrOptions },
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
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
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
        Request::ExtractText { inputs, layout } => convert::extract_text(&inputs, layout, dest, ctx),
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

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[unit]).replace('.', ",") }
}
