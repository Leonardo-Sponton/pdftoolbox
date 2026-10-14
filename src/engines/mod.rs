//! Adaptadores para executáveis externos (qpdf, Poppler, Ghostscript).
//!
//! Regras: argumentos sempre separados (nunca via shell), caminhos sempre
//! absolutos (um nome iniciado por “-” não vira opção), cancelamento por
//! `kill` do processo filho e captura de código de saída e stderr.

pub mod ghostscript;
pub mod libreoffice;
pub mod poppler;
pub mod qpdf;
pub mod tesseract;

use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use crate::error::{OpError, OpResult};
use crate::jobs::CancelToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Qpdf,
    Pdftoppm,
    Pdfinfo,
    Pdftotext,
    Ghostscript,
    LibreOffice,
    Tesseract,
}

impl Tool {
    pub const ALL: [Tool; 7] = [
        Tool::Qpdf,
        Tool::Pdftoppm,
        Tool::Pdfinfo,
        Tool::Pdftotext,
        Tool::Ghostscript,
        Tool::LibreOffice,
        Tool::Tesseract,
    ];

    pub fn program(self) -> &'static str {
        match self {
            Tool::Qpdf => "qpdf",
            Tool::Pdftoppm => "pdftoppm",
            Tool::Pdfinfo => "pdfinfo",
            Tool::Pdftotext => "pdftotext",
            Tool::Ghostscript => "gs",
            Tool::LibreOffice => "soffice",
            Tool::Tesseract => "tesseract",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Qpdf => "qpdf",
            Tool::Pdftoppm => "pdftoppm (Poppler)",
            Tool::Pdfinfo => "pdfinfo (Poppler)",
            Tool::Pdftotext => "pdftotext (Poppler)",
            Tool::Ghostscript => "Ghostscript",
            Tool::LibreOffice => "LibreOffice",
            Tool::Tesseract => "Tesseract",
        }
    }

    pub fn purpose(self) -> &'static str {
        match self {
            Tool::Qpdf => "Mesclar, dividir, extrair, remover, girar, organizar, compressão sem perda e verificação",
            Tool::Pdftoppm => "Miniaturas e conversão de PDF para PNG, JPEG e TIFF",
            Tool::Pdfinfo => "Leitura de informações das páginas e dos metadados",
            Tool::Pdftotext => "Extração de texto",
            Tool::Ghostscript => "Compressão com redução de imagens (níveis Recomendada e Máxima)",
            Tool::LibreOffice => "Conversão de documentos, planilhas e apresentações para PDF",
            Tool::Tesseract => "Reconhecimento de texto (OCR) em documentos digitalizados",
        }
    }

    /// Pacote por família de distribuição: (Debian/Ubuntu, Arch, Fedora).
    fn packages(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Tool::Qpdf => ("qpdf", "qpdf", "qpdf"),
            Tool::Pdftoppm | Tool::Pdfinfo | Tool::Pdftotext => ("poppler-utils", "poppler", "poppler-utils"),
            Tool::Ghostscript => ("ghostscript", "ghostscript", "ghostscript"),
            Tool::LibreOffice => (
                "libreoffice-writer libreoffice-calc libreoffice-impress",
                "libreoffice-fresh",
                "libreoffice-writer libreoffice-calc libreoffice-impress",
            ),
            Tool::Tesseract => (
                "tesseract-ocr tesseract-ocr-por",
                "tesseract tesseract-data-por tesseract-data-eng",
                "tesseract tesseract-langpack-por",
            ),
        }
    }

    pub fn install_command(self) -> String {
        let (deb, arch, fedora) = self.packages();
        install_line(deb, arch, fedora)
    }

    /// Procura o executável no PATH.
    pub fn find(self) -> Option<PathBuf> {
        find_in_path(self.program())
    }

    pub fn require(self) -> OpResult<PathBuf> {
        self.find().ok_or(OpError::MissingTool(self))
    }

    pub fn version(self) -> Option<String> {
        let exe = self.find()?;
        let arg = match self {
            Tool::Qpdf | Tool::Ghostscript | Tool::LibreOffice | Tool::Tesseract => "--version",
            Tool::Pdftoppm | Tool::Pdfinfo | Tool::Pdftotext => "-v",
        };
        let out = Command::new(exe).arg(arg).stdin(Stdio::null()).output().ok()?;
        let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
        let line = String::from_utf8_lossy(&text).lines().next()?.trim().to_string();
        // "qpdf version 12.4.2", "pdftoppm version 26.08.0", "10.08.0",
        // "LibreOffice 26.8.1.2 a1b2c3…"
        if self == Tool::LibreOffice {
            return line.split_whitespace().nth(1).map(str::to_string);
        }
        Some(line.rsplit(' ').next().unwrap_or(&line).to_string())
    }
}

/// Comando de instalação para a distribuição atual (pacotes por família).
pub fn install_line(deb: &str, arch: &str, fedora: &str) -> String {
    match distro_family() {
        Distro::Debian => format!("sudo apt install {deb}"),
        Distro::Arch => format!("sudo pacman -S {arch}"),
        Distro::Fedora => format!("sudo dnf install {fedora}"),
        Distro::Unknown => format!("Debian/Ubuntu: sudo apt install {deb} · Arch: sudo pacman -S {arch}"),
    }
}

/// Primeiro executável chamado `program` nas pastas do PATH.
fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(program)).find(|p| is_executable(p))
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

enum Distro {
    Debian,
    Arch,
    Fedora,
    Unknown,
}

fn distro_family() -> Distro {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let ids: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("ID=").or_else(|| l.strip_prefix("ID_LIKE=")))
        .flat_map(|v| v.trim_matches('"').split_whitespace())
        .collect();
    if ids.iter().any(|i| matches!(*i, "debian" | "ubuntu")) {
        Distro::Debian
    } else if ids.iter().any(|i| matches!(*i, "arch")) {
        Distro::Arch
    } else if ids.iter().any(|i| matches!(*i, "fedora" | "rhel")) {
        Distro::Fedora
    } else {
        Distro::Unknown
    }
}

pub struct ToolOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Limite de stderr guardado (o fim é o mais útil).
const STDERR_LIMIT: usize = 8 * 1024;

/// Executa uma ferramenta e espera o fim, com cancelamento.
/// Não interpreta o código de saída: isso é responsabilidade do adaptador.
pub fn run<I, S>(tool: Tool, args: I, cancel: &CancelToken) -> OpResult<ToolOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let exe = tool.require()?;
    run_command(tool, Command::new(exe), args, cancel)
}

/// Executa sem acesso à rede quando o sistema permite (`unshare -rn`, sem
/// privilégios). Para motores que poderiam buscar recursos externos citados no
/// documento (LibreOffice). Devolve também se o isolamento foi aplicado.
pub fn run_offline<I, S>(tool: Tool, args: I, cancel: &CancelToken) -> OpResult<(ToolOutput, bool)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let exe = tool.require()?;
    match unshare_path() {
        Some(unshare) => {
            let mut cmd = Command::new(unshare);
            cmd.arg("--user").arg("--map-current-user").arg("--net").arg("--").arg(exe);
            Ok((run_command(tool, cmd, args, cancel)?, true))
        }
        None => Ok((run_command(tool, Command::new(exe), args, cancel)?, false)),
    }
}

/// `unshare`, se existir e conseguir criar um namespace de rede sem privilégios
/// neste sistema (testado uma vez por execução).
fn unshare_path() -> Option<&'static PathBuf> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let exe = find_in_path("unshare")?;
            let ok = Command::new(&exe)
                .args(["--user", "--map-current-user", "--net", "--", "true"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            ok.then_some(exe)
        })
        .as_ref()
}

/// Se a conversão de documentos roda isolada da rede neste sistema.
pub fn network_isolation_available() -> bool {
    unshare_path().is_some()
}

fn run_command<I, S>(tool: Tool, mut cmd: Command, args: I, cancel: &CancelToken) -> OpResult<ToolOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    cancel.check()?;
    if tool == Tool::Tesseract {
        // Uma thread por processo: as páginas já são reconhecidas em paralelo.
        cmd.env("OMP_THREAD_LIMIT", "1");
    }
    // Grupo de processos próprio: o cancelamento encerra também os filhos
    // (o LibreOffice, por exemplo, roda em mais de um processo).
    let mut child = cmd
        .args(args)
        .process_group(0)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| OpError::io(format!("iniciar {}", tool.program()), e))?;

    let pid = child.id();
    running().lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(pid);
    let unregister = || running().lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|p| *p != pid);
    let mut out_pipe = child.stdout.take().expect("stdout piped");
    let mut err_pipe = child.stderr.take().expect("stderr piped");
    let out_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });

    let status = loop {
        if cancel.is_cancelled() {
            kill_group(pid);
            let _ = child.kill();
            let _ = child.wait();
            unregister();
            let _ = out_reader.join();
            let _ = err_reader.join();
            return Err(OpError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                unregister();
                return Err(OpError::io(format!("aguardar {}", tool.program()), e));
            }
        }
    };
    // Já colhido: o número do grupo pode ser reutilizado, então não se envia
    // mais nenhum sinal a ele.
    unregister();

    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&stderr[stderr.len().saturating_sub(STDERR_LIMIT)..]).into_owned();
    Ok(ToolOutput { code: status.code(), stdout, stderr })
}

/// Grupos de processos em execução, para encerrá-los se o programa fechar
/// no meio de uma tarefa (o LibreOffice, por exemplo, não morre sozinho).
fn running() -> &'static std::sync::Mutex<Vec<u32>> {
    static RUNNING: OnceLock<std::sync::Mutex<Vec<u32>>> = OnceLock::new();
    RUNNING.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Encerra todos os processos externos ainda em execução. Chamado ao sair.
pub fn kill_all_running() {
    let groups = std::mem::take(&mut *running().lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for pid in groups {
        kill_group(pid);
    }
}

fn kill_group(pid: u32) {
    // SAFETY: `killpg` só envia um sinal; o grupo foi criado para este filho.
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// Executa e exige um dos códigos de saída aceitos.
pub fn run_ok<I, S>(tool: Tool, args: I, ok_codes: &[i32], cancel: &CancelToken) -> OpResult<ToolOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let out = run(tool, args, cancel)?;
    match out.code {
        Some(c) if ok_codes.contains(&c) => Ok(out),
        code => Err(OpError::ToolFailed { tool, code, stderr: out.stderr }),
    }
}
