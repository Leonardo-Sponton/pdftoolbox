#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use pdftoolbox::operations::{Destination, Outcome, Request, execute};
use pdftoolbox::pdfwriter::{A4_POINTS, text_pdf};
use pdftoolbox::storage::JobDir;
use pdftoolbox::{OpResult, jobs::JobCtx};

/// Pasta de teste apagada automaticamente; o nome tem espaço e Unicode
/// de propósito, para exercitar caminhos incomuns em todos os testes.
pub struct Sandbox {
    dir: JobDir,
    pub root: PathBuf,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = JobDir::new_in(&std::env::temp_dir().join("pdftoolbox-testes")).unwrap();
        let root = dir.path().join("pasta com espaço — ção 日本");
        std::fs::create_dir(&root).unwrap();
        Sandbox { dir, root }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// PDF de texto com uma página por rótulo.
    pub fn pdf(&self, name: &str, labels: &[&str]) -> PathBuf {
        let p = self.path(name);
        text_pdf(&p, labels, A4_POINTS).unwrap();
        p
    }

    pub fn dest(&self) -> Destination {
        Destination { dir: self.root.clone(), name: None }
    }

    pub fn subdir(&self, name: &str) -> PathBuf {
        let p = self.root.join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    pub fn _keep(&self) -> &JobDir {
        &self.dir
    }
}

pub fn run(req: Request, dest: &Destination) -> OpResult<Outcome> {
    execute(req, dest, &JobCtx::silent())
}

/// Texto de cada página, via pdftotext (Poppler).
pub fn page_texts(pdf: &Path) -> Vec<String> {
    let out = Command::new("pdftotext").args(["-layout"]).arg(pdf).arg("-").output().unwrap();
    assert!(out.status.success(), "pdftotext falhou");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pages: Vec<String> = text.split('\x0c').map(|p| p.trim().to_string()).collect();
    if pages.last().is_some_and(String::is_empty) {
        pages.pop();
    }
    pages
}

/// Rotação de cada página, via pdfinfo.
pub fn rotations(pdf: &Path) -> Vec<u32> {
    let out = Command::new("pdfinfo").args(["-f", "1", "-l", "9999"]).arg(pdf).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("Page") && l.contains("rot:"))
        .map(|l| l.rsplit(' ').next().unwrap().parse().unwrap())
        .collect()
}

/// Tamanhos das páginas em pontos (largura, altura), via pdfinfo.
pub fn page_sizes(pdf: &Path) -> Vec<(f32, f32)> {
    let out = Command::new("pdfinfo").args(["-f", "1", "-l", "9999"]).arg(pdf).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("Page") && l.contains("size:"))
        .map(|l| {
            let nums: Vec<f32> =
                l.split_once("size:").unwrap().1.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            (nums[0], nums[1])
        })
        .collect()
}

pub fn has(tool: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {tool}")]).output().is_ok_and(|o| o.status.success())
}
