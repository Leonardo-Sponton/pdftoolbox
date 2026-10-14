//! Fase 2: numeração, marca d’água, metadados, senha, texto e Office → PDF.

mod common;

use std::path::Path;
use std::process::Command;

use common::*;
use image::{ImageBuffer, Rgb};
use pdftoolbox::OpError;
use pdftoolbox::operations::{
    DocFields, ImageLayout, MetadataChange, NumberFormat, NumberStyle, Position, Request, Restrictions, Secret,
    WatermarkStyle,
};

/// Palavras de uma página com caixa (x0, y0, x1, y1) no sistema da página
/// exibida (origem no canto superior esquerdo), e o tamanho da página.
/// Palavra e caixa (x0, y0, x1, y1).
type Word = (String, f32, f32, f32, f32);

fn words(pdf: &Path, page: u32) -> ((f32, f32), Vec<Word>) {
    let p = page.to_string();
    let out = Command::new("pdftotext").args(["-bbox", "-f", &p, "-l", &p]).arg(pdf).arg("-").output().unwrap();
    let html = String::from_utf8_lossy(&out.stdout).into_owned();
    let attr = |tag: &str, name: &str| -> f32 {
        let i = tag.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
        tag[i..].split('"').next().unwrap().parse().unwrap()
    };
    let page_tag = html.split("<page ").nth(1).unwrap().split('>').next().unwrap();
    let size = (attr(page_tag, "width"), attr(page_tag, "height"));
    let list = html
        .split("<word ")
        .skip(1)
        .map(|w| {
            let (tag, rest) = w.split_once('>').unwrap();
            let text = rest.split("</word>").next().unwrap().to_string();
            (text, attr(tag, "xMin"), attr(tag, "yMin"), attr(tag, "xMax"), attr(tag, "yMax"))
        })
        .collect();
    (size, list)
}

fn pdfinfo(pdf: &Path, key: &str) -> String {
    let out = Command::new("pdfinfo").args(["-enc", "UTF-8"]).arg(pdf).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key}:")).map(|v| v.trim().to_string()))
        .unwrap_or_default()
}

fn number_style(position: Position, format: NumberFormat) -> NumberStyle {
    NumberStyle { position, format, first_number: 1, size: 11.0, margin_mm: 10.0 }
}

#[test]
fn numbers_land_in_the_chosen_corner_even_on_rotated_pages() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["Um", "Dois", "Tres"]);
    // Página 2 girada: o número deve seguir a página como ela é exibida.
    let rotated = run(Request::Rotate { inputs: vec![doc], spec: Some("2".into()), angle: 90 }, &sb.dest()).unwrap();
    let rotated = rotated.outputs[0].path.clone();
    let style = number_style(Position::BottomRight, NumberFormat::Number);
    let out = run(Request::NumberPages { inputs: vec![rotated], spec: None, style }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(rotations(pdf), [0, 90, 0], "a rotação original é mantida");
    let margin = 10.0 * 72.0 / 25.4;
    for page in 1..=3u32 {
        let ((w, h), list) = words(pdf, page);
        // O -bbox informa o tamanho sem a rotação, mas as palavras já na página exibida.
        let (w, h) = if page == 2 { (h, w) } else { (w, h) };
        let n = list.iter().find(|wd| wd.0 == page.to_string()).unwrap_or_else(|| panic!("número da página {page}"));
        assert!((n.3 - (w - margin)).abs() < 2.0, "página {page}: borda direita {} ≠ {}", n.3, w - margin);
        assert!(h - n.4 > margin - 4.0 && h - n.4 < margin + 4.0, "página {page}: perto da base");
    }
    // O texto original continua lá (nada foi rasterizado).
    assert!(words(pdf, 2).1.iter().any(|wd| wd.0 == "Dois"));
}

#[test]
fn numbers_only_chosen_pages_with_custom_format() {
    let sb = Sandbox::new();
    let doc = sb.pdf("capa e miolo.pdf", &["Capa", "A", "B"]);
    let mut style = number_style(Position::TopCenter, NumberFormat::PageOfTotal);
    style.first_number = 1;
    let out = run(Request::NumberPages { inputs: vec![doc], spec: Some("2-3".into()), style }, &sb.dest()).unwrap();
    let texts = page_texts(&out.outputs[0].path);
    assert_eq!(texts[0], "Capa");
    assert!(texts[1].contains("Página 1 de 2"), "{:?}", texts[1]);
    assert!(texts[2].contains("Página 2 de 2"), "{:?}", texts[2]);
    assert!(out.outputs[0].path.file_name().unwrap().to_string_lossy().ends_with("- numerado.pdf"));
}

#[test]
fn watermark_is_added_and_odd_characters_are_refused() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["Texto"]);
    let style = WatermarkStyle {
        text: "CONFIDENCIAL — Ação".into(),
        scale: 0.6,
        opacity: 0.25,
        diagonal: true,
        color: (0.8, 0.1, 0.1),
    };
    // Na diagonal, a extração de texto picota as letras; o teste de conteúdo usa a horizontal.
    let flat = WatermarkStyle { diagonal: false, ..style.clone() };
    let out = run(Request::Watermark { inputs: vec![doc.clone()], spec: None, style: flat }, &sb.dest()).unwrap();
    let text = page_texts(&out.outputs[0].path).join(" ");
    assert!(text.contains("CONFIDENCIAL — Ação") && text.contains("Texto"), "{text}");
    let diagonal = run(Request::Watermark { inputs: vec![doc.clone()], spec: None, style: style.clone() }, &sb.dest());
    assert_eq!(page_texts(&diagonal.unwrap().outputs[0].path).len(), 1);

    let bad = WatermarkStyle { text: "Olá ✓".into(), ..style };
    let err = run(Request::Watermark { inputs: vec![doc], spec: None, style: bad }, &sb.dest()).unwrap_err();
    assert!(matches!(err, OpError::InvalidRange(ref m) if m.contains('✓')), "{err:?}");
}

#[test]
fn metadata_can_be_set_edited_and_removed() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["A", "B"]);
    let fields = DocFields {
        title: "Relatório “anual”".into(),
        author: "Ana Souza".into(),
        subject: "Finanças".into(),
        keywords: "pdf, teste".into(),
    };
    let out = run(Request::Metadata { input: doc, change: MetadataChange::Set(fields.clone()) }, &sb.dest()).unwrap();
    let edited = out.outputs[0].path.clone();
    assert_eq!(pdfinfo(&edited, "Title"), "Relatório “anual”");
    assert_eq!(pdfinfo(&edited, "Author"), "Ana Souza");
    assert_eq!(pdfinfo(&edited, "Subject"), "Finanças");
    assert_eq!(pdfinfo(&edited, "Keywords"), "pdf, teste");
    assert_eq!(pdfinfo(&edited, "Pages"), "2");

    // Apagar um campo na interface remove só ele.
    let fewer = DocFields { keywords: String::new(), ..fields };
    let out = run(Request::Metadata { input: edited.clone(), change: MetadataChange::Set(fewer) }, &sb.dest()).unwrap();
    assert_eq!(pdfinfo(&out.outputs[0].path, "Title"), "Relatório “anual”");
    assert_eq!(pdfinfo(&out.outputs[0].path, "Keywords"), "");

    let out = run(Request::Metadata { input: edited, change: MetadataChange::RemoveAll }, &sb.dest()).unwrap();
    let stripped = &out.outputs[0].path;
    assert_eq!(pdfinfo(stripped, "Title"), "");
    assert_eq!(pdfinfo(stripped, "Author"), "");
    assert_eq!(pdfinfo(stripped, "ModDate"), "", "nem a data de modificação fica");
    assert_eq!(pdfinfo(stripped, "Pages"), "2");
}

#[test]
fn metadata_edit_works_when_info_is_inside_an_object_stream() {
    let sb = Sandbox::new();
    let plain = sb.pdf("plain.pdf", &["A"]);
    // Primeiro grava um título; depois comprime em fluxos de objetos, como
    // fazem o Ghostscript e muitos geradores.
    let titled = DocFields { title: "Antigo".into(), ..Default::default() };
    let out = run(Request::Metadata { input: plain, change: MetadataChange::Set(titled) }, &sb.dest()).unwrap();
    let packed = sb.path("compactado.pdf");
    let ok = Command::new("qpdf").arg("--object-streams=generate").arg(&out.outputs[0].path).arg(&packed).status();
    assert!(ok.unwrap().success());
    let show = Command::new("qpdf").arg("--show-xref").arg(&packed).output().unwrap();
    assert!(String::from_utf8_lossy(&show.stdout).contains("compressed"), "o Info deve estar comprimido");

    let fields = DocFields { title: "Novo título".into(), author: "Bia".into(), ..Default::default() };
    let out = run(Request::Metadata { input: packed, change: MetadataChange::Set(fields) }, &sb.dest()).unwrap();
    let edited = &out.outputs[0].path;
    assert_eq!(pdfinfo(edited, "Title"), "Novo título");
    assert_eq!(pdfinfo(edited, "Author"), "Bia");
    let check = Command::new("qpdf").arg("--check").arg(edited).output().unwrap();
    let report = String::from_utf8_lossy(&check.stdout);
    assert!(check.status.success() && !report.contains("WARNING"), "{report}");
}

fn requires_password(pdf: &Path) -> i32 {
    Command::new("qpdf").arg("--requires-password").arg(pdf).status().unwrap().code().unwrap()
}

#[test]
fn protect_then_unlock_with_the_right_password_only() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["Segredo"]);
    let password = Secret::new("senha com espaço ç");
    let out =
        run(Request::Protect { inputs: vec![doc.clone()], password: password.clone(), restrictions: None }, &sb.dest())
            .unwrap();
    let locked = out.outputs[0].path.clone();
    assert_eq!(requires_password(&locked), 0, "exige senha para abrir");
    assert_eq!(requires_password(&doc), 2, "o original continua sem senha");

    let wrong = run(Request::Unlock { inputs: vec![locked.clone()], password: Secret::new("errada") }, &sb.dest());
    assert!(matches!(wrong, Err(OpError::WrongPassword(_))), "{wrong:?}");

    let out = run(Request::Unlock { inputs: vec![locked.clone()], password }, &sb.dest()).unwrap();
    let open = &out.outputs[0].path;
    assert_eq!(requires_password(open), 2);
    assert_eq!(page_texts(open), ["Segredo"]);

    let again = run(Request::Unlock { inputs: vec![doc], password: Secret::new("x") }, &sb.dest());
    assert!(matches!(again, Err(OpError::NotEncrypted(_))), "{again:?}");

    // Ferramentas comuns recusam o arquivo protegido com a mensagem de senha.
    let rot = run(Request::Rotate { inputs: vec![locked], spec: None, angle: 90 }, &sb.dest());
    assert!(matches!(rot, Err(OpError::PasswordProtected(_))), "{rot:?}");
}

#[test]
fn restrictions_need_the_owner_password_to_be_removed() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["Texto"]);
    let restrictions =
        Restrictions { owner: Secret::new("dono"), allow_print: false, allow_copy: false, allow_edit: true };
    let out = run(
        Request::Protect {
            inputs: vec![doc.clone()],
            password: Secret::new("abrir"),
            restrictions: Some(restrictions),
        },
        &sb.dest(),
    )
    .unwrap();
    let locked = out.outputs[0].path.clone();
    let report = Command::new("qpdf").args(["--password=dono", "--show-encryption"]).arg(&locked).output().unwrap();
    let report = String::from_utf8_lossy(&report.stdout);
    assert!(report.contains("print high resolution: not allowed"), "{report}");
    assert!(report.contains("extract for any purpose: not allowed"), "{report}");

    // A senha de abertura abre, mas não remove as restrições do autor.
    let user = run(Request::Unlock { inputs: vec![locked.clone()], password: Secret::new("abrir") }, &sb.dest());
    assert!(matches!(user, Err(OpError::OwnerPasswordRequired(_))), "{user:?}");
    let owner = run(Request::Unlock { inputs: vec![locked], password: Secret::new("dono") }, &sb.dest()).unwrap();
    assert_eq!(requires_password(&owner.outputs[0].path), 2);

    // Senhas iguais tornariam as restrições inúteis.
    let same = Restrictions { owner: Secret::new("abrir"), allow_print: true, allow_copy: true, allow_edit: true };
    let err = run(
        Request::Protect { inputs: vec![doc], password: Secret::new("abrir"), restrictions: Some(same) },
        &sb.dest(),
    );
    assert!(matches!(err, Err(OpError::InvalidRange(_))));
}

#[test]
fn restrictions_only_file_requires_owner_password() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["Texto"]);
    let restricted = sb.path("restrito.pdf");
    let ok = Command::new("qpdf")
        .arg(&doc)
        .args(["--encrypt", "--user-password=", "--owner-password=dono", "--bits=256", "--print=none", "--"])
        .arg(&restricted)
        .status()
        .unwrap();
    assert!(ok.success());
    assert_eq!(requires_password(&restricted), 3);
    let wrong = run(Request::Unlock { inputs: vec![restricted.clone()], password: Secret::new("palpite") }, &sb.dest());
    assert!(matches!(wrong, Err(OpError::WrongPassword(_) | OpError::OwnerPasswordRequired(_))), "{wrong:?}");
    let empty = run(Request::Unlock { inputs: vec![restricted.clone()], password: Secret::new("") }, &sb.dest());
    assert!(matches!(empty, Err(OpError::OwnerPasswordRequired(_))), "{empty:?}");
    let out = run(Request::Unlock { inputs: vec![restricted], password: Secret::new("dono") }, &sb.dest()).unwrap();
    assert_eq!(requires_password(&out.outputs[0].path), 2);
}

#[test]
fn text_extraction_writes_utf8_and_skips_scans() {
    let sb = Sandbox::new();
    let doc = sb.pdf("relatório.pdf", &["Primeira pagina", "Segunda pagina"]);
    let out = run(Request::ExtractText { inputs: vec![doc], layout: false }, &sb.dest()).unwrap();
    let txt = &out.outputs[0].path;
    assert_eq!(txt.file_name().unwrap(), "relatório.txt");
    let text = std::fs::read_to_string(txt).unwrap();
    assert!(text.contains("Primeira pagina") && text.contains("Segunda pagina"), "{text}");

    // Uma "digitalização" (só imagem) não gera arquivo vazio.
    let png = sb.path("scan.png");
    ImageBuffer::from_fn(200, 280, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 128u8])).save(&png).unwrap();
    let scan =
        run(Request::ImagesToPdf { inputs: vec![png], layout: ImageLayout::default(), one_file: true }, &sb.dest())
            .unwrap();
    let out =
        run(Request::ExtractText { inputs: vec![scan.outputs[0].path.clone()], layout: true }, &sb.dest()).unwrap();
    assert!(out.outputs.is_empty());
    assert!(out.notes[0].contains("OCR"), "{:?}", out.notes);
}

#[test]
fn office_documents_become_pdf() {
    if !has("soffice") {
        eprintln!("LibreOffice ausente: teste ignorado");
        return;
    }
    let sb = Sandbox::new();
    let csv = sb.path("planilha de preços.csv");
    std::fs::write(&csv, "Produto,Preço\nCafé,12\nPão,3\n").unwrap();
    let txt = sb.path("notas.txt");
    std::fs::write(&txt, "Linha um\nLinha dois com acentuação\n").unwrap();
    let out = run(Request::OfficeToPdf { inputs: vec![csv, txt] }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 2, "{:?}", out.failures);
    let names: Vec<String> =
        out.outputs.iter().map(|o| o.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["planilha de preços.pdf", "notas.pdf"]);
    let sheet = page_texts(&out.outputs[0].path).join(" ");
    assert!(sheet.contains("Café") && sheet.contains("Preço"), "{sheet}");
    let notes = page_texts(&out.outputs[1].path).join(" ");
    assert!(notes.contains("acentuação"), "{notes}");
}
