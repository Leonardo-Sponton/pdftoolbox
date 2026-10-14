//! Núcleo do PDFToolbox: operações, motores externos, tarefas e armazenamento.
//!
//! Este crate não depende de GTK; a interface (em `main.rs` e `ui/`) apenas
//! chama as operações daqui em uma thread de trabalho.

// Primeiro: as macros `tr!` e `trn!` ficam visíveis nos módulos seguintes.
#[macro_use]
pub mod i18n;

pub mod annotate;
pub mod catalog;
pub mod deskew;
pub mod engines;
pub mod error;
pub mod human;
pub mod jobs;
pub mod operations;
pub mod pages;
pub mod pdfwriter;
pub mod preview;
pub mod secret;
pub mod stamp;
pub mod storage;

pub use error::{OpError, OpResult};
