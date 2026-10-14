//! Página genérica de ferramenta: arquivos → opções → destino → executar →
//! progresso → resultado. As opções específicas ficam em `Options`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gio, glib};

use pdftoolbox::catalog::{self, Accepts, ToolId, ToolInfo};
use pdftoolbox::engines::Tool;
use pdftoolbox::error::display_name;
use pdftoolbox::jobs::{self, CancelToken, JobEvent};
use pdftoolbox::operations::{
    self, CompressLevel, Destination, ImageFormat, ImageLayout, Outcome, PageSize, Request, SplitMode, human_size,
};
use pdftoolbox::preview::Plan;
use pdftoolbox::{OpError, OpResult, pages};

use pdftoolbox::preview;

use super::page_preview::{LayoutPreview, PagePreview};
use super::thumbs::{self, FileFacts};
use super::{AppState, primary_menu, widgets};

enum Options {
    None,
    Split { mode: adw::ComboRow, ranges: adw::EntryRow, every: adw::SpinRow },
    Extract { spec: adw::EntryRow, separate: adw::SwitchRow },
    Remove { spec: adw::EntryRow },
    Rotate { angle: adw::ComboRow, spec: adw::EntryRow },
    Images { size: adw::ComboRow, margin: adw::SpinRow, one_file: adw::SwitchRow },
    ToImages { format: adw::ComboRow, dpi: adw::SpinRow, quality: adw::SpinRow },
    Compress { level: adw::ComboRow },
}

pub struct ToolPage {
    app: Weak<AppState>,
    info: &'static ToolInfo,
    files: RefCell<Vec<PathBuf>>,
    file_info: Rc<RefCell<HashMap<PathBuf, FileFacts>>>,
    files_group: adw::PreferencesGroup,
    file_rows: RefCell<Vec<adw::ActionRow>>,
    empty_row: adw::ActionRow,
    options: Options,
    options_group: adw::PreferencesGroup,
    preview: Option<Rc<PagePreview>>,
    layout: Option<LayoutPreview>,
    plan_ok: Cell<bool>,
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
}

fn combo(title: &str, items: &[&str], selected: u32) -> adw::ComboRow {
    let row = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(items)).build();
    row.set_selected(selected);
    row
}

fn spin(title: &str, subtitle: &str, min: f64, max: f64, step: f64, value: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_value(value);
    row
}

fn entry(title: &str) -> adw::EntryRow {
    adw::EntryRow::builder().title(title).build()
}

fn run_label(id: ToolId) -> &'static str {
    match id {
        ToolId::Merge => "Mesclar",
        ToolId::Split => "Dividir",
        ToolId::Extract => "Extrair",
        ToolId::Remove => "Remover páginas",
        ToolId::Rotate => "Girar",
        ToolId::ImagesToPdf | ToolId::PdfToImages => "Converter",
        ToolId::Compress => "Comprimir",
        ToolId::Organize => "Salvar",
    }
}

fn compress_explanation(level: CompressLevel) -> String {
    let base = match level {
        CompressLevel::Lossless => {
            "Reorganiza a estrutura do arquivo sem alterar imagens nem texto. A qualidade é idêntica; \
             a economia costuma ser pequena em PDFs já otimizados."
        }
        CompressLevel::Recommended => {
            "Reduz as imagens para 150 ppp e as recomprime em JPEG. Bom para tela e e-mail; há perda leve \
             de qualidade nas imagens. O texto continua pesquisável."
        }
        CompressLevel::Strong => {
            "Reduz as imagens para 96 ppp. Gera o menor arquivo, com perda visível ao ampliar ou imprimir."
        }
    };
    let mut text = base.to_string();
    if level != CompressLevel::Lossless {
        text.push_str(
            " O documento é reescrito pelo Ghostscript: confira o resultado se ele tiver formulários, links ou anexos.",
        );
        if Tool::Ghostscript.find().is_none() {
            text.push_str(" ⚠ O Ghostscript não está instalado; este nível não está disponível.");
        }
    }
    text
}

fn level_from(i: u32) -> CompressLevel {
    match i {
        0 => CompressLevel::Lossless,
        2 => CompressLevel::Strong,
        _ => CompressLevel::Recommended,
    }
}

/// Constrói as linhas de opções da ferramenta dentro de `group`.
fn build_options(info: &ToolInfo, group: &adw::PreferencesGroup) -> Options {
    match info.id {
        ToolId::Merge => {
            group.set_description(Some("Os arquivos são unidos na ordem da lista acima. Use as setas para reordenar."));
            Options::None
        }
        ToolId::Split => {
            let mode = combo("Modo", &["Por intervalos", "A cada N páginas", "Uma página por arquivo"], 0);
            let ranges = entry("Intervalos — cada um vira um arquivo (ex.: 1-3, 4-8, 9-)");
            let every = spin("Páginas por arquivo", "", 1.0, 10_000.0, 1.0, 2.0);
            every.set_visible(false);
            let (r, e) = (ranges.clone(), every.clone());
            mode.connect_selected_notify(move |m| {
                r.set_visible(m.selected() == 0);
                e.set_visible(m.selected() == 1);
            });
            group.add(&mode);
            group.add(&ranges);
            group.add(&every);
            Options::Split { mode, ranges, every }
        }
        ToolId::Extract => {
            let spec = entry("Páginas a extrair (ex.: 1-3, 7, 10-)");
            let separate = adw::SwitchRow::builder()
                .title("Um arquivo por página")
                .subtitle("Desligado: todas as páginas escolhidas em um único PDF, na ordem digitada")
                .build();
            group.add(&spec);
            group.add(&separate);
            Options::Extract { spec, separate }
        }
        ToolId::Remove => {
            let spec = entry("Páginas a remover (ex.: 2, 5-7)");
            group.add(&spec);
            Options::Remove { spec }
        }
        ToolId::Rotate => {
            let angle = combo(
                "Rotação",
                &["↻ 90° no sentido horário", "180° (de cabeça para baixo)", "↺ 90° no sentido anti-horário"],
                0,
            );
            let spec = entry("Páginas (deixe vazio para todas)");
            group.add(&angle);
            group.add(&spec);
            Options::Rotate { angle, spec }
        }
        ToolId::ImagesToPdf => {
            let size = combo("Tamanho da página", &["A4", "Carta", "Igual à imagem"], 0);
            let margin = spin("Margem", "Em milímetros", 0.0, 50.0, 1.0, 10.0);
            let one_file = adw::SwitchRow::builder()
                .title("Juntar em um único PDF")
                .subtitle("Desligado: um PDF para cada imagem")
                .active(true)
                .build();
            let m = margin.clone();
            size.connect_selected_notify(move |s| m.set_sensitive(s.selected() != 2));
            group.set_description(Some(
                "JPEGs são incorporados sem recompressão. A orientação gravada pela câmera (EXIF) é respeitada.",
            ));
            group.add(&size);
            group.add(&margin);
            group.add(&one_file);
            Options::Images { size, margin, one_file }
        }
        ToolId::PdfToImages => {
            let format = combo("Formato", &["PNG — sem perda", "JPEG — arquivos menores", "TIFF — sem perda (LZW)"], 0);
            let dpi =
                spin("Resolução", "Pontos por polegada: 150 para tela, 300 para impressão", 36.0, 1200.0, 10.0, 150.0);
            let quality = spin("Qualidade do JPEG", "De 10 a 100", 10.0, 100.0, 5.0, 90.0);
            quality.set_visible(false);
            let q = quality.clone();
            format.connect_selected_notify(move |f| q.set_visible(f.selected() == 1));
            group.add(&format);
            group.add(&dpi);
            group.add(&quality);
            Options::ToImages { format, dpi, quality }
        }
        ToolId::Compress => {
            let level = combo("Nível", &["Sem perda", "Recomendada", "Máxima"], 1);
            group.set_description(Some(&compress_explanation(CompressLevel::Recommended)));
            let g = group.clone();
            level.connect_selected_notify(move |l| {
                g.set_description(Some(&compress_explanation(level_from(l.selected()))))
            });
            group.add(&level);
            Options::Compress { level }
        }
        ToolId::Organize => Options::None,
    }
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

    // Dependências
    let missing: Vec<Tool> = info.needs.iter().copied().filter(|t| t.find().is_none()).collect();
    let deps_ok = missing.is_empty();
    if !deps_ok {
        let group = adw::PreferencesGroup::builder()
            .title("Dependência ausente")
            .description("Esta ferramenta não pode ser usada até que o componente abaixo seja instalado. Rode o comando em um terminal e reabra a ferramenta.")
            .build();
        for tool in &missing {
            let row = widgets::plain_row(tool.label(), &tool.install_command());
            row.set_subtitle_selectable(true);
            row.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            group.add(&row);
        }
        let more = gtk::Button::builder().label("Ver todas as dependências").halign(gtk::Align::Start).build();
        more.set_action_name(Some("win.dependencies"));
        more.add_css_class("flat");
        group.set_header_suffix(Some(&more));
        body.append(&group);
    }

    // Arquivos
    let add_button = gtk::Button::builder()
        .label("Adicionar…")
        .valign(gtk::Align::Center)
        .tooltip_text("Adicionar arquivos (Ctrl+O)")
        .build();
    add_button.add_css_class("flat");
    let files_group = adw::PreferencesGroup::builder()
        .title(if info.multiple { "Arquivos" } else { "Arquivo" })
        .header_suffix(&add_button)
        .build();
    let empty_row = widgets::plain_row(
        "Nenhum arquivo",
        match info.accepts {
            Accepts::Pdf => "Arraste PDFs para esta janela ou clique em Adicionar",
            Accepts::Images => "Arraste imagens para esta janela ou clique em Adicionar",
        },
    );
    empty_row.add_css_class("dim-label");
    files_group.add(&empty_row);
    body.append(&files_group);

    // Opções
    let options_group = adw::PreferencesGroup::builder().title("Opções").build();
    let options = build_options(info, &options_group);
    if !matches!(options, Options::None) || options_group.description().is_some() {
        body.append(&options_group);
    }

    // Pré-visualização
    let preview = match info.id {
        ToolId::Rotate | ToolId::Extract | ToolId::Remove => Some(PagePreview::new(true)),
        ToolId::Split => Some(PagePreview::new(false)),
        _ => None,
    };
    let layout = (info.id == ToolId::ImagesToPdf).then(LayoutPreview::new);
    if let Some(p) = &preview {
        body.append(&p.group);
    }
    if let Some(l) = &layout {
        body.append(&l.group);
    }

    // Destino
    let dest_group = adw::PreferencesGroup::builder()
        .title("Destino")
        .description("O original nunca é alterado. Se já existir um arquivo com o mesmo nome, o novo recebe um número, como “nome (2).pdf”.")
        .build();
    let dest_row = widgets::plain_row("Pasta", "A mesma do primeiro arquivo");
    let dest_reset = widgets::icon_button("edit-undo-symbolic", "Voltar a usar a pasta do arquivo original");
    dest_reset.set_visible(false);
    let change = gtk::Button::builder().label("Alterar…").valign(gtk::Align::Center).build();
    dest_row.add_suffix(&dest_reset);
    dest_row.add_suffix(&change);
    dest_row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    let name_row = adw::EntryRow::builder().title("Nome do arquivo (opcional)").build();
    dest_group.add(&dest_row);
    dest_group.add(&name_row);
    body.append(&dest_group);

    // Ação e progresso
    let run_button = gtk::Button::builder().label(run_label(info.id)).halign(gtk::Align::Center).build();
    run_button.add_css_class("pill");
    run_button.add_css_class("suggested-action");
    run_button.set_tooltip_text(Some("Executar (Ctrl+Enter)"));
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
        file_info: Rc::new(RefCell::new(HashMap::new())),
        files_group,
        file_rows: RefCell::new(Vec::new()),
        empty_row,
        options,
        options_group,
        preview,
        layout,
        plan_ok: Cell::new(true),
        from_click: Cell::new(false),
        dest_dir: RefCell::new(None),
        dest_custom: Cell::new(false),
        dest_row,
        dest_reset: dest_reset.clone(),
        name_row,
        run_button: run_button.clone(),
        cancel_button: cancel_button.clone(),
        progress,
        status,
        result_slot,
        scroll: scroll.clone(),
        running: RefCell::new(None),
        deps_ok,
    });
    run_button.set_sensitive(deps_ok);

    // Ações da página (Ctrl+O e Ctrl+Enter)
    let group = gio::SimpleActionGroup::new();
    let add_action = gio::SimpleAction::new("add", None);
    let p = Rc::downgrade(&page);
    add_action.connect_activate(move |_, _| {
        if let Some(p) = p.upgrade() {
            p.pick_files();
        }
    });
    let run_action = gio::SimpleAction::new("run", None);
    let p = Rc::downgrade(&page);
    run_action.connect_activate(move |_, _| {
        if let Some(p) = p.upgrade() {
            p.run();
        }
    });
    group.add_action(&add_action);
    group.add_action(&run_action);
    nav_page.insert_action_group("page", Some(&group));
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Global);
    for (accel, action) in [("<Control>Return", "page.run"), ("<Control>KP_Enter", "page.run")] {
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(accel),
            Some(gtk::NamedAction::new(action)),
        ));
    }
    nav_page.add_controller(shortcuts);

    add_button.set_action_name(Some("page.add"));
    let p = Rc::downgrade(&page);
    run_button.connect_clicked(move |_| {
        if let Some(p) = p.upgrade() {
            p.run();
        }
    });
    let p = Rc::downgrade(&page);
    cancel_button.connect_clicked(move |b| {
        if let Some(p) = p.upgrade()
            && let Some(token) = p.running.borrow().as_ref()
        {
            token.cancel();
            b.set_sensitive(false);
            p.status.set_text("Cancelando…");
        }
    });
    let p = Rc::downgrade(&page);
    change.connect_clicked(move |_| {
        if let Some(p) = p.upgrade() {
            p.choose_folder();
        }
    });
    let p = Rc::downgrade(&page);
    dest_reset.connect_clicked(move |_| {
        if let Some(p) = p.upgrade() {
            p.dest_custom.set(false);
            p.update_destination();
        }
    });
    let p = Rc::downgrade(&page);
    widgets::add_file_drop(&nav_page, move |files| {
        if let Some(p) = p.upgrade() {
            p.add_files(files);
        }
    });
    // Qualquer mudança nas opções atualiza nome, pré-visualização e botão.
    let p = Rc::downgrade(&page);
    let refresh = move || {
        if let Some(p) = p.upgrade()
            && !p.from_click.get()
        {
            p.update_name_row();
            p.refresh_preview();
        }
    };
    match &page.options {
        Options::Split { mode, ranges, every } => {
            let r = refresh.clone();
            mode.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            ranges.connect_changed(move |_| r());
            let r = refresh.clone();
            every.connect_value_notify(move |_| r());
        }
        Options::Extract { spec, separate } => {
            let r = refresh.clone();
            separate.connect_active_notify(move |_| r());
            let r = refresh.clone();
            spec.connect_changed(move |_| r());
        }
        Options::Remove { spec } => {
            let r = refresh.clone();
            spec.connect_changed(move |_| r());
        }
        Options::Rotate { angle, spec } => {
            let r = refresh.clone();
            angle.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            spec.connect_changed(move |_| r());
        }
        Options::Images { size, margin, one_file } => {
            let r = refresh.clone();
            one_file.connect_active_notify(move |_| r());
            let r = refresh.clone();
            size.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            margin.connect_value_notify(move |_| r());
        }
        Options::ToImages { dpi, .. } => {
            let r = refresh.clone();
            dpi.connect_value_notify(move |_| r());
        }
        Options::Compress { .. } | Options::None => {}
    }
    if let Some(preview) = &page.preview {
        let r = refresh.clone();
        preview.connect_loaded(r);
        let p = Rc::downgrade(&page);
        preview.connect_page_clicked(move |n| {
            if let Some(p) = p.upgrade() {
                p.toggle_page(n);
            }
        });
    }

    page.add_files(files);
    page.update_name_row();
    // Ferramentas que pedem páginas já abrem com o cursor no campo.
    let first_entry = match &page.options {
        Options::Split { ranges, .. } => Some(ranges.clone()),
        Options::Extract { spec, .. } | Options::Remove { spec } | Options::Rotate { spec, .. } => Some(spec.clone()),
        _ => None,
    };
    if let Some(entry) = first_entry {
        nav_page.connect_shown(move |_| {
            entry.grab_focus();
        });
    }
    // Enter no campo executa a ferramenta.
    for row in page.entries() {
        row.set_show_apply_button(false);
        let p = Rc::downgrade(&page);
        row.connect_entry_activated(move |_| {
            if let Some(p) = p.upgrade() {
                p.run();
            }
        });
    }
    // O estado vive enquanto o widget da página existir.
    let keep = page.clone();
    nav_page.connect_destroy(move |_| {
        let _ = &keep;
    });
    nav_page
}

impl ToolPage {
    fn entries(&self) -> Vec<adw::EntryRow> {
        match &self.options {
            Options::Split { ranges, .. } => vec![ranges.clone()],
            Options::Extract { spec, .. } | Options::Remove { spec } | Options::Rotate { spec, .. } => {
                vec![spec.clone()]
            }
            _ => vec![],
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
        let p = self.clone();
        glib::spawn_future_local(async move {
            let files = widgets::pick_files(&win, Some(p.info.accepts), p.info.multiple).await;
            p.add_files(files);
        });
    }

    fn choose_folder(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let p = self.clone();
        glib::spawn_future_local(async move {
            let initial = p.dest_dir.borrow().clone();
            if let Some(dir) = widgets::pick_folder(&win, initial.as_deref()).await {
                *p.dest_dir.borrow_mut() = Some(dir.clone());
                p.dest_custom.set(true);
                if let Some(app) = p.app.upgrade() {
                    let mut s = app.settings.borrow_mut();
                    s.last_dir = Some(dir);
                    s.save();
                }
                p.update_destination();
            }
        });
    }

    fn allows_duplicates(&self) -> bool {
        matches!(self.info.id, ToolId::Merge | ToolId::ImagesToPdf)
    }

    pub fn add_files(self: &Rc<Self>, incoming: Vec<PathBuf>) {
        if incoming.is_empty() {
            return;
        }
        let (ok, rejected): (Vec<PathBuf>, Vec<PathBuf>) =
            incoming.into_iter().partition(|f| catalog::extension_ok(self.info.accepts, f));
        if !rejected.is_empty() {
            let what = match self.info.accepts {
                Accepts::Pdf => "somente PDFs",
                Accepts::Images => "somente imagens JPEG, PNG, WebP, GIF, BMP ou TIFF",
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
                    if !self.allows_duplicates() && files.contains(&f) {
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

    fn rebuild_files(self: &Rc<Self>) {
        for row in self.file_rows.borrow_mut().drain(..) {
            self.files_group.remove(&row);
        }
        let files = self.files.borrow().clone();
        self.empty_row.set_visible(files.is_empty());
        let ordered = self.info.multiple && matches!(self.info.id, ToolId::Merge | ToolId::ImagesToPdf);
        for (i, path) in files.iter().enumerate() {
            let is_pdf = self.info.accepts == Accepts::Pdf;
            let subtitle = self.file_info.borrow().get(path).map(|f| f.line(is_pdf)).unwrap_or_else(|| "Lendo…".into());
            let row = widgets::plain_row(&display_name(path), &subtitle);
            row.set_tooltip_text(Some(&path.to_string_lossy()));
            if ordered && files.len() > 1 {
                let n = gtk::Label::new(Some(&(i + 1).to_string()));
                n.add_css_class("badge");
                n.add_css_class("acento");
                n.set_valign(gtk::Align::Center);
                n.set_tooltip_text(Some("Posição no resultado"));
                row.add_prefix(&n);
            }
            let thumb = thumbs::picture(path, 40, 48);
            thumb.set_margin_top(6);
            thumb.set_margin_bottom(6);
            row.add_prefix(&thumb);
            if ordered && files.len() > 1 {
                let up = widgets::icon_button("go-up-symbolic", "Mover para cima");
                let down = widgets::icon_button("go-down-symbolic", "Mover para baixo");
                up.set_sensitive(i > 0);
                down.set_sensitive(i + 1 < files.len());
                for (b, delta) in [(&up, -1isize), (&down, 1)] {
                    let p = Rc::downgrade(self);
                    b.connect_clicked(move |_| {
                        if let Some(p) = p.upgrade() {
                            let j = i as isize + delta;
                            p.files.borrow_mut().swap(i, j as usize);
                            p.rebuild_files();
                        }
                    });
                }
                row.add_suffix(&up);
                row.add_suffix(&down);
            }
            let remove = widgets::icon_button("list-remove-symbolic", "Remover da lista");
            let p = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.files.borrow_mut().remove(i);
                    p.rebuild_files();
                }
            });
            row.add_suffix(&remove);
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
        self.refresh_preview();
    }

    /// Lê páginas/dimensões em segundo plano e atualiza a linha.
    fn load_info(self: &Rc<Self>, path: PathBuf, row: adw::ActionRow) {
        let is_pdf = self.info.accepts == Accepts::Pdf;
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let facts = thumbs::facts(&path).await;
            row.set_subtitle(&facts.line(is_pdf));
            if facts.problem.is_some() {
                row.add_css_class("texto-erro");
            }
            if let Some(p) = p.upgrade() {
                p.file_info.borrow_mut().insert(path, facts);
                p.refresh_preview();
            }
        });
    }

    /// Plano da ferramenta para o arquivo mostrado na pré-visualização.
    /// `None` quando ainda não há o que mostrar (sem arquivo ou sem páginas digitadas).
    fn plan(&self) -> Option<Result<Plan, String>> {
        let total = self.preview.as_ref()?.total()?;
        let spec_of = |row: &adw::EntryRow| row.text().trim().to_string();
        let result = match &self.options {
            Options::Rotate { angle, spec } => preview::rotate(&spec_of(spec), angle_of(angle.selected()), total),
            Options::Extract { spec, separate } => {
                let t = spec_of(spec);
                if t.is_empty() {
                    return None;
                }
                preview::extract(&t, separate.is_active(), total)
            }
            Options::Remove { spec } => {
                let t = spec_of(spec);
                if t.is_empty() {
                    return None;
                }
                preview::remove(&t, total)
            }
            Options::Split { mode, ranges, every } => {
                let mode = match mode.selected() {
                    0 => {
                        let t = spec_of(ranges);
                        if t.is_empty() {
                            return None;
                        }
                        SplitMode::Ranges(t)
                    }
                    1 => SplitMode::Every(every.value() as u32),
                    _ => SplitMode::EachPage,
                };
                preview::split(&mode, total)
            }
            _ => return None,
        };
        Some(result.map_err(|e| e.to_string()))
    }

    /// Atualiza pré-visualizações, resumos e o rótulo do botão principal.
    fn refresh_preview(&self) {
        let files = self.files.borrow().clone();
        match &self.options {
            Options::Images { size, margin, .. } => {
                if let Some(layout) = &self.layout {
                    let info = self.file_info.borrow();
                    let images: Vec<(PathBuf, Option<(f32, f32)>)> =
                        files.iter().map(|f| (f.clone(), info.get(f).and_then(|i| i.dims))).collect();
                    layout.update(&images, image_layout(size.selected(), margin.value()));
                }
            }
            Options::ToImages { dpi, .. } => {
                let info = self.file_info.borrow();
                let base = "Cada página vira um arquivo de imagem.";
                let text = match files.first().and_then(|f| info.get(f).map(|i| (f, i))) {
                    Some((f, facts)) if facts.dims.is_some() => {
                        let (w, h) = facts.dims.unwrap();
                        let d = dpi.value();
                        let (pw, ph) = ((w as f64 / 72.0 * d).round(), (h as f64 / 72.0 * d).round());
                        format!(
                            "{base} Em {} ppp, a página 1 de “{}” terá {} × {} px (≈ {} megapixels).",
                            d as u32,
                            display_name(f),
                            pw,
                            ph,
                            format!("{:.1}", pw * ph / 1e6).replace('.', ",")
                        )
                    }
                    _ => base.to_string(),
                };
                self.options_group.set_description(Some(&glib::markup_escape_text(&text)));
            }
            Options::None if self.info.id == ToolId::Merge => {
                let info = self.file_info.borrow();
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

        // Grade de páginas, validação ao vivo e botão principal.
        let Some(preview) = &self.preview else { return };
        let plan = self.plan();
        let idle = match &self.options {
            Options::Extract { .. } => Some("Digite as páginas a extrair ou clique nelas abaixo."),
            Options::Remove { .. } => Some("Digite as páginas a remover ou clique nelas abaixo."),
            Options::Split { .. } => Some("Digite os intervalos; cada um vira um arquivo, com uma cor."),
            _ => None,
        };
        let entry = self.entries().into_iter().next();
        match &plan {
            Some(Ok(plan)) => preview.apply(Ok(plan), None, !self.from_click.get()),
            Some(Err(e)) => preview.apply(Err(e.clone()), None, false),
            None => preview.apply(Err(String::new()), idle, false),
        }
        let invalid = matches!(plan, Some(Err(_)));
        if let Some(entry) = entry {
            if invalid {
                entry.add_css_class("error");
            } else {
                entry.remove_css_class("error");
            }
        }
        self.plan_ok.set(!invalid);
        let waiting = plan.is_none() && idle.is_some() && preview.total().is_some();
        self.run_button.set_sensitive(self.deps_ok && !invalid && !waiting);
        if self.running.borrow().is_none() {
            self.run_button.set_label(&self.action_label(plan.as_ref().and_then(|p| p.as_ref().ok())));
        }
    }

    /// Rótulo do botão que diz exatamente o que vai acontecer.
    fn action_label(&self, plan: Option<&Plan>) -> String {
        let base = run_label(self.info.id).to_string();
        let Some(plan) = plan else { return base };
        let count = |f: &dyn Fn(&preview::MarkKind) -> bool| plan.marks.iter().filter(|m| f(&m.kind)).count();
        let pages = |n: usize| if n == 1 { "1 página".to_string() } else { format!("{n} páginas") };
        match &self.options {
            Options::Rotate { .. } if self.files.borrow().len() > 1 => {
                format!("Girar {} arquivos", self.files.borrow().len())
            }
            Options::Rotate { .. } => {
                let n = count(&|k| *k == preview::MarkKind::Selected);
                if n == plan.marks.len() { "Girar todas as páginas".into() } else { format!("Girar {}", pages(n)) }
            }
            Options::Extract { spec, .. } => {
                let n = pages::parse_list(&spec.text(), plan.marks.len() as u32).map(|l| l.len()).unwrap_or(0);
                format!("Extrair {}", pages(n))
            }
            Options::Remove { .. } => format!("Remover {}", pages(count(&|k| *k == preview::MarkKind::Removed))),
            Options::Split { .. } => {
                let n = plan
                    .marks
                    .iter()
                    .filter_map(|m| match m.kind {
                        preview::MarkKind::Group(g) => Some(g + 1),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                // Intervalos sobrepostos: o maior índice de grupo é a contagem de arquivos.
                if n == 1 { "Dividir em 1 arquivo".into() } else { format!("Dividir em {n} arquivos") }
            }
            _ => base,
        }
    }

    /// Clique numa página da pré-visualização: marca ou desmarca no campo.
    fn toggle_page(&self, page: u32) {
        let Some(total) = self.preview.as_ref().and_then(|p| p.total()) else { return };
        let (row, ordered) = match &self.options {
            Options::Rotate { spec, .. } | Options::Remove { spec } => (spec, false),
            Options::Extract { spec, .. } => (spec, true),
            _ => return,
        };
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
            let first = self.files.borrow().first().and_then(|f| f.parent().map(Path::to_path_buf));
            *self.dest_dir.borrow_mut() = first;
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
        let n = self.files.borrow().len();
        match &self.options {
            Options::Extract { separate, .. } => !separate.is_active(),
            Options::Images { one_file, .. } => one_file.is_active() || n <= 1,
            Options::Split { .. } | Options::ToImages { .. } => false,
            Options::Rotate { .. } | Options::Compress { .. } => n <= 1,
            Options::Remove { .. } | Options::None => true,
        }
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
        let first = files[0].clone();
        let need_text = |row: &adw::EntryRow, what: &str| -> OpResult<String> {
            let t = row.text().trim().to_string();
            if t.is_empty() { Err(OpError::InvalidRange(format!("Informe {what}."))) } else { Ok(t) }
        };
        Ok(match &self.options {
            Options::None => Request::Merge { inputs: files },
            Options::Split { mode, ranges, every } => Request::Split {
                input: first,
                mode: match mode.selected() {
                    0 => SplitMode::Ranges(need_text(ranges, "os intervalos de páginas")?),
                    1 => SplitMode::Every(every.value() as u32),
                    _ => SplitMode::EachPage,
                },
            },
            Options::Extract { spec, separate } => Request::Extract {
                input: first,
                spec: need_text(spec, "as páginas a extrair")?,
                separate: separate.is_active(),
            },
            Options::Remove { spec } => {
                Request::Remove { input: first, spec: need_text(spec, "as páginas a remover")? }
            }
            Options::Rotate { angle, spec } => Request::Rotate {
                inputs: files,
                spec: Some(spec.text().to_string()).filter(|s| !s.trim().is_empty()),
                angle: angle_of(angle.selected()),
            },
            Options::Images { size, margin, one_file } => Request::ImagesToPdf {
                inputs: files,
                layout: image_layout(size.selected(), margin.value()),
                one_file: one_file.is_active(),
            },
            Options::ToImages { format, dpi, quality } => Request::PdfToImages {
                inputs: files,
                format: match format.selected() {
                    0 => ImageFormat::Png,
                    1 => ImageFormat::Jpeg,
                    _ => ImageFormat::Tiff,
                },
                dpi: dpi.value() as u32,
                quality: quality.value() as u8,
            },
            Options::Compress { level } => Request::Compress { inputs: files, level: level_from(level.selected()) },
        })
    }

    fn set_running(&self, running: bool) {
        self.run_button.set_visible(!running);
        self.cancel_button.set_visible(running);
        self.cancel_button.set_sensitive(true);
        self.progress.set_visible(running);
        self.status.set_visible(running);
        self.files_group.set_sensitive(!running);
    }

    fn run(self: &Rc<Self>) {
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

        let n = self.files.borrow().len();
        let title = if n == 1 {
            format!("{} — {}", self.info.title, display_name(&self.files.borrow()[0]))
        } else {
            format!("{} — {n} arquivos", self.info.title)
        };
        let handle = jobs::spawn(move |ctx| operations::execute(request, &dest, ctx));
        *self.running.borrow_mut() = Some(handle.cancel.clone());
        let task = app.tasks.add(&title, handle.cancel.clone());

        let p = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = handle.events.recv().await {
                match event {
                    JobEvent::Queued => p.status.set_text("Na fila…"),
                    JobEvent::Progress(f, text) => {
                        p.progress.set_fraction(f);
                        p.status.set_text(&text);
                        task.progress(f, &text);
                    }
                    JobEvent::Finished(result) => {
                        *p.running.borrow_mut() = None;
                        p.set_running(false);
                        match &result {
                            Ok(outcome) => task.finish(&summary(outcome), folder_of(outcome)),
                            Err(e) => task.finish(e.title(), None),
                        }
                        p.show_result(result);
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
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            if let Some(child) = scroll.child()
                && let Some(bounds) = slot.compute_bounds(&child)
            {
                let adj = scroll.vadjustment();
                let bottom = bounds.y() as f64 + bounds.height() as f64;
                if bottom > adj.value() + adj.page_size() {
                    adj.set_value((bottom - adj.page_size() + 24.0).min(bounds.y() as f64 - 12.0).max(0.0));
                }
            }
        });
    }

    fn show_error(&self, e: &OpError) {
        clear(&self.result_slot);
        self.result_slot.append(&error_group(e));
        self.reveal_result();
        let retry = matches!(e, OpError::ToolFailed { .. } | OpError::Io { .. } | OpError::OutputInvalid(_));
        if retry {
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
        } else if outcome.failures.is_empty() {
            if outcome.outputs.is_empty() { "Nada a fazer" } else { "Concluído" }
        } else {
            "Concluído com falhas"
        };
        let group = adw::PreferencesGroup::builder().title(title).build();
        if !outcome.notes.is_empty() {
            group.set_description(Some(&glib::markup_escape_text(&outcome.notes.join("\n"))));
        }
        let new_button = gtk::Button::builder().label("Começar de novo").valign(gtk::Align::Center).build();
        new_button.add_css_class("flat");
        let p = Rc::downgrade(self);
        new_button.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.files.borrow_mut().clear();
                p.rebuild_files();
                clear(&p.result_slot);
            }
        });
        group.set_header_suffix(Some(&new_button));

        if let Some(folder) = &outcome.folder {
            let row =
                widgets::plain_row(&display_name(folder), &format!("Pasta com {} arquivo(s)", outcome.outputs.len()));
            row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
            let open = widgets::icon_button("folder-open-symbolic", "Abrir pasta");
            let (w, f) = (win.clone(), folder.clone());
            open.connect_clicked(move |_| widgets::show_in_folder(&w, &f));
            row.add_suffix(&open);
            group.add(&row);
        }
        const MAX_ROWS: usize = 12;
        for out in outcome.outputs.iter().take(if outcome.folder.is_some() { 3 } else { MAX_ROWS }) {
            let row = widgets::plain_row(&display_name(&out.path), &human_size(out.size));
            row.set_tooltip_text(Some(&out.path.to_string_lossy()));
            let open = widgets::icon_button("document-open-symbolic", "Abrir");
            let (w, path) = (win.clone(), out.path.clone());
            open.connect_clicked(move |_| widgets::open_file(&w, &path));
            let show = widgets::icon_button("folder-open-symbolic", "Mostrar na pasta");
            let (w, path) = (win.clone(), out.path.clone());
            show.connect_clicked(move |_| widgets::show_in_folder(&w, &path));
            row.add_suffix(&open);
            row.add_suffix(&show);
            row.set_activatable_widget(Some(&open));
            group.add(&row);
        }
        let shown = if outcome.folder.is_some() { 3 } else { MAX_ROWS };
        if outcome.outputs.len() > shown {
            group.add(&widgets::plain_row(&format!("e mais {} arquivo(s)", outcome.outputs.len() - shown), ""));
        }
        for (file, err) in &outcome.failures {
            group.add(&failure_row(&display_name(file), err));
        }
        self.result_slot.append(&group);
        self.reveal_result();
        match outcome.outputs.as_slice() {
            [] => {}
            [one] => self.toast(&format!("Salvo: {}", display_name(&one.path))),
            many => self.toast(&format!("{} arquivos salvos", many.len())),
        }
    }
}

fn angle_of(selected: u32) -> i32 {
    match selected {
        0 => 90,
        1 => 180,
        _ => 270,
    }
}

fn image_layout(size: u32, margin: f64) -> ImageLayout {
    ImageLayout {
        size: match size {
            0 => PageSize::A4,
            1 => PageSize::Letter,
            _ => PageSize::FitImage,
        },
        margin_mm: margin as f32,
    }
}

fn summary(outcome: &Outcome) -> String {
    let mut s = match outcome.outputs.len() {
        0 => "Nenhum arquivo criado".to_string(),
        1 => format!("Concluído: {}", display_name(&outcome.outputs[0].path)),
        n => format!("Concluído: {n} arquivos"),
    };
    if !outcome.failures.is_empty() {
        s.push_str(&format!(" · {} falha(s)", outcome.failures.len()));
    }
    if outcome.cancelled {
        s.push_str(" · cancelado");
    }
    s
}

fn folder_of(outcome: &Outcome) -> Option<PathBuf> {
    outcome.folder.clone().or_else(|| outcome.outputs.first().map(|o| o.path.clone()))
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
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
    let technical = matches!(
        e,
        OpError::ToolFailed { .. } | OpError::Io { .. } | OpError::InvalidPdf { .. } | OpError::OutputInvalid(_)
    );
    if technical {
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
