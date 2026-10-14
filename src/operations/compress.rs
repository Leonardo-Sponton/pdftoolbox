//! Compressão: sem perda (qpdf) ou com reamostragem de imagens (Ghostscript + qpdf).

use std::path::PathBuf;

use super::{Destination, Outcome, batch, human_size, output_name, verify_pdf};
use crate::engines::{Tool, ghostscript, qpdf};
use crate::error::OpResult;
use crate::jobs::JobCtx;
use crate::storage::{self, JobDir, stem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressLevel {
    /// Só reorganização estrutural; nenhuma imagem é alterada.
    Lossless,
    /// Imagens a 150 ppp, JPEG de boa qualidade.
    Recommended,
    /// Imagens a 96 ppp; perda visível em ampliação.
    Strong,
}

impl CompressLevel {
    pub fn needs(self) -> &'static [Tool] {
        match self {
            CompressLevel::Lossless => &[Tool::Qpdf],
            _ => &[Tool::Qpdf, Tool::Ghostscript],
        }
    }
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
        let original = std::fs::metadata(input).map_err(|e| crate::OpError::io_path("ler", input, e))?.len();
        let job = JobDir::new()?;
        let result = job.file("comprimido.pdf");
        match level {
            CompressLevel::Lossless => {
                ctx.report(base + 0.3 / n as f64, &format!("Otimizando {}", stem(input)));
                qpdf::optimize_lossless(input, &result, &ctx.cancel)?;
            }
            CompressLevel::Recommended | CompressLevel::Strong => {
                let (preset, dpi) = if level == CompressLevel::Strong { ("screen", 96) } else { ("ebook", 150) };
                ctx.report(base + 0.1 / n as f64, &format!("Recomprimindo imagens de {}", stem(input)));
                let rewritten = job.file("gs.pdf");
                ghostscript::downsample(input, &rewritten, preset, dpi, &ctx.cancel)?;
                ctx.report(base + 0.7 / n as f64, &format!("Otimizando a estrutura de {}", stem(input)));
                qpdf::optimize_lossless(&rewritten, &result, &ctx.cancel)?;
            }
        }
        verify_pdf(&result, pages, ctx)?;
        let size = std::fs::metadata(&result).map_err(|e| crate::OpError::io("ler o resultado", e))?.len();
        let saving = 1.0 - size as f64 / original.max(1) as f64;
        if saving < MIN_SAVING {
            outcome.notes.push(format!(
                "“{}” já está otimizado: o resultado teria {} (original: {}). Nenhum arquivo novo foi criado.",
                stem(input),
                human_size(size),
                human_size(original)
            ));
            return Ok(());
        }
        let suggested = format!("{} - comprimido", stem(input));
        let name =
            if n == 1 { output_name(dest, &suggested, "pdf") } else { storage::sanitize_file_name(&suggested, "pdf") };
        outcome.push(storage::commit(&result, &dest.dir, &name)?);
        outcome.notes.push(format!(
            "“{}”: {} → {} (−{:.0}%).",
            stem(input),
            human_size(original),
            human_size(size),
            saving * 100.0
        ));
        Ok(())
    })
}
