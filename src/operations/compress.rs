//! Compressão: sem perda (qpdf) ou com reamostragem de imagens (Ghostscript + qpdf).

use std::path::{Path, PathBuf};

use super::{Destination, Outcome, batch, batch_output_name, verify_pdf};
use crate::engines::{Tool, ghostscript, qpdf};
use crate::error::{OpError, OpResult};
use crate::human;
use crate::jobs::{CancelToken, JobCtx};
use crate::storage::{self, JobDir, stem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressLevel {
    /// Só reorganização estrutural; nenhuma imagem é alterada.
    Lossless,
    /// Imagens a 150 ppp, JPEG de boa qualidade.
    Recommended,
    /// Imagens a 96 ppp; perda visível em ampliação.
    Strong,
    /// Imagens na resolução escolhida (ppp), JPEG de boa qualidade.
    Custom(u32),
}

/// Limites aceitos para a resolução personalizada, em ppp.
pub const CUSTOM_DPI: std::ops::RangeInclusive<u32> = 36..=600;

impl CompressLevel {
    pub fn needs(self) -> &'static [Tool] {
        match self {
            CompressLevel::Lossless => &[Tool::Qpdf],
            _ => &[Tool::Qpdf, Tool::Ghostscript],
        }
    }

    /// Predefinição do Ghostscript e resolução das imagens (`None` = sem perda).
    fn settings(self) -> Option<(&'static str, u32)> {
        match self {
            CompressLevel::Lossless => None,
            CompressLevel::Recommended => Some(("ebook", 150)),
            CompressLevel::Strong => Some(("screen", 96)),
            CompressLevel::Custom(dpi) => Some(("ebook", dpi.clamp(*CUSTOM_DPI.start(), *CUSTOM_DPI.end()))),
        }
    }
}

fn file_size(path: &Path) -> OpResult<u64> {
    Ok(std::fs::metadata(path).map_err(|e| OpError::io_path("ler", path, e))?.len())
}

/// Aplica `level` a `input`, gravando em `out` (dentro de `job`).
fn compress_file(input: &Path, level: CompressLevel, job: &JobDir, out: &Path, cancel: &CancelToken) -> OpResult<()> {
    match level.settings() {
        None => qpdf::optimize_lossless(input, out, cancel),
        Some((preset, dpi)) => {
            let rewritten = job.file("gs.pdf");
            ghostscript::downsample(input, &rewritten, preset, dpi, cancel)?;
            qpdf::optimize_lossless(&rewritten, out, cancel)
        }
    }
}

/// Tamanho previsto do resultado da compressão.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    pub original: u64,
    pub estimated: u64,
    /// O arquivo inteiro foi comprimido (sem amostragem): o valor é o real.
    pub exact: bool,
}

impl Estimate {
    /// Economia prevista, de 0 a 1 (negativa se o arquivo cresceria).
    pub fn saving(&self) -> f64 {
        1.0 - self.estimated as f64 / self.original.max(1) as f64
    }

    /// Abaixo da economia mínima, a compressão não grava um arquivo novo.
    pub fn worthwhile(&self) -> bool {
        self.saving() >= MIN_SAVING
    }
}

/// Até este número de páginas, a estimativa comprime o documento inteiro.
const SAMPLE_PAGES: u32 = 8;

/// Páginas da amostra: `SAMPLE_PAGES` espalhadas pelo documento.
fn sample_pages(total: u32) -> Vec<u32> {
    if total <= SAMPLE_PAGES {
        return (1..=total).collect();
    }
    let mut pages: Vec<u32> =
        (0..SAMPLE_PAGES).map(|i| 1 + (i as u64 * (total - 1) as u64 / (SAMPLE_PAGES - 1) as u64) as u32).collect();
    pages.dedup();
    pages
}

/// Prevê o tamanho do resultado sem gravar nada fora da pasta temporária.
/// Documentos curtos são comprimidos inteiros (resultado exato); nos longos,
/// uma amostra de páginas espalhadas é extraída e comprimida, e a proporção
/// obtida é aplicada ao arquivo todo.
pub fn estimate(input: &Path, level: CompressLevel, cancel: &CancelToken) -> OpResult<Estimate> {
    for tool in level.needs() {
        tool.require()?;
    }
    let original = file_size(input)?;
    let total = qpdf::page_count(input, cancel)?;
    let job = JobDir::new()?;
    let result = job.file("estimativa.pdf");
    let pages = sample_pages(total);
    if pages.len() as u32 == total {
        compress_file(input, level, &job, &result, cancel)?;
        let size = file_size(&result)?;
        return Ok(Estimate { original, estimated: size, exact: true });
    }
    // A amostra é extraída sem perda e otimizada do mesmo jeito, para que a
    // proporção reflita só o efeito do nível escolhido.
    let extracted = job.file("amostra-bruta.pdf");
    let sources: Vec<(&Path, u32)> = pages.iter().map(|&p| (input, p)).collect();
    qpdf::assemble(&sources, &extracted, cancel)?;
    let sample = job.file("amostra.pdf");
    qpdf::optimize_lossless(&extracted, &sample, cancel)?;
    compress_file(&sample, level, &job, &result, cancel)?;
    let before = file_size(&sample)?;
    let after = file_size(&result)?;
    let ratio = after as f64 / before.max(1) as f64;
    let estimated = ((original as f64 * ratio).round() as u64).max(1);
    Ok(Estimate { original, estimated, exact: false })
}

/// Economia mínima para valer a pena gravar um novo arquivo.
const MIN_SAVING: f64 = 0.01;

pub fn compress(inputs: &[PathBuf], level: CompressLevel, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    for tool in level.needs() {
        tool.require()?;
    }
    let n = inputs.len();
    batch(inputs, |i, input, outcome| {
        let base = i as f64 / n as f64;
        ctx.report(base, &format!("Analisando {}", stem(input)));
        let pages = qpdf::page_count(input, &ctx.cancel)?;
        let original = file_size(input)?;
        let job = JobDir::new()?;
        let result = job.file("comprimido.pdf");
        let step = if level == CompressLevel::Lossless { "Otimizando" } else { "Recomprimindo imagens de" };
        ctx.report(base + 0.1 / n as f64, &format!("{step} {}", stem(input)));
        compress_file(input, level, &job, &result, &ctx.cancel)?;
        verify_pdf(&result, pages, ctx)?;
        let size = file_size(&result)?;
        let saving = 1.0 - size as f64 / original.max(1) as f64;
        if saving < MIN_SAVING {
            outcome.notes.push(format!(
                "“{}” já está otimizado: o resultado teria {} (original: {}). Nenhum arquivo novo foi criado.",
                stem(input),
                human::size(size),
                human::size(original)
            ));
            return Ok(());
        }
        let suggested = format!("{} - comprimido", stem(input));
        let name = batch_output_name(dest, n, &suggested, "pdf");
        outcome.push(storage::commit(&result, &dest.dir, &name)?);
        outcome.notes.push(format!(
            "“{}”: {} → {} (−{:.0}%).",
            stem(input),
            human::size(original),
            human::size(size),
            saving * 100.0
        ));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_spreads_over_the_document() {
        assert_eq!(sample_pages(3), [1, 2, 3]);
        let s = sample_pages(100);
        assert_eq!(s.len(), SAMPLE_PAGES as usize);
        assert_eq!((s[0], *s.last().unwrap()), (1, 100));
        assert!(s.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(sample_pages(9).len(), 8);
    }

    #[test]
    fn custom_dpi_is_clamped() {
        assert_eq!(CompressLevel::Custom(5).settings(), Some(("ebook", 36)));
        assert_eq!(CompressLevel::Custom(200).settings(), Some(("ebook", 200)));
        assert_eq!(CompressLevel::Custom(5000).settings(), Some(("ebook", 600)));
    }
}
