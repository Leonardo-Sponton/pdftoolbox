//! Operações estruturais com qpdf: nada é rasterizado, então texto, fontes e
//! vetores são preservados.

use std::ffi::OsString;
use std::path::Path;

use super::{Tool, run, run_ok};
use crate::error::{OpError, OpResult};
use crate::jobs::CancelToken;
use crate::pages::compact_list;

/// qpdf: 0 = sucesso, 3 = sucesso com avisos, 2 = erro.
const OK: &[i32] = &[0, 3];

/// Abre o documento e devolve o número de páginas, distinguindo arquivo
/// inválido de arquivo protegido por senha.
pub fn page_count(path: &Path, cancel: &CancelToken) -> OpResult<u32> {
    std::fs::File::open(path).map_err(|e| OpError::io_path("abrir", path, e))?;
    let out = run(Tool::Qpdf, [OsString::from("--show-npages"), path.into()], cancel)?;
    if matches!(out.code, Some(0) | Some(3)) {
        let text = String::from_utf8_lossy(&out.stdout);
        return text.trim().parse().map_err(|_| OpError::InvalidPdf {
            file: path.to_path_buf(),
            detail: format!("saída inesperada do qpdf: {text:?}"),
        });
    }
    if requires_password(path, cancel)? {
        return Err(OpError::PasswordProtected(path.to_path_buf()));
    }
    Err(OpError::InvalidPdf { file: path.to_path_buf(), detail: out.stderr })
}

/// `--requires-password`: 0 = exige senha, 2 = não criptografado,
/// 3 = criptografado sem senha de abertura.
fn requires_password(path: &Path, cancel: &CancelToken) -> OpResult<bool> {
    let out = run(Tool::Qpdf, [OsString::from("--requires-password"), path.into()], cancel)?;
    Ok(out.code == Some(0))
}

/// Monta um novo PDF com a sequência de páginas indicada. Páginas
/// consecutivas do mesmo arquivo são agrupadas em um único argumento.
/// O primeiro arquivo é a entrada primária, preservando metadados do documento.
pub fn assemble(sources: &[(&Path, u32)], out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let Some((primary, _)) = sources.first() else {
        return Err(OpError::EmptySelection);
    };
    let mut args: Vec<OsString> = vec![primary.into(), "--pages".into()];
    let mut i = 0;
    while i < sources.len() {
        let file = sources[i].0;
        let mut pages = Vec::new();
        while i < sources.len() && sources[i].0 == file {
            pages.push(sources[i].1);
            i += 1;
        }
        args.push(file.into());
        args.push(compact_list(&pages).into());
    }
    args.push("--".into());
    args.push(out.into());
    run_ok(Tool::Qpdf, args, OK, cancel)?;
    Ok(())
}

/// Aplica rotações relativas (em graus, múltiplos de 90) a páginas do arquivo.
pub fn rotate(input: &Path, rotations: &[(i32, Vec<u32>)], out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let mut args: Vec<OsString> = vec![input.into()];
    for (angle, pages) in rotations {
        let angle = angle.rem_euclid(360);
        if angle == 0 || pages.is_empty() {
            continue;
        }
        args.push(format!("--rotate=+{angle}:{}", compact_list(pages)).into());
    }
    args.push(out.into());
    run_ok(Tool::Qpdf, args, OK, cancel)?;
    Ok(())
}

/// Otimização estrutural sem perda: fluxos de objetos, recompressão Flate no
/// nível máximo e remoção de recursos não referenciados.
pub fn optimize_lossless(input: &Path, out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let args: Vec<OsString> = vec![
        "--object-streams=generate".into(),
        "--compress-streams=y".into(),
        "--recompress-flate".into(),
        "--compression-level=9".into(),
        "--remove-unreferenced-resources=yes".into(),
        input.into(),
        out.into(),
    ];
    run_ok(Tool::Qpdf, args, OK, cancel)?;
    Ok(())
}

/// Verificação estrutural completa (`qpdf --check`).
pub fn check(path: &Path, cancel: &CancelToken) -> OpResult<()> {
    let out = run(Tool::Qpdf, [OsString::from("--check"), path.into()], cancel)?;
    match out.code {
        Some(0) | Some(3) => Ok(()),
        _ => Err(OpError::OutputInvalid(format!(
            "qpdf --check falhou: {}",
            String::from_utf8_lossy(&out.stdout).lines().chain(out.stderr.lines()).last().unwrap_or("")
        ))),
    }
}
