//! Proteger com senha e remover senha (qpdf, AES-256).
//!
//! As senhas chegam como `Secret`, são entregues ao qpdf por arquivos 0600
//! dentro da pasta da tarefa (apagada ao fim) e nunca vão para a linha de
//! comando, para logs ou para mensagens de erro. Não há nenhum mecanismo de
//! descoberta de senha: sem a senha correta, nada é feito.

use std::path::{Path, PathBuf};

use super::{Destination, Outcome, batch, output_name, verify_pdf};
use crate::engines::qpdf::{self, Encryption};
use crate::error::{OpError, OpResult};
use crate::jobs::{CancelToken, JobCtx};
use crate::secret::Secret;
use crate::storage::{self, JobDir, stem};

/// Restrições gravadas no documento, válidas para quem abrir com a senha de
/// abertura. Só a senha de permissões (`owner`) permite remover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restrictions {
    pub owner: Secret,
    pub allow_print: bool,
    pub allow_copy: bool,
    pub allow_edit: bool,
}

fn line_ok(password: &Secret) -> OpResult<()> {
    if password.expose().contains(['\n', '\r']) {
        return Err(OpError::InvalidRange("A senha não pode conter quebras de linha.".into()));
    }
    Ok(())
}

pub fn protect(
    inputs: &[PathBuf],
    password: &Secret,
    restrictions: Option<&Restrictions>,
    dest: &Destination,
    ctx: &JobCtx,
) -> OpResult<Outcome> {
    if password.is_empty() {
        return Err(OpError::InvalidRange("Digite a senha de abertura.".into()));
    }
    line_ok(password)?;
    if let Some(r) = restrictions {
        line_ok(&r.owner)?;
        if r.owner.is_empty() || r.owner == *password {
            return Err(OpError::InvalidRange(
                "A senha de permissões precisa ser diferente da senha de abertura; senão as restrições \
                 não protegem nada."
                    .into(),
            ));
        }
    }
    let n = inputs.len();
    batch(inputs, |i, input, outcome| {
        ctx.report(i as f64 / n as f64, &format!("Protegendo {}", stem(input)));
        match qpdf::encryption(input, &ctx.cancel)? {
            Encryption::None => {}
            Encryption::Password => return Err(OpError::PasswordProtected(input.to_path_buf())),
            Encryption::RestrictionsOnly => {
                return Err(OpError::InvalidRange(format!(
                    "“{}” já tem restrições definidas pelo autor. Remova-as antes com “Remover senha”, \
                     usando a senha de permissões.",
                    crate::error::display_name(input)
                )));
            }
        }
        let total = qpdf::page_count(input, &ctx.cancel)?;
        let job = JobDir::new()?;
        let owner = restrictions.map_or(password, |r| &r.owner);
        let mut args = format!(
            "--encrypt\n--user-password={}\n--owner-password={}\n--bits=256\n",
            password.expose(),
            owner.expose()
        );
        if let Some(r) = restrictions {
            if !r.allow_print {
                args.push_str("--print=none\n");
            }
            if !r.allow_copy {
                args.push_str("--extract=n\n");
            }
            if !r.allow_edit {
                args.push_str("--modify=none\n");
            }
        }
        args.push_str("--\n");
        let args = Secret::new(args);
        let args_file = job.file("opcoes");
        storage::write_private(&args_file, args.expose().as_bytes())?;
        let tmp = job.file("protegido.pdf");
        qpdf::encrypt(input, &args_file, &tmp, &ctx.cancel)?;
        std::fs::remove_file(&args_file).map_err(|e| OpError::io("apagar arquivo temporário", e))?;

        // Verificação: exige senha, abre com ela e tem as mesmas páginas.
        if qpdf::encryption(&tmp, &ctx.cancel)? != Encryption::Password {
            return Err(OpError::OutputInvalid("o arquivo gerado não ficou protegido".into()));
        }
        let pw_file = job.file("senha");
        storage::write_private(&pw_file, password.expose().as_bytes())?;
        let pages = qpdf::page_count_with_password(&tmp, &pw_file, &ctx.cancel)?;
        if pages != total {
            return Err(OpError::OutputInvalid(format!("esperava {total} página(s), mas o arquivo tem {pages}")));
        }
        let base = format!("{} - protegido", stem(input));
        let name = if n == 1 { output_name(dest, &base, "pdf") } else { storage::sanitize_file_name(&base, "pdf") };
        outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
        Ok(())
    })
}

pub fn unlock(inputs: &[PathBuf], password: &Secret, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    line_ok(password)?;
    let n = inputs.len();
    batch(inputs, |i, input, outcome| {
        ctx.report(i as f64 / n as f64, &format!("Desbloqueando {}", stem(input)));
        let kind = qpdf::encryption(input, &ctx.cancel)?;
        if kind == Encryption::None {
            return Err(OpError::NotEncrypted(input.to_path_buf()));
        }
        let job = JobDir::new()?;
        let pw_file = job.file("senha");
        storage::write_private(&pw_file, password.expose().as_bytes())?;
        let check = qpdf::check_password(input, &pw_file, &ctx.cancel)?;
        // Sem a senha de permissões, não se removem restrições do autor.
        if !check.owner && check.restricted {
            return Err(OpError::OwnerPasswordRequired(input.to_path_buf()));
        }
        // Arquivo só com restrições: a senha vazia "abre", mas não é a de permissões.
        if kind == Encryption::RestrictionsOnly && !check.owner {
            return Err(if password.is_empty() {
                OpError::OwnerPasswordRequired(input.to_path_buf())
            } else {
                OpError::WrongPassword(input.to_path_buf())
            });
        }
        let total = qpdf::page_count_with_password(input, &pw_file, &ctx.cancel)?;
        let tmp = job.file("desbloqueado.pdf");
        qpdf::decrypt(input, &pw_file, &tmp, &ctx.cancel)?;
        if qpdf::encryption(&tmp, &ctx.cancel)? != Encryption::None {
            return Err(OpError::OutputInvalid("o arquivo gerado ainda está criptografado".into()));
        }
        verify_pdf(&tmp, total, ctx)?;
        let base = format!("{} - sem senha", stem(input));
        let name = if n == 1 { output_name(dest, &base, "pdf") } else { storage::sanitize_file_name(&base, "pdf") };
        outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
        Ok(())
    })
}

/// Cópia sem criptografia de um PDF com senha, para as demais ferramentas
/// trabalharem nela. Fica numa pasta própria (0700) com o mesmo nome do
/// original — os resultados recebem os nomes de sempre — e é apagada quando
/// esta estrutura é descartada (ou, depois de uma queda, na abertura seguinte).
pub struct UnlockedCopy {
    _dir: JobDir,
    pub path: PathBuf,
    pub original: PathBuf,
}

/// Abre `input` com a senha e grava a cópia desbloqueada. Só aceita PDFs que
/// exigem senha para abrir; se o autor restringiu impressão, cópia ou
/// edição, exige a senha de permissões (como “Remover senha”): a senha de
/// leitura não serve para contornar as restrições.
pub fn unlocked_copy(input: &Path, password: &Secret, cancel: &CancelToken) -> OpResult<UnlockedCopy> {
    line_ok(password)?;
    if qpdf::encryption(input, cancel)? != Encryption::Password {
        return Err(OpError::NotEncrypted(input.to_path_buf()));
    }
    let dir = JobDir::new_in(&storage::unlocked_root())?;
    let pw_file = dir.file(".senha");
    storage::write_private(&pw_file, password.expose().as_bytes())?;
    let check = qpdf::check_password(input, &pw_file, cancel);
    let result = check.and_then(|check| {
        if !check.owner && check.restricted {
            return Err(OpError::OwnerPasswordRequired(input.to_path_buf()));
        }
        let total = qpdf::page_count_with_password(input, &pw_file, cancel)?;
        let name = input.file_name().map(PathBuf::from).unwrap_or_else(|| "documento.pdf".into());
        let path = dir.path().join(name);
        qpdf::decrypt(input, &pw_file, &path, cancel)?;
        if qpdf::encryption(&path, cancel)? != Encryption::None {
            return Err(OpError::OutputInvalid("a cópia desbloqueada ainda está criptografada".into()));
        }
        let pages = qpdf::page_count(&path, cancel)?;
        if pages != total {
            return Err(OpError::OutputInvalid(format!("esperava {total} página(s), mas a cópia tem {pages}")));
        }
        Ok(path)
    });
    let _ = std::fs::remove_file(&pw_file);
    Ok(UnlockedCopy { _dir: dir, path: result?, original: input.to_path_buf() })
}
