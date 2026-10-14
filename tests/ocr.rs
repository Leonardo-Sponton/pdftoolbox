//! OCR com o Tesseract sobre “digitalizações” geradas aqui: PDFs de texto
//! renderizados como imagem e remontados só com as imagens.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;
use pdftoolbox::OpError;
use pdftoolbox::operations::{ImageLayout, OcrOptions, PageSize, Request};

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

fn opts() -> OcrOptions {
    OcrOptions { languages: vec!["por".into(), "eng".into()], skip_text_pages: true, straighten: true }
}

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
    let out = run(Request::Ocr { inputs: vec![doc.clone()], options: opts() }, &sb.dest()).unwrap();
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
    let out = run(Request::Ocr { inputs: vec![doc], options: opts() }, &sb.dest()).unwrap();
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

    let out = run(Request::Ocr { inputs: vec![mixed.clone()], options: opts() }, &sb.dest()).unwrap();
    let texts: Vec<String> = page_texts(&out.outputs[0].path).iter().map(|t| normalized(t)).collect();
    assert_eq!(texts[0], "pagina digitada original", "página com texto ficou como estava, sem duplicar");
    assert!(texts[1].contains("pagina escaneada depois"), "{:?}", texts[1]);
    assert!(
        out.notes.iter().any(|n| n.contains("em 1 de 1 página analisada") && n.contains("1 página já tinha texto")),
        "{:?}",
        out.notes
    );

    // Sem pular, a página digitada também é reconhecida (texto em dobro).
    let all = OcrOptions { skip_text_pages: false, ..opts() };
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
    let out = run(Request::Ocr { inputs: vec![typed], options: opts() }, &sb.dest()).unwrap();
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
    let err = run(Request::Ocr { inputs: vec![blank], options: opts() }, &sb.dest()).unwrap_err();
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

    let out = run(Request::Ocr { inputs: vec![input.clone()], options: opts() }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(rotations(pdf), [0, 0], "a página deitada foi endireitada");
    let texts: Vec<String> = page_texts(pdf).iter().map(|t| normalized(t)).collect();
    assert!(texts[0].contains("documento que estava deitado"), "{texts:?}");
    assert!(texts[1].contains("segunda pagina normal"), "{texts:?}");
    assert!(out.notes.iter().any(|n| n.contains("1 página estava deitada")), "{:?}", out.notes);

    // Sem endireitar, a rotação fica como estava e o texto continua correto.
    let keep = OcrOptions { straighten: false, ..opts() };
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
    let out = run(Request::Ocr { inputs: vec![upside.outputs[0].path.clone()], options: opts() }, &sb.dest()).unwrap();
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
    let keep = OcrOptions { straighten: false, ..opts() };
    let out = run(Request::Ocr { inputs: vec![input], options: keep }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(rotations(pdf), [90]);
    assert!(normalized(&page_texts(pdf)[0]).contains("documento girado"), "{:?}", page_texts(pdf));
}

/// Página de texto corrido (várias linhas em 12 pt), como um ofício.
fn letter_pdf(path: &Path, lines: &[&str]) {
    let mut content = String::from("BT /F1 12 Tf 16 TL 72 760 Td\n");
    for line in lines {
        content.push_str(&format!("({line}) Tj T*\n"));
    }
    content.push_str("ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R >> >> \
         /Contents 5 0 R >>"
            .into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objs.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).bytes());
    }
    let xref = pdf.len();
    pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offsets {
        pdf.extend(format!("{o:010} 00000 n \n").bytes());
    }
    pdf.extend(format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
    std::fs::write(path, pdf).unwrap();
}

/// Digitalização torta: a página renderizada em 200 ppp é girada `angle`
/// graus (positivo = linhas descendo para a direita) e montada como imagem.
fn tilted_scan(sb: &Sandbox, name: &str, lines: &[&str], angle: f32) -> PathBuf {
    let text = sb.path(&format!("{name}-texto.pdf"));
    letter_pdf(&text, lines);
    let dir = sb.subdir(&format!("{name}-paginas"));
    let ok = Command::new("pdftoppm")
        .args(["-r", "200", "-gray", "-singlefile"])
        .arg(&text)
        .arg(dir.join("p"))
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let page = image::open(dir.join("p.pgm")).unwrap().into_luma8();
    // `straighten(img, a)` deixa na horizontal linhas inclinadas `a`; com −a,
    // linhas horizontais ficam inclinadas `a`.
    let (tilted, _) = pdftoolbox::deskew::straighten(&page, -angle);
    let png = dir.join(format!("{name}.png"));
    tilted.save(&png).unwrap();
    let mut dest = sb.dest();
    dest.name = Some(format!("{name}.pdf"));
    let layout = ImageLayout { size: PageSize::A4, margin_mm: 0.0 };
    let out = run(Request::ImagesToPdf { inputs: vec![png], layout, one_file: true }, &dest).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_file(&text).unwrap();
    out.outputs[0].path.clone()
}

const LETTER: [&str; 12] = [
    "Declaro para os devidos fins que o portador deste documento",
    "compareceu a esta reparticao no dia de hoje para tratar de",
    "assuntos relativos ao contrato de locacao do imovel situado",
    "na rua das Laranjeiras, numero quinhentos e quarenta e dois.",
    "O pagamento do aluguel devera ser feito ate o quinto dia util",
    "de cada mes, mediante deposito na conta indicada pelo locador.",
    "Em caso de atraso, incidira multa de dois por cento sobre o",
    "valor devido, acrescida de juros de um por cento ao mes.",
    "As partes elegem o foro da comarca da capital para dirimir",
    "quaisquer duvidas oriundas deste contrato, renunciando a",
    "qualquer outro, por mais privilegiado que seja. E por estarem",
    "assim justas e contratadas, assinam o presente instrumento.",
];

/// Fração das palavras do texto original encontradas, na ordem, no texto extraído.
fn recall(expected: &[&str], got: &str) -> f32 {
    let got: Vec<String> = got.split_whitespace().map(|w| w.to_lowercase()).collect();
    let words: Vec<String> = expected.iter().flat_map(|l| l.split_whitespace()).map(str::to_lowercase).collect();
    let found = words.iter().filter(|w| got.contains(w)).count();
    found as f32 / words.len() as f32
}

#[test]
fn tilted_scans_are_read_straight() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    for angle in [-6.0f32, 9.0] {
        let doc = tilted_scan(&sb, &format!("torta {angle}"), &LETTER, angle);
        let out = run(Request::Ocr { inputs: vec![doc.clone()], options: opts() }, &sb.dest()).unwrap();
        let pdf = &out.outputs[0].path;
        // A página do PDF não é girada: só a cópia usada na leitura.
        assert_eq!(image_count(pdf), image_count(&doc));
        assert_eq!(rotations(pdf), [0]);
        let text = page_texts(pdf).join("\n");
        let r = recall(&LETTER, &text);
        assert!(r >= 0.95, "{angle}°: só {:.0}% das palavras\n{text}", r * 100.0);
        // As linhas saem inteiras e na ordem (a camada é horizontal).
        let flat = normalized(&text);
        assert!(flat.contains("declaro para os devidos fins"), "{flat}");
        assert!(flat.find("declaro").unwrap() < flat.find("assinam").unwrap());
        let note = out.notes.iter().find(|n| n.contains("torta")).expect("nota sobre a inclinação");
        let shown = format!("{:.1}", angle.abs()).replace('.', ",");
        assert!(note.contains(&format!("({shown}°)")) || note.contains("torta ("), "{note}");
    }
}

#[test]
fn sideways_and_tilted_scans_are_straightened_and_read() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    for turn in [90, 270] {
        let doc = tilted_scan(&sb, &format!("torta {turn}"), &LETTER, 5.0);
        let sideways = run(Request::Rotate { inputs: vec![doc], spec: None, angle: turn }, &sb.dest()).unwrap();
        let input = sideways.outputs[0].path.clone();
        let out = run(Request::Ocr { inputs: vec![input], options: opts() }, &sb.dest()).unwrap();
        let pdf = &out.outputs[0].path;
        assert_eq!(rotations(pdf), [0], "{turn}°: a página deitada foi endireitada");
        let text = page_texts(pdf).join("\n");
        let r = recall(&LETTER, &text);
        assert!(r >= 0.95, "{turn}°: só {:.0}% das palavras\n{text}", r * 100.0);
        assert!(normalized(&text).contains("declaro para os devidos fins"), "{text}");
    }
}

#[test]
fn text_extraction_can_recognize_scanned_pages_first() {
    if !ocr_ready() {
        return;
    }
    let sb = Sandbox::new();
    let typed = sb.pdf("digitado.pdf", &["Pagina digitada original"]);
    let scanned = scan(&sb, "escaneada", &["Pagina escaneada depois"]);
    let mut dest = sb.dest();
    dest.name = Some("misto".into());
    let mixed = run(Request::Merge { inputs: vec![typed, scanned] }, &dest).unwrap().outputs[0].path.clone();

    let pdfs = || {
        std::fs::read_dir(&sb.root)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "pdf"))
            .count()
    };
    let before = pdfs();
    let out = run(Request::ExtractText { inputs: vec![mixed.clone()], layout: false, ocr: Some(opts()) }, &sb.dest())
        .unwrap();
    assert_eq!(out.outputs.len(), 1, "só o .txt é gravado: {:?}", out.outputs);
    let txt = &out.outputs[0].path;
    assert_eq!(txt.extension().unwrap(), "txt");
    let text = normalized(&std::fs::read_to_string(txt).unwrap());
    assert!(text.contains("pagina digitada original"), "{text}");
    assert!(text.contains("pagina escaneada depois"), "{text}");
    assert_eq!(text.matches("digitada").count(), 1, "páginas com texto não são reconhecidas de novo");
    assert!(out.notes.iter().any(|n| n.contains("em 1 de 1 página analisada")), "{:?}", out.notes);
    assert!(out.notes.iter().any(|n| n.contains("pode errar")));
    assert_eq!(pdfs(), before, "nenhum PDF novo na pasta, só o .txt");
}
