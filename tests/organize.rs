mod common;

use common::*;
use pdftoolbox::OpError;
use pdftoolbox::operations::{Destination, PageItem, Request, SplitMode};

#[test]
fn merge_keeps_order_and_page_count() {
    let sb = Sandbox::new();
    let a = sb.pdf("a.pdf", &["A1", "A2", "A3"]);
    let b = sb.pdf("b com espaço.pdf", &["B1", "B2"]);
    let out = run(Request::Merge { inputs: vec![b.clone(), a.clone()] }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 1);
    assert_eq!(page_texts(&out.outputs[0].path), ["B1", "B2", "A1", "A2", "A3"]);
}

#[test]
fn merge_accepts_duplicate_inputs() {
    let sb = Sandbox::new();
    let a = sb.pdf("a.pdf", &["A1", "A2"]);
    let out = run(Request::Merge { inputs: vec![a.clone(), a.clone()] }, &sb.dest()).unwrap();
    assert_eq!(page_texts(&out.outputs[0].path), ["A1", "A2", "A1", "A2"]);
}

#[test]
fn merge_needs_two_files() {
    let sb = Sandbox::new();
    let a = sb.pdf("a.pdf", &["A1"]);
    assert!(matches!(run(Request::Merge { inputs: vec![a] }, &sb.dest()), Err(OpError::InvalidRange(_))));
}

#[test]
fn split_by_ranges_produces_expected_pages() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3", "P4"]);
    let out = run(Request::Split { input: doc, mode: SplitMode::Ranges("1-2, 4".into()) }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 2);
    assert_eq!(page_texts(&out.outputs[0].path), ["P1", "P2"]);
    assert_eq!(page_texts(&out.outputs[1].path), ["P4"]);
    assert!(out.outputs[0].path.file_name().unwrap().to_string_lossy().contains("páginas 1-2"));
}

#[test]
fn split_each_page_groups_into_folder() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3", "P4", "P5"]);
    let out = run(Request::Split { input: doc, mode: SplitMode::EachPage }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 5);
    let folder = out.folder.expect("pasta criada");
    for (i, f) in out.outputs.iter().enumerate() {
        assert_eq!(f.path.parent().unwrap(), folder);
        assert_eq!(page_texts(&f.path), [format!("P{}", i + 1)]);
    }
}

#[test]
fn split_every_n() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3"]);
    let out = run(Request::Split { input: doc, mode: SplitMode::Every(2) }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 2);
    assert_eq!(page_texts(&out.outputs[1].path), ["P3"]);
}

#[test]
fn split_into_identical_single_file_is_rejected() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2"]);
    let r = run(Request::Split { input: doc, mode: SplitMode::Ranges("1-2".into()) }, &sb.dest());
    assert!(matches!(r, Err(OpError::InvalidRange(_))));
}

#[test]
fn extract_respects_typed_order() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3"]);
    let out = run(Request::Extract { input: doc, spec: "3,1".into(), separate: false }, &sb.dest()).unwrap();
    assert_eq!(page_texts(&out.outputs[0].path), ["P3", "P1"]);
}

#[test]
fn extract_separate_files() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3"]);
    let out = run(Request::Extract { input: doc, spec: "2-3".into(), separate: true }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 2);
    assert_eq!(page_texts(&out.outputs[1].path), ["P3"]);
}

#[test]
fn page_out_of_range_is_reported() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2"]);
    let r = run(Request::Extract { input: doc, spec: "1-5".into(), separate: false }, &sb.dest());
    assert!(matches!(r, Err(OpError::PageOutOfRange { page: 5, total: 2 })));
}

#[test]
fn remove_pages() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3"]);
    let out = run(Request::Remove { input: doc.clone(), spec: "2".into() }, &sb.dest()).unwrap();
    assert_eq!(page_texts(&out.outputs[0].path), ["P1", "P3"]);
    let all = run(Request::Remove { input: doc, spec: "1-3".into() }, &sb.dest());
    assert!(matches!(all, Err(OpError::InvalidRange(_))));
}

#[test]
fn rotate_selected_pages() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3"]);
    let out = run(Request::Rotate { inputs: vec![doc], spec: Some("1,3".into()), angle: 90 }, &sb.dest()).unwrap();
    assert_eq!(rotations(&out.outputs[0].path), [90, 0, 90]);
}

#[test]
fn rotate_batch_all_pages() {
    let sb = Sandbox::new();
    let a = sb.pdf("a.pdf", &["A1", "A2"]);
    let b = sb.pdf("b.pdf", &["B1"]);
    let out = run(Request::Rotate { inputs: vec![a, b], spec: None, angle: 180 }, &sb.dest()).unwrap();
    assert_eq!(out.outputs.len(), 2);
    assert_eq!(rotations(&out.outputs[0].path), [180, 180]);
    assert_eq!(rotations(&out.outputs[1].path), [180]);
}

#[test]
fn organize_reorders_duplicates_and_rotates() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2", "P3"]);
    let pages = vec![
        PageItem { page: 3, rotation: 90 },
        PageItem { page: 1, rotation: 0 },
        PageItem { page: 1, rotation: -90 },
    ];
    let out = run(Request::Organize { input: doc, pages }, &sb.dest()).unwrap();
    let path = &out.outputs[0].path;
    assert_eq!(page_texts(path), ["P3", "P1", "P1"]);
    assert_eq!(rotations(path), [90, 0, 270]);
}

#[test]
fn organize_inserts_blank_pages_sized_like_neighbours() {
    let sb = Sandbox::new();
    let portrait = sb.pdf("retrato.pdf", &["P1"]);
    let landscape = sb.path("paisagem.pdf");
    pdftoolbox::pdfwriter::text_pdf(&landscape, &["L2"], (842.0, 595.0)).unwrap();
    let doc = run(Request::Merge { inputs: vec![portrait, landscape] }, &sb.dest()).unwrap().outputs.remove(0).path;
    let pages = vec![
        PageItem::blank(), // sem anterior: copia a seguinte (retrato)
        PageItem { page: 1, rotation: 0 },
        PageItem::blank(), // depois do retrato
        PageItem { page: 2, rotation: 0 },
        PageItem::blank(), // depois da paisagem
        PageItem { page: 1, rotation: 90 },
        PageItem { page: 0, rotation: 180 }, // depois do retrato girado: deitada, e girada ela mesma
    ];
    let out = run(Request::Organize { input: doc, pages }, &sb.dest()).unwrap();
    let path = &out.outputs[0].path;
    assert_eq!(page_texts(path), ["", "P1", "", "L2", "", "P1", ""]);
    let sizes: Vec<(i32, i32)> = page_sizes(path).iter().map(|&(w, h)| (w.round() as i32, h.round() as i32)).collect();
    // pdfinfo informa o tamanho sem a rotação; a 6ª e a 7ª têm /Rotate.
    assert_eq!(sizes, [(595, 842), (595, 842), (595, 842), (842, 595), (842, 595), (595, 842), (842, 595)]);
    assert_eq!(rotations(path), [0, 0, 0, 0, 0, 90, 180]);
    assert!(out.notes[0].contains("4 delas em branco"), "{:?}", out.notes);
}

#[test]
fn organize_needs_an_original_page() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1"]);
    let r = run(Request::Organize { input: doc, pages: vec![PageItem::blank()] }, &sb.dest());
    assert!(matches!(r, Err(OpError::InvalidRange(_))));
}

#[test]
fn invalid_and_empty_files_are_rejected() {
    let sb = Sandbox::new();
    let empty = sb.path("vazio.pdf");
    std::fs::write(&empty, b"").unwrap();
    let junk = sb.path("lixo.pdf");
    std::fs::write(&junk, b"isto nao e um pdf").unwrap();
    for f in [empty, junk] {
        let r = run(Request::Extract { input: f, spec: "1".into(), separate: false }, &sb.dest());
        assert!(matches!(r, Err(OpError::InvalidPdf { .. })), "{r:?}");
    }
    let missing = run(Request::Remove { input: sb.path("nao-existe.pdf"), spec: "1".into() }, &sb.dest());
    assert!(matches!(missing, Err(OpError::Io { .. })));
}

#[test]
fn password_protected_file_is_detected() {
    let sb = Sandbox::new();
    let plain = sb.pdf("aberto.pdf", &["P1"]);
    let locked = sb.path("protegido.pdf");
    let st = std::process::Command::new("qpdf")
        .args(["--encrypt", "senha-usuario", "senha-dono", "256", "--"])
        .arg(&plain)
        .arg(&locked)
        .status()
        .unwrap();
    assert!(st.success());
    let r = run(Request::Remove { input: locked, spec: "1".into() }, &sb.dest());
    assert!(matches!(r, Err(OpError::PasswordProtected(_))), "{r:?}");
}

#[test]
fn never_overwrites_existing_files_or_the_original() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2"]);
    let before = std::fs::read(&doc).unwrap();
    // Pede explicitamente o mesmo nome do original.
    let dest = Destination { dir: sb.root.clone(), name: Some("doc.pdf".into()) };
    let out = run(Request::Remove { input: doc.clone(), spec: "1".into() }, &dest).unwrap();
    assert_eq!(out.outputs[0].path, sb.path("doc (2).pdf"));
    assert_eq!(std::fs::read(&doc).unwrap(), before, "o original foi alterado");
    let again = run(Request::Remove { input: doc, spec: "1".into() }, &dest).unwrap();
    assert_eq!(again.outputs[0].path, sb.path("doc (3).pdf"));
    // Nenhum arquivo temporário .part ficou para trás.
    let leftovers: Vec<_> = std::fs::read_dir(&sb.root)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn file_names_starting_with_dash_are_not_options() {
    let sb = Sandbox::new();
    let doc = sb.pdf("-o arquivo.pdf", &["P1", "P2"]);
    let out = run(Request::Extract { input: doc, spec: "2".into(), separate: false }, &sb.dest()).unwrap();
    assert_eq!(page_texts(&out.outputs[0].path), ["P2"]);
}

#[test]
fn unwritable_destination_fails_cleanly() {
    use std::os::unix::fs::PermissionsExt;
    if std::fs::read_to_string("/proc/self/status").unwrap().contains("Uid:\t0\t") {
        return; // root ignora permissões
    }
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2"]);
    let ro = sb.subdir("somente leitura");
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    let dest = Destination { dir: ro.clone(), name: None };
    let r = run(Request::Extract { input: doc, spec: "1".into(), separate: false }, &dest);
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = r.unwrap_err();
    assert!(matches!(err, OpError::Io { .. }));
    assert_eq!(err.title(), "Permissão insuficiente");
}

#[test]
fn cancellation_creates_no_output() {
    use pdftoolbox::jobs::{CancelToken, JobCtx};
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2"]);
    let out_dir = sb.subdir("saida");
    let cancel = CancelToken::new();
    cancel.cancel();
    let ctx = JobCtx::new(cancel, |_, _| {});
    let dest = Destination { dir: out_dir.clone(), name: None };
    let r = pdftoolbox::operations::execute(Request::Split { input: doc, mode: SplitMode::EachPage }, &dest, &ctx);
    assert!(matches!(r, Err(OpError::Cancelled)));
    assert_eq!(std::fs::read_dir(out_dir).unwrap().count(), 0);
}

#[test]
fn engine_failure_is_reported_as_tool_failure() {
    use pdftoolbox::engines::qpdf;
    use pdftoolbox::jobs::CancelToken;
    let sb = Sandbox::new();
    let junk = sb.path("lixo.pdf");
    std::fs::write(&junk, b"%PDF-1.4 quebrado").unwrap();
    let r = qpdf::assemble(&[(junk.as_path(), 1)], &sb.path("saida.pdf"), &CancelToken::new());
    match r {
        Err(OpError::ToolFailed { code, .. }) => assert_eq!(code, Some(2)),
        other => panic!("esperava ToolFailed, veio {other:?}"),
    }
}

#[test]
fn long_page_selection_gets_a_short_file_name() {
    let sb = Sandbox::new();
    let labels: Vec<String> = (1..=300).map(|i| format!("P{i}")).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let doc = sb.pdf("longo.pdf", &refs);
    let spec: Vec<String> = (1..=300).step_by(2).map(|p| p.to_string()).collect();
    let out = run(Request::Extract { input: doc, spec: spec.join(","), separate: false }, &sb.dest()).unwrap();
    let name = out.outputs[0].path.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name, "longo - 150 páginas selecionadas.pdf");
    assert_eq!(page_texts(&out.outputs[0].path).len(), 150);
}

#[test]
fn very_long_custom_name_is_truncated_safely() {
    let sb = Sandbox::new();
    let doc = sb.pdf("doc.pdf", &["P1", "P2"]);
    let dest = Destination { dir: sb.root.clone(), name: Some("é".repeat(200)) };
    let out = run(Request::Remove { input: doc, spec: "1".into() }, &dest).unwrap();
    assert!(out.outputs[0].path.file_name().unwrap().len() <= 255);
}
