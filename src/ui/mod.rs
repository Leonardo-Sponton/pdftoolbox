//! Interface GTK4/libadwaita. Nenhum processamento acontece aqui: as páginas
//! montam uma `Request` e a entregam a `pdftoolbox::jobs::spawn`.

mod annotator;
mod dialogs;
mod home;
mod organizer;
mod page_preview;
mod page_thumb;
mod page_zoom;
mod settings;
mod tasks;
mod theme;
mod thumbs;
mod tool_page;
mod widgets;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use pdftoolbox::catalog::{ToolId, ToolInfo};

pub use settings::{Settings, Theme};

pub const APP_ID: &str = "io.github.leonardo_sponton.PDFToolbox";

pub struct AppState {
    pub window: adw::ApplicationWindow,
    pub nav: adw::NavigationView,
    pub toasts: adw::ToastOverlay,
    pub settings: RefCell<Settings>,
    pub tasks: Rc<tasks::Tasks>,
    home: RefCell<Option<Rc<home::Home>>>,
}

pub type App = Rc<AppState>;

impl AppState {
    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::builder().title(glib::markup_escape_text(text)).timeout(4).build());
    }

    /// Abre a página de uma ferramenta, já com os arquivos indicados.
    pub fn open_tool(self: &Rc<Self>, info: &'static ToolInfo, files: Vec<PathBuf>) {
        {
            let mut s = self.settings.borrow_mut();
            s.touch_recent(info.key);
            s.save();
        }
        if let Some(home) = self.home.borrow().as_ref() {
            home.refresh_recent();
        }
        let page = match info.id {
            ToolId::Organize => organizer::build(self, files),
            ToolId::Annotate => annotator::build(self, files),
            _ => tool_page::build(self, info, files),
        };
        self.nav.push(&page);
    }

    pub fn stage_files(&self, files: Vec<PathBuf>) {
        if let Some(home) = self.home.borrow().as_ref() {
            home.add(files);
        }
        self.nav.pop_to_tag("inicio");
    }
}

pub fn startup(app: &adw::Application) {
    Settings::load().theme.apply();
    gtk::Window::set_default_icon_name(APP_ID);

    let display = gtk::gdk::Display::default().expect("display");
    // Ícones simbólicos próprios, embutidos no binário (veja build.rs).
    gio::resources_register_include!("pdftoolbox.gresource").expect("recursos embutidos");
    gtk::IconTheme::for_display(&display).add_resource_path("/io/github/leonardo_sponton/PDFToolbox/icons");
    let css = gtk::CssProvider::new();
    css.load_from_string(include_str!("style.css"));
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    theme::install(&display);

    let quit = gio::ActionEntry::builder("quit")
        .activate(|app: &adw::Application, _, _| {
            for w in app.windows() {
                w.close();
            }
        })
        .build();
    let theme = gio::ActionEntry::builder("theme")
        .parameter_type(Some(glib::VariantTy::STRING))
        .state(Settings::load().theme.key().to_variant())
        .activate(|_, action, param| {
            let key = param.and_then(|p| p.get::<String>()).unwrap_or_default();
            let theme = Theme::from_key(&key);
            theme.apply();
            action.set_state(&theme.key().to_variant());
            let mut s = Settings::load();
            s.theme = theme;
            s.save();
        })
        .build();
    app.add_action_entries([quit, theme]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("win.open", &["<Control>o"]);
    app.set_accels_for_action("win.search", &["<Control>f"]);
    app.set_accels_for_action("win.tasks", &["<Control>t"]);
    app.set_accels_for_action("win.home", &["<Alt>Home"]);
}

thread_local! {
    static STATE: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn activate(app: &adw::Application, files: &[gio::File]) {
    let state = STATE.with(|s| s.borrow().clone()).unwrap_or_else(|| {
        let st = build_window(app);
        STATE.with(|s| *s.borrow_mut() = Some(st.clone()));
        st
    });
    let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).collect();
    // Gancho de desenvolvimento/testes: abre uma ferramenta diretamente.
    let direct = std::env::var("PDFTOOLBOX_FERRAMENTA").ok().and_then(|k| pdftoolbox::catalog::by_key(&k));
    if let Some(info) = direct {
        state.open_tool(info, paths);
    } else if !paths.is_empty() {
        state.stage_files(paths);
    }
    state.window.present();
}

fn build_window(app: &adw::Application) -> App {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("PDFToolbox")
        .default_width(980)
        .default_height(720)
        .width_request(360)
        .height_request(400)
        .build();
    let nav = adw::NavigationView::new();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&nav));
    window.set_content(Some(&toasts));

    let state = Rc::new(AppState {
        window: window.clone(),
        nav: nav.clone(),
        toasts,
        settings: RefCell::new(Settings::load()),
        tasks: tasks::Tasks::new(),
        home: RefCell::new(None),
    });

    let home = home::Home::build(&state);
    nav.add(&home.page);
    *state.home.borrow_mut() = Some(home.clone());

    let weak = Rc::downgrade(&state);
    let actions = [
        gio::ActionEntry::builder("open")
            .activate({
                let weak = weak.clone();
                move |_: &adw::ApplicationWindow, _, _| {
                    let Some(st) = weak.upgrade() else { return };
                    // Na tela inicial abre arquivos; nas ferramentas, delega à ação da página.
                    let Some(page) = st.nav.visible_page() else { return };
                    if page.tag().as_deref() == Some("inicio") {
                        if let Some(home) = st.home.borrow().clone() {
                            home.open_files();
                        }
                    } else {
                        let _ = page.activate_action("page.add", None);
                    }
                }
            })
            .build(),
        gio::ActionEntry::builder("search")
            .activate({
                let weak = weak.clone();
                move |_: &adw::ApplicationWindow, _, _| {
                    if let Some(st) = weak.upgrade() {
                        st.nav.pop_to_tag("inicio");
                        if let Some(home) = st.home.borrow().clone() {
                            home.focus_search();
                        }
                    }
                }
            })
            .build(),
        gio::ActionEntry::builder("tasks")
            .activate({
                let weak = weak.clone();
                move |_: &adw::ApplicationWindow, _, _| {
                    if let Some(st) = weak.upgrade() {
                        st.tasks.present(&st.window);
                    }
                }
            })
            .build(),
        gio::ActionEntry::builder("dependencies")
            .activate({
                let weak = weak.clone();
                move |_: &adw::ApplicationWindow, _, _| {
                    if let Some(st) = weak.upgrade() {
                        dialogs::dependencies(&st.window);
                    }
                }
            })
            .build(),
        gio::ActionEntry::builder("about")
            .activate({
                let weak = weak.clone();
                move |_: &adw::ApplicationWindow, _, _| {
                    if let Some(st) = weak.upgrade() {
                        dialogs::about(&st.window);
                    }
                }
            })
            .build(),
        gio::ActionEntry::builder("home")
            .activate({
                let weak = weak.clone();
                move |_: &adw::ApplicationWindow, _, _| {
                    if let Some(st) = weak.upgrade() {
                        st.nav.pop_to_tag("inicio");
                    }
                }
            })
            .build(),
    ];
    window.add_action_entries(actions);

    // Avisar antes de fechar com tarefas em andamento ou alterações não
    // salvas (páginas que não podem sair sozinhas: Organizar e Anotar).
    window.connect_close_request({
        let weak = weak.clone();
        move |win| {
            let Some(st) = weak.upgrade() else { return glib::Propagation::Proceed };
            let unsaved =
                st.nav.navigation_stack().iter::<adw::NavigationPage>().flatten().any(|p| !p.can_pop());
            let running = st.tasks.running();
            if running == 0 && !unsaved {
                return glib::Propagation::Proceed;
            }
            let (title, body) = match (running, unsaved) {
                (0, _) => (
                    "Descartar as alterações?",
                    "Há alterações que ainda não foram salvas. Se fechar agora, elas serão perdidas; os PDFs originais não foram alterados.",
                ),
                (_, false) => (
                    "Há tarefas em andamento",
                    "Se fechar agora, as tarefas serão interrompidas e os arquivos parciais descartados.",
                ),
                _ => (
                    "Há tarefas em andamento e alterações não salvas",
                    "Se fechar agora, as tarefas serão interrompidas, os arquivos parciais descartados e as alterações não salvas perdidas.",
                ),
            };
            let dialog = adw::AlertDialog::new(Some(title), Some(body));
            dialog.add_responses(&[("ficar", "Continuar aqui"), ("sair", "Fechar mesmo assim")]);
            dialog.set_response_appearance("sair", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("ficar"));
            dialog.set_close_response("ficar");
            let win2 = win.clone();
            dialog.connect_response(None, move |_, resp| {
                if resp == "sair" {
                    win2.destroy();
                }
            });
            dialog.present(Some(win));
            glib::Propagation::Stop
        }
    });
    state
}

/// Menu principal, compartilhado pelas páginas.
pub fn primary_menu() -> gtk::MenuButton {
    let theme = gio::Menu::new();
    theme.append(Some("Seguir o sistema"), Some("app.theme::sistema"));
    theme.append(Some("Claro"), Some("app.theme::claro"));
    theme.append(Some("Escuro"), Some("app.theme::escuro"));
    let menu = gio::Menu::new();
    menu.append_section(Some("Tema"), &theme);
    let more = gio::Menu::new();
    more.append(Some("Tarefas"), Some("win.tasks"));
    more.append(Some("Dependências"), Some("win.dependencies"));
    more.append(Some("Sobre o PDFToolbox"), Some("win.about"));
    menu.append_section(None, &more);
    let button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Menu principal")
        .primary(true)
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Menu principal")]);
    button
}
