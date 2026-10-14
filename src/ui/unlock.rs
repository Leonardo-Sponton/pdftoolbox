//! PDFs com senha nas ferramentas: ao entrar um arquivo que exige senha,
//! um diálogo pede a senha e a ferramenta passa a trabalhar numa cópia
//! temporária desbloqueada (`operations::unlocked_copy`). O original não
//! muda; destino padrão, dicas e rótulos continuam se referindo a ele.
//!
//! As cópias valem durante a sessão (abrir o mesmo arquivo em outra
//! ferramenta não pede a senha de novo, enquanto ele não mudar) e são
//! apagadas ao sair.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use adw::prelude::*;
use gtk::glib;

use pdftoolbox::OpError;
use pdftoolbox::catalog::{self, Accepts};
use pdftoolbox::engines::qpdf::{self, Encryption};
use pdftoolbox::error::display_name;
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::{Secret, UnlockedCopy, unlocked_copy};

use super::{AppState, widgets};

struct Entry {
    copy: UnlockedCopy,
    modified: Option<SystemTime>,
}

thread_local! {
    static COPIES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// O arquivo que o usuário escolheu: o original, se `path` é uma cópia desbloqueada.
pub fn original(path: &Path) -> PathBuf {
    COPIES.with(|c| {
        c.borrow().iter().find(|e| e.copy.path == path).map_or_else(|| path.to_path_buf(), |e| e.copy.original.clone())
    })
}

/// `path` é uma cópia desbloqueada (o resultado sairá sem senha).
pub fn is_copy(path: &Path) -> bool {
    COPIES.with(|c| c.borrow().iter().any(|e| e.copy.path == path))
}

/// Cópia já desbloqueada nesta sessão, se o original não mudou desde então.
fn cached(original: &Path) -> Option<PathBuf> {
    let now = modified(original);
    COPIES.with(|c| {
        let mut c = c.borrow_mut();
        // Original alterado: a cópia antiga não serve mais.
        c.retain(|e| e.copy.original != original || e.modified == now);
        c.iter().find(|e| e.copy.original == original).map(|e| e.copy.path.clone())
    })
}

/// Troca cada PDF que exige senha por uma cópia desbloqueada, pedindo a
/// senha. Arquivos cuja senha o usuário não informou ficam de fora (com um
/// aviso). Os demais arquivos passam como estão.
pub async fn prepare(app: &Rc<AppState>, files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut ready = Vec::with_capacity(files.len());
    let mut skipped = Vec::new();
    for file in files {
        if !catalog::extension_ok(Accepts::Pdf, &file) || is_copy(&file) {
            ready.push(file);
            continue;
        }
        if let Some(copy) = cached(&file) {
            ready.push(copy);
            continue;
        }
        let f = file.clone();
        let kind = widgets::blocking(move || qpdf::encryption(&f, &CancelToken::new())).await;
        if !matches!(kind, Ok(Encryption::Password)) {
            // Sem senha, ou ilegível: a ferramenta mostra o que houver.
            ready.push(file);
            continue;
        }
        match ask(app, &file).await {
            Some(copy) => ready.push(copy),
            None => skipped.push(file),
        }
    }
    match skipped.as_slice() {
        [] => {}
        [one] => app.toast(&format!("“{}” ficou de fora: é preciso a senha para abri-lo.", display_name(one))),
        many => app.toast(&format!("{} PDFs ficaram de fora: é preciso a senha para abri-los.", many.len())),
    }
    ready
}

/// Pede a senha até ela abrir o arquivo ou o usuário desistir.
async fn ask(app: &Rc<AppState>, file: &Path) -> Option<PathBuf> {
    let mut problem: Option<String> = None;
    loop {
        let password = dialog(app, file, problem.as_deref()).await?;
        let (f, pw) = (file.to_path_buf(), password);
        let result = widgets::blocking(move || unlocked_copy(&f, &pw, &CancelToken::new())).await;
        match result {
            Ok(copy) => {
                let path = copy.path.clone();
                let modified = modified(file);
                COPIES.with(|c| c.borrow_mut().push(Entry { copy, modified }));
                return Some(path);
            }
            Err(OpError::WrongPassword(_)) => {
                problem = Some("Senha incorreta. Confira maiúsculas, minúsculas e acentos.".into());
            }
            Err(e @ OpError::OwnerPasswordRequired(_)) => problem = Some(e.to_string()),
            Err(e) => {
                app.toast(&e.to_string());
                return None;
            }
        }
    }
}

/// Diálogo de senha. `None` = cancelado.
async fn dialog(app: &Rc<AppState>, file: &Path, problem: Option<&str>) -> Option<Secret> {
    let dialog = adw::AlertDialog::builder()
        .heading("PDF protegido por senha")
        .body(format!(
            "Digite a senha de “{}”. A ferramenta trabalha numa cópia temporária desbloqueada; o original \
             continua como está e o resultado é salvo sem senha.",
            display_name(file)
        ))
        .close_response("cancelar")
        .default_response("abrir")
        .build();
    dialog.add_response("cancelar", "Cancelar");
    dialog.add_response("abrir", "Abrir");
    dialog.set_response_appearance("abrir", adw::ResponseAppearance::Suggested);
    let entry = gtk::PasswordEntry::builder().show_peek_icon(true).activates_default(true).build();
    entry.update_property(&[gtk::accessible::Property::Label("Senha do documento")]);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.append(&entry);
    if let Some(text) = problem {
        let label = gtk::Label::builder().label(text).wrap(true).xalign(0.0).build();
        label.add_css_class("texto-erro");
        label.set_accessible_role(gtk::AccessibleRole::Alert);
        column.append(&label);
    }
    dialog.set_extra_child(Some(&column));
    dialog.set_response_enabled("abrir", false);
    let d = dialog.downgrade();
    entry.connect_changed(move |e| {
        if let Some(d) = d.upgrade() {
            d.set_response_enabled("abrir", !e.text().is_empty());
        }
    });
    let e = entry.clone();
    glib::idle_add_local_once(move || {
        e.grab_focus();
    });
    let response = dialog.choose_future(Some(&app.window)).await;
    let text = entry.text().to_string();
    entry.set_text("");
    (response == "abrir" && !text.is_empty()).then(|| Secret::new(text))
}
