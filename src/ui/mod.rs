//! Interface GTK4/libadwaita. Nenhum processamento acontece aqui: as páginas
//! montam uma `Request` e a entregam a `pdftoolbox::jobs::spawn`.

mod annotator;
mod dialogs;
mod editor;
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
mod unlock;
mod widgets;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use pdftoolbox::catalog::{ToolId, ToolInfo};
use pdftoolbox::i18n::{self, Preference};

pub use settings::{Settings, Theme};

pub const APP_ID: &str = "io.github.leonardo_sponton.PDFToolbox";
/// Etiqueta da tela inicial na navegação.
const HOME: &str = "inicio";

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

    /// Refaz os textos com o idioma atual: menu principal, tela inicial
    /// (mantendo os arquivos escolhidos) e lista de tarefas. Diálogos e
    /// páginas abertos daqui em diante já nascem no novo idioma; as páginas
    /// de ferramentas já abertas mantêm o anterior até serem fechadas, para
    /// não perder opções, senhas digitadas e alterações não salvas.
    fn relocalize(self: &Rc<Self>) {
        refresh_primary_menus();
        self.tasks.relocalize();
        let old = self.home.borrow_mut().take();
        let home = home::Home::build(self);
        // A etiqueta precisa ser única na navegação: a nova tela entra sem
        // ela, a antiga sai, e só então a etiqueta passa para a nova.
        home.page.set_tag(None);
        self.nav.add(&home.page);
        let mut stack: Vec<adw::NavigationPage> =
            self.nav.navigation_stack().iter::<adw::NavigationPage>().flatten().collect();
        let tools_open = stack.len() > 1;
        match stack.first_mut() {
            Some(first) if first.tag().as_deref() == Some(HOME) => *first = home.page.clone(),
            _ => stack.insert(0, home.page.clone()),
        }
        self.nav.replace(&stack);
        if let Some(old) = &old {
            self.nav.remove(&old.page);
            home.add(old.staged_files());
        }
        home.page.set_tag(Some(HOME));
        *self.home.borrow_mut() = Some(home);
        if tools_open {
            self.toast(tr!("As ferramentas já abertas mudam de idioma quando forem abertas de novo."));
        }
    }

    pub fn stage_files(&self, files: Vec<PathBuf>) {
        if let Some(home) = self.home.borrow().as_ref() {
            home.add(files);
        }
        self.nav.pop_to_tag(HOME);
    }
}

/// Aplica a preferência de idioma guardada, antes de o GTK iniciar.
pub fn init_language() {
    let language = i18n::apply(Settings::load().language);
    // SAFETY: chamado no início de `main`, antes de existir outra thread.
    unsafe { i18n::align_system_libraries(language, i18n::system()) };
}

/// Altera e grava as preferências. Usa a cópia da janela, se existir: senão
/// o próximo save() dela (recentes, opções) desfaria a alteração.
fn update_settings(change: impl FnOnce(&mut Settings)) {
    match STATE.with(|s| s.borrow().clone()) {
        Some(st) => {
            let mut s = st.settings.borrow_mut();
            change(&mut s);
            s.save();
        }
        None => {
            let mut s = Settings::load();
            change(&mut s);
            s.save();
        }
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
            update_settings(|s| s.theme = theme);
        })
        .build();
    let language = gio::ActionEntry::builder("language")
        .parameter_type(Some(glib::VariantTy::STRING))
        .state(Settings::load().language.key().to_variant())
        .activate(|_, action, param| {
            let key = param.and_then(|p| p.get::<String>()).unwrap_or_default();
            let preference = Preference::from_key(&key);
            action.set_state(&preference.key().to_variant());
            update_settings(|s| s.language = preference);
            let before = i18n::current();
            if i18n::apply(preference) != before
                && let Some(st) = STATE.with(|s| s.borrow().clone())
            {
                st.relocalize();
            }
        })
        .build();
    app.add_action_entries([quit, theme, language]);
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

    let action = |name: &str, run: fn(&AppState)| {
        gio::ActionEntry::builder(name)
            .activate(glib::clone!(
                #[weak]
                state,
                move |_: &adw::ApplicationWindow, _, _| run(&state)
            ))
            .build()
    };
    window.add_action_entries([
        action("open", |st| {
            // Na tela inicial abre arquivos; nas ferramentas, delega à ação da página.
            let Some(page) = st.nav.visible_page() else { return };
            if page.tag().as_deref() == Some(HOME) {
                if let Some(home) = st.home.borrow().clone() {
                    home.open_files();
                }
            } else {
                let _ = page.activate_action("page.add", None);
            }
        }),
        action("search", |st| {
            st.nav.pop_to_tag(HOME);
            if let Some(home) = st.home.borrow().clone() {
                home.focus_search();
            }
        }),
        action("tasks", |st| st.tasks.present(&st.window)),
        action("dependencies", |st| dialogs::dependencies(&st.window)),
        action("about", |st| dialogs::about(&st.window)),
        action("home", |st| {
            st.nav.pop_to_tag(HOME);
        }),
    ]);
    window.connect_close_request(glib::clone!(
        #[weak]
        state,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |win| confirm_close(&state, win)
    ));
    state
}

/// Avisa antes de fechar com tarefas em andamento ou alterações não salvas
/// (páginas que não podem sair sozinhas: Organizar e Anotar).
fn confirm_close(st: &AppState, win: &adw::ApplicationWindow) -> glib::Propagation {
    let unsaved = st.nav.navigation_stack().iter::<adw::NavigationPage>().flatten().any(|p| !p.can_pop());
    let running = st.tasks.running();
    if running == 0 && !unsaved {
        return glib::Propagation::Proceed;
    }
    let (title, body) = match (running, unsaved) {
        (0, _) => (
            tr!("Descartar as alterações?"),
            tr!("Há alterações que ainda não foram salvas. Se fechar agora, elas serão perdidas; os PDFs originais \
                 não foram alterados."),
        ),
        (_, false) => (
            tr!("Há tarefas em andamento"),
            tr!("Se fechar agora, as tarefas serão interrompidas e os arquivos parciais descartados."),
        ),
        _ => (
            tr!("Há tarefas em andamento e alterações não salvas"),
            tr!("Se fechar agora, as tarefas serão interrompidas, os arquivos parciais descartados e as alterações \
                 não salvas perdidas."),
        ),
    };
    let dialog = adw::AlertDialog::new(Some(title), Some(body));
    dialog.add_responses(&[("ficar", tr!("Continuar aqui")), ("sair", tr!("Fechar mesmo assim"))]);
    dialog.set_response_appearance("sair", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("ficar"));
    dialog.set_close_response("ficar");
    let window = win.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "sair" {
            window.destroy();
        }
    });
    dialog.present(Some(win));
    glib::Propagation::Stop
}

thread_local! {
    /// Modelo único do menu principal: refeito no lugar quando o idioma
    /// muda, e todos os botões de menu já criados mostram os novos textos.
    static PRIMARY_MENU: gio::Menu = gio::Menu::new();
    static MENU_BUTTONS: RefCell<Vec<glib::WeakRef<gtk::MenuButton>>> = const { RefCell::new(Vec::new()) };
}

fn fill_primary_menu(menu: &gio::Menu) {
    menu.remove_all();
    let theme = gio::Menu::new();
    theme.append(Some(tr!("Seguir o sistema")), Some("app.theme::sistema"));
    theme.append(Some(tr!("Claro")), Some("app.theme::claro"));
    theme.append(Some(tr!("Escuro")), Some("app.theme::escuro"));
    theme.append(Some(tr!("Minimalista")), Some("app.theme::minimalista"));
    menu.append_section(Some(tr!("Tema")), &theme);
    // Os nomes dos idiomas ficam na própria língua, para serem achados
    // mesmo com a interface num idioma que a pessoa não lê.
    let language = gio::Menu::new();
    for preference in Preference::ALL {
        let label = match preference {
            Preference::System => tr!("Seguir o sistema"),
            Preference::Fixed(language) => language.native_name(),
        };
        language.append(Some(label), Some(&format!("app.language::{}", preference.key())));
    }
    menu.append_section(Some(tr!("Idioma · Language")), &language);
    let more = gio::Menu::new();
    more.append(Some(tr!("Tarefas")), Some("win.tasks"));
    more.append(Some(tr!("Dependências")), Some("win.dependencies"));
    more.append(Some(tr!("Sobre o PDFToolbox")), Some("win.about"));
    menu.append_section(None, &more);
}

fn refresh_primary_menus() {
    PRIMARY_MENU.with(fill_primary_menu);
    MENU_BUTTONS.with(|buttons| {
        buttons.borrow_mut().retain(|weak| match weak.upgrade() {
            Some(button) => {
                label_menu_button(&button);
                true
            }
            None => false,
        });
    });
}

fn label_menu_button(button: &gtk::MenuButton) {
    button.set_tooltip_text(Some(tr!("Menu principal")));
    button.update_property(&[gtk::accessible::Property::Label(tr!("Menu principal"))]);
}

/// Botão do menu principal, compartilhado pelas páginas.
pub fn primary_menu() -> gtk::MenuButton {
    let menu = PRIMARY_MENU.with(|menu| {
        if menu.n_items() == 0 {
            fill_primary_menu(menu);
        }
        menu.clone()
    });
    let button = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).build();
    label_menu_button(&button);
    MENU_BUTTONS.with(|buttons| buttons.borrow_mut().push(button.downgrade()));
    button
}
