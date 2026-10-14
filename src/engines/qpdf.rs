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

/// Sobrepõe a página N de `layer` à página N de `input` (carimbos). O qpdf
/// encaixa a camada na área visível de cada página, respeitando a rotação.
pub fn overlay(input: &Path, layer: &Path, out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let args: Vec<OsString> = vec![input.into(), "--overlay".into(), layer.into(), "--".into(), out.into()];
    run_ok(Tool::Qpdf, args, OK, cancel)?;
    Ok(())
}

/// Sobrepõe as páginas de `layer`, em ordem, às páginas `to` de `input`
/// (as demais ficam como estão).
pub fn overlay_pages(input: &Path, layer: &Path, to: &[u32], out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let list: Vec<String> = to.iter().map(u32::to_string).collect();
    let args: Vec<OsString> = vec![
        input.into(),
        "--overlay".into(),
        layer.into(),
        format!("--to={}", list.join(",")).into(),
        format!("--from=1-{}", to.len()).into(),
        "--".into(),
        out.into(),
    ];
    run_ok(Tool::Qpdf, args, OK, cancel)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    None,
    /// Abre sem senha, mas tem permissões definidas pelo autor.
    RestrictionsOnly,
    /// Exige senha para abrir.
    Password,
}

pub fn encryption(path: &Path, cancel: &CancelToken) -> OpResult<Encryption> {
    std::fs::File::open(path).map_err(|e| OpError::io_path("abrir", path, e))?;
    let out = run(Tool::Qpdf, [OsString::from("--requires-password"), path.into()], cancel)?;
    match out.code {
        Some(0) => Ok(Encryption::Password),
        Some(3) => Ok(Encryption::RestrictionsOnly),
        Some(2) if out.stderr.trim().is_empty() => Ok(Encryption::None),
        _ => Err(OpError::InvalidPdf { file: path.to_path_buf(), detail: out.stderr }),
    }
}

/// O que a senha do arquivo `password_file` permite neste documento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordCheck {
    /// É a senha de permissões (do proprietário).
    pub owner: bool,
    /// O documento proíbe alguma ação (imprimir, copiar, editar…).
    pub restricted: bool,
}

pub fn check_password(path: &Path, password_file: &Path, cancel: &CancelToken) -> OpResult<PasswordCheck> {
    let mut arg = OsString::from("--password-file=");
    arg.push(password_file);
    let out = run(Tool::Qpdf, [arg, "--show-encryption".into(), path.into()], cancel)?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_password_check(&text, &out.stderr).ok_or_else(|| OpError::WrongPassword(path.to_path_buf()))
}

fn parse_password_check(stdout: &str, stderr: &str) -> Option<PasswordCheck> {
    if stdout.contains("Incorrect password") || stderr.contains("invalid password") {
        return None;
    }
    let owner = stdout.contains("Supplied password is owner password");
    let user = stdout.contains("Supplied password is user password");
    if !owner && !user {
        return None;
    }
    let restricted = stdout.lines().any(|l| l.trim_end().ends_with(": not allowed"));
    Some(PasswordCheck { owner, restricted })
}

/// Grava uma cópia sem criptografia, abrindo com a senha do arquivo indicado.
pub fn decrypt(input: &Path, password_file: &Path, out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let mut arg = OsString::from("--password-file=");
    arg.push(password_file);
    run_ok(Tool::Qpdf, [arg, "--decrypt".into(), input.into(), out.into()], OK, cancel)?;
    Ok(())
}

/// Número de páginas abrindo com a senha do arquivo indicado.
pub fn page_count_with_password(path: &Path, password_file: &Path, cancel: &CancelToken) -> OpResult<u32> {
    let mut arg = OsString::from("--password-file=");
    arg.push(password_file);
    let out = run_ok(Tool::Qpdf, [arg, "--show-npages".into(), path.into()], OK, cancel)?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| OpError::OutputInvalid("o qpdf não informou o número de páginas".into()))
}

/// Criptografa com AES-256. As opções (senhas incluídas) ficam em
/// `args_file`, lido pelo qpdf como `@arquivo`: assim as senhas não aparecem
/// na lista de processos do sistema.
pub fn encrypt(input: &Path, args_file: &Path, out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let mut at = OsString::from("@");
    at.push(args_file);
    run_ok(Tool::Qpdf, [input.as_os_str().to_owned(), at, out.into()], OK, cancel)?;
    Ok(())
}

/// Remove o dicionário de informações e os metadados XMP.
pub fn strip_metadata(input: &Path, out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let args: Vec<OsString> = vec!["--remove-info".into(), "--remove-metadata".into(), input.into(), out.into()];
    run_ok(Tool::Qpdf, args, OK, cancel)?;
    Ok(())
}

/// Lê o trailer e o maior número de objeto, para montar uma atualização JSON.
pub fn trailer(input: &Path, cancel: &CancelToken) -> OpResult<(serde_json::Value, u64)> {
    let args: Vec<OsString> =
        vec!["--json=2".into(), "--json-key=qpdf".into(), "--json-object=trailer".into(), input.into()];
    let out = run_ok(Tool::Qpdf, args, OK, cancel)?;
    let bad = |what: &str| OpError::OutputInvalid(format!("JSON do qpdf sem {what}"));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| bad("formato válido"))?;
    let qpdf = json.get("qpdf").and_then(|q| q.as_array()).ok_or_else(|| bad("a chave qpdf"))?;
    let max =
        qpdf.first().and_then(|h| h.get("maxobjectid")).and_then(|m| m.as_u64()).ok_or_else(|| bad("maxobjectid"))?;
    let trailer = qpdf
        .get(1)
        .and_then(|o| o.get("trailer"))
        .and_then(|t| t.get("value"))
        .cloned()
        .ok_or_else(|| bad("trailer"))?;
    Ok((trailer, max))
}

/// Aplica uma atualização no formato JSON v2 do qpdf.
pub fn update_from_json(input: &Path, json: &Path, out: &Path, cancel: &CancelToken) -> OpResult<()> {
    let mut arg = OsString::from("--update-from-json=");
    arg.push(json);
    run_ok(Tool::Qpdf, [input.as_os_str().to_owned(), arg, out.into()], OK, cancel)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_check_reads_qpdf_report() {
        let user = "R = 6\nSupplied password is user password\nprint high resolution: not allowed\n";
        assert_eq!(parse_password_check(user, ""), Some(PasswordCheck { owner: false, restricted: true }));
        let owner = "R = 6\nSupplied password is owner password\nprint high resolution: allowed\n";
        assert_eq!(parse_password_check(owner, ""), Some(PasswordCheck { owner: true, restricted: false }));
        assert_eq!(parse_password_check("Incorrect password supplied\nR = 6\n", ""), None);
        assert_eq!(parse_password_check("File is not encrypted\n", ""), None);
    }
}
