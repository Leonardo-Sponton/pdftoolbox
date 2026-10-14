//! O idioma da interface muda só as mensagens: os arquivos gerados (nomes,
//! páginas e o que é escrito neles) são os mesmos em português e inglês.

mod common;

use std::path::{Path, PathBuf};

use common::{Sandbox, page_texts, run};
use pdftoolbox::i18n::{Language, with_language};
use pdftoolbox::operations::{NumberFormat, NumberStyle, Outcome, Position, Request, SplitMode};

/// Caminhos das saídas relativos à pasta de destino, para comparar sandboxes.
fn relative(outcome: &Outcome, root: &Path) -> Vec<PathBuf> {
    outcome.outputs.iter().map(|o| o.path.strip_prefix(root).unwrap().to_path_buf()).collect()
}

/// Executa a mesma sequência de operações num idioma; devolve nomes das
/// saídas, texto de cada página delas e as notas.
fn run_in(language: Language) -> (Vec<PathBuf>, Vec<Vec<String>>, Vec<String>) {
    with_language(language, || {
        let sb = Sandbox::new();
        let doc = sb.pdf("relatório.pdf", &["Um", "Dois", "Tres", "Quatro", "Cinco"]);
        let split = run(Request::Split { input: doc.clone(), mode: SplitMode::EachPage }, &sb.dest()).unwrap();
        let style = NumberStyle {
            position: Position::BottomCenter,
            format: NumberFormat::PageOfTotal,
            first_number: 1,
            size: 11.0,
            margin_mm: 10.0,
        };
        let numbered = run(Request::NumberPages { inputs: vec![doc.clone()], spec: None, style }, &sb.dest()).unwrap();
        let removed = run(Request::Remove { input: doc, spec: "2".into() }, &sb.dest()).unwrap();
        let mut names = Vec::new();
        let mut texts = Vec::new();
        let mut notes = Vec::new();
        for outcome in [&split, &numbered, &removed] {
            names.extend(relative(outcome, &sb.root));
            texts.extend(outcome.outputs.iter().map(|o| page_texts(&o.path)));
            notes.extend(outcome.notes.iter().cloned());
        }
        (names, texts, notes)
    })
}

#[test]
fn output_files_do_not_depend_on_the_interface_language() {
    let (pt_names, pt_texts, pt_notes) = run_in(Language::Portuguese);
    let (en_names, en_texts, en_notes) = run_in(Language::English);
    assert_eq!(pt_names, en_names, "nomes dos arquivos gerados");
    assert_eq!(pt_texts, en_texts, "conteúdo das páginas");
    // A numeração escolhida grava o mesmo texto nos dois idiomas.
    assert!(en_texts.iter().flatten().any(|t| t.contains("Página 1 de 5")), "{en_texts:?}");
    // Já as mensagens para a pessoa seguem o idioma.
    assert_eq!(pt_notes, ["1 página removida; restam 4."]);
    assert_eq!(en_notes, ["1 page removed; 4 left."]);
}

#[test]
fn errors_are_reported_in_the_interface_language() {
    let sb = Sandbox::new();
    let doc = sb.pdf("curto.pdf", &["A", "B"]);
    let request = || Request::Remove { input: doc.clone(), spec: "7".into() };
    let error = run(request(), &sb.dest()).unwrap_err();
    // A mensagem é montada ao ser exibida: o mesmo erro aparece no idioma
    // atual da interface, mesmo que ele tenha mudado depois da falha.
    assert_eq!(error.to_string(), "A página 7 não existe: o documento tem 2 páginas.");
    with_language(Language::English, || {
        assert_eq!(error.to_string(), "Page 7 does not exist: the document has 2 pages.");
        assert_eq!(error.title(), "Invalid page selection");
        let en = run(request(), &sb.dest()).unwrap_err();
        assert_eq!(en.to_string(), error.to_string());
    });
}
