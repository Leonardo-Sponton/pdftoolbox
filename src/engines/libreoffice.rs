//! Conversão de documentos de escritório para PDF com o LibreOffice em modo
//! sem interface (`--headless`).
//!
//! - Perfil de usuário descartável dentro da pasta da tarefa: não toca no
//!   perfil do usuário, não conflita com um LibreOffice aberto e permite
//!   conversões em paralelo.
//! - Sem rede quando o sistema permite (`engines::run_offline`), para que o
//!   documento não busque imagens ou vínculos externos.
//! - Macros não são executadas: o perfil novo usa o nível de segurança padrão
//!   (alto), que bloqueia macros não assinadas.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{Tool, run_offline};
use crate::error::{OpError, OpResult};
use crate::jobs::CancelToken;

/// Converte `input` para PDF dentro de `out_dir`, usando `profile` como perfil
/// temporário. Devolve o PDF gerado e se a conversão rodou isolada da rede.
pub fn to_pdf(input: &Path, out_dir: &Path, profile: &Path, cancel: &CancelToken) -> OpResult<(PathBuf, bool)> {
    let profile_url = format!("-env:UserInstallation=file://{}", url_path(profile));
    let args: Vec<OsString> = vec![
        profile_url.into(),
        "--headless".into(),
        "--norestore".into(),
        "--nolockcheck".into(),
        "--nodefault".into(),
        "--convert-to".into(),
        "pdf".into(),
        "--outdir".into(),
        out_dir.into(),
        input.into(),
    ];
    let (out, isolated) = run_offline(Tool::LibreOffice, args, cancel)?;
    let stem = input.file_stem().unwrap_or_default();
    let pdf = out_dir.join(Path::new(stem).with_extension("pdf"));
    if out.code == Some(0) && pdf.is_file() {
        return Ok((pdf, isolated));
    }
    // O LibreOffice costuma sair com 0 mesmo quando não converte; o stderr
    // traz o motivo ("source file could not be loaded", formato desconhecido…).
    let mut detail = out.stderr.trim().to_string();
    if detail.is_empty() {
        detail = String::from_utf8_lossy(&out.stdout).trim().to_string();
    }
    Err(OpError::ToolFailed { tool: Tool::LibreOffice, code: out.code, stderr: detail })
}

/// Caminho absoluto em forma de URL `file://` (espaços e caracteres especiais
/// codificados), como o `-env:UserInstallation` exige.
fn url_path(path: &Path) -> String {
    let mut s = String::new();
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => s.push(b as char),
            _ => s.push_str(&format!("%{b:02X}")),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::url_path;
    use std::path::Path;

    #[test]
    fn profile_path_is_url_encoded() {
        assert_eq!(url_path(Path::new("/tmp/a b/perfil")), "/tmp/a%20b/perfil");
        assert_eq!(url_path(Path::new("/tmp/ção")), "/tmp/%C3%A7%C3%A3o");
    }
}
