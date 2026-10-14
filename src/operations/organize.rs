//! Mesclar, dividir, extrair, remover, girar e organizar — tudo estrutural (qpdf).

use std::path::{Path, PathBuf};

use super::{Destination, Outcome, SplitMode, batch, output_name, verify_pdf};
use crate::engines::qpdf;
use crate::error::{OpError, OpResult};
use crate::jobs::JobCtx;
use crate::pages::{self, compact_list};
use crate::storage::{self, JobDir, stem};

/// Página na ordem final: número original e rotação adicional em graus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageItem {
    pub page: u32,
    pub rotation: i32,
}

fn page_label(pages: &[u32]) -> String {
    let list = compact_list(pages);
    match pages {
        [one] => format!("página {one}"),
        _ if list.len() <= 40 => format!("páginas {list}"),
        _ => format!("{} páginas selecionadas", pages.len()),
    }
}

/// Gera um PDF com `pages` de `input` no arquivo temporário `tmp` e verifica.
fn build(input: &Path, pages: &[u32], tmp: &Path, ctx: &JobCtx) -> OpResult<()> {
    let sources: Vec<(&Path, u32)> = pages.iter().map(|&p| (input, p)).collect();
    qpdf::assemble(&sources, tmp, &ctx.cancel)?;
    verify_pdf(tmp, pages.len() as u32, ctx)
}

pub fn merge(inputs: &[PathBuf], dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    if inputs.len() < 2 {
        return Err(OpError::InvalidRange("Selecione pelo menos dois PDFs para mesclar.".into()));
    }
    let mut sources = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        ctx.report(i as f64 / (inputs.len() + 1) as f64, &format!("Lendo {}", storage::stem(input)));
        let n = qpdf::page_count(input, &ctx.cancel)?;
        sources.extend((1..=n).map(|p| (input.as_path(), p)));
    }
    ctx.report(0.8, "Mesclando");
    let job = JobDir::new()?;
    let tmp = job.file("mesclado.pdf");
    qpdf::assemble(&sources, &tmp, &ctx.cancel)?;
    verify_pdf(&tmp, sources.len() as u32, ctx)?;
    let mut outcome = Outcome::default();
    let name = output_name(dest, "Documentos mesclados", "pdf");
    outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
    outcome.notes.push(format!("{} páginas de {} arquivos.", sources.len(), inputs.len()));
    Ok(outcome)
}

/// Gera vários arquivos, um por grupo de páginas, e grava todos só no fim.
fn write_groups(input: &Path, groups: &[Vec<u32>], dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    let job = JobDir::new()?;
    let mut temps = Vec::new();
    for (i, group) in groups.iter().enumerate() {
        ctx.report(i as f64 / groups.len() as f64, &format!("Gerando arquivo {} de {}", i + 1, groups.len()));
        let tmp = job.file(&format!("parte-{i}.pdf"));
        build(input, group, &tmp, ctx)?;
        temps.push((tmp, format!("{} - {}.pdf", stem(input), page_label(group))));
    }
    let mut outcome = Outcome::default();
    let dir = if temps.len() > 3 {
        let folder = storage::create_unique_dir(&dest.dir, &format!("{} (dividido)", stem(input)))?;
        outcome.folder = Some(folder.clone());
        folder
    } else {
        dest.dir.clone()
    };
    for (tmp, name) in temps {
        outcome.push(storage::commit(&tmp, &dir, &storage::sanitize_file_name(&name, "pdf"))?);
    }
    Ok(outcome)
}

pub fn split(input: &Path, mode: &SplitMode, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    let total = qpdf::page_count(input, &ctx.cancel)?;
    let groups = match mode {
        SplitMode::Ranges(spec) => pages::parse_groups(spec, total)?,
        SplitMode::Every(n) => pages::chunks(total, *n),
        SplitMode::EachPage => pages::chunks(total, 1),
    };
    if groups.len() == 1 && groups[0].len() as u32 == total {
        return Err(OpError::InvalidRange(
            "A divisão resultaria em um único arquivo igual ao original. Ajuste os intervalos.".into(),
        ));
    }
    write_groups(input, &groups, dest, ctx)
}

pub fn extract(input: &Path, spec: &str, separate: bool, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    let total = qpdf::page_count(input, &ctx.cancel)?;
    let list = pages::parse_list(spec, total)?;
    if separate {
        let groups: Vec<Vec<u32>> = list.iter().map(|&p| vec![p]).collect();
        return write_groups(input, &groups, dest, ctx);
    }
    let job = JobDir::new()?;
    let tmp = job.file("extraido.pdf");
    ctx.report(0.3, "Extraindo páginas");
    build(input, &list, &tmp, ctx)?;
    let mut outcome = Outcome::default();
    let name = output_name(dest, &format!("{} - {}", stem(input), page_label(&list)), "pdf");
    outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
    Ok(outcome)
}

pub fn remove(input: &Path, spec: &str, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    let total = qpdf::page_count(input, &ctx.cancel)?;
    let removed = pages::parse_list(spec, total)?;
    let keep = pages::complement(&removed, total);
    if keep.is_empty() {
        return Err(OpError::InvalidRange("Não é possível remover todas as páginas do documento.".into()));
    }
    let job = JobDir::new()?;
    let tmp = job.file("editado.pdf");
    ctx.report(0.3, "Removendo páginas");
    build(input, &keep, &tmp, ctx)?;
    let mut outcome = Outcome::default();
    let name = output_name(dest, &format!("{} - sem {}", stem(input), page_label(&removed)), "pdf");
    outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
    outcome.notes.push(format!("{} página(s) removida(s); restam {}.", total - keep.len() as u32, keep.len()));
    Ok(outcome)
}

pub fn rotate(
    inputs: &[PathBuf],
    spec: Option<&str>,
    angle: i32,
    dest: &Destination,
    ctx: &JobCtx,
) -> OpResult<Outcome> {
    let n = inputs.len();
    batch(inputs, |i, input, outcome| {
        ctx.report(i as f64 / n as f64, &format!("Girando {}", stem(input)));
        let total = qpdf::page_count(input, &ctx.cancel)?;
        let pages = match spec.map(str::trim).filter(|s| !s.is_empty()) {
            Some(spec) => pages::parse_list(spec, total)?,
            None => (1..=total).collect(),
        };
        let job = JobDir::new()?;
        let tmp = job.file("girado.pdf");
        qpdf::rotate(input, &[(angle, pages)], &tmp, &ctx.cancel)?;
        verify_pdf(&tmp, total, ctx)?;
        let name = if n == 1 {
            output_name(dest, &format!("{} - girado", stem(input)), "pdf")
        } else {
            storage::sanitize_file_name(&format!("{} - girado", stem(input)), "pdf")
        };
        outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
        Ok(())
    })
}

/// Reordena, duplica, exclui e gira páginas em uma única saída.
pub fn organize(input: &Path, items: &[PageItem], dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    if items.is_empty() {
        return Err(OpError::InvalidRange("O documento final não pode ficar sem páginas.".into()));
    }
    let total = qpdf::page_count(input, &ctx.cancel)?;
    if let Some(bad) = items.iter().find(|it| it.page == 0 || it.page > total) {
        return Err(OpError::PageOutOfRange { page: bad.page, total });
    }
    let job = JobDir::new()?;
    let ordered = job.file("ordenado.pdf");
    ctx.report(0.2, "Reorganizando páginas");
    let order: Vec<u32> = items.iter().map(|it| it.page).collect();
    build(input, &order, &ordered, ctx)?;

    // Rotações referem-se às posições no documento final.
    let mut rotations: Vec<(i32, Vec<u32>)> = Vec::new();
    for angle in [90, 180, 270] {
        let positions: Vec<u32> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.rotation.rem_euclid(360) == angle)
            .map(|(i, _)| i as u32 + 1)
            .collect();
        if !positions.is_empty() {
            rotations.push((angle, positions));
        }
    }
    let result = if rotations.is_empty() {
        ordered
    } else {
        ctx.report(0.6, "Aplicando rotações");
        let rotated = job.file("organizado.pdf");
        qpdf::rotate(&ordered, &rotations, &rotated, &ctx.cancel)?;
        verify_pdf(&rotated, items.len() as u32, ctx)?;
        rotated
    };
    let mut outcome = Outcome::default();
    let name = output_name(dest, &format!("{} - organizado", stem(input)), "pdf");
    outcome.push(storage::commit(&result, &dest.dir, &name)?);
    outcome.notes.push(format!("{} página(s) no documento final.", items.len()));
    Ok(outcome)
}
