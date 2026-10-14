//! Lista de tarefas da sessão (em andamento e concluídas).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;

use pdftoolbox::jobs::CancelToken;

use super::widgets;

pub struct Tasks {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    empty: adw::StatusPage,
    running: Cell<u32>,
}

/// Uma tarefa exibida na lista; atualizada pela página da ferramenta.
#[derive(Clone)]
pub struct TaskRow {
    tasks: Rc<Tasks>,
    row: adw::ActionRow,
    progress: gtk::ProgressBar,
    cancel_button: gtk::Button,
    done: Rc<Cell<bool>>,
}

impl Tasks {
    pub fn new() -> Rc<Tasks> {
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        list.add_css_class("boxed-list");
        let empty = adw::StatusPage::builder()
            .icon_name("emblem-ok-symbolic")
            .title("Nenhuma tarefa nesta sessão")
            .description("As operações executadas aparecem aqui, com o progresso e o resultado.")
            .build();
        empty.add_css_class("compact");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&empty);
        content.append(&list);
        list.set_visible(false);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&content)
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&scroll));
        let dialog =
            adw::Dialog::builder().title("Tarefas").content_width(520).content_height(460).child(&toolbar).build();
        Rc::new(Tasks { dialog, list, empty, running: Cell::new(0) })
    }

    pub fn present(&self, parent: &impl IsA<gtk::Widget>) {
        self.dialog.present(Some(parent));
    }

    pub fn running(&self) -> u32 {
        self.running.get()
    }

    fn set_running(&self, n: u32) {
        self.running.set(n);
    }

    pub fn add(self: &Rc<Self>, title: &str, cancel: CancelToken) -> TaskRow {
        let row = widgets::plain_row(title, "Na fila…");
        let progress = gtk::ProgressBar::builder().valign(gtk::Align::Center).width_request(90).build();
        let cancel_button = widgets::icon_button("process-stop-symbolic", "Cancelar tarefa");
        cancel_button.connect_clicked(move |b| {
            cancel.cancel();
            b.set_sensitive(false);
        });
        row.add_suffix(&progress);
        row.add_suffix(&cancel_button);
        self.list.prepend(&row);
        self.list.set_visible(true);
        self.empty.set_visible(false);
        self.set_running(self.running.get() + 1);
        TaskRow { tasks: self.clone(), row, progress, cancel_button, done: Rc::new(Cell::new(false)) }
    }
}

impl TaskRow {
    pub fn progress(&self, fraction: f64, text: &str) {
        self.progress.set_fraction(fraction);
        self.row.set_subtitle(text);
    }

    pub fn finish(&self, summary: &str, folder: Option<PathBuf>) {
        if self.done.replace(true) {
            return;
        }
        self.row.set_subtitle(summary);
        self.progress.set_visible(false);
        self.cancel_button.set_visible(false);
        if let Some(folder) = folder {
            let open = widgets::icon_button("folder-open-symbolic", "Mostrar na pasta");
            open.connect_clicked(move |b| {
                if let Some(win) = b.root().and_downcast::<gtk::Window>() {
                    widgets::show_in_folder(&win, &folder);
                }
            });
            self.row.add_suffix(&open);
        }
        self.tasks.set_running(self.tasks.running.get().saturating_sub(1));
    }
}
