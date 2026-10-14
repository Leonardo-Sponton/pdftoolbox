//! Adaptadores para executáveis externos (qpdf, Poppler, Ghostscript).
//!
//! Regras: argumentos sempre separados (nunca via shell), caminhos sempre
//! absolutos (um nome iniciado por “-” não vira opção), cancelamento por
//! `kill` do processo filho e captura de código de saída e stderr.

pub mod ghostscript;
pub mod poppler;
pub mod qpdf;

use std::ffi::OsStr;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{OpError, OpResult};
use crate::jobs::CancelToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Qpdf,
    Pdftoppm,
    Pdfinfo,
    Ghostscript,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Qpdf, Tool::Pdftoppm, Tool::Pdfinfo, Tool::Ghostscript];

    pub fn program(self) -> &'static str {
        match self {
            Tool::Qpdf => "qpdf",
            Tool::Pdftoppm => "pdftoppm",
            Tool::Pdfinfo => "pdfinfo",
            Tool::Ghostscript => "gs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Qpdf => "qpdf",
            Tool::Pdftoppm => "pdftoppm (Poppler)",
            Tool::Pdfinfo => "pdfinfo (Poppler)",
            Tool::Ghostscript => "Ghostscript",
        }
    }

    pub fn purpose(self) -> &'static str {
        match self {
            Tool::Qpdf => "Mesclar, dividir, extrair, remover, girar, organizar, compressão sem perda e verificação",
            Tool::Pdftoppm => "Miniaturas e conversão de PDF para PNG, JPEG e TIFF",
            Tool::Pdfinfo => "Leitura de informações das páginas",
            Tool::Ghostscript => "Compressão com redução de imagens (níveis Recomendada e Máxima)",
        }
    }

    /// Pacote por família de distribuição: (Debian/Ubuntu, Arch, Fedora).
    fn packages(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Tool::Qpdf => ("qpdf", "qpdf", "qpdf"),
            Tool::Pdftoppm | Tool::Pdfinfo => ("poppler-utils", "poppler", "poppler-utils"),
            Tool::Ghostscript => ("ghostscript", "ghostscript", "ghostscript"),
        }
    }

    pub fn install_command(self) -> String {
        let (deb, arch, fedora) = self.packages();
        match distro_family() {
            Distro::Debian => format!("sudo apt install {deb}"),
            Distro::Arch => format!("sudo pacman -S {arch}"),
            Distro::Fedora => format!("sudo dnf install {fedora}"),
            Distro::Unknown => format!("Debian/Ubuntu: sudo apt install {deb} · Arch: sudo pacman -S {arch}"),
        }
    }

    /// Procura o executável no PATH.
    pub fn find(self) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).map(|dir| dir.join(self.program())).find(|p| is_executable(p))
    }

    pub fn require(self) -> OpResult<PathBuf> {
        self.find().ok_or(OpError::MissingTool(self))
    }

    pub fn version(self) -> Option<String> {
        let exe = self.find()?;
        let arg = match self {
            Tool::Qpdf | Tool::Ghostscript => "--version",
            Tool::Pdftoppm | Tool::Pdfinfo => "-v",
        };
        let out = Command::new(exe).arg(arg).stdin(Stdio::null()).output().ok()?;
        let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
        let line = String::from_utf8_lossy(&text).lines().next()?.trim().to_string();
        // "qpdf version 12.4.2", "pdftoppm version 26.08.0", "10.08.0"
        Some(line.rsplit(' ').next().unwrap_or(&line).to_string())
    }
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
    pub elapsed: Duration,
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
    cancel.check()?;
    let start = Instant::now();
    let mut child = Command::new(exe)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| OpError::io(format!("iniciar {}", tool.program()), e))?;

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
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_reader.join();
            let _ = err_reader.join();
            return Err(OpError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(OpError::io(format!("aguardar {}", tool.program()), e)),
        }
    };

    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&stderr[stderr.len().saturating_sub(STDERR_LIMIT)..]).into_owned();
    Ok(ToolOutput { code: status.code(), stdout, stderr, elapsed: start.elapsed() })
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
