//! Execução da ferramenta: progresso, cancelamento e o resultado (arquivos
//! gravados, observações e erros).

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use pdftoolbox::catalog::ToolId;
use pdftoolbox::jobs::{self, JobEvent};
use pdftoolbox::operations::{self, Destination, Outcome};
use pdftoolbox::{OpError, OpResult, human};

use super::super::{unlock, widgets};
use super::ToolPage;
use super::texts::outcome_summary;

/// Arquivos listados no resultado; numa pasta nova, só os primeiros.
const MAX_ROWS: usize = 12;
const MAX_ROWS_IN_FOLDER: usize = 3;

impl ToolPage {
    fn set_running(&self, running: bool) {
        // O foco acompanha o botão visível: sem isso, ao esconder o botão
        // focado o GTK leva o foco para o topo da janela.
        let had_focus = self.run_button.has_focus() || self.cancel_button.has_focus();
        self.run_button.set_visible(!running);
        self.cancel_button.set_visible(running);
        self.cancel_button.set_sensitive(true);
        self.progress.set_visible(running);
        self.status.set_visible(running);
        self.files_group.set_sensitive(!running);
        let focus_lost = self.window().is_some_and(|w| gtk::prelude::GtkWindowExt::focus(&w).is_none());
        if had_focus || running || focus_lost {
            if running {
                self.cancel_button.grab_focus();
            } else {
                self.run_button.grab_focus();
            }
        }
    }

    pub(super) fn cancel(&self) {
        if let Some(token) = self.running.borrow().as_ref() {
            token.cancel();
            self.cancel_button.set_sensitive(false);
            self.status.set_text("Cancelando…");
        }
    }

    pub(super) fn run(self: &Rc<Self>) {
        if !self.deps_ok || self.running.borrow().is_some() {
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        let request = match self.request() {
            Ok(r) => r,
            Err(e) => {
                self.show_error(&e);
                return;
            }
        };
        let Some(dir) = self.dest_dir.borrow().clone() else {
            self.show_error(&OpError::NoInput);
            return;
        };
        let name = self.single_output().then(|| self.name_row.text().to_string());
        let dest = Destination { dir, name };

        clear(&self.result_slot);
        self.set_running(true);
        self.progress.set_fraction(0.0);
        self.status.set_text("Preparando…");

        let title = match self.files.borrow().as_slice() {
            [one] => format!("{} — {}", self.info.title, human::file_name(one)),
            many => format!("{} — {} arquivos", self.info.title, many.len()),
        };
        let handle = jobs::spawn(move |ctx| operations::execute(request, &dest, ctx));
        self.running.replace(Some(handle.cancel.clone()));
        let task = app.tasks.add(&title, handle.cancel.clone());

        let page = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = handle.events.recv().await {
                match event {
                    JobEvent::Queued => page.status.set_text("Na fila…"),
                    JobEvent::Progress(f, text) => {
                        page.progress.set_fraction(f);
                        page.status.set_text(&text);
                        task.progress(f, &text);
                    }
                    JobEvent::Finished(result) => {
                        page.running.replace(None);
                        page.set_running(false);
                        match &result {
                            Ok(outcome) => task.finish(&outcome_summary(outcome), folder_of(outcome)),
                            Err(e) => task.finish(e.title(), None),
                        }
                        page.show_result(result);
                        break;
                    }
                }
            }
        });
    }

    /// Rola a página até o resultado (ou erro) recém-mostrado.
    fn reveal_result(&self) {
        let (slot, scroll) = (self.result_slot.clone(), self.scroll.clone());
        // Espera o layout das linhas novas para saber a altura real.
        glib::timeout_add_local_once(Duration::from_millis(150), move || {
            if let Some(child) = scroll.child()
                && let Some(bounds) = slot.compute_bounds(&child)
            {
                let adj = scroll.vadjustment();
                let (top, bottom) = (f64::from(bounds.y()), f64::from(bounds.y()) + f64::from(bounds.height()));
                if bottom > adj.value() + adj.page_size() {
                    adj.set_value((bottom - adj.page_size() + 24.0).min(top - 12.0).max(0.0));
                }
            }
        });
    }

    fn show_error(&self, e: &OpError) {
        clear(&self.result_slot);
        self.result_slot.append(&error_group(e));
        self.reveal_result();
        // Falhas do motor ou do disco podem passar numa nova tentativa.
        if matches!(e, OpError::ToolFailed { .. } | OpError::Io { .. } | OpError::OutputInvalid(_)) {
            self.run_button.set_label("Tentar novamente");
        } else {
            self.refresh_preview();
        }
    }

    fn show_result(self: &Rc<Self>, result: OpResult<Outcome>) {
        let outcome = match result {
            Ok(o) => o,
            Err(e) => {
                self.show_error(&e);
                return;
            }
        };
        self.refresh_preview();
        let Some(win) = self.window() else { return };
        let title = if outcome.cancelled {
            "Cancelado — alguns arquivos já haviam sido gravados"
        } else if !outcome.failures.is_empty() {
            "Concluído com falhas"
        } else if outcome.outputs.is_empty() {
            "Nada a fazer"
        } else {
            "Concluído"
        };
        let group = adw::PreferencesGroup::builder().title(title).build();
        let mut notes = outcome.notes.clone();
        let from_copy = self.files.borrow().iter().any(|f| unlock::is_copy(f));
        if from_copy && !outcome.outputs.is_empty() && self.info.id != ToolId::Protect {
            notes.push(
                "O resultado foi salvo sem senha (o original continua protegido). Para protegê-lo, use \
                 “Proteger com senha”."
                    .into(),
            );
        }
        if !notes.is_empty() {
            group.set_description(Some(&glib::markup_escape_text(&notes.join("\n"))));
        }
        let new_button = gtk::Button::builder().label("Começar de novo").valign(gtk::Align::Center).build();
        new_button.add_css_class("flat");
        new_button.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| {
                page.files.borrow_mut().clear();
                page.rebuild_files();
                clear(&page.result_slot);
            }
        ));
        group.set_header_suffix(Some(&new_button));

        if let Some(folder) = &outcome.folder {
            let row = widgets::plain_row(
                &human::file_name(folder),
                &format!("Pasta com {} arquivo(s)", outcome.outputs.len()),
            );
            row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
            let open = widgets::icon_button("folder-open-symbolic", "Abrir pasta");
            let (w, f) = (win.clone(), folder.clone());
            open.connect_clicked(move |_| widgets::show_in_folder(&w, &f));
            row.add_suffix(&open);
            group.add(&row);
        }
        let shown = if outcome.folder.is_some() { MAX_ROWS_IN_FOLDER } else { MAX_ROWS };
        for out in outcome.outputs.iter().take(shown) {
            group.add(&output_row(&win, &out.path, out.size));
        }
        if outcome.outputs.len() > shown {
            group.add(&widgets::plain_row(&format!("e mais {} arquivo(s)", outcome.outputs.len() - shown), ""));
        }
        for (file, err) in &outcome.failures {
            group.add(&failure_row(&human::file_name(file), err));
        }
        self.result_slot.append(&group);
        self.reveal_result();
        match outcome.outputs.as_slice() {
            [] => {}
            [one] => self.toast(&format!("Salvo: {}", human::file_name(&one.path))),
            many => self.toast(&format!("{} arquivos salvos", many.len())),
        }
    }
}

/// Arquivo gravado, com botões para abrir e mostrar na pasta.
fn output_row(win: &adw::ApplicationWindow, path: &std::path::Path, size: u64) -> adw::ActionRow {
    let row = widgets::plain_row(&human::file_name(path), &human::size(size));
    row.set_tooltip_text(Some(&path.to_string_lossy()));
    let open = widgets::icon_button("document-open-symbolic", "Abrir");
    let (w, p) = (win.clone(), path.to_path_buf());
    open.connect_clicked(move |_| widgets::open_file(&w, &p));
    let show = widgets::icon_button("folder-open-symbolic", "Mostrar na pasta");
    let (w, p) = (win.clone(), path.to_path_buf());
    show.connect_clicked(move |_| widgets::show_in_folder(&w, &p));
    row.add_suffix(&open);
    row.add_suffix(&show);
    row.set_activatable_widget(Some(&open));
    row
}

/// O que mostrar na tarefa concluída: a pasta criada ou o primeiro arquivo.
fn folder_of(outcome: &Outcome) -> Option<PathBuf> {
    outcome.folder.clone().or_else(|| outcome.outputs.first().map(|o| o.path.clone()))
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// Erros com detalhe técnico útil para relatar um problema.
fn is_technical(e: &OpError) -> bool {
    matches!(
        e,
        OpError::ToolFailed { .. } | OpError::Io { .. } | OpError::InvalidPdf { .. } | OpError::OutputInvalid(_)
    )
}

fn failure_row(title: &str, err: &OpError) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::builder().use_markup(false).build();
    row.set_title(title);
    row.set_subtitle(&err.to_string());
    row.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
    let details = gtk::Label::builder()
        .label(err.technical())
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();
    details.add_css_class("monospace");
    details.add_css_class("caption");
    row.add_row(&details);
    row
}

fn error_group(e: &OpError) -> adw::PreferencesGroup {
    let group =
        adw::PreferencesGroup::builder().title(e.title()).description(glib::markup_escape_text(&e.to_string())).build();
    if is_technical(e) {
        let row = failure_row("Detalhes técnicos", e);
        row.set_subtitle("Útil para relatar um problema. Não inclui o conteúdo do documento.");
        group.add(&row);
    }
    if let OpError::MissingTool(t) = e {
        let row = widgets::plain_row(t.label(), &t.install_command());
        row.set_subtitle_selectable(true);
        group.add(&row);
    }
    group
}
