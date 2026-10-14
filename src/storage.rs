//! Arquivos temporários por tarefa e gravação segura do resultado.
//!
//! - Cada tarefa tem um diretório próprio (0700) apagado ao terminar, inclusive
//!   em falha ou cancelamento (via `Drop`).
//! - O resultado é copiado para um arquivo oculto `.part` no destino e só então
//!   ligado ao nome final com `link()`, que falha se o nome já existir: um
//!   arquivo existente — inclusive o original — nunca é sobrescrito. Em caso de
//!   colisão, usa-se “nome (2).pdf”, “nome (3).pdf”…

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{OpError, OpResult};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_token() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{}-{:x}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed), nanos & 0xffff_ffff)
}

/// `$XDG_CACHE_HOME/pdftoolbox` (ou `~/.cache/pdftoolbox`).
pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("pdftoolbox")
}

/// `$XDG_CONFIG_HOME/pdftoolbox` (ou `~/.config/pdftoolbox`).
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("pdftoolbox")
}

fn jobs_root() -> PathBuf {
    cache_dir().join("tarefas")
}

/// Diretório temporário exclusivo de uma tarefa.
pub struct JobDir {
    path: PathBuf,
}

impl JobDir {
    pub fn new() -> OpResult<Self> {
        Self::new_in(&jobs_root())
    }

    pub fn new_in(root: &Path) -> OpResult<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)
            .map_err(|e| OpError::io("criar a pasta temporária", e))?;
        let path = root.join(unique_token());
        fs::DirBuilder::new().mode(0o700).create(&path).map_err(|e| OpError::io("criar a pasta temporária", e))?;
        Ok(JobDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    pub fn subdir(&self, name: &str) -> OpResult<PathBuf> {
        let p = self.path.join(name);
        fs::create_dir(&p).map_err(|e| OpError::io("criar a pasta temporária", e))?;
        Ok(p)
    }
}

impl Drop for JobDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Remove diretórios de tarefas deixados por processos que já terminaram
/// (por exemplo, após uma queda do programa).
pub fn cleanup_stale_jobs() {
    let Ok(entries) = fs::read_dir(jobs_root()) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let alive = name
            .split('-')
            .next()
            .and_then(|pid| pid.parse::<u32>().ok())
            .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists());
        if !alive {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Remove os diretórios de tarefas deste processo. Chamado ao sair, depois
/// de encerrar os motores: tarefas interrompidas não deixam páginas
/// renderizadas e outros temporários para trás.
pub fn cleanup_own_jobs() {
    let Ok(entries) = fs::read_dir(jobs_root()) else { return };
    let prefix = format!("{}-", std::process::id());
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Grava um arquivo legível só pelo usuário (0600). Usado para entregar senhas
/// ao qpdf dentro da pasta da tarefa, que é apagada ao fim.
pub fn write_private(path: &Path, data: &[u8]) -> OpResult<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| OpError::io("criar arquivo temporário", e))?;
    f.write_all(data).map_err(|e| OpError::io("gravar arquivo temporário", e))
}

/// Torna um nome digitado pelo usuário seguro como nome de arquivo e garante a extensão.
pub fn sanitize_file_name(name: &str, extension: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\0' || c.is_control() { '_' } else { c }).collect();
    let mut cleaned = cleaned.trim().trim_start_matches('.').trim().to_string();
    if cleaned.is_empty() {
        cleaned = "documento".into();
    }
    let ext = format!(".{extension}");
    if !cleaned.to_lowercase().ends_with(&ext) {
        cleaned.push_str(&ext);
    }
    // Limite de 255 bytes por componente no ext4; corta respeitando UTF-8.
    while cleaned.len() > 240 {
        let stem_end = cleaned.len() - ext.len();
        let mut cut = stem_end - 1;
        while !cleaned.is_char_boundary(cut) {
            cut -= 1;
        }
        cleaned.replace_range(cut..stem_end, "");
    }
    cleaned
}

/// Nome do arquivo sem extensão, para compor nomes de saída.
pub fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "documento".into())
}

fn numbered(name: &str, n: u32) -> String {
    if n <= 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({n}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({n})"),
    }
}

/// Primeiro caminho livre em `dir` para `name`, sem criar nada.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    (1..).map(|n| dir.join(numbered(name, n))).find(|p| fs::symlink_metadata(p).is_err()).expect("sequência infinita")
}

/// Verifica se é possível criar arquivos em `dir`.
pub fn check_writable(dir: &Path) -> OpResult<()> {
    let meta = fs::metadata(dir).map_err(|e| OpError::io_path("acessar a pasta de destino", dir, e))?;
    if !meta.is_dir() {
        return Err(OpError::io_path(
            "usar a pasta de destino",
            dir,
            io::Error::new(io::ErrorKind::NotADirectory, "não é uma pasta"),
        ));
    }
    let probe = dir.join(format!(".pdftoolbox-teste-{}", unique_token()));
    fs::File::create(&probe).map_err(|e| OpError::io_path("gravar na pasta", dir, e))?;
    let _ = fs::remove_file(probe);
    Ok(())
}

/// Move o arquivo temporário `tmp` para `dir/name` sem sobrescrever nada.
/// Devolve o caminho final (que pode ter recebido um sufixo numérico).
pub fn commit(tmp: &Path, dir: &Path, name: &str) -> OpResult<PathBuf> {
    // Nome curto: o nome final pode ter até ~240 bytes e o limite é 255.
    let part = dir.join(format!(".pdftoolbox-{}.part", unique_token()));
    let result = commit_inner(tmp, &part, dir, name);
    let _ = fs::remove_file(&part);
    result
}

fn commit_inner(tmp: &Path, part: &Path, dir: &Path, name: &str) -> OpResult<PathBuf> {
    let write_err = |e| OpError::io_path("gravar o resultado em", dir, e);
    if fs::rename(tmp, part).is_err() {
        // Sistemas de arquivos diferentes: copiar.
        fs::copy(tmp, part).map_err(write_err)?;
    }
    fs::File::open(part).and_then(|f| f.sync_all()).map_err(write_err)?;
    let _ = fs::set_permissions(part, fs::Permissions::from_mode(0o644 & !umask()));

    for n in 1u32.. {
        let target = dir.join(numbered(name, n));
        match fs::hard_link(part, &target) {
            Ok(()) => {
                sync_dir(dir);
                return Ok(target);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                // Sistemas sem hard links (FAT/exFAT): verificação + rename.
                if fs::symlink_metadata(&target).is_ok() {
                    continue;
                }
                fs::rename(part, &target).map_err(write_err)?;
                sync_dir(dir);
                return Ok(target);
            }
        }
    }
    unreachable!()
}

fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

fn umask() -> u32 {
    // Lido de /proc para não alterar a máscara do processo.
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| l.strip_prefix("Umask:")).and_then(|v| u32::from_str_radix(v.trim(), 8).ok())
        })
        .unwrap_or(0o022)
}

/// Cria uma subpasta nova em `dir` (com sufixo numérico em caso de colisão).
pub fn create_unique_dir(dir: &Path, name: &str) -> OpResult<PathBuf> {
    for n in 1u32.. {
        let target = if n == 1 { dir.join(name) } else { dir.join(format!("{name} ({n})")) };
        match fs::create_dir(&target) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(OpError::io_path("criar a pasta", &target, e)),
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_file_name("a/b", "pdf"), "a_b.pdf");
        assert_eq!(sanitize_file_name("  ", "pdf"), "documento.pdf");
        assert_eq!(sanitize_file_name("..oculto", "pdf"), "oculto.pdf");
        assert_eq!(sanitize_file_name("Relatório.PDF", "pdf"), "Relatório.PDF");
        assert!(sanitize_file_name(&"ç".repeat(300), "pdf").len() <= 240);
    }

    #[test]
    fn numbers_before_extension() {
        assert_eq!(numbered("a.pdf", 1), "a.pdf");
        assert_eq!(numbered("a.pdf", 2), "a (2).pdf");
        assert_eq!(numbered("pasta", 3), "pasta (3)");
    }
}
