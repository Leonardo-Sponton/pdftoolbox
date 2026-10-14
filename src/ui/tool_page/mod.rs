//! Página genérica de ferramenta: arquivos → opções → destino → executar →
//! progresso → resultado.
//!
//! - `options`: linhas de opções de cada ferramenta;
//! - `form`: leitura dessas linhas (pedido, plano, validação, rótulos);
//! - `background`: campos preenchidos em segundo plano;
//! - `run`: execução e resultado;
//! - `texts`: textos que dependem só de dados.

mod background;
mod form;
mod options;
mod run;
mod texts;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gio, glib};

use pdftoolbox::catalog::{self, Accepts, ToolId, ToolInfo};
use pdftoolbox::engines::Tool;
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::Request;
use pdftoolbox::preview::{self, MarkKind, Plan};
use pdftoolbox::{OpError, OpResult, human};

use super::page_preview::{LayoutPreview, PagePreview, StampPreview, TextPreview};
use super::thumbs::{self, FileFacts};
use super::{AppState, primary_menu, unlock, widgets};
use form::FormIssue;
use options::Options;
use texts::run_label;

pub struct ToolPage {
    app: Weak<AppState>,
    info: &'static ToolInfo,
    files: RefCell<Vec<PathBuf>>,
    file_info: RefCell<HashMap<PathBuf, FileFacts>>,
    files_group: adw::PreferencesGroup,
    file_rows: RefCell<Vec<adw::ActionRow>>,
    empty_row: adw::ActionRow,
    options: Options,
    options_group: adw::PreferencesGroup,
    preview: Option<Rc<PagePreview>>,
    layout: Option<LayoutPreview>,
    stamp_preview: Option<Rc<StampPreview>>,
    text_preview: Option<Rc<TextPreview>>,
    /// Aviso de validação do formulário, logo abaixo das opções.
    form_hint: gtk::Label,
    /// Verdadeiro enquanto um clique na grade reescreve o campo de páginas.
    from_click: Cell<bool>,
    dest_dir: RefCell<Option<PathBuf>>,
    dest_custom: Cell<bool>,
    dest_row: adw::ActionRow,
    dest_reset: gtk::Button,
    name_row: adw::EntryRow,
    run_button: gtk::Button,
    cancel_button: gtk::Button,
    progress: gtk::ProgressBar,
    status: gtk::Label,
    result_slot: gtk::Box,
    scroll: gtk::ScrolledWindow,
    running: RefCell<Option<CancelToken>>,
    deps_ok: bool,
    /// OCR: páginas com texto e total, por arquivo (`None` = lendo).
    text_pages: RefCell<HashMap<PathBuf, Option<(u32, u32)>>>,
}

/// Grupo que avisa das dependências que faltam, com o comando de instalação.
fn missing_tools_group(missing: &[Tool]) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Dependência ausente")
        .description(
            "Esta ferramenta não pode ser usada até que o componente abaixo seja instalado. Rode o comando em um \
             terminal e reabra a ferramenta.",
        )
        .build();
    for tool in missing {
        let row = widgets::plain_row(tool.label(), &tool.install_command());
        row.set_subtitle_selectable(true);
        row.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
        group.add(&row);
    }
    let more = gtk::Button::builder().label("Ver todas as dependências").halign(gtk::Align::Start).build();
    more.set_action_name(Some("win.dependencies"));
    more.add_css_class("flat");
    group.set_header_suffix(Some(&more));
    group
}

fn files_group(info: &ToolInfo) -> (adw::PreferencesGroup, adw::ActionRow) {
    let add_button = gtk::Button::builder()
        .label("Adicionar…")
        .valign(gtk::Align::Center)
        .tooltip_text("Adicionar arquivos (Ctrl+O)")
        .action_name("page.add")
        .build();
    add_button.add_css_class("flat");
    let group = adw::PreferencesGroup::builder()
        .title(if info.multiple { "Arquivos" } else { "Arquivo" })
        .header_suffix(&add_button)
        .build();
    let empty_row = widgets::plain_row(
        "Nenhum arquivo",
        match info.accepts {
            Accepts::Pdf => "Arraste PDFs para esta janela ou clique em Adicionar",
            Accepts::Images => "Arraste imagens para esta janela ou clique em Adicionar",
            Accepts::Office => "Arraste documentos, planilhas ou apresentações para esta janela",
        },
    );
    empty_row.add_css_class("dim-label");
    group.add(&empty_row);
    (group, empty_row)
}

/// Destino: pasta (com “Alterar…” e “voltar à pasta do original”) e nome.
struct DestinationRows {
    group: adw::PreferencesGroup,
    folder: adw::ActionRow,
    reset: gtk::Button,
    change: gtk::Button,
    name: adw::EntryRow,
}

fn destination_rows() -> DestinationRows {
    let group = adw::PreferencesGroup::builder()
        .title("Destino")
        .description(
            "O original nunca é alterado. Se já existir um arquivo com o mesmo nome, o novo recebe um número, como \
             “nome (2)”.",
        )
        .build();
    let folder = widgets::plain_row("Pasta", "A mesma do primeiro arquivo");
    let reset = widgets::icon_button("edit-undo-symbolic", "Voltar a usar a pasta do arquivo original");
    reset.set_visible(false);
    let change = gtk::Button::builder().label("Alterar…").valign(gtk::Align::Center).build();
    folder.add_suffix(&reset);
    folder.add_suffix(&change);
    folder.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    let name = adw::EntryRow::builder().title("Nome do arquivo (opcional)").build();
    group.add(&folder);
    group.add(&name);
    DestinationRows { group, folder, reset, change, name }
}

pub fn build(app: &Rc<AppState>, info: &'static ToolInfo, files: Vec<PathBuf>) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    header.pack_end(&primary_menu());

    let body = gtk::Box::new(gtk::Orientation::Vertical, 24);
    body.set_margin_top(18);
    body.set_margin_bottom(32);
    body.set_margin_start(16);
    body.set_margin_end(16);

    let summary = gtk::Label::builder().label(info.summary).wrap(true).xalign(0.0).build();
    summary.add_css_class("dim-label");
    body.append(&summary);

    let missing: Vec<Tool> = info.needs.iter().copied().filter(|t| t.find().is_none()).collect();
    let deps_ok = missing.is_empty();
    if !deps_ok {
        body.append(&missing_tools_group(&missing));
    }

    let (files_group, empty_row) = files_group(info);
    body.append(&files_group);

    let options_group = adw::PreferencesGroup::builder().title("Opções").build();
    let options = Options::build(info, &options_group);
    if !matches!(options, Options::None) || options_group.description().is_some() {
        body.append(&options_group);
    }
    let form_hint = gtk::Label::builder().wrap(true).xalign(0.0).visible(false).build();
    form_hint.set_accessible_role(gtk::AccessibleRole::Status);
    body.append(&form_hint);

    let stamp_preview = matches!(info.id, ToolId::NumberPages | ToolId::Watermark).then(StampPreview::new);
    if let Some(s) = &stamp_preview {
        body.append(&s.group);
    }
    let text_preview = (info.id == ToolId::PdfToText).then(TextPreview::new);
    if let Some(t) = &text_preview {
        body.append(&t.group);
    }
    let preview = match info.id {
        ToolId::Rotate | ToolId::Extract | ToolId::Remove | ToolId::NumberPages | ToolId::Watermark => {
            Some(PagePreview::new(true))
        }
        ToolId::Split => Some(PagePreview::new(false)),
        _ => None,
    };
    if let Some(p) = &preview {
        body.append(&p.group);
    }
    let layout = (info.id == ToolId::ImagesToPdf).then(LayoutPreview::new);
    if let Some(l) = &layout {
        body.append(&l.group);
    }

    let dest = destination_rows();
    body.append(&dest.group);

    let run_button = gtk::Button::builder()
        .label(run_label(info.id))
        .halign(gtk::Align::Center)
        .tooltip_text("Executar (Ctrl+Enter)")
        .sensitive(deps_ok)
        .build();
    run_button.add_css_class("pill");
    run_button.add_css_class("suggested-action");
    let cancel_button = gtk::Button::builder().label("Cancelar").halign(gtk::Align::Center).visible(false).build();
    cancel_button.add_css_class("pill");
    let progress = gtk::ProgressBar::builder().visible(false).show_text(false).build();
    let status = gtk::Label::builder().wrap(true).visible(false).justify(gtk::Justification::Center).build();
    status.add_css_class("dim-label");
    status.set_accessible_role(gtk::AccessibleRole::Status);
    let action_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    action_box.append(&run_button);
    action_box.append(&progress);
    action_box.append(&status);
    action_box.append(&cancel_button);
    body.append(&action_box);

    let result_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&result_slot);

    let clamp = adw::Clamp::builder().maximum_size(720).child(&body).build();
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroll));
    let nav_page = adw::NavigationPage::with_tag(&toolbar, info.title, info.key);

    let page = Rc::new(ToolPage {
        app: Rc::downgrade(app),
        info,
        files: RefCell::new(Vec::new()),
        file_info: RefCell::new(HashMap::new()),
        files_group,
        file_rows: RefCell::new(Vec::new()),
        empty_row,
        options,
        options_group,
        preview,
        layout,
        stamp_preview,
        text_preview,
        form_hint,
        from_click: Cell::new(false),
        dest_dir: RefCell::new(None),
        dest_custom: Cell::new(false),
        dest_row: dest.folder,
        dest_reset: dest.reset.clone(),
        name_row: dest.name,
        run_button: run_button.clone(),
        cancel_button: cancel_button.clone(),
        progress,
        status,
        result_slot,
        scroll,
        running: RefCell::new(None),
        deps_ok,
        text_pages: RefCell::new(HashMap::new()),
    });

    // Ações da página (Ctrl+O e Ctrl+Enter)
    let actions = gio::SimpleActionGroup::new();
    let add_action = gio::SimpleAction::new("add", None);
    add_action.connect_activate(glib::clone!(
        #[weak]
        page,
        move |_, _| page.pick_files()
    ));
    let run_action = gio::SimpleAction::new("run", None);
    run_action.connect_activate(glib::clone!(
        #[weak]
        page,
        move |_, _| page.run()
    ));
    actions.add_action(&add_action);
    actions.add_action(&run_action);
    nav_page.insert_action_group("page", Some(&actions));
    let shortcuts = widgets::shortcuts(&[("<Control>Return", "page.run"), ("<Control>KP_Enter", "page.run")]);
    shortcuts.set_scope(gtk::ShortcutScope::Global);
    nav_page.add_controller(shortcuts);

    // Sem ação: a sensibilidade do botão é controlada pela validação.
    run_button.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.run()
    ));
    cancel_button.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.cancel()
    ));
    dest.change.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.choose_folder()
    ));
    dest.reset.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| {
            page.dest_custom.set(false);
            page.update_destination();
        }
    ));
    widgets::add_file_drop(
        &nav_page,
        glib::clone!(
            #[weak]
            page,
            move |files| page.add_files(files)
        ),
    );
    page.connect_options();

    page.add_files(files);
    page.update_name_row();
    let entries = page.options.entries();
    // Ferramentas que pedem algo digitado já abrem com o cursor no campo.
    if let Some(first) = entries.first().cloned() {
        nav_page.connect_shown(move |_| {
            first.grab_focus();
        });
    }
    // Enter no campo executa a ferramenta.
    for row in entries {
        row.set_show_apply_button(false);
        row.connect_entry_activated(glib::clone!(
            #[weak]
            page,
            move |_| page.run()
        ));
    }
    widgets::keep_alive(&nav_page, page);
    nav_page
}

impl ToolPage {
    /// Liga as opções à página: mudanças atualizam nome, pré-visualização e
    /// botão; algumas ferramentas têm reações próprias.
    fn connect_options(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let refresh: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(page) = weak.upgrade()
                && !page.from_click.get()
            {
                page.update_name_row();
                page.refresh_preview();
            }
        });
        self.options.connect_changes(&refresh);
        match &self.options {
            Options::Text { layout, ocr, ocr_languages } => {
                self.load_text_ocr(ocr.clone(), ocr_languages.clone());
                layout.connect_active_notify(glib::clone!(
                    #[weak(rename_to = page)]
                    self,
                    move |l| {
                        if let Some(t) = &page.text_preview {
                            t.update(page.files.borrow().first().map(PathBuf::as_path), l.is_active());
                        }
                    }
                ));
            }
            Options::Ocr { languages, .. } => self.load_languages(languages.clone(), refresh.clone()),
            Options::Compress(c) => {
                c.level.connect_selected_notify(glib::clone!(
                    #[weak(rename_to = page)]
                    self,
                    move |_| page.refresh_estimate()
                ));
                // Enquanto o valor muda (setas, digitação), espera parar antes de calcular.
                c.dpi.connect_value_notify(glib::clone!(
                    #[weak(rename_to = page)]
                    self,
                    move |_| page.refresh_estimate_later()
                ));
            }
            _ => {}
        }
        if let Some(preview) = &self.preview {
            preview.connect_loaded(move || refresh());
            preview.connect_page_clicked(glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |n| page.toggle_page(n)
            ));
        }
    }

    fn window(&self) -> Option<adw::ApplicationWindow> {
        self.app.upgrade().map(|a| a.window.clone())
    }

    fn toast(&self, text: &str) {
        if let Some(app) = self.app.upgrade() {
            app.toast(text);
        }
    }

    fn pick_files(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let page = self.clone();
        glib::spawn_future_local(async move {
            let files = widgets::pick_files(&win, Some(page.info.accepts), page.info.multiple).await;
            page.add_files(files);
        });
    }

    fn choose_folder(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let page = self.clone();
        glib::spawn_future_local(async move {
            let initial = page.dest_dir.borrow().clone();
            if let Some(dir) = widgets::pick_folder(&win, initial.as_deref()).await {
                page.dest_dir.replace(Some(dir.clone()));
                page.dest_custom.set(true);
                if let Some(app) = page.app.upgrade() {
                    let mut s = app.settings.borrow_mut();
                    s.last_dir = Some(dir);
                    s.save();
                }
                page.update_destination();
            }
        });
    }

    /// Mesclar e Imagens para PDF: a ordem dos arquivos é a do resultado, e
    /// o mesmo arquivo pode entrar mais de uma vez.
    fn ordered_inputs(&self) -> bool {
        matches!(self.info.id, ToolId::Merge | ToolId::ImagesToPdf)
    }

    /// Acrescenta arquivos; PDFs com senha entram como cópia desbloqueada
    /// (a senha é pedida antes), exceto em “Remover senha”.
    pub fn add_files(self: &Rc<Self>, incoming: Vec<PathBuf>) {
        if incoming.is_empty() {
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        if self.info.id == ToolId::Unlock || self.info.accepts != Accepts::Pdf {
            self.add_ready_files(incoming);
            return;
        }
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let ready = unlock::prepare(&app, incoming).await;
            if let Some(page) = page.upgrade() {
                page.add_ready_files(ready);
            }
        });
    }

    fn add_ready_files(self: &Rc<Self>, incoming: Vec<PathBuf>) {
        if incoming.is_empty() {
            return;
        }
        let (ok, rejected): (Vec<PathBuf>, Vec<PathBuf>) =
            incoming.into_iter().partition(|f| catalog::extension_ok(self.info.accepts, f));
        if !rejected.is_empty() {
            let what = match self.info.accepts {
                Accepts::Pdf => "somente PDFs",
                Accepts::Images => "somente imagens JPEG, PNG, WebP, GIF, BMP ou TIFF",
                Accepts::Office => "somente documentos, planilhas e apresentações",
            };
            self.toast(&format!("{} arquivo(s) ignorado(s): esta ferramenta aceita {what}.", rejected.len()));
        }
        {
            let mut files = self.files.borrow_mut();
            if !self.info.multiple {
                files.clear();
                files.extend(ok.into_iter().take(1));
            } else {
                let mut skipped = 0;
                for f in ok {
                    if !self.ordered_inputs() && files.contains(&f) {
                        skipped += 1;
                    } else {
                        files.push(f);
                    }
                }
                if skipped > 0 {
                    self.toast(&format!("{skipped} arquivo(s) já estava(m) na lista."));
                }
            }
        }
        self.rebuild_files();
    }

    /// Refaz a lista de arquivos e tudo o que depende dela.
    fn rebuild_files(self: &Rc<Self>) {
        for row in self.file_rows.borrow_mut().drain(..) {
            self.files_group.remove(&row);
        }
        let files = self.files.borrow().clone();
        self.empty_row.set_visible(files.is_empty());
        for (i, path) in files.iter().enumerate() {
            let row = self.file_row(i, path, files.len());
            self.files_group.add(&row);
            self.file_rows.borrow_mut().push(row.clone());
            if !self.file_info.borrow().contains_key(path) {
                self.load_info(path.clone(), row);
            }
        }
        self.update_destination();
        self.update_name_row();
        if let Some(preview) = &self.preview {
            preview.set_files(&files);
        }
        match &self.options {
            Options::Text { layout, .. } => {
                if let Some(t) = &self.text_preview {
                    t.update(files.first().map(PathBuf::as_path), layout.is_active());
                }
            }
            Options::Metadata(m) => self.load_metadata(m, files.first().cloned()),
            Options::Compress(_) => self.refresh_estimate(),
            Options::Ocr { .. } => self.load_text_pages(&files),
            _ => {}
        }
        self.refresh_preview();
    }

    /// Linha do arquivo `i` de `count`: miniatura, nome, detalhes, botões
    /// para reordenar (quando a ordem importa) e para tirar da lista.
    fn file_row(self: &Rc<Self>, i: usize, path: &Path, count: usize) -> adw::ActionRow {
        let is_pdf = self.info.accepts == Accepts::Pdf;
        let unlock_tool = self.info.id == ToolId::Unlock;
        let subtitle = self
            .file_info
            .borrow()
            .get(path)
            .map_or_else(|| "Lendo…".into(), |f| file_line(path, f, is_pdf, unlock_tool));
        let row = widgets::plain_row(&human::file_name(path), &subtitle);
        row.set_tooltip_text(Some(&unlock::original(path).to_string_lossy()));
        let reorderable = self.info.multiple && self.ordered_inputs() && count > 1;
        if reorderable {
            let n = gtk::Label::new(Some(&(i + 1).to_string()));
            n.add_css_class("badge");
            n.add_css_class("acento");
            n.set_valign(gtk::Align::Center);
            n.set_tooltip_text(Some("Posição no resultado"));
            row.add_prefix(&n);
        }
        if thumbs::is_office(path) {
            let icon = gtk::Image::from_icon_name("pdftoolbox-office-para-pdf-symbolic");
            icon.set_pixel_size(32);
            icon.add_css_class("icone-documento");
            icon.set_margin_top(8);
            icon.set_margin_bottom(8);
            row.add_prefix(&icon);
        } else {
            let thumb = thumbs::picture(path, 40, 48);
            thumb.set_margin_top(6);
            thumb.set_margin_bottom(6);
            row.add_prefix(&thumb);
        }
        if reorderable {
            let up = widgets::icon_button("go-up-symbolic", "Mover para cima");
            let down = widgets::icon_button("go-down-symbolic", "Mover para baixo");
            up.set_sensitive(i > 0);
            down.set_sensitive(i + 1 < count);
            for (button, other) in [(&up, i.saturating_sub(1)), (&down, i + 1)] {
                button.connect_clicked(glib::clone!(
                    #[weak(rename_to = page)]
                    self,
                    move |_| {
                        page.files.borrow_mut().swap(i, other);
                        page.rebuild_files();
                    }
                ));
            }
            row.add_suffix(&up);
            row.add_suffix(&down);
        }
        let remove = widgets::icon_button("list-remove-symbolic", "Remover da lista");
        remove.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| {
                page.files.borrow_mut().remove(i);
                page.rebuild_files();
            }
        ));
        row.add_suffix(&remove);
        row
    }

    /// Lê páginas/dimensões em segundo plano e atualiza a linha.
    fn load_info(self: &Rc<Self>, path: PathBuf, row: adw::ActionRow) {
        let is_pdf = self.info.accepts == Accepts::Pdf;
        let unlock_tool = self.info.id == ToolId::Unlock;
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let facts = thumbs::facts(&path).await;
            row.set_subtitle(&file_line(&path, &facts, is_pdf, unlock_tool));
            if facts.problem.is_some() || (facts.locked && !unlock_tool) {
                row.add_css_class("texto-erro");
            }
            if let Some(page) = page.upgrade() {
                page.file_info.borrow_mut().insert(path, facts);
                page.refresh_preview();
            }
        });
    }

    /// Plano da ferramenta para o arquivo mostrado na pré-visualização.
    fn plan(&self) -> Option<Result<Plan, String>> {
        self.options.plan(self.preview.as_ref()?.total()?)
    }

    /// Atualiza descrições, validação, pré-visualizações e o botão principal.
    fn refresh_preview(&self) {
        self.update_description();
        let form_ok = self.show_form_check();
        let Some(preview) = &self.preview else {
            self.run_button.set_sensitive(self.deps_ok && form_ok);
            self.update_run_label(None);
            return;
        };
        let plan = self.plan();
        let idle = self.options.idle_hint();
        match &plan {
            Some(Ok(plan)) => preview.apply(Ok(plan), None, !self.from_click.get()),
            Some(Err(e)) => preview.apply(Err(e.clone()), None, false),
            None => preview.apply(Err(String::new()), idle, false),
        }
        let invalid = matches!(plan, Some(Err(_)));
        if let Some(entry) = self.options.spec_entry() {
            if invalid {
                entry.add_css_class("error");
            } else {
                entry.remove_css_class("error");
            }
        }
        // Sem páginas digitadas, a ferramenta ainda não tem o que fazer.
        let waiting = plan.is_none() && idle.is_some() && preview.total().is_some();
        self.run_button.set_sensitive(self.deps_ok && !invalid && !waiting && form_ok);
        let plan = plan.as_ref().and_then(|p| p.as_ref().ok());
        self.refresh_stamp(plan, form_ok);
        self.update_run_label(plan);
    }

    /// Descrição do grupo de opções que depende dos arquivos ou das opções.
    fn update_description(&self) {
        let files = self.files.borrow();
        let info = self.file_info.borrow();
        match &self.options {
            Options::Images { .. } => {
                if let (Some(layout), Some(image_layout)) = (&self.layout, self.options.image_layout()) {
                    let images: Vec<(PathBuf, Option<(f32, f32)>)> =
                        files.iter().map(|f| (f.clone(), info.get(f).and_then(|i| i.dims))).collect();
                    layout.update(&images, image_layout);
                }
            }
            Options::ToImages { dpi, .. } => {
                let base = "Cada página vira um arquivo de imagem.";
                let first = files.first().and_then(|f| Some((f, info.get(f)?.dims?)));
                let text = match first {
                    Some((f, (w, h))) => {
                        let d = dpi.value();
                        let (pw, ph) = ((f64::from(w) / 72.0 * d).round(), (f64::from(h) / 72.0 * d).round());
                        format!(
                            "{base} Em {} ppp, a página 1 de “{}” terá {} × {} px (≈ {} megapixels).",
                            d as u32,
                            human::file_name(f),
                            pw,
                            ph,
                            human::decimal(pw * ph / 1e6, 1)
                        )
                    }
                    None => base.to_string(),
                };
                self.options_group.set_description(Some(&glib::markup_escape_text(&text)));
            }
            Options::Ocr { skip, .. } => {
                let text = match self.ocr_counts() {
                    Some((with_text, total)) => {
                        format!(
                            "{} Leva alguns segundos por página.",
                            texts::ocr_summary(with_text, total, skip.is_active())
                        )
                    }
                    None => "As páginas digitalizadas continuam como estão; o texto reconhecido entra numa \
                             camada invisível por cima, que permite buscar e copiar."
                        .into(),
                };
                self.options_group.set_description(Some(&text));
            }
            Options::Metadata(m) => {
                self.options_group.set_description(Some(if m.remove_all.is_active() {
                    "Título, autor, assunto, palavras-chave, programas e datas serão apagados."
                } else {
                    "Campos deixados em branco são removidos do arquivo."
                }));
            }
            Options::None if self.info.id == ToolId::Merge => {
                let pages: Option<u32> = files.iter().map(|f| info.get(f).and_then(|i| i.pages)).sum();
                let mut text =
                    "Os arquivos são unidos na ordem da lista acima. Use as setas para reordenar.".to_string();
                if let (Some(n), true) = (pages, files.len() >= 2) {
                    text.push_str(&format!(" Resultado: um PDF com {n} páginas."));
                }
                self.options_group.set_description(Some(&text));
            }
            _ => {}
        }
    }

    /// Mostra a validação dos campos que não são páginas (senhas, texto…).
    /// Devolve se o formulário pode ser executado.
    fn show_form_check(&self) -> bool {
        let (form_ok, hint, is_error) = match self.options.check(self.ocr_counts()) {
            Ok(warning) => (true, warning, false),
            Err(FormIssue::Incomplete(msg)) => (false, Some(msg).filter(|m| !m.is_empty()), false),
            Err(FormIssue::Invalid(msg)) => (false, Some(msg), true),
        };
        self.form_hint.set_visible(hint.is_some());
        self.form_hint.set_text(hint.as_deref().unwrap_or(""));
        self.form_hint.set_css_classes(if is_error { &["texto-erro"] } else { &["texto-aviso"] });
        form_ok
    }

    /// O botão principal diz o que vai acontecer (fora da execução).
    fn update_run_label(&self, plan: Option<&Plan>) {
        if self.running.borrow().is_none() {
            let label = self.options.action_label(run_label(self.info.id), self.files.borrow().len(), plan);
            self.run_button.set_label(&label);
        }
    }

    /// “Como vai ficar”: primeira página afetada, com o carimbo real.
    fn refresh_stamp(&self, plan: Option<&Plan>, form_ok: bool) {
        let Some(sp) = &self.stamp_preview else { return };
        let path = self.preview.as_ref().and_then(|p| p.current());
        let (Some(plan), Some(path), true) = (plan, path, form_ok) else {
            sp.hide();
            return;
        };
        let chosen = plan.marks.iter().filter(|m| m.kind != MarkKind::Normal).count() as u32;
        match plan.first_marked().and_then(|page| Some((page, self.options.stamp_preview(page, chosen)?))) {
            Some((page, (caption, layer))) => sp.show(&path, page, &caption, layer),
            None => sp.hide(),
        }
    }

    /// Clique numa página da pré-visualização: marca ou desmarca no campo.
    fn toggle_page(&self, page: u32) {
        let Some(total) = self.preview.as_ref().and_then(|p| p.total()) else { return };
        let Some((row, ordered)) = self.options.clickable_spec() else { return };
        let text = preview::toggle_page(&row.text(), page, total, ordered);
        // `set_text` apaga e reinsere o texto (dois avisos de mudança); o
        // estado intermediário vazio não deve ser mostrado nem mover a grade.
        self.from_click.set(true);
        row.set_text(&text);
        row.set_position(-1);
        self.update_name_row();
        self.refresh_preview();
        self.from_click.set(false);
    }

    fn update_destination(&self) {
        if !self.dest_custom.get() {
            let first = self.files.borrow().first().and_then(|f| unlock::original(f).parent().map(Path::to_path_buf));
            self.dest_dir.replace(first);
        }
        self.dest_reset.set_visible(self.dest_custom.get());
        let label = match (&*self.dest_dir.borrow(), self.dest_custom.get()) {
            (Some(d), true) => widgets::path_label(d),
            (Some(d), false) => format!("{} (pasta do original)", widgets::path_label(d)),
            (None, _) => "A mesma do primeiro arquivo".into(),
        };
        self.dest_row.set_subtitle(&label);
    }

    fn single_output(&self) -> bool {
        self.options.single_output(self.files.borrow().len())
    }

    fn update_name_row(&self) {
        self.name_row.set_visible(self.single_output());
    }

    fn request(&self) -> OpResult<Request> {
        let files = self.files.borrow().clone();
        if files.len() < self.info.min_files {
            return Err(if self.info.min_files > 1 {
                OpError::InvalidRange(format!("Adicione pelo menos {} arquivos.", self.info.min_files))
            } else {
                OpError::NoInput
            });
        }
        self.options.request(files)
    }
}

/// Linha de detalhes de um arquivo da lista.
fn file_line(path: &Path, facts: &FileFacts, is_pdf: bool, unlock_tool: bool) -> String {
    if facts.locked && !unlock_tool {
        "🔒 Protegido por senha — adicione de novo e digite a senha".into()
    } else if unlock::is_copy(path) {
        format!("🔓 Aberto com a senha; o resultado sai sem senha · {}", facts.line(is_pdf))
    } else {
        facts.line(is_pdf)
    }
}
