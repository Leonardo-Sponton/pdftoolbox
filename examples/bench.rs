//! Benchmark reproduzível das operações principais, com documentos gerados
//! na hora: `cargo run --release --example bench -- [pasta]`

use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{ImageBuffer, Rgb};
use pdftoolbox::jobs::JobCtx;
use pdftoolbox::operations::{
    CompressLevel, Destination, ImageFormat, ImageLayout, Request, SplitMode, execute, human_size,
};
use pdftoolbox::pdfwriter::{A4_POINTS, text_pdf};

fn size(p: &Path) -> String {
    human_size(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
}

fn time(label: &str, input: &str, f: impl FnOnce()) {
    let t = Instant::now();
    f();
    println!("| {label} | {input} | {:.2} s |", t.elapsed().as_secs_f64());
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "bench-saida".into()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ctx = JobCtx::silent();
    let dest = Destination { dir: dir.clone(), name: None };
    let run = |req| execute(req, &dest, &ctx).expect("operação");

    // Documento de texto com 1000 páginas.
    let labels: Vec<String> = (1..=1000).map(|i| format!("Pagina de teste {i} - texto de exemplo")).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let text = dir.join("texto-1000.pdf");
    text_pdf(&text, &refs, A4_POINTS).unwrap();

    // 20 fotos sintéticas 2400×1800 (ruído + gradiente), em JPEG.
    let mut photos = Vec::new();
    for n in 0..20u32 {
        let mut seed = 7 + n;
        let img = ImageBuffer::from_fn(2400, 1800, |x, y| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = (seed >> 24) as u8 / 4;
            Rgb([(x / 10) as u8 ^ noise, (y / 8) as u8, noise.wrapping_mul(3)])
        });
        let p = dir.join(format!("foto-{n:02}.jpg"));
        img.save(&p).unwrap();
        photos.push(p);
    }

    println!("| Operação | Entrada | Tempo |\n|---|---|---|");
    let text_desc = format!("1000 páginas de texto, {}", size(&text));
    time("Mesclar (2 cópias)", &text_desc, || {
        run(Request::Merge { inputs: vec![text.clone(), text.clone()] });
    });
    time("Extrair 500 páginas alternadas", &text_desc, || {
        let spec: Vec<String> = (1..=1000).step_by(2).map(|p| p.to_string()).collect();
        run(Request::Extract { input: text.clone(), spec: spec.join(","), separate: false });
    });
    time("Dividir em blocos de 100", &text_desc, || {
        run(Request::Split { input: text.clone(), mode: SplitMode::Every(100) });
    });
    time("Girar todas as páginas", &text_desc, || {
        run(Request::Rotate { inputs: vec![text.clone()], spec: None, angle: 90 });
    });
    let mut photos_pdf = PathBuf::new();
    time("Imagens → PDF (JPEG sem recompressão)", "20 JPEGs 2400×1800", || {
        let out = run(Request::ImagesToPdf { inputs: photos.clone(), layout: ImageLayout::default(), one_file: true });
        photos_pdf = out.outputs[0].path.clone();
    });
    let photo_desc = format!("20 páginas com fotos, {}", size(&photos_pdf));
    time("PDF → PNG a 150 ppp", &photo_desc, || {
        run(Request::PdfToImages { inputs: vec![photos_pdf.clone()], format: ImageFormat::Png, dpi: 150, quality: 90 });
    });
    for (label, level) in [
        ("Comprimir — sem perda", CompressLevel::Lossless),
        ("Comprimir — recomendada (150 ppp)", CompressLevel::Recommended),
        ("Comprimir — máxima (96 ppp)", CompressLevel::Strong),
    ] {
        let mut note = String::new();
        time(label, &photo_desc, || {
            let out = run(Request::Compress { inputs: vec![photos_pdf.clone()], level });
            note = out.notes.join(" ");
        });
        println!("|  ↳ {note} | | |");
    }
}
