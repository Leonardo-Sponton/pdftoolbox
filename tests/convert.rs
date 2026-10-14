mod common;

use common::*;
use image::{ImageBuffer, Rgb, Rgba};
use pdftoolbox::OpError;
use pdftoolbox::jobs::{CancelToken, JobCtx};
use pdftoolbox::operations::{CompressLevel, Destination, ImageFormat, ImageLayout, PageSize, Request, execute};

/// JPEG 20×10: metade esquerda vermelha, direita azul.
fn two_tone_jpeg(path: &std::path::Path) {
    let img = ImageBuffer::from_fn(20, 10, |x, _| if x < 10 { Rgb([255u8, 0, 0]) } else { Rgb([0, 0, 255]) });
    img.save(path).unwrap();
}

/// Insere um segmento APP1/EXIF com a orientação dada logo após o SOI.
fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = b"MM\x00\x2a\x00\x00\x00\x08".to_vec();
    tiff.extend_from_slice(&1u16.to_be_bytes()); // 1 entrada
    tiff.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
    tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut app1 = b"Exif\x00\x00".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn render_png(pdf: &std::path::Path, out_base: &std::path::Path) -> image::RgbImage {
    let st = std::process::Command::new("pdftoppm")
        .args(["-r", "72", "-png", "-singlefile"])
        .arg(pdf)
        .arg(out_base)
        .status()
        .unwrap();
    assert!(st.success());
    image::open(out_base.with_extension("png")).unwrap().to_rgb8()
}

fn is_red(p: &Rgb<u8>) -> bool {
    p[0] > 180 && p[2] < 90
}
fn is_blue(p: &Rgb<u8>) -> bool {
    p[2] > 180 && p[0] < 90
}

#[test]
fn jpeg_is_embedded_without_recompression() {
    let sb = Sandbox::new();
    let jpg = sb.path("foto ção.jpg");
    two_tone_jpeg(&jpg);
    let layout = ImageLayout { size: PageSize::FitImage, margin_mm: 0.0 };
    let out = run(Request::ImagesToPdf { inputs: vec![jpg.clone()], layout, one_file: true }, &sb.dest()).unwrap();
    let pdf = std::fs::read(&out.outputs[0].path).unwrap();
    let jpeg = std::fs::read(&jpg).unwrap();
    assert!(pdf.windows(10).any(|w| w == b"/DCTDecode"));
    assert!(pdf.windows(jpeg.len()).any(|w| w == jpeg.as_slice()), "JPEG deveria estar idêntico dentro do PDF");
    assert_eq!(page_sizes(&out.outputs[0].path), [(15.0, 7.5)]); // 20×10 px a 96 ppp
}

#[test]
fn exif_orientation_is_applied() {
    let sb = Sandbox::new();
    let plain = sb.path("plano.jpg");
    two_tone_jpeg(&plain);
    let rotated = sb.path("girado.jpg");
    // 6 = girar 90° no sentido horário para exibir.
    std::fs::write(&rotated, with_exif_orientation(&std::fs::read(&plain).unwrap(), 6)).unwrap();
    let layout = ImageLayout { size: PageSize::FitImage, margin_mm: 0.0 };
    let out = run(Request::ImagesToPdf { inputs: vec![rotated], layout, one_file: true }, &sb.dest()).unwrap();
    let pdf = &out.outputs[0].path;
    assert_eq!(page_sizes(pdf), [(7.5, 15.0)]);
    // Com 90° horário, a metade esquerda (vermelha) fica em cima.
    let big = sb.path("grande.pdf");
    let layout = ImageLayout { size: PageSize::A4, margin_mm: 0.0 };
    let girado = sb.path("girado.jpg");
    let out = run(
        Request::ImagesToPdf { inputs: vec![girado], layout, one_file: true },
        &Destination { dir: sb.root.clone(), name: Some("grande".into()) },
    )
    .unwrap();
    assert_eq!(out.outputs[0].path, big);
    let img = render_png(&big, &sb.path("render"));
    let (w, h) = img.dimensions();
    assert!(is_red(img.get_pixel(w / 2, h / 4)), "topo deveria ser vermelho");
    assert!(is_blue(img.get_pixel(w / 2, h * 3 / 4)), "base deveria ser azul");
}

#[test]
fn png_with_alpha_and_a4_layout() {
    let sb = Sandbox::new();
    let png = sb.path("transparente.png");
    ImageBuffer::from_fn(300, 100, |x, _| Rgba([0u8, 128, 0, if x < 150 { 255 } else { 0 }])).save(&png).unwrap();
    let bmp = sb.path("quadro.bmp");
    ImageBuffer::from_fn(50, 80, |_, _| Rgb([10u8, 20, 30])).save(&bmp).unwrap();
    let out = run(
        Request::ImagesToPdf { inputs: vec![png, bmp], layout: ImageLayout::default(), one_file: true },
        &sb.dest(),
    )
    .unwrap();
    let pdf = &out.outputs[0].path;
    let sizes = page_sizes(pdf);
    assert_eq!(sizes.len(), 2);
    assert!(sizes[0].0 > sizes[0].1, "imagem larga → A4 paisagem");
    assert!(sizes[1].0 < sizes[1].1, "imagem alta → A4 retrato");
    assert!(std::fs::read(pdf).unwrap().windows(6).any(|w| w == b"/SMask"));
}

#[test]
fn images_one_pdf_each() {
    let sb = Sandbox::new();
    let a = sb.path("a.png");
    let b = sb.path("b.png");
    ImageBuffer::from_fn(10, 10, |_, _| Rgb([1u8, 2, 3])).save(&a).unwrap();
    ImageBuffer::from_fn(10, 10, |_, _| Rgb([1u8, 2, 3])).save(&b).unwrap();
    let out =
        run(Request::ImagesToPdf { inputs: vec![a, b], layout: ImageLayout::default(), one_file: false }, &sb.dest())
            .unwrap();
    assert_eq!(out.outputs.len(), 2);
    assert_eq!(out.outputs[1].path, sb.path("b.pdf"));
}

#[test]
fn bad_image_is_reported() {
    let sb = Sandbox::new();
    let fake = sb.path("falsa.png");
    std::fs::write(&fake, b"nao sou imagem").unwrap();
    let r =
        run(Request::ImagesToPdf { inputs: vec![fake], layout: ImageLayout::default(), one_file: true }, &sb.dest());
    assert!(matches!(r, Err(OpError::UnsupportedImage { .. })), "{r:?}");
}

#[test]
fn pdf_to_png_renders_every_page_at_requested_dpi() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2"]);
    let out =
        run(Request::PdfToImages { inputs: vec![doc], format: ImageFormat::Png, dpi: 72, quality: 90 }, &sb.dest())
            .unwrap();
    assert_eq!(out.outputs.len(), 2);
    assert!(out.folder.is_some());
    let img = image::open(&out.outputs[0].path).unwrap();
    assert!((595..=596).contains(&img.width()) && img.height() == 842, "{}x{}", img.width(), img.height());
    assert!(out.outputs[1].path.to_string_lossy().ends_with("doc - página 2.png"));
}

#[test]
fn pdf_to_jpeg_and_tiff() {
    let sb = Sandbox::new();
    let doc = sb.pdf("uma.pdf", &["P1"]);
    for (format, ext) in [(ImageFormat::Jpeg, "jpg"), (ImageFormat::Tiff, "tif")] {
        let out =
            run(Request::PdfToImages { inputs: vec![doc.clone()], format, dpi: 50, quality: 80 }, &sb.dest()).unwrap();
        assert_eq!(out.outputs.len(), 1);
        assert!(out.folder.is_none(), "uma página não precisa de pasta");
        assert_eq!(out.outputs[0].path.extension().unwrap(), ext);
        image::open(&out.outputs[0].path).expect("imagem válida");
    }
}

#[test]
fn cancel_during_rendering_leaves_nothing() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3", "P4"]);
    let out_dir = sb.subdir("saida");
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let ctx = JobCtx::new(cancel, move |_, text| {
        if text.contains("2 de 4") {
            trigger.cancel();
        }
    });
    let dest = Destination { dir: out_dir.clone(), name: None };
    let r = execute(
        Request::PdfToImages { inputs: vec![doc], format: ImageFormat::Png, dpi: 30, quality: 90 },
        &dest,
        &ctx,
    );
    assert!(matches!(r, Err(OpError::Cancelled)), "{r:?}");
    assert_eq!(std::fs::read_dir(out_dir).unwrap().count(), 0);
}

/// PDF com um fluxo de conteúdo grande e redundante, sem compressão.
fn bloated_pdf(sb: &Sandbox) -> std::path::PathBuf {
    let long = "texto repetido ".repeat(3000);
    let pages: Vec<&str> = vec![long.as_str(); 3];
    sb.pdf("inchado.pdf", &pages)
}

#[test]
fn lossless_compression_saves_and_keeps_pages() {
    let sb = Sandbox::new();
    let doc = bloated_pdf(&sb);
    let original = std::fs::metadata(&doc).unwrap().len();
    let out = run(Request::Compress { inputs: vec![doc], level: CompressLevel::Lossless }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 1);
    assert!(out.outputs[0].size < original / 5, "{} vs {original}", out.outputs[0].size);
    assert_eq!(page_texts(&out.outputs[0].path).len(), 3);

    // Comprimir de novo não traz ganho: nenhum arquivo é criado e há explicação.
    let again = run(
        Request::Compress { inputs: vec![out.outputs[0].path.clone()], level: CompressLevel::Lossless },
        &sb.dest(),
    )
    .unwrap();
    assert!(again.outputs.is_empty());
    assert!(again.notes[0].contains("já está otimizado"));
}

#[test]
fn batch_failure_does_not_stop_other_files() {
    let sb = Sandbox::new();
    let good = bloated_pdf(&sb);
    let bad = sb.path("ruim.pdf");
    std::fs::write(&bad, b"lixo").unwrap();
    let out =
        run(Request::Compress { inputs: vec![bad.clone(), good], level: CompressLevel::Lossless }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 1);
    assert_eq!(out.failures.len(), 1);
    assert_eq!(out.failures[0].0, bad);
}

#[test]
fn image_downsampling_with_ghostscript() {
    if !has("gs") {
        eprintln!("Ghostscript ausente: teste ignorado");
        return;
    }
    let sb = Sandbox::new();
    // Ruído não comprime bem com Flate; reamostrar para 150 ppp deve reduzir bastante.
    let mut seed = 12345u32;
    let noise = ImageBuffer::from_fn(1600, 1600, |_, _| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let v = (seed >> 16) as u8;
        Rgb([v, v.wrapping_mul(3), v.wrapping_add(90)])
    });
    let png = sb.path("ruido.png");
    noise.save(&png).unwrap();
    let pdf =
        run(Request::ImagesToPdf { inputs: vec![png], layout: ImageLayout::default(), one_file: true }, &sb.dest())
            .unwrap()
            .outputs
            .remove(0);
    let out = run(Request::Compress { inputs: vec![pdf.path.clone()], level: CompressLevel::Recommended }, &sb.dest())
        .unwrap();
    assert_eq!(out.outputs.len(), 1, "{:?}", out.notes);
    assert!(out.outputs[0].size < pdf.size / 2, "{} vs {}", out.outputs[0].size, pdf.size);
    assert_eq!(page_sizes(&out.outputs[0].path).len(), 1);
}
