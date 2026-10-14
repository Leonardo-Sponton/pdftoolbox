//! Tela inicial: área de soltar arquivos, busca, recentes e categorias.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;

use pdftoolbox::catalog::{self, Accepts, Category, TOOLS, ToolId, ToolInfo};
use pdftoolbox::human;

use super::{AppState, primary_menu, thumbs, widgets};

pub struct Home {
    pub page: adw::NavigationPage,
    app: Weak<AppState>,
    search_bar: gtk::SearchBar,
    search: gtk::SearchEntry,
    empty_box: gtk::Box,
    staged_box: gtk::Box,
    staged_title: gtk::Label,
    staged_flow: gtk::FlowBox,
    suggest: gtk::Button,
    suggest_content: adw::ButtonContent,
    staged: RefCell<Vec<PathBuf>>,
    /// PDFs escolhidos que exigem senha (descobertos ao ler os arquivos).
    locked: RefCell<Vec<PathBuf>>,
    /// PDFs com a maior parte das páginas sem texto selecionável (digitalizações).
    scanned: RefCell<Vec<PathBuf>>,
    recent_box: gtk::FlowBox,
    recent_section: gtk::Box,
    cards: RefCell<Vec<ToolCard>>,
    recent_cards: RefCell<Vec<ToolCard>>,
    sections: Vec<(Category, gtk::Box)>,
    no_results: adw::StatusPage,
}

/// Cartão de ferramenta com os elementos que mudam conforme os arquivos escolhidos.
struct ToolCard {
    info: &'static ToolInfo,
    child: gtk::FlowBoxChild,
    button: gtk::Button,
    reason: gtk::Label,
    badge: gtk::Label,
}

impl ToolCard {
    /// Mostra se a ferramenta aceita os arquivos escolhidos e se é a sugerida.
    fn reflect(&self, files: &[PathBuf], suggested: Option<ToolId>) {
        let ok = files.is_empty() || files.iter().any(|f| catalog::extension_ok(self.info.accepts, f));
        self.button.set_sensitive(ok);
        self.reason.set_visible(!ok);
        self.reason.set_text(match self.info.accepts {
            Accepts::Pdf => tr!("Só aceita PDFs"),
            Accepts::Images => tr!("Só aceita imagens"),
            Accepts::Office => tr!("Só aceita documentos, planilhas e apresentações"),
        });
        let is_suggested = ok && suggested == Some(self.info.id);
        self.badge.set_visible(is_suggested);
        if is_suggested {
            self.button.add_css_class("sugerida");
        } else {
            self.button.remove_css_class("sugerida");
        }
    }
}

/// Ferramenta mais provável para os arquivos escolhidos. `locked` são os PDFs
/// já identificados como protegidos por senha; `scanned`, os sem texto.
fn suggestion(files: &[PathBuf], locked: &[PathBuf], scanned: &[PathBuf]) -> Option<ToolId> {
    let count = |a: Accepts| files.iter().filter(|f| catalog::extension_ok(a, f)).count();
    let (pdfs, images, office) = (count(Accepts::Pdf), count(Accepts::Images), count(Accepts::Office));
    // Só PDFs com senha: o mais provável é querer tirar a senha. (As demais
    // ferramentas também os abrem, pedindo a senha.)
    let all_locked = files.iter().all(|f| locked.contains(f));
    match (pdfs, images, office) {
        (0, 0, 0) => None,
        (0, _, 0) => Some(ToolId::ImagesToPdf),
        (0, 0, _) => Some(ToolId::OfficeToPdf),
        (_, 0, 0) if all_locked => Some(ToolId::Unlock),
        // Só digitalizações: tornar o texto pesquisável vem antes do resto.
        (_, 0, 0) if files.iter().all(|f| scanned.contains(f)) => Some(ToolId::Ocr),
        (1, 0, 0) => Some(ToolId::Organize),
        (_, 0, 0) => Some(ToolId::Merge),
        _ => None,
    }
}

/// “2 PDFs, 1 imagem e 3 documentos prontos”.
fn staged_title(files: &[PathBuf]) -> String {
    let count = |a: Accepts| files.iter().filter(|f| catalog::extension_ok(a, f)).count();
    let (pdfs, images, office) = (count(Accepts::Pdf), count(Accepts::Images), count(Accepts::Office));
    let mut parts = Vec::new();
    if pdfs > 0 {
        parts.push(trn!(pdfs, "{n} PDF", "{n} PDFs", n = pdfs));
    }
    if images > 0 {
        parts.push(trn!(images, "{n} imagem", "{n} imagens", n = images));
    }
    if office > 0 {
        parts.push(trn!(office, "{n} documento", "{n} documentos", n = office));
    }
    let list = match parts.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => tr!("{first} e {last}", first = init.join(", "), last),
    };
    // “imagem” é feminino; com outros tipos na mistura, o plural fica no masculino.
    let feminine = images == files.len();
    match (files.len() > 1, feminine) {
        (false, true) => tr!("{list} pronta", list),
        (false, false) => tr!("{list} pronto", list),
        (true, true) => tr!("{list} prontas", list),
        (true, false) => tr!("{list} prontos", list),
    }
}

fn tool_card(info: &'static ToolInfo, on_click: impl Fn(&'static ToolInfo) + 'static) -> ToolCard {
    let icon = gtk::Image::builder().icon_name(info.icon).pixel_size(28).halign(gtk::Align::Start).build();
    icon.add_css_class("tool-icon");
    let badge = gtk::Label::builder()
        .label(tr!("Sugerida"))
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .hexpand(true)
        .visible(false)
        .build();
    badge.add_css_class("badge");
    badge.add_css_class("ok");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    top.append(&icon);
    top.append(&badge);
    // Larguras máximas limitam a largura natural, para o FlowBox caber 2–3 colunas.
    let title = gtk::Label::builder().label(info.title()).xalign(0.0).wrap(true).max_width_chars(24).build();
    title.add_css_class("heading");
    let summary = gtk::Label::builder()
        .label(info.summary())
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(3)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(28)
        .width_chars(20)
        .build();
    summary.add_css_class("dim-label");
    summary.add_css_class("caption");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.append(&top);
    content.append(&title);
    content.append(&summary);
    let reason = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).build();
    reason.add_css_class("caption");
    reason.add_css_class("texto-aviso");
    content.append(&reason);
    let missing: Vec<&str> = info.needs.iter().filter(|t| t.find().is_none()).map(|t| t.label()).collect();
    if !missing.is_empty() {
        let warn = gtk::Label::builder()
            .label(tr!("Requer {programs}", programs = missing.join(", ")))
            .xalign(0.0)
            .wrap(true)
            .build();
        warn.add_css_class("warning");
        warn.add_css_class("caption");
        content.append(&warn);
    }
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("card");
    button.add_css_class("tool-card");
    button.update_property(&[
        gtk::accessible::Property::Label(info.title()),
        gtk::accessible::Property::Description(info.summary()),
    ]);
    button.connect_clicked(move |_| on_click(info));
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&button));
    child.set_focusable(false);
    ToolCard { info, child, button, reason, badge }
}

fn card_grid() -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(3)
        .column_spacing(12)
        .row_spacing(12)
        .build()
}

fn section(title: &str) -> (gtk::Box, gtk::FlowBox) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let label = gtk::Label::builder().label(title).xalign(0.0).build();
    label.add_css_class("title-4");
    label.add_css_class("category-title");
    let grid = card_grid();
    b.append(&label);
    b.append(&grid);
    (b, grid)
}

impl Home {
    pub fn build(app: &Rc<AppState>) -> Rc<Home> {
        let header = adw::HeaderBar::new();
        let open_button =
            gtk::Button::builder().label(tr!("Abrir")).tooltip_text(tr!("Abrir arquivos (Ctrl+O)")).build();
        header.pack_start(&open_button);
        header.pack_end(&primary_menu());
        let tasks_button = widgets::icon_button("view-list-bullet-symbolic", tr!("Tarefas (Ctrl+T)"));
        tasks_button.set_action_name(Some("win.tasks"));
        header.pack_end(&tasks_button);
        let search_toggle = gtk::ToggleButton::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text(tr!("Buscar ferramentas (Ctrl+F)"))
            .build();
        header.pack_end(&search_toggle);

        let search = gtk::SearchEntry::builder()
            .placeholder_text(tr!("Buscar ferramenta — por exemplo: juntar, jpg, girar"))
            .hexpand(true)
            .build();
        let search_bar =
            gtk::SearchBar::builder().child(&adw::Clamp::builder().maximum_size(560).child(&search).build()).build();
        search_bar.connect_entry(&search);
        search_bar.set_key_capture_widget(Some(&app.window));
        search_toggle.bind_property("active", &search_bar, "search-mode-enabled").bidirectional().build();

        // Área de soltar
        let drop_icon = gtk::Image::builder().icon_name("document-open-symbolic").pixel_size(48).build();
        drop_icon.add_css_class("dim-label");
        let drop_title = gtk::Label::builder()
            .label(tr!("Arraste PDFs ou imagens para cá"))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        drop_title.add_css_class("title-3");
        let drop_sub = gtk::Label::builder()
            .label(tr!("Tudo é processado neste computador. Nenhum arquivo é enviado para a internet."))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        drop_sub.add_css_class("dim-label");
        let pick = gtk::Button::builder().label(tr!("Escolher arquivos…")).halign(gtk::Align::Center).build();
        pick.add_css_class("pill");
        pick.add_css_class("suggested-action");
        let empty_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        for w in
            [drop_icon.upcast_ref::<gtk::Widget>(), drop_title.upcast_ref(), drop_sub.upcast_ref(), pick.upcast_ref()]
        {
            empty_box.append(w);
        }

        // Arquivos escolhidos: cartões com a primeira página.
        let staged_title = gtk::Label::builder().xalign(0.0).wrap(true).build();
        staged_title.add_css_class("title-3");
        staged_title.add_css_class("titulo-pagina");
        let staged_sub = gtk::Label::builder()
            .label(tr!("Escolha uma ferramenta abaixo. As que não aceitam esses arquivos ficam esmaecidas; arraste mais arquivos para cá a qualquer momento."))
            .xalign(0.0)
            .wrap(true)
            .build();
        staged_sub.add_css_class("dim-label");
        let add_more = gtk::Button::builder().label(tr!("Adicionar mais…")).valign(gtk::Align::Center).build();
        let clear_button = gtk::Button::builder().label(tr!("Limpar")).valign(gtk::Align::Center).build();
        clear_button.add_css_class("flat");
        clear_button.set_tooltip_text(Some(tr!("Tirar todos os arquivos da lista (nada é apagado do disco)")));
        let head_text = gtk::Box::new(gtk::Orientation::Vertical, 4);
        head_text.set_hexpand(true);
        head_text.append(&staged_title);
        head_text.append(&staged_sub);
        let suggest_content = adw::ButtonContent::new();
        let suggest = gtk::Button::builder().child(&suggest_content).valign(gtk::Align::Center).visible(false).build();
        suggest.add_css_class("suggested-action");
        // Os botões quebram linha em janelas estreitas.
        let buttons = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(1)
            .max_children_per_line(3)
            .column_spacing(8)
            .row_spacing(8)
            .halign(gtk::Align::Start)
            .build();
        for b in [&suggest, &add_more, &clear_button] {
            let child = gtk::FlowBoxChild::new();
            child.set_child(Some(b));
            child.set_focusable(false);
            buttons.append(&child);
            // O botão de sugestão some junto com o seu contêiner.
            if b == &suggest {
                suggest.bind_property("visible", &child, "visible").sync_create().build();
            }
        }
        let head = gtk::Box::new(gtk::Orientation::Vertical, 10);
        head.append(&head_text);
        head.append(&buttons);
        let staged_flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(2)
            .max_children_per_line(6)
            .column_spacing(12)
            .row_spacing(12)
            .homogeneous(true)
            .can_focus(false)
            .build();
        let staged_box = gtk::Box::new(gtk::Orientation::Vertical, 14);
        staged_box.append(&head);
        staged_box.append(&staged_flow);
        staged_box.set_visible(false);

        let drop_zone = gtk::Box::new(gtk::Orientation::Vertical, 8);
        drop_zone.add_css_class("drop-zone");
        drop_zone.append(&empty_box);
        drop_zone.append(&staged_box);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 24);
        body.set_margin_top(18);
        body.set_margin_bottom(32);
        body.set_margin_start(16);
        body.set_margin_end(16);
        body.append(&drop_zone);

        let (recent_section, recent_box) = section(tr!("Usadas recentemente"));
        body.append(&recent_section);

        let no_results = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title(tr!("Nenhuma ferramenta encontrada"))
            .description(tr!("Tente outras palavras, como “unir”, “jpg” ou “reduzir”."))
            .visible(false)
            .build();
        body.append(&no_results);

        let home = Rc::new_cyclic(|weak: &Weak<Home>| {
            let mut sections = Vec::new();
            let mut cards = Vec::new();
            for cat in Category::ALL {
                let (sec, grid) = section(cat.title());
                for info in TOOLS.iter().filter(|t| t.category == cat) {
                    let w = weak.clone();
                    let card = tool_card(info, move |info| {
                        if let Some(h) = w.upgrade() {
                            h.launch(info);
                        }
                    });
                    grid.append(&card.child);
                    cards.push(card);
                }
                body.append(&sec);
                sections.push((cat, sec));
            }
            let note = gtk::Label::builder()
                .label(tr!("Próximas ferramentas no roteiro do projeto: redação definitiva, formulários, PDF/A e comparação de documentos."))
                .wrap(true)
                .xalign(0.0)
                .build();
            note.add_css_class("dim-label");
            note.add_css_class("caption");
            body.append(&note);

            let clamp = adw::Clamp::builder().maximum_size(920).child(&body).build();
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&clamp)
                .vexpand(true)
                .build();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.append(&search_bar);
            content.append(&scroll);
            let toolbar = adw::ToolbarView::new();
            toolbar.add_top_bar(&header);
            toolbar.set_content(Some(&content));
            let page = adw::NavigationPage::with_tag(&toolbar, "PDFToolbox", super::HOME);

            Home {
                page,
                app: Rc::downgrade(app),
                search_bar: search_bar.clone(),
                search: search.clone(),
                empty_box,
                staged_box,
                staged_title,
                staged_flow,
                suggest: suggest.clone(),
                suggest_content,
                staged: RefCell::new(Vec::new()),
                locked: RefCell::new(Vec::new()),
                scanned: RefCell::new(Vec::new()),
                recent_box,
                recent_section,
                cards: RefCell::new(cards),
                recent_cards: RefCell::new(Vec::new()),
                sections,
                no_results,
            }
        });

        search.connect_search_changed(glib::clone!(
            #[weak]
            home,
            move |_| home.filter()
        ));
        search.connect_activate(glib::clone!(
            #[weak]
            home,
            move |_| {
                let query = home.search.text();
                if let Some(info) = TOOLS.iter().find(|t| catalog::matches(t, &query)) {
                    home.launch(info);
                }
            }
        ));
        suggest.connect_clicked(glib::clone!(
            #[weak]
            home,
            move |_| {
                if let Some(id) = home.suggestion() {
                    home.launch(catalog::info(id));
                }
            }
        ));
        clear_button.connect_clicked(glib::clone!(
            #[weak]
            home,
            move |_| home.stage(Vec::new())
        ));
        for b in [&open_button, &pick, &add_more] {
            b.connect_clicked(glib::clone!(
                #[weak]
                home,
                move |_| home.open_files()
            ));
        }
        widgets::add_file_drop(
            &home.page,
            glib::clone!(
                #[weak]
                home,
                move |files| home.add(files)
            ),
        );
        home.refresh_recent();
        home
    }

    /// Arquivos escolhidos e ainda não levados a uma ferramenta.
    pub fn staged_files(&self) -> Vec<PathBuf> {
        self.staged.borrow().clone()
    }

    pub fn focus_search(&self) {
        self.search_bar.set_search_mode(true);
        self.search.grab_focus();
    }

    pub fn open_files(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let h = self.clone();
        glib::spawn_future_local(async move {
            let files = widgets::pick_files(&app.window, None, true).await;
            if !files.is_empty() {
                h.add(files);
            }
        });
    }

    /// Acrescenta arquivos aos já escolhidos (sem repetir).
    pub fn add(self: &Rc<Self>, files: Vec<PathBuf>) {
        let mut all = self.staged.borrow().clone();
        for f in files {
            if !all.contains(&f) {
                all.push(f);
            }
        }
        self.stage(all);
    }

    /// Guarda arquivos abertos/soltos na tela inicial até o usuário escolher a ferramenta.
    pub fn stage(self: &Rc<Self>, files: Vec<PathBuf>) {
        let files: Vec<PathBuf> = files
            .into_iter()
            .filter(|f| [Accepts::Pdf, Accepts::Images, Accepts::Office].iter().any(|a| catalog::extension_ok(*a, f)))
            .collect();
        self.locked.borrow_mut().retain(|f| files.contains(f));
        self.scanned.borrow_mut().retain(|f| files.contains(f));
        self.empty_box.set_visible(files.is_empty());
        self.staged_box.set_visible(!files.is_empty());
        if !files.is_empty() {
            self.staged_title.set_text(&staged_title(&files));
        }
        // Cartões dos arquivos (recriados: a lista é curta e não tem navegação por teclado).
        while let Some(c) = self.staged_flow.first_child() {
            self.staged_flow.remove(&c);
        }
        for path in &files {
            self.staged_flow.append(&self.file_card(path));
        }
        *self.staged.borrow_mut() = files;
        self.refresh_suggestion();
    }

    /// Ferramenta sugerida para os arquivos escolhidos.
    fn suggestion(&self) -> Option<ToolId> {
        suggestion(&self.staged.borrow(), &self.locked.borrow(), &self.scanned.borrow())
    }

    /// Recalcula a sugestão e o estado dos cartões de ferramenta.
    fn refresh_suggestion(&self) {
        let files = self.staged.borrow().clone();
        let suggested = self.suggestion();
        // Atalho para a ferramenta mais provável, junto dos arquivos.
        self.suggest.set_visible(suggested.is_some());
        if let Some(id) = suggested {
            let info = catalog::info(id);
            self.suggest_content.set_icon_name(info.icon);
            self.suggest_content.set_label(info.title());
            self.suggest
                .set_tooltip_text(Some(&tr!("Sugestão para estes arquivos: {summary}", summary = info.summary())));
        }
        for card in self.cards.borrow().iter().chain(self.recent_cards.borrow().iter()) {
            card.reflect(&files, suggested);
        }
    }

    /// Cartão de um arquivo escolhido: primeira página, nome, páginas e tamanho.
    fn file_card(self: &Rc<Self>, path: &Path) -> gtk::FlowBoxChild {
        let thumb = thumbs::picture(path, 112, 132);
        let holder = gtk::Box::builder().height_request(140).valign(gtk::Align::End).build();
        thumb.set_hexpand(true);
        thumb.set_valign(gtk::Align::End);
        if thumbs::is_office(path) {
            // Sem miniatura (seria preciso converter): ícone do tipo de arquivo.
            let icon = gtk::Image::builder()
                .icon_name("pdftoolbox-office-para-pdf-symbolic")
                .pixel_size(56)
                .hexpand(true)
                .valign(gtk::Align::Center)
                .vexpand(true)
                .build();
            icon.add_css_class("tool-icon");
            icon.add_css_class("icone-documento");
            holder.append(&icon);
        } else {
            holder.append(&thumb);
        }
        let remove = widgets::icon_button("window-close-symbolic", tr!("Tirar da lista"));
        // Com contorno e fundo próprio, visível sobre qualquer miniatura e tema.
        remove.remove_css_class("flat");
        remove.add_css_class("circular");
        remove.add_css_class("tirar-arquivo");
        remove.set_halign(gtk::Align::End);
        remove.set_valign(gtk::Align::Start);
        let overlay = gtk::Overlay::builder().child(&holder).build();
        overlay.add_overlay(&remove);
        let name = gtk::Label::builder()
            .label(human::file_name(path))
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(18)
            .build();
        name.add_css_class("nome");
        name.add_css_class("caption");
        let facts = gtk::Label::builder().label(tr!("Lendo…")).wrap(true).justify(gtk::Justification::Center).build();
        facts.add_css_class("caption");
        facts.add_css_class("dim-label");
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("arquivo-card");
        card.append(&overlay);
        card.append(&name);
        card.append(&facts);
        card.set_tooltip_text(Some(&path.to_string_lossy()));
        let is_pdf = catalog::extension_ok(Accepts::Pdf, path);
        let (f, c, t, p) = (facts.downgrade(), card.downgrade(), thumb.downgrade(), path.to_path_buf());
        let h = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let info = thumbs::facts(&p).await;
            if let Some(t) = t.upgrade()
                && !t.has_texture()
                && let Some((w, h)) = info.dims
            {
                // Formato certo enquanto a miniatura carrega.
                t.set_placeholder(w as f64, h as f64);
            }
            if let (Some(f), Some(c)) = (f.upgrade(), c.upgrade()) {
                f.set_text(&info.line(is_pdf));
                if info.problem.is_some() {
                    f.remove_css_class("dim-label");
                    f.add_css_class("texto-erro");
                    c.add_css_class("problema");
                }
            }
            if (info.locked || info.mostly_scanned())
                && let Some(h) = h.upgrade()
                && h.staged.borrow().contains(&p)
            {
                if info.locked {
                    h.locked.borrow_mut().push(p.clone());
                } else {
                    h.scanned.borrow_mut().push(p.clone());
                }
                h.refresh_suggestion();
            }
        });
        let p = path.to_path_buf();
        remove.connect_clicked(glib::clone!(
            #[weak(rename_to = home)]
            self,
            move |_| {
                let rest: Vec<PathBuf> = home.staged.borrow().iter().filter(|f| **f != p).cloned().collect();
                home.stage(rest);
            }
        ));
        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&card));
        child.set_focusable(false);
        child
    }

    fn launch(self: &Rc<Self>, info: &'static ToolInfo) {
        let Some(app) = self.app.upgrade() else { return };
        let files: Vec<PathBuf> =
            self.staged.borrow().iter().filter(|f| catalog::extension_ok(info.accepts, f)).cloned().collect();
        self.stage(Vec::new());
        app.open_tool(info, files);
    }

    pub fn refresh_recent(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        while let Some(child) = self.recent_box.first_child() {
            self.recent_box.remove(&child);
        }
        let recent: Vec<&'static ToolInfo> =
            app.settings.borrow().recent.iter().filter_map(|k| catalog::by_key(k)).take(3).collect();
        let staged = self.staged.borrow().clone();
        let suggested = self.suggestion();
        let mut cards = Vec::new();
        for info in &recent {
            let card = tool_card(
                info,
                glib::clone!(
                    #[weak(rename_to = home)]
                    self,
                    move |info| home.launch(info)
                ),
            );
            card.reflect(&staged, suggested);
            self.recent_box.append(&card.child);
            cards.push(card);
        }
        *self.recent_cards.borrow_mut() = cards;
        self.recent_section.set_visible(!recent.is_empty() && self.search.text().is_empty());
    }

    fn filter(&self) {
        let q = self.search.text();
        let searching = !q.trim().is_empty();
        let mut any = false;
        for card in self.cards.borrow().iter() {
            let show = catalog::matches(card.info, &q);
            card.child.set_visible(show);
            any |= show;
        }
        for (cat, sec) in &self.sections {
            let has = self.cards.borrow().iter().any(|c| c.info.category == *cat && c.child.is_visible());
            sec.set_visible(has);
        }
        self.recent_section.set_visible(!searching && self.recent_box.first_child().is_some());
        self.no_results.set_visible(!any);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_suggest_ocr_and_locked_pdfs_suggest_unlock() {
        let (scan, typed, locked) =
            (PathBuf::from("/a/scan.pdf"), PathBuf::from("/a/b.pdf"), PathBuf::from("/a/c.pdf"));
        let scanned = [scan.clone()];
        assert_eq!(suggestion(std::slice::from_ref(&scan), &[], &scanned), Some(ToolId::Ocr));
        assert_eq!(suggestion(&[scan.clone(), typed.clone()], &[], &scanned), Some(ToolId::Merge));
        assert_eq!(suggestion(&[typed], &[], &scanned), Some(ToolId::Organize));
        let both = [scan, locked.clone()];
        assert_eq!(suggestion(&both, std::slice::from_ref(&locked), &scanned), Some(ToolId::Merge));
        assert_eq!(suggestion(std::slice::from_ref(&locked), std::slice::from_ref(&locked), &[]), Some(ToolId::Unlock));
    }
}
