//! Anotar e assinar: textos, destaque, moldura e imagem de assinatura.

mod common;

use std::path::Path;
use std::process::Command;

use common::*;
use image::{Rgb, RgbImage};
use pdftoolbox::operations::{Annotation, AnnotationKind as Kind, Request};

fn item(kind: Kind, page: u32, x: f32, y: f32, w: f32, h: f32) -> Annotation {
    Annotation { kind, page, all_pages: false, x, y, w, h, opacity: 1.0 }
}

fn text(t: &str, page: u32, x: f32, y: f32) -> Annotation {
    item(Kind::Text { text: t.into(), size: 14.0, bold: false, color: (0.0, 0.0, 0.6) }, page, x, y, 0.0, 0.0)
}

/// Página `page` renderizada a 72 ppp (1 px = 1 pt).
fn render(pdf: &Path, page: u32, dir: &Path) -> image::RgbImage {
    let p = page.to_string();
    let prefix = dir.join(format!("render{page}"));
    let ok = Command::new("pdftoppm")
        .args(["-r", "72", "-png", "-singlefile", "-f", &p, "-l", &p])
        .arg(pdf)
        .arg(&prefix)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    image::open(prefix.with_extension("png")).unwrap().into_rgb8()
}

/// Bounding box (x0, y0, x1, y1) de uma palavra, em pontos a partir do topo.
fn word_box(pdf: &Path, page: u32, word: &str) -> Option<(f32, f32, f32, f32)> {
    let p = page.to_string();
    let out = Command::new("pdftotext").args(["-bbox", "-f", &p, "-l", &p]).arg(pdf).arg("-").output().unwrap();
    let html = String::from_utf8_lossy(&out.stdout).into_owned();
    let attr = |tag: &str, name: &str| -> f32 {
        let i = tag.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
        tag[i..].split('"').next().unwrap().parse().unwrap()
    };
    html.split("<word ").skip(1).find_map(|w| {
        let (tag, rest) = w.split_once('>').unwrap();
        (rest.split("</word>").next() == Some(word))
            .then(|| (attr(tag, "xMin"), attr(tag, "yMin"), attr(tag, "xMax"), attr(tag, "yMax")))
    })
}

/// Assinatura de teste: papel branco com um traço preto horizontal no meio.
fn signature(path: &Path) {
    let mut img = RgbImage::from_pixel(200, 80, Rgb([255, 255, 255]));
    for x in 20..180 {
        for y in 36..44 {
            img.put_pixel(x, y, Rgb([10, 10, 10]));
        }
    }
    img.save(path).unwrap();
}

#[test]
fn text_lands_where_it_was_placed_and_keeps_the_original() {
    let sb = Sandbox::new();
    let doc = sb.pdf("contrato.pdf", &["Clausula", "Assinaturas"]);
    let items = vec![text("Aprovado\nJoão", 2, 0.5, 0.25), text("Rubrica", 1, 0.8, 0.9)];
    let out = run(Request::Annotate { input: doc, items }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert!(pdf.file_name().unwrap().to_string_lossy().contains("anotado"));
    let (w, h) = page_sizes(pdf)[1];
    let b = word_box(pdf, 2, "Aprovado").expect("texto na página 2");
    assert!((b.0 - 0.5 * w).abs() < 1.5, "x {} ≠ {}", b.0, 0.5 * w);
    assert!(b.1 >= 0.25 * h - 1.0 && b.1 < 0.25 * h + 14.0, "logo abaixo do topo da caixa");
    let joao = word_box(pdf, 2, "João").expect("segunda linha com acento");
    assert!(joao.1 > b.1 + 10.0, "segunda linha abaixo da primeira");
    assert!(word_box(pdf, 1, "Aprovado").is_none(), "não vaza para outra página");
    assert!(word_box(pdf, 1, "Rubrica").is_some());
    assert!(word_box(pdf, 2, "Assinaturas").is_some(), "o texto original continua selecionável");
}

#[test]
fn repeated_items_appear_on_every_page() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["A", "B", "C"]);
    let mut stamp = text("Visto", 1, 0.05, 0.05);
    stamp.all_pages = true;
    let out = run(Request::Annotate { input: doc, items: vec![stamp] }, &sb.dest()).unwrap();
    for page in 1..=3 {
        assert!(word_box(&out.outputs[0].path, page, "Visto").is_some(), "página {page}");
    }
}

#[test]
fn signature_without_paper_and_highlight_that_keeps_text_visible() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["Texto"]);
    let sig = sb.path("assinatura.png");
    signature(&sig);
    let (pw, ph) = page_sizes(&doc)[0];
    // Destaque amarelo cobrindo a área; a assinatura sobre ele, sem o papel
    // branco: onde era papel, o amarelo deve continuar visível.
    let items = vec![
        item(Kind::Highlight { color: (1.0, 0.9, 0.0) }, 1, 0.5, 0.5, 220.0, 100.0),
        item(Kind::Image { path: sig.clone(), remove_white: true }, 1, 0.5, 0.5, 200.0, 80.0),
        item(Kind::Frame { color: (1.0, 0.0, 0.0), line_width: 4.0 }, 1, 0.1, 0.1, 100.0, 50.0),
    ];
    let out = run(Request::Annotate { input: doc.clone(), items }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert!(out.notes.iter().any(|n| n.contains("sem validade de assinatura digital")));
    let img = render(pdf, 1, &sb.root);
    let (x0, y0) = ((0.5 * pw) as u32, (0.5 * ph) as u32);
    let paper = img.get_pixel(x0 + 5, y0 + 5).0;
    assert!(paper[0] > 200 && paper[1] > 180 && paper[2] < 80, "papel removido, destaque visível: {paper:?}");
    let ink = img.get_pixel(x0 + 100, y0 + 40).0;
    assert!(ink.iter().all(|&c| c < 80), "traço da assinatura: {ink:?}");
    let frame = img.get_pixel((0.1 * pw) as u32 + 1, (0.1 * ph) as u32 + 20).0;
    assert!(frame[0] > 200 && frame[1] < 60, "moldura vermelha: {frame:?}");
    let inside = img.get_pixel((0.1 * pw) as u32 + 50, (0.1 * ph) as u32 + 25).0;
    assert!(inside.iter().all(|&c| c > 240), "moldura não é preenchida: {inside:?}");

    // Sem remover o fundo, o papel branco cobre o destaque.
    let items = vec![
        item(Kind::Highlight { color: (1.0, 0.9, 0.0) }, 1, 0.5, 0.5, 220.0, 100.0),
        item(Kind::Image { path: sig, remove_white: false }, 1, 0.5, 0.5, 200.0, 80.0),
    ];
    let mut dest = sb.dest();
    dest.name = Some("com papel".into());
    let out = run(Request::Annotate { input: doc, items }, &dest).unwrap();
    let img = render(&out.outputs[0].path, 1, &sb.root);
    let paper = img.get_pixel(x0 + 5, y0 + 5).0;
    assert!(paper.iter().all(|&c| c > 240), "papel branco mantido: {paper:?}");
}

#[test]
fn highlight_does_not_hide_the_text_below() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["Importante"]);
    let b = word_box(&doc, 1, "Importante").unwrap();
    let (pw, ph) = page_sizes(&doc)[0];
    let hl = item(
        Kind::Highlight { color: (1.0, 0.9, 0.0) },
        1,
        (b.0 - 2.0) / pw,
        (b.1 - 2.0) / ph,
        b.2 - b.0 + 4.0,
        b.3 - b.1 + 4.0,
    );
    let out = run(Request::Annotate { input: doc, items: vec![hl] }, &sb.dest()).unwrap();
    let img = render(&out.outputs[0].path, 1, &sb.root);
    let area = (b.0 as u32..b.2 as u32).flat_map(|x| (b.1 as u32..b.3 as u32).map(move |y| (x, y)));
    let darkest = area.map(|(x, y)| img.get_pixel(x, y).0.iter().map(|&c| c as u32).sum::<u32>()).min().unwrap();
    assert!(darkest < 150, "o texto preto continua visível sob o destaque");
}

#[test]
fn refuses_empty_text_unsupported_characters_and_missing_images() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["A"]);
    let bad = [
        vec![],
        vec![text("   ", 1, 0.1, 0.1)],
        vec![text("✓ ok", 1, 0.1, 0.1)],
        vec![text("ok", 2, 0.1, 0.1)],
        vec![item(Kind::Image { path: sb.path("nao-existe.png"), remove_white: false }, 1, 0.1, 0.1, 50.0, 20.0)],
    ];
    for items in bad {
        assert!(run(Request::Annotate { input: doc.clone(), items }, &sb.dest()).is_err());
    }
    assert_eq!(std::fs::read_dir(&sb.root).unwrap().count(), 1, "nada foi gravado");
}

#[test]
fn position_follows_the_displayed_page_when_rotated() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["A", "B"]);
    let rotated = run(Request::Rotate { inputs: vec![doc], spec: Some("2".into()), angle: 90 }, &sb.dest()).unwrap();
    let input = rotated.outputs[0].path.clone();
    let out = run(Request::Annotate { input, items: vec![text("Canto", 2, 0.7, 0.1)] }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(rotations(pdf), [0, 90]);
    // Página exibida em paisagem: 842 × 595.
    let (w, h) = page_sizes(pdf)[0];
    let (dw, dh) = (w.max(h), w.min(h));
    let b = word_box(pdf, 2, "Canto").expect("texto na página girada");
    assert!((b.0 - 0.7 * dw).abs() < 1.5, "x {} ≠ {}", b.0, 0.7 * dw);
    assert!(b.1 >= 0.1 * dh - 1.0 && b.1 < 0.1 * dh + 14.0, "y {}", b.1);
    let img = render(pdf, 2, &sb.root);
    assert!(img.width() > img.height(), "renderizada em paisagem");
}
