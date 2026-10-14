//! Partes comuns aos editores de um PDF (Organizar e Anotar): cabeçalho com
//! “Salvar”, telas de escolha/carregamento/erro, confirmação antes de
//! descartar alterações, histórico de desfazer e a gravação em segundo plano.
//! Os dois só gravam ao salvar, sempre num arquivo novo.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use pdftoolbox::catalog::ToolInfo;
use pdftoolbox::jobs::{self, JobEvent};
use pdftoolbox::operations::{self, Destination, Outcome, Request};
use pdftoolbox::{OpError, OpResult, human};

use super::tasks::TaskRow;
use super::{AppState, primary_menu, widgets};

/// Nomes das telas do editor.
const EMPTY: &str = "vazio";
const LOADING: &str = "carregando";
const ERROR: &str = "erro";
const CONTENT: &str = "conteudo";

/// Textos e ação da tela inicial (nenhum documento aberto).
pub struct EmptyState<'a> {
    pub icon: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    /// Ação que abre um PDF (“page.open”, “page.add”…).
    pub open_action: &'a str,
}

/// Cabeçalho, telas e barra inferior de um editor.
pub struct EditorChrome {
    /// Fraca: a página mantém o editor vivo (`widgets::keep_alive`).
    nav_page: glib::WeakRef<adw::NavigationPage>,
    pub save: adw::SplitButton,
    pub action_bar: gtk::ActionBar,
    stack: gtk::Stack,
    error_page: adw::StatusPage,
}

impl EditorChrome {
    /// Monta a página com `content` (o editor em si) e `action_bar`.
    /// “Salvar” ativa `page.run`; “Salvar em outra pasta…”, `page.save-as`;
    /// Esc, Alt+← e o botão voltar ativam `page.leave`. Devolve também a
    /// página de navegação.
    pub fn new(
        info: &ToolInfo,
        empty: &EmptyState,
        content: &impl IsA<gtk::Widget>,
        action_bar: gtk::ActionBar,
    ) -> (EditorChrome, adw::NavigationPage) {
        let header = adw::HeaderBar::new();
        // Com alterações não salvas, a página não sai sozinha (can-pop falso):
        // este botão substitui o “voltar” e pede confirmação.
        let back = widgets::icon_button("go-previous-symbolic", tr!("Voltar"));
        back.set_action_name(Some("page.leave"));
        back.set_visible(false);
        header.pack_start(&back);
        let open = gtk::Button::builder()
            .label(tr!("Abrir outro…"))
            .tooltip_text(tr!("Escolher outro PDF (Ctrl+O)"))
            .action_name(empty.open_action)
            .build();
        header.pack_start(&open);
        header.pack_end(&primary_menu());
        let save_menu = gio::Menu::new();
        save_menu.append(Some(tr!("Salvar em outra pasta…")), Some("page.save-as"));
        let save = adw::SplitButton::builder()
            .label(tr!("Salvar"))
            .menu_model(&save_menu)
            .tooltip_text(tr!("Salvar como novo arquivo na pasta do original (Ctrl+S)"))
            .action_name("page.run")
            .sensitive(false)
            .build();
        save.add_css_class("suggested-action");
        header.pack_end(&save);

        let choose = gtk::Button::builder()
            .label(tr!("Escolher PDF…"))
            .halign(gtk::Align::Center)
            .action_name(empty.open_action)
            .build();
        choose.add_css_class("pill");
        choose.add_css_class("suggested-action");
        let empty_page = adw::StatusPage::builder()
            .icon_name(empty.icon)
            .title(empty.title)
            .description(empty.description)
            .child(&choose)
            .build();
        let loading = adw::StatusPage::builder()
            .title(tr!("Abrindo documento…"))
            .child(&gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build())
            .build();
        let error_choose = gtk::Button::builder()
            .label(tr!("Escolher outro PDF…"))
            .halign(gtk::Align::Center)
            .action_name(empty.open_action)
            .build();
        error_choose.add_css_class("pill");
        let error_page = adw::StatusPage::builder().icon_name("dialog-warning-symbolic").child(&error_choose).build();
        let stack = gtk::Stack::new();
        stack.add_named(&empty_page, Some(EMPTY));
        stack.add_named(&loading, Some(LOADING));
        stack.add_named(&error_page, Some(ERROR));
        stack.add_named(content, Some(CONTENT));

        action_bar.set_revealed(false);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&stack));
        toolbar.add_bottom_bar(&action_bar);
        let nav_page = adw::NavigationPage::with_tag(&toolbar, info.title(), info.key);
        nav_page.bind_property("can-pop", &back, "visible").invert_boolean().sync_create().build();
        // Esc e Alt+← antes do AdwNavigationView, que os consome mesmo quando a
        // página não pode sair sozinha.
        let back_keys = widgets::shortcuts(&[("Escape", "page.leave"), ("<Alt>Left", "page.leave")]);
        back_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        nav_page.add_controller(back_keys);
        let chrome = EditorChrome { nav_page: nav_page.downgrade(), save, action_bar, stack, error_page };
        (chrome, nav_page)
    }

    /// `page` é a página deste editor.
    pub fn is_page(&self, page: &adw::NavigationPage) -> bool {
        self.nav_page.upgrade().as_ref() == Some(page)
    }

    pub fn set_title(&self, title: &str) {
        if let Some(page) = self.nav_page.upgrade() {
            page.set_title(title);
        }
    }

    /// Com alterações não salvas, a página não sai sozinha (pede confirmação).
    pub fn set_dirty(&self, dirty: bool) {
        if let Some(page) = self.nav_page.upgrade() {
            page.set_can_pop(!dirty);
        }
    }

    pub fn show_loading(&self) {
        self.stack.set_visible_child_name(LOADING);
        self.action_bar.set_revealed(false);
        self.save.set_sensitive(false);
    }

    pub fn show_content(&self) {
        self.stack.set_visible_child_name(CONTENT);
        self.action_bar.set_revealed(true);
    }

    /// Abertura desistida (senha não informada): volta ao documento que
    /// estava aberto, ou à tela inicial.
    pub fn restore(&self, has_document: bool) {
        self.stack.set_visible_child_name(if has_document { CONTENT } else { EMPTY });
        self.action_bar.set_revealed(has_document);
        self.save.set_sensitive(has_document);
    }

    pub fn show_error(&self, e: &OpError) {
        self.error_page.set_title(e.title());
        self.error_page.set_description(Some(&glib::markup_escape_text(&e.to_string())));
        self.stack.set_visible_child_name(ERROR);
    }

    pub fn set_saving(&self, saving: bool) {
        self.save.set_label(if saving { tr!("Salvando…") } else { tr!("Salvar") });
        if saving {
            self.save.set_sensitive(false);
        }
    }
}

/// Estados anteriores para “Desfazer”, com limite.
pub struct History<T> {
    states: RefCell<Vec<T>>,
}

impl<T> History<T> {
    const LIMIT: usize = 100;

    pub fn new() -> Self {
        History { states: RefCell::new(Vec::new()) }
    }

    pub fn push(&self, state: T) {
        let mut states = self.states.borrow_mut();
        states.push(state);
        if states.len() > Self::LIMIT {
            states.remove(0);
        }
    }

    pub fn pop(&self) -> Option<T> {
        self.states.borrow_mut().pop()
    }

    pub fn is_empty(&self) -> bool {
        self.states.borrow().is_empty()
    }

    pub fn clear(&self) {
        self.states.borrow_mut().clear();
    }
}

/// Pergunta antes de descartar alterações; `then` roda se o usuário aceitar.
pub fn confirm_discard(app: &AppState, title: &str, body: &str, keep_label: &str, then: impl Fn() + 'static) {
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.add_response("cancel", keep_label);
    dialog.add_response("discard", tr!("Descartar"));
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response == "discard" {
            then();
        }
    });
    dialog.present(Some(&app.window));
}

/// Grava o documento em segundo plano, com a tarefa na lista de tarefas.
/// Ao terminar, chama `done(gravado)` e avisa o resultado: um aviso com
/// “Abrir” ou o erro, com os detalhes técnicos quando houver.
pub fn save(app: &Rc<AppState>, title: &str, request: Request, dir: PathBuf, done: impl FnOnce(bool) + 'static) {
    let dest = Destination { dir, name: None };
    let handle = jobs::spawn(move |ctx| operations::execute(request, &dest, ctx));
    let task = app.tasks.add(title, handle.cancel.clone());
    let app = app.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = handle.events.recv().await {
            match event {
                JobEvent::Queued => {}
                JobEvent::Progress(f, text) => task.progress(f, &text),
                JobEvent::Finished(result) => {
                    report_save(&app, &task, result, done);
                    break;
                }
            }
        }
    });
}

fn report_save(app: &AppState, task: &TaskRow, result: OpResult<Outcome>, done: impl FnOnce(bool)) {
    let saved = result.as_ref().ok().and_then(|o| o.outputs.first()).map(|o| o.path.clone());
    done(saved.is_some());
    match (result, saved) {
        (Ok(_), Some(path)) => {
            task.finish(&tr!("Concluído: {file}", file = human::file_name(&path)), Some(path.clone()));
            let toast = adw::Toast::builder()
                .title(glib::markup_escape_text(&tr!("Salvo como “{file}”", file = human::file_name(&path))))
                .button_label(tr!("Abrir"))
                .timeout(6)
                .build();
            let win = app.window.clone();
            toast.connect_button_clicked(move |_| widgets::open_file(&win, &path));
            app.toasts.add_toast(toast);
        }
        (Ok(_), None) => task.finish(tr!("Nenhum arquivo criado"), None),
        (Err(e), _) => {
            task.finish(e.title(), None);
            show_error_dialog(&app.window, &e);
        }
    }
}

fn show_error_dialog(win: &adw::ApplicationWindow, e: &OpError) {
    let dialog = adw::AlertDialog::new(Some(e.title()), Some(&e.to_string()));
    if matches!(e, OpError::ToolFailed { .. } | OpError::Io { .. } | OpError::OutputInvalid(_)) {
        let details = gtk::Label::builder().label(e.technical()).wrap(true).selectable(true).xalign(0.0).build();
        details.add_css_class("monospace");
        details.add_css_class("caption");
        let expander = gtk::Expander::builder().label(tr!("Detalhes técnicos")).child(&details).build();
        dialog.set_extra_child(Some(&expander));
    }
    dialog.add_response("ok", tr!("Fechar"));
    dialog.present(Some(win));
}
