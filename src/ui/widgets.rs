//! Pequenos utilitários de interface compartilhados.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use pdftoolbox::catalog::{self, Accepts};

/// Executa `f` em uma thread do pool do GIO sem bloquear a interface.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    gio::spawn_blocking(f).await.expect("tarefa em segundo plano entrou em pânico")
}

fn filter_for(accepts: Accepts) -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    match accepts {
        Accepts::Pdf => {
            filter.set_name(Some("Documentos PDF"));
            filter.add_mime_type("application/pdf");
            filter.add_suffix("pdf");
        }
        Accepts::Images => {
            filter.set_name(Some("Imagens"));
            for ext in catalog::IMAGE_EXTENSIONS {
                filter.add_suffix(ext);
            }
            for mime in ["image/jpeg", "image/png", "image/webp", "image/gif", "image/bmp", "image/tiff"] {
                filter.add_mime_type(mime);
            }
        }
        Accepts::Office => {
            filter.set_name(Some("Documentos, planilhas e apresentações"));
            for ext in catalog::OFFICE_EXTENSIONS {
                filter.add_suffix(ext);
            }
        }
    }
    filter
}

fn files_from_model(model: &gio::ListModel) -> Vec<PathBuf> {
    (0..model.n_items()).filter_map(|i| model.item(i).and_downcast::<gio::File>()).filter_map(|f| f.path()).collect()
}

/// Seletor de arquivos do sistema (portal no Flatpak/GNOME).
pub async fn pick_files(parent: &impl IsA<gtk::Window>, accepts: Option<Accepts>, multiple: bool) -> Vec<PathBuf> {
    let dialog = gtk::FileDialog::builder().modal(true).build();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    match accepts {
        Some(a) => {
            let f = filter_for(a);
            filters.append(&f);
            dialog.set_default_filter(Some(&f));
        }
        None => {
            let all = gtk::FileFilter::new();
            all.set_name(Some("PDFs, imagens e documentos"));
            all.add_suffix("pdf");
            for ext in catalog::IMAGE_EXTENSIONS.iter().chain(catalog::OFFICE_EXTENSIONS) {
                all.add_suffix(ext);
            }
            filters.append(&all);
            filters.append(&filter_for(Accepts::Pdf));
            filters.append(&filter_for(Accepts::Images));
            filters.append(&filter_for(Accepts::Office));
        }
    }
    dialog.set_filters(Some(&filters));
    if multiple {
        dialog.set_title("Adicionar arquivos");
        dialog.open_multiple_future(Some(parent)).await.map(|m| files_from_model(&m)).unwrap_or_default()
    } else {
        dialog.set_title("Escolher arquivo");
        dialog.open_future(Some(parent)).await.ok().and_then(|f| f.path()).into_iter().collect()
    }
}

pub async fn pick_folder(parent: &impl IsA<gtk::Window>, initial: Option<&Path>) -> Option<PathBuf> {
    let dialog = gtk::FileDialog::builder().title("Escolher pasta de destino").modal(true).build();
    if let Some(dir) = initial {
        dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
    }
    dialog.select_folder_future(Some(parent)).await.ok().and_then(|f| f.path())
}

/// Aceita arquivos arrastados para `widget`.
pub fn add_file_drop(widget: &impl IsA<gtk::Widget>, on_drop: impl Fn(Vec<PathBuf>) + 'static) {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    target.connect_drop(move |_, value, _, _| {
        let Ok(list) = value.get::<gdk::FileList>() else { return false };
        let paths: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
        if paths.is_empty() {
            return false;
        }
        on_drop(paths);
        true
    });
    widget.add_controller(target);
}

pub fn open_file(parent: &impl IsA<gtk::Window>, path: &Path) {
    gtk::FileLauncher::new(Some(&gio::File::for_path(path))).launch(Some(parent), gio::Cancellable::NONE, |_| {});
}

pub fn show_in_folder(parent: &impl IsA<gtk::Window>, path: &Path) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    if path.is_dir() {
        launcher.launch(Some(parent), gio::Cancellable::NONE, |_| {});
    } else {
        launcher.open_containing_folder(Some(parent), gio::Cancellable::NONE, |_| {});
    }
}

/// Linha de preferências que exibe texto literal (nomes de arquivo podem ter “&” e “<”).
pub fn plain_row(title: &str, subtitle: &str) -> adw::ActionRow {
    // use-markup precisa ser desligado antes de definir o texto.
    let row = adw::ActionRow::builder().use_markup(false).build();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row
}

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::builder().icon_name(icon).tooltip_text(tooltip).valign(gtk::Align::Center).build();
    b.add_css_class("flat");
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b
}

pub fn path_label(path: &Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "Pasta pessoal".into(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}
