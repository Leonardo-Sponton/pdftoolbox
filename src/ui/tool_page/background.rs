//! Campos preenchidos em segundo plano: idiomas do OCR, metadados do
//! arquivo, páginas com texto e a estimativa de compressão.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use pdftoolbox::engines::{Tool, poppler};
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::{self, Estimate};

use super::super::widgets;
use super::ToolPage;
use super::options::{LanguageRows, MetadataRows, Options};
use super::texts::{estimate_text, friendly_date};

/// Idiomas ligados de início: os escolhidos da última vez (se ainda
/// instalados); senão português e inglês; senão o único instalado.
fn preferred_languages(installed: &[String], saved: &[String]) -> Vec<String> {
    let pick = |wanted: &dyn Fn(&str) -> bool| -> Vec<String> {
        installed.iter().filter(|c| wanted(c.as_str())).cloned().collect()
    };
    let saved_ones = pick(&|c| saved.iter().any(|s| s == c));
    if !saved_ones.is_empty() {
        return saved_ones;
    }
    let defaults = pick(&|c| ["por", "eng"].contains(&c));
    if !defaults.is_empty() || installed.len() != 1 {
        return defaults;
    }
    installed.to_vec()
}

async fn installed_languages() -> Vec<String> {
    widgets::blocking(|| operations::installed_languages(&CancelToken::new()).unwrap_or_default()).await
}

impl ToolPage {
    fn saved_languages(&self) -> Vec<String> {
        self.app.upgrade().map(|a| a.settings.borrow().ocr_languages.clone()).unwrap_or_default()
    }

    /// PDF para texto: o OCR usa os idiomas escolhidos na ferramenta OCR
    /// (ou português e inglês), entre os instalados.
    pub(super) fn load_text_ocr(&self, row: adw::SwitchRow, languages: Rc<RefCell<Vec<String>>>) {
        if Tool::Tesseract.find().is_none() {
            row.set_subtitle(&tr!("Requer o Tesseract: {command}", command = Tool::Tesseract.install_command()));
            return;
        }
        let saved = self.saved_languages();
        glib::spawn_future_local(async move {
            let installed = installed_languages().await;
            let mut chosen = preferred_languages(&installed, &saved);
            if chosen.is_empty() {
                chosen = installed.into_iter().take(1).collect();
            }
            if chosen.is_empty() {
                row.set_subtitle(tr!("Nenhum idioma do Tesseract instalado"));
                row.set_active(false);
                return;
            }
            let names: Vec<String> = chosen.iter().map(|c| operations::language_name_in_text(c)).collect();
            let languages_text = match names.as_slice() {
                [init @ .., last] if !init.is_empty() => tr!("{first} e {last}", first = init.join(", "), last),
                _ => names.join(""),
            };
            row.set_subtitle(&tr!(
                "Só nas páginas sem texto · em {languages} (idiomas da ferramenta OCR) · leva alguns segundos por \
                 página",
                languages = languages_text
            ));
            *languages.borrow_mut() = chosen;
            row.set_sensitive(true);
        });
    }

    /// OCR: um interruptor por idioma instalado, ligados os preferidos. A
    /// escolha é lembrada nas preferências.
    pub(super) fn load_languages(&self, rows: Rc<LanguageRows>, refresh: Rc<dyn Fn()>) {
        if !self.deps_ok {
            rows.row.set_subtitle(tr!("O Tesseract não está instalado"));
            return;
        }
        let saved = self.saved_languages();
        let app = self.app.clone();
        glib::spawn_future_local(async move {
            let installed = installed_languages().await;
            let on = preferred_languages(&installed, &saved);
            for code in &installed {
                let name = operations::language_name(code);
                let switch = adw::SwitchRow::builder().title(&name).active(on.contains(code)).build();
                if name != *code {
                    switch.set_subtitle(code);
                }
                let (refresh, weak_rows, app) = (refresh.clone(), Rc::downgrade(&rows), app.clone());
                switch.connect_active_notify(move |_| {
                    if let Some(rows) = weak_rows.upgrade() {
                        rows.update_subtitle();
                        if let Some(app) = app.upgrade() {
                            let mut s = app.settings.borrow_mut();
                            s.ocr_languages = rows.selected();
                            s.save();
                        }
                    }
                    refresh();
                });
                rows.row.add_row(&switch);
                rows.switches.borrow_mut().push((code.clone(), switch));
            }
            let example = if installed.iter().any(|c| c == "spa") { "fra" } else { "spa" };
            let more = widgets::plain_row(
                tr!("Outros idiomas"),
                &tr!(
                    "Instale os dados do Tesseract do idioma e reabra esta ferramenta. Exemplo ({language}): {command}",
                    language = operations::language_name_in_text(example),
                    command = operations::language_install(example)
                ),
            );
            more.set_subtitle_selectable(true);
            more.add_css_class("dim-label");
            rows.row.add_row(&more);
            rows.row.set_expanded(installed.is_empty());
            rows.update_subtitle();
            refresh();
        });
    }

    /// Preenche os campos com os metadados do arquivo (só ao trocar de
    /// arquivo, para não apagar o que o usuário já digitou).
    pub(super) fn load_metadata(&self, m: &MetadataRows, file: Option<PathBuf>) {
        if *m.loaded.borrow() == file {
            return;
        }
        m.loaded.replace(file.clone());
        let fields = m.fields();
        for field in &fields {
            field.set_text("");
        }
        for row in &m.facts {
            row.set_subtitle("—");
        }
        let Some(file) = file else { return };
        let facts = m.facts.clone();
        let loaded = m.loaded.clone();
        let page_alive = m.title.downgrade();
        glib::spawn_future_local(async move {
            let path = file.clone();
            let info = widgets::blocking(move || poppler::doc_info(&path, &CancelToken::new()).ok()).await;
            // A página pode ter sido fechada ou o arquivo trocado nesse meio-tempo.
            if page_alive.upgrade().is_none() || loaded.borrow().as_ref() != Some(&file) {
                return;
            }
            let Some(info) = info else { return };
            for (row, value) in fields.iter().zip([&info.title, &info.author, &info.subject, &info.keywords]) {
                if row.text().is_empty() {
                    row.set_text(value);
                }
            }
            let dates = [friendly_date(&info.created), friendly_date(&info.modified)];
            let values = [&info.creator, &info.producer, &dates[0], &dates[1]];
            for (row, value) in facts.iter().zip(values) {
                row.set_subtitle(if value.is_empty() { "—" } else { value.as_str() });
            }
        });
    }

    /// OCR: conta, em segundo plano, as páginas com texto de cada arquivo
    /// ainda não lido.
    pub(super) fn load_text_pages(self: &Rc<Self>, files: &[PathBuf]) {
        for path in files {
            if self.text_pages.borrow().contains_key(path) {
                continue;
            }
            self.text_pages.borrow_mut().insert(path.clone(), None);
            let page = Rc::downgrade(self);
            let path = path.clone();
            glib::spawn_future_local(async move {
                let f = path.clone();
                let counts = widgets::blocking(move || operations::text_pages(&f, &CancelToken::new()).ok()).await;
                if let Some(page) = page.upgrade() {
                    if let Some(c) = counts {
                        page.text_pages.borrow_mut().insert(path, Some(c));
                    }
                    page.refresh_preview();
                }
            });
        }
    }

    /// OCR: soma das páginas com texto e do total dos arquivos da lista,
    /// quando todos já foram lidos.
    pub(super) fn ocr_counts(&self) -> Option<(u32, u32)> {
        let files = self.files.borrow();
        if files.is_empty() {
            return None;
        }
        let known = self.text_pages.borrow();
        files.iter().try_fold((0, 0), |(a, b), f| known.get(f).copied().flatten().map(|(t, n)| (a + t, b + n)))
    }

    /// Recalcula a estimativa depois de uma pausa (resolução sendo digitada).
    pub(super) fn refresh_estimate_later(self: &Rc<Self>) {
        let Options::Compress(c) = &self.options else { return };
        let generation = c.restart();
        // O valor anterior não vale mais para a nova resolução.
        if !self.files.borrow().is_empty() {
            c.show_calculating(tr!("Calculando…"));
        }
        let page = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(500), move || {
            if let Some(page) = page.upgrade()
                && let Options::Compress(c) = &page.options
                && c.generation.get() == generation
            {
                page.refresh_estimate();
            }
        });
    }

    /// Mostra a estimativa do nível escolhido, calculando o que faltar em
    /// segundo plano (um arquivo de cada vez; mudar o nível cancela o cálculo).
    pub(super) fn refresh_estimate(self: &Rc<Self>) {
        let Options::Compress(c) = &self.options else { return };
        let generation = c.restart();
        let files = self.files.borrow().clone();
        let level = c.level();
        let missing_tool = level.needs().iter().any(|t| t.find().is_none());
        c.estimate.set_visible(!missing_tool);
        c.spinner.set_visible(false);
        c.spinner.set_spinning(false);
        if files.is_empty() || missing_tool {
            c.estimate.set_subtitle(tr!("Adicione um arquivo para ver a estimativa."));
            return;
        }
        let todo: Vec<PathBuf> =
            files.iter().filter(|f| !c.estimates.borrow().contains_key(&((*f).clone(), level))).cloned().collect();
        if todo.is_empty() {
            let known = c.estimates.borrow();
            let results: Vec<(&Path, &Result<Estimate, String>)> =
                files.iter().map(|f| (f.as_path(), &known[&(f.clone(), level)])).collect();
            c.estimate.set_subtitle(&estimate_text(&results));
            return;
        }
        c.show_calculating(if files.len() == 1 {
            tr!("Calculando… (comprimindo algumas páginas numa pasta temporária)")
        } else {
            tr!("Calculando… (comprimindo algumas páginas de cada arquivo numa pasta temporária)")
        });
        let cancel = CancelToken::new();
        c.pending.replace(Some(cancel.clone()));
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            for file in todo {
                let (f, token) = (file.clone(), cancel.clone());
                let result = widgets::blocking(move || operations::estimate_compression(&f, level, &token)).await;
                if cancel.is_cancelled() {
                    return;
                }
                let Some(page) = page.upgrade() else { return };
                let Options::Compress(c) = &page.options else { return };
                if c.generation.get() != generation {
                    return;
                }
                c.estimates.borrow_mut().insert((file, level), result.map_err(|e| e.title().to_string()));
            }
            if let Some(page) = page.upgrade() {
                page.refresh_estimate();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::preferred_languages;

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn languages_start_from_the_last_choice_then_portuguese_and_english() {
        let installed = codes(&["por", "eng", "spa"]);
        assert_eq!(preferred_languages(&installed, &codes(&["spa", "deu"])), ["spa"]);
        assert_eq!(preferred_languages(&installed, &codes(&["deu"])), ["por", "eng"]);
        assert_eq!(preferred_languages(&codes(&["deu"]), &[]), ["deu"]);
        assert!(preferred_languages(&codes(&["deu", "fra"]), &[]).is_empty());
    }
}
