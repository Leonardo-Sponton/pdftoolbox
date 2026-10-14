//! Gera documentos de amostra para testar a interface manualmente:
//! `cargo run --example amostras -- <pasta>`

use std::path::PathBuf;

use pdftoolbox::pdfwriter::{A4_POINTS, text_pdf};

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "amostras".into()));
    std::fs::create_dir_all(&dir).expect("criar pasta");
    let docs: [(&str, usize); 3] = [("Relatório anual.pdf", 12), ("Contrato & anexos.pdf", 3), ("Nota fiscal.pdf", 1)];
    for (name, pages) in docs {
        let labels: Vec<String> = (1..=pages).map(|i| format!("{name} - pagina {i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        text_pdf(&dir.join(name), &refs, A4_POINTS).expect("gravar amostra");
    }
    println!("Amostras em {}", dir.display());
}
