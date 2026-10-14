//! OCR com o Tesseract sobre “digitalizações” geradas aqui: PDFs de texto
//! renderizados como imagem e remontados só com as imagens.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;
use pdftoolbox::OpError;
use pdftoolbox::operations::{ImageLayout, OcrLanguage, OcrOptions, PageSize, Request};

fn ocr_ready() -> bool {
    let ok = has("tesseract") && {
        let out = Command::new("tesseract").arg("--list-langs").output().unwrap();
        let langs = String::from_utf8_lossy(&out.stdout).into_owned();
        langs.lines().any(|l| l == "por") && langs.lines().any(|l| l == "eng")
    };
    if !ok {
        eprintln!("tesseract com por+eng não instalado: teste ignorado");
    }
    ok
}

const OPTS: OcrOptions =
    OcrOptions { language: OcrLanguage::PortugueseEnglish, skip_text_pages: true, straighten: true };

/// “Digitaliza” `labels` (uma página cada): renderiza a 150 ppp e monta um PDF
/// só com as imagens, sem nenhum texto selecionável.
fn scan(sb: &Sandbox, name: &str, labels: &[&str]) -> PathBuf {
    let text = sb.pdf(&format!("{name}-original.pdf"), labels);
    let dir = sb.subdir(&format!("{name}-paginas"));
    let ok =
        Command::new("pdftoppm").args(["-r", "150", "-png"]).arg(&text).arg(dir.join("p")).status().unwrap().success();
    assert!(ok);
    let mut images: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    images.sort();
    let mut dest = sb.dest();
    dest.name = Some(format!("{name}.pdf"));
    let layout = ImageLayout { size: PageSize::A4, margin_mm: 0.0 };
    let out = run(Request::ImagesToPdf { inputs: images, layout, one_file: true }, &dest).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_file(&text).unwrap();
    let pdf = out.outputs[0].path.clone();
    assert!(page_texts(&pdf).iter().all(String::is_empty), "a digitalização não pode ter texto");
    pdf
}

fn image_count(pdf: &Path) -> usize {
    let out = Command::new("pdfimages").arg("-list").arg(pdf).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().skip(2).count()
}

fn normalized(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

#[test]
fn scanned_pages_become_searchable_without_touching_the_images() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let doc = scan(&sb, "digitalizado", &["Contrato de locacao residencial", "Pagamento mensal de aluguel"]);
    let out = run(Request::Ocr { inputs: vec![doc.clone()], options: OPTS }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert!(pdf.file_name().unwrap().to_string_lossy().ends_with(" - OCR.pdf"));
    let texts: Vec<String> = page_texts(pdf).iter().map(|t| normalized(t)).collect();
    assert_eq!(texts.len(), 2);
    assert!(texts[0].contains("contrato de locacao residencial"), "página 1: {:?}", texts[0]);
    assert!(texts[1].contains("pagamento mensal de aluguel"), "página 2: {:?}", texts[1]);
    // As imagens são as mesmas (nenhuma rasterização nova) e o arquivo quase não cresce.
    assert_eq!(image_count(pdf), image_count(&doc));
    let (before, after) = (std::fs::metadata(&doc).unwrap().len(), std::fs::metadata(pdf).unwrap().len());
    assert!(after < before + 2 * 40_000, "cresceu demais: {before} → {after}");
    assert_eq!(page_sizes(pdf), page_sizes(&doc));
    assert!(out.notes.iter().any(|n| n.contains("em 2 de 2 páginas analisadas")), "{:?}", out.notes);
    assert!(out.notes.iter().any(|n| n.contains("pode errar")));
}

#[test]
fn text_lands_over_the_word_in_the_image() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let doc = scan(&sb, "posicao", &["Posicionamento"]);
    let out = run(Request::Ocr { inputs: vec![doc], options: OPTS }, &sb.dest()).unwrap();
    let html = Command::new("pdftotext").args(["-bbox"]).arg(&out.outputs[0].path).arg("-").output().unwrap();
    let html = String::from_utf8_lossy(&html.stdout).into_owned();
    let tag = html.split("<word ").nth(1).expect("uma palavra reconhecida");
    let attr = |name: &str| -> f32 {
        let i = tag.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
        tag[i..].split('"').next().unwrap().parse().unwrap()
    };
    // `text_pdf` escreve em 24 pt a partir de x = 56, com a linha de base a 80 pt do topo.
    assert!((attr("xMin") - 56.0).abs() < 6.0, "xMin {}", attr("xMin"));
    let (top, bottom) = (attr("yMin"), attr("yMax"));
    assert!(top > 50.0 && top < 70.0 && bottom > 75.0 && bottom < 90.0, "y {top}–{bottom}");
}

#[test]
fn only_pages_without_text_are_recognized() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let typed = sb.pdf("digitado.pdf", &["Pagina digitada original"]);
    let scanned = scan(&sb, "escaneada", &["Pagina escaneada depois"]);
    let mut dest = sb.dest();
    dest.name = Some("misto".into());
    let mixed = run(Request::Merge { inputs: vec![typed, scanned] }, &dest).unwrap().outputs[0].path.clone();

    let out = run(Request::Ocr { inputs: vec![mixed.clone()], options: OPTS }, &sb.dest()).unwrap();
    let texts: Vec<String> = page_texts(&out.outputs[0].path).iter().map(|t| normalized(t)).collect();
    assert_eq!(texts[0], "pagina digitada original", "página com texto ficou como estava, sem duplicar");
    assert!(texts[1].contains("pagina escaneada depois"), "{:?}", texts[1]);
    assert!(
        out.notes.iter().any(|n| n.contains("em 1 de 1 página analisada") && n.contains("1 página já tinha texto")),
        "{:?}",
        out.notes
    );

    // Sem pular, a página digitada também é reconhecida (texto em dobro).
    let all = OcrOptions { skip_text_pages: false, ..OPTS };
    let mut dest = sb.dest();
    dest.name = Some("tudo".into());
    let out = run(Request::Ocr { inputs: vec![mixed], options: all }, &dest).unwrap();
    let first = normalized(&page_texts(&out.outputs[0].path)[0]);
    assert_eq!(first.matches("digitada").count(), 2, "{first:?}");
}

#[test]
fn documents_that_already_have_text_produce_no_file() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let typed = sb.pdf("digitado.pdf", &["Ja tem texto"]);
    let out = run(Request::Ocr { inputs: vec![typed], options: OPTS }, &sb.dest()).unwrap();
    assert!(out.outputs.is_empty());
    assert!(out.notes[0].contains("já tem texto selecionável"));
    assert_eq!(std::fs::read_dir(&sb.root).unwrap().count(), 1);
}

#[test]
fn blank_scans_are_reported_instead_of_saved() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let blank = scan(&sb, "branco", &[""]);
    let err = run(Request::Ocr { inputs: vec![blank], options: OPTS }, &sb.dest()).unwrap_err();
    assert!(matches!(err, OpError::OutputInvalid(_)), "{err:?}");
    assert!(err.to_string().contains("nenhum texto foi reconhecido"));
    assert_eq!(std::fs::read_dir(&sb.root).unwrap().count(), 1, "só a digitalização em branco");
}

#[test]
fn sideways_pages_are_recognized_and_straightened() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let doc = scan(&sb, "deitado", &["Documento que estava deitado", "Segunda pagina normal"]);
    // Página 1 exibida deitada (texto de lado).
    let sideways = run(Request::Rotate { inputs: vec![doc], spec: Some("1".into()), angle: 90 }, &sb.dest()).unwrap();
    let input = sideways.outputs[0].path.clone();

    let out = run(Request::Ocr { inputs: vec![input.clone()], options: OPTS }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(rotations(pdf), [0, 0], "a página deitada foi endireitada");
    let texts: Vec<String> = page_texts(pdf).iter().map(|t| normalized(t)).collect();
    assert!(texts[0].contains("documento que estava deitado"), "{texts:?}");
    assert!(texts[1].contains("segunda pagina normal"), "{texts:?}");
    assert!(out.notes.iter().any(|n| n.contains("1 página estava deitada")), "{:?}", out.notes);

    // Sem endireitar, a rotação fica como estava e o texto continua correto.
    let keep = OcrOptions { straighten: false, ..OPTS };
    let mut dest = sb.dest();
    dest.name = Some("sem girar".into());
    let out = run(Request::Ocr { inputs: vec![input], options: keep }, &dest).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(rotations(pdf), [90, 0]);
    let first = normalized(&page_texts(pdf)[0]);
    assert!(first.contains("documento que estava deitado"), "{first:?}");
}

#[test]
fn upside_down_scans_are_read_and_turned() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let doc = scan(&sb, "invertido", &["Digitalizado de cabeca para baixo"]);
    let upside = run(Request::Rotate { inputs: vec![doc], spec: None, angle: 180 }, &sb.dest()).unwrap();
    let out = run(Request::Ocr { inputs: vec![upside.outputs[0].path.clone()], options: OPTS }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(rotations(pdf), [0]);
    let text = normalized(&page_texts(pdf)[0]);
    assert!(text.contains("digitalizado de cabeca para baixo"), "{text:?}");
}

#[test]
fn rotated_scan_pages_keep_their_rotation_and_get_text() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let doc = scan(&sb, "girado", &["Documento girado"]);
    let rotated = run(Request::Rotate { inputs: vec![doc], spec: None, angle: 90 }, &sb.dest()).unwrap();
    let input = rotated.outputs[0].path.clone();
    let keep = OcrOptions { straighten: false, ..OPTS };
    let out = run(Request::Ocr { inputs: vec![input], options: keep }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(rotations(pdf), [90]);
    assert!(normalized(&page_texts(pdf)[0]).contains("documento girado"), "{:?}", page_texts(pdf));
}
