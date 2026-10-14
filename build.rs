//! Compila os ícones simbólicos próprios em um GResource embutido no binário.
//! Requer `glib-compile-resources` (pacote libglib2.0-dev-bin no Debian/Ubuntu,
//! glib2 no Arch), já necessário para compilar qualquer aplicativo GTK.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let dir = "data/resources";
    let xml = format!("{dir}/pdftoolbox.gresource.xml");
    println!("cargo::rerun-if-changed={dir}");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pdftoolbox.gresource");
    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir")
        .arg(dir)
        .arg("--target")
        .arg(&out)
        .arg(&xml)
        .status()
        .expect("glib-compile-resources não encontrado (instale libglib2.0-dev-bin ou glib2)");
    assert!(status.success(), "glib-compile-resources falhou");
}
