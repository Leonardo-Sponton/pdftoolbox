//! Anotar e assinar: a página aparece grande; textos, assinaturas (imagens),
//! destaques e molduras são posicionados arrastando com o mouse ou pelo
//! teclado. A prévia é desenhada na hora com as mesmas medidas da operação
//! (fonte Helvetica e caixas de `pdftoolbox::annotate`); o resultado é gravado
//! pela camada sobreposta do qpdf (`Request::Annotate`), sem rasterizar.
//!
//! A página começa ajustada à área de desenho e pode ser aproximada
//! (Ctrl+roda, pinça, Ctrl+mais/menos, Ctrl+0 volta a ajustar); aproximada,
//! arrastar fora das anotações move a vista.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{cairo, gio, glib};

use pdftoolbox::annotate::{self, ASCENT, LEADING};
use pdftoolbox::catalog::{self, Accepts, ToolId};
use pdftoolbox::engines::{poppler, qpdf};
use pdftoolbox::error::display_name;
use pdftoolbox::jobs::{self, CancelToken, JobEvent};
use pdftoolbox::operations::{self, Annotation, AnnotationKind as Kind, Destination, Request};
use pdftoolbox::storage::JobDir;
use pdftoolbox::{OpError, OpResult, pdfwriter};

use super::{AppState, primary_menu, widgets};

const MM: f32 = 72.0 / 25.4;
/// Lado maior da página renderizada para a tela, em pixels.
const RENDER_SIDE: u32 = 1400;
/// Maior lado renderizado ao aproximar.
const MAX_RENDER_SIDE: u32 = 5000;
/// Passo dos botões e teclas de zoom.
const ZOOM_STEP: f64 = 1.25;
/// Lado maior da prévia de uma imagem inserida.
const IMAGE_SIDE: u32 = 900;
/// Margem em volta da página, em pixels.
const MARGIN: f64 = 24.0;
/// Tamanho da alça de redimensionar, em pixels.
const HANDLE: f64 = 10.0;

const COLORS: [(&str, (f32, f32, f32)); 7] = [
    ("Preto", (0.0, 0.0, 0.0)),
    ("Azul", (0.05, 0.2, 0.65)),
    ("Vermelho", (0.8, 0.1, 0.1)),
    ("Verde", (0.1, 0.5, 0.2)),
    ("Amarelo", (1.0, 0.88, 0.2)),
    ("Rosa", (1.0, 0.6, 0.8)),
    ("Azul-claro", (0.55, 0.85, 1.0)),
];
const BLACK: usize = 0;
const RED: usize = 2;
const YELLOW: usize = 4;

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Move,
    Resize,
}

#[derive(Clone)]
struct Drag {
    index: usize,
    mode: DragMode,
    start: Annotation,
    moved: bool,
}

/// Campos do painel da anotação selecionada.
struct Props {
    group: adw::PreferencesGroup,
    text: gtk::TextView,
    text_box: gtk::Frame,
    warning: gtk::Label,
    font_size: adw::SpinRow,
    bold: adw::SwitchRow,
    color: adw::ComboRow,
    width: adw::SpinRow,
    height: adw::SpinRow,
    line: adw::SpinRow,
    remove_white: adw::SwitchRow,
    all_pages: adw::SwitchRow,
}

struct Annotator {
    app: Weak<AppState>,
    nav_page: glib::WeakRef<adw::NavigationPage>,
    input: RefCell<Option<PathBuf>>,
    sizes: RefCell<Vec<(f32, f32)>>,
    page: Cell<u32>,
    items: RefCell<Vec<Annotation>>,
    selected: Cell<Option<usize>>,
    undo: RefCell<Vec<Vec<Annotation>>>,
    pages: RefCell<HashMap<u32, cairo::ImageSurface>>,
    images: RefCell<HashMap<(PathBuf, bool), Option<cairo::ImageSurface>>>,
    render_gen: Cell<u64>,
    drag: RefCell<Option<Drag>>,
    /// Preenchendo o painel por código: ignora os sinais de alteração.
    filling: Cell<bool>,
    canvas: gtk::DrawingArea,
    /// Rolagem em volta da página (quando aproximada).
    view: gtk::ScrolledWindow,
    /// Pixels por ponto escolhidos pelo zoom; `None` = ajustada à área.
    zoom: Cell<Option<f64>>,
    zoom_label: gtk::Button,
    zoom_out: gtk::Button,
    zoom_in: gtk::Button,
    /// Ponto a manter sob o ponteiro depois do zoom (fração do conteúdo e
    /// posição na vista, por eixo).
    anchor: Cell<Option<super::page_zoom::ZoomAnchor>>,
    /// Arrastando a vista: posição das barras de rolagem no início.
    pan: Cell<Option<(f64, f64)>>,
    stack: gtk::Stack,
    error_page: adw::StatusPage,
    save: adw::SplitButton,
    page_label: gtk::Label,
    prev: gtk::Button,
    next: gtk::Button,
    undo_button: gtk::Button,
    action_bar: gtk::ActionBar,
    props: Props,
    list: gtk::ListBox,
    list_group: adw::PreferencesGroup,
    dest_dir: RefCell<Option<PathBuf>>,
    saving: Cell<bool>,
    /// Anotações como estavam no último salvamento (para avisar ao sair).
    saved: RefCell<Vec<Annotation>>,
}

fn add_button(icon: &str, label: &str, tooltip: &str) -> gtk::Button {
    let content = adw::ButtonContent::builder().icon_name(icon).label(label).build();
    gtk::Button::builder().child(&content).tooltip_text(tooltip).build()
}

fn spin(title: &str, min: f64, max: f64, step: f64, digits: u32) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(title);
    row.set_digits(digits);
    row
}

fn color_index(c: (f32, f32, f32)) -> u32 {
    COLORS.iter().position(|(_, rgb)| *rgb == c).unwrap_or(BLACK) as u32
}

/// Converte RGBA (não pré-multiplicado) para uma superfície ARGB32 do cairo.
fn surface_from_rgba(img: image::RgbaImage) -> Option<cairo::ImageSurface> {
    let (w, h) = img.dimensions();
    let stride = cairo::Format::ARgb32.stride_for_width(w).ok()? as usize;
    let mut data = vec![0u8; stride * h as usize];
    for (y, row) in img.rows().enumerate() {
        for (x, p) in row.enumerate() {
            let [r, g, b, a] = p.0;
            let pm = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
            let px = u32::from(a) << 24 | u32::from(pm(r)) << 16 | u32::from(pm(g)) << 8 | u32::from(pm(b));
            let i = y * stride + x * 4;
            data[i..i + 4].copy_from_slice(&px.to_ne_bytes());
        }
    }
    cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, w as i32, h as i32, stride as i32).ok()
}

fn describe(item: &Annotation) -> String {
    let what = match &item.kind {
        Kind::Text { text, .. } => {
            let first = text.lines().next().unwrap_or("").trim();
            let short: String = first.chars().take(28).collect();
            let more = if first.chars().count() > 28 || text.trim().lines().count() > 1 { "…" } else { "" };
            format!("Texto “{short}{more}”")
        }
        Kind::Image { path, .. } => format!("Imagem {}", display_name(path)),
        Kind::Highlight { .. } => "Destaque".into(),
        Kind::Frame { .. } => "Moldura".into(),
    };
    let wheres = if item.all_pages { "todas as páginas".to_string() } else { format!("pág. {}", item.page) };
    format!("{what} · {wheres}")
}

pub fn build(app: &Rc<AppState>, files: Vec<PathBuf>) -> adw::NavigationPage {
    let info = catalog::info(ToolId::Annotate);
    let header = adw::HeaderBar::new();
    // Com anotações não salvas, a página não sai sozinha (can-pop falso):
    // este botão substitui o “voltar” e pede confirmação.
    let back = widgets::icon_button("go-previous-symbolic", "Voltar");
    back.set_action_name(Some("page.leave"));
    back.set_visible(false);
    header.pack_start(&back);
    let open = gtk::Button::builder().label("Abrir outro…").tooltip_text("Escolher outro PDF (Ctrl+O)").build();
    header.pack_start(&open);
    header.pack_end(&primary_menu());
    let save_menu = gio::Menu::new();
    save_menu.append(Some("Salvar em outra pasta…"), Some("page.save-as"));
    let save = adw::SplitButton::builder()
        .label("Salvar")
        .menu_model(&save_menu)
        .tooltip_text("Salvar como novo arquivo na pasta do original (Ctrl+S)")
        .sensitive(false)
        .build();
    save.add_css_class("suggested-action");
    header.pack_end(&save);

    // Página (área de desenho)
    let canvas = gtk::DrawingArea::builder().hexpand(true).vexpand(true).focusable(true).can_focus(true).build();
    canvas.update_property(&[gtk::accessible::Property::Label(
        "Página do documento. Setas movem a anotação selecionada, Shift+setas movem mais, mais e menos mudam o \
         tamanho, Delete remove, Tab passa para a próxima anotação, Page Up e Page Down trocam de página.",
    )]);

    // Painel lateral
    let add_text = add_button("pdftoolbox-anotar-symbolic", "Texto", "Escrever um texto na página");
    let add_image = add_button(
        "pdftoolbox-imagens-para-pdf-symbolic",
        "Assinatura ou imagem…",
        "Inserir uma imagem, como a foto ou digitalização da sua assinatura",
    );
    let add_date = add_button("x-office-calendar-symbolic", "Data de hoje", "Escrever a data de hoje");
    let add_highlight = add_button("pdftoolbox-marca-texto-symbolic", "Destaque", "Marca-texto sobre um trecho");
    let add_frame = add_button("pdftoolbox-moldura-symbolic", "Moldura", "Retângulo com contorno");
    // Grade simples: num GtkFlowBox o foco do teclado para na célula, e
    // Enter não chega ao botão.
    let add_grid = gtk::Grid::builder().column_spacing(6).row_spacing(6).column_homogeneous(true).build();
    for (i, b) in [&add_text, &add_date, &add_highlight, &add_frame].into_iter().enumerate() {
        add_grid.attach(b, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    add_grid.attach(&add_image, 0, 2, 2, 1);
    let add_group = adw::PreferencesGroup::builder()
        .title("Adicionar")
        .description("Entra no centro da página atual; depois arraste para posicionar.")
        .build();
    add_group.add(&add_grid);

    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .accepts_tab(false)
        .height_request(72)
        .build();
    text.update_property(&[gtk::accessible::Property::Label("Texto da anotação")]);
    let text_box = gtk::Frame::builder().child(&text).margin_bottom(12).build();
    let warning = gtk::Label::builder().wrap(true).xalign(0.0).visible(false).margin_bottom(12).build();
    warning.add_css_class("error");
    warning.add_css_class("caption");
    let props_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    props_box.append(&text_box);
    props_box.append(&warning);
    let list_rows = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list_rows.add_css_class("boxed-list");
    props_box.append(&list_rows);
    let font_size = spin("Tamanho da letra", 4.0, 144.0, 1.0, 0);
    font_size.set_subtitle("Em pontos");
    let bold = adw::SwitchRow::builder().title("Negrito").build();
    let color =
        adw::ComboRow::builder().title("Cor").model(&gtk::StringList::new(&COLORS.map(|(name, _)| name))).build();
    let width = spin("Largura", 2.0, 600.0, 1.0, 0);
    width.set_subtitle("Em milímetros");
    let height = spin("Altura", 2.0, 600.0, 1.0, 0);
    height.set_subtitle("Em milímetros");
    let line = spin("Espessura do contorno", 0.5, 12.0, 0.5, 1);
    line.set_subtitle("Em pontos");
    let remove_white = adw::SwitchRow::builder()
        .title("Remover fundo branco")
        .subtitle("Deixa transparente o papel em volta da assinatura")
        .build();
    let all_pages = adw::SwitchRow::builder()
        .title("Repetir em todas as páginas")
        .subtitle("Útil para rubricas e carimbos")
        .build();
    let delete = gtk::Button::builder()
        .child(&adw::ButtonContent::builder().icon_name("user-trash-symbolic").label("Remover anotação").build())
        .tooltip_text("Remover a anotação selecionada (Delete)")
        .halign(gtk::Align::Start)
        .margin_top(12)
        .build();
    delete.add_css_class("destructive-action");
    for row in [
        font_size.upcast_ref::<gtk::Widget>(),
        bold.upcast_ref(),
        color.upcast_ref(),
        width.upcast_ref(),
        height.upcast_ref(),
        line.upcast_ref(),
        remove_white.upcast_ref(),
        all_pages.upcast_ref(),
    ] {
        list_rows.append(row);
    }
    props_box.append(&delete);
    let props_group = adw::PreferencesGroup::builder().title("Anotação selecionada").visible(false).build();
    props_group.add(&props_box);
    let hint = gtk::Label::builder()
        .label(
            "Clique numa anotação para selecioná-la. Arraste para mover; arraste o canto inferior direito para \
             mudar o tamanho.",
        )
        .wrap(true)
        .xalign(0.0)
        .build();
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list.add_css_class("boxed-list");
    let list_group = adw::PreferencesGroup::builder().title("Anotações").visible(false).build();
    list_group.add(&list);
    let note = gtk::Label::builder()
        .label(
            "A assinatura entra como imagem: é uma assinatura visual, sem a validade jurídica de uma assinatura \
             digital com certificado.",
        )
        .wrap(true)
        .xalign(0.0)
        .build();
    note.add_css_class("dim-label");
    note.add_css_class("caption");
    let side = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    side.append(&add_group);
    side.append(&hint);
    side.append(&props_group);
    side.append(&list_group);
    side.append(&note);
    let side_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&side)
        .width_request(360)
        .hexpand(false)
        .build();
    // Viewport própria: sem `scroll-to-focus`, focar a página (que é maior
    // que a vista com zoom) não rola para o canto dela.
    let viewport = gtk::Viewport::builder().child(&canvas).scroll_to_focus(false).build();
    let view = gtk::ScrolledWindow::builder().child(&viewport).hexpand(true).vexpand(true).build();
    let editor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    editor.append(&view);
    editor.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    editor.append(&side_scroll);

    let choose = gtk::Button::builder().label("Escolher PDF…").halign(gtk::Align::Center).build();
    choose.add_css_class("pill");
    choose.add_css_class("suggested-action");
    let empty = adw::StatusPage::builder()
        .icon_name(info.icon)
        .title("Escolha um PDF para anotar")
        .description(
            "Ou arraste o arquivo para esta janela. Você poderá escrever textos, destacar trechos e colocar a \
             imagem da sua assinatura.",
        )
        .child(&choose)
        .build();
    let loading = adw::StatusPage::builder()
        .title("Abrindo documento…")
        .child(&gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build())
        .build();
    let error_choose = gtk::Button::builder().label("Escolher outro PDF…").halign(gtk::Align::Center).build();
    error_choose.add_css_class("pill");
    let error_page = adw::StatusPage::builder().icon_name("dialog-warning-symbolic").child(&error_choose).build();
    let stack = gtk::Stack::new();
    stack.add_named(&empty, Some("vazio"));
    stack.add_named(&loading, Some("carregando"));
    stack.add_named(&error_page, Some("erro"));
    stack.add_named(&editor, Some("editor"));

    let action_bar = gtk::ActionBar::new();
    let prev = widgets::icon_button("go-previous-symbolic", "Página anterior (Page Up)");
    let next = widgets::icon_button("go-next-symbolic", "Próxima página (Page Down)");
    let page_label = gtk::Label::new(None);
    page_label.add_css_class("numeric");
    let nav_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    nav_box.append(&prev);
    nav_box.append(&page_label);
    nav_box.append(&next);
    action_bar.set_center_widget(Some(&nav_box));
    let undo_button = widgets::icon_button("edit-undo-symbolic", "Desfazer (Ctrl+Z)");
    action_bar.pack_end(&undo_button);
    let zoom_out = widgets::icon_button("zoom-out-symbolic", "Afastar (Ctrl+−)");
    let zoom_in = widgets::icon_button("zoom-in-symbolic", "Aproximar (Ctrl++)");
    let zoom_label =
        gtk::Button::builder().label(super::page_zoom::zoom_label(1.0)).tooltip_text("Ajustar à área (Ctrl+0)").build();
    zoom_label.add_css_class("flat");
    zoom_label.add_css_class("numeric");
    let zoom_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    zoom_box.add_css_class("linked");
    zoom_box.append(&zoom_out);
    zoom_box.append(&zoom_label);
    zoom_box.append(&zoom_in);
    action_bar.pack_start(&zoom_box);
    action_bar.set_revealed(false);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    toolbar.add_bottom_bar(&action_bar);
    let nav_page = adw::NavigationPage::with_tag(&toolbar, info.title, info.key);
    nav_page.bind_property("can-pop", &back, "visible").invert_boolean().sync_create().build();

    let ann = Rc::new(Annotator {
        app: Rc::downgrade(app),
        nav_page: nav_page.downgrade(),
        input: RefCell::new(None),
        sizes: RefCell::new(Vec::new()),
        page: Cell::new(1),
        items: RefCell::new(Vec::new()),
        selected: Cell::new(None),
        undo: RefCell::new(Vec::new()),
        pages: RefCell::new(HashMap::new()),
        images: RefCell::new(HashMap::new()),
        render_gen: Cell::new(0),
        drag: RefCell::new(None),
        filling: Cell::new(false),
        canvas: canvas.clone(),
        view: view.clone(),
        zoom: Cell::new(None),
        zoom_label: zoom_label.clone(),
        zoom_out: zoom_out.clone(),
        zoom_in: zoom_in.clone(),
        anchor: Cell::new(None),
        pan: Cell::new(None),
        stack,
        error_page,
        save: save.clone(),
        page_label,
        prev: prev.clone(),
        next: next.clone(),
        undo_button: undo_button.clone(),
        action_bar,
        props: Props {
            group: props_group,
            text: text.clone(),
            text_box,
            warning,
            font_size: font_size.clone(),
            bold: bold.clone(),
            color: color.clone(),
            width: width.clone(),
            height: height.clone(),
            line: line.clone(),
            remove_white: remove_white.clone(),
            all_pages: all_pages.clone(),
        },
        list,
        list_group,
        dest_dir: RefCell::new(None),
        saving: Cell::new(false),
        saved: RefCell::new(Vec::new()),
    });

    // Desenho e interação na página
    let a = Rc::downgrade(&ann);
    canvas.set_draw_func(move |_, cr, w, h| {
        if let Some(a) = a.upgrade() {
            a.draw(cr, w as f64, h as f64);
        }
    });
    let drag = gtk::GestureDrag::new();
    let a = Rc::downgrade(&ann);
    drag.connect_drag_begin(move |_, x, y| {
        if let Some(a) = a.upgrade() {
            a.drag_begin(x, y);
        }
    });
    let a = Rc::downgrade(&ann);
    drag.connect_drag_update(move |_, dx, dy| {
        if let Some(a) = a.upgrade() {
            a.drag_update(dx, dy);
        }
    });
    let a = Rc::downgrade(&ann);
    drag.connect_drag_end(move |_, _, _| {
        if let Some(a) = a.upgrade() {
            a.drag.borrow_mut().take();
            a.refresh_panel();
        }
    });
    canvas.add_controller(drag);
    let motion = gtk::EventControllerMotion::new();
    let a = Rc::downgrade(&ann);
    motion.connect_motion(move |_, x, y| {
        if let Some(a) = a.upgrade() {
            a.update_cursor(x, y);
        }
    });
    canvas.add_controller(motion);
    let keys = gtk::EventControllerKey::new();
    let a = Rc::downgrade(&ann);
    keys.connect_key_pressed(move |_, key, _, state| match a.upgrade() {
        Some(a) => a.key(key, state),
        None => glib::Propagation::Proceed,
    });
    canvas.add_controller(keys);
    ann.connect_view_gestures();
    let a = Rc::downgrade(&ann);
    canvas.connect_has_focus_notify(move |_| {
        if let Some(a) = a.upgrade() {
            a.canvas.queue_draw();
        }
    });

    // Ações e atalhos
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: fn(&Rc<Annotator>)| {
        let action = gio::SimpleAction::new(name, None);
        let a = Rc::downgrade(&ann);
        action.connect_activate(move |_, _| {
            if let Some(a) = a.upgrade() {
                f(&a);
            }
        });
        group.add_action(&action);
    };
    add("open", |a| a.pick());
    add("run", |a| a.save_to(None));
    add("save-as", |a| a.save_as());
    add("undo", |a| a.undo());
    add("prev", |a| a.go_to(a.page.get().saturating_sub(1)));
    add("next", |a| a.go_to(a.page.get() + 1));
    add("add-text", |a| a.add_text("Texto"));
    add("add-date", |a| {
        let today = glib::DateTime::now_local().ok().and_then(|d| d.format("%d/%m/%Y").ok());
        a.add_text(today.as_deref().unwrap_or("Data"));
    });
    add("add-image", |a| a.pick_image());
    add("add-highlight", |a| a.add_shape(true));
    add("add-frame", |a| a.add_shape(false));
    add("leave", |a| a.leave());
    add("zoom-in", |a| a.zoom_by(ZOOM_STEP, None));
    add("zoom-out", |a| a.zoom_by(1.0 / ZOOM_STEP, None));
    add("zoom-fit", |a| a.set_zoom(None, None));
    nav_page.insert_action_group("page", Some(&group));
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Global);
    for (accel, action) in [
        ("<Control>s", "page.run"),
        ("<Control>z", "page.undo"),
        ("<Control>o", "page.open"),
        ("<Control>plus", "page.zoom-in"),
        ("<Control>equal", "page.zoom-in"),
        ("<Control>KP_Add", "page.zoom-in"),
        ("<Control>minus", "page.zoom-out"),
        ("<Control>KP_Subtract", "page.zoom-out"),
        ("<Control>0", "page.zoom-fit"),
        ("<Control>KP_0", "page.zoom-fit"),
    ] {
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(accel),
            Some(gtk::NamedAction::new(action)),
        ));
    }
    nav_page.add_controller(shortcuts);
    // Esc e Alt+← antes do AdwNavigationView, que os consome mesmo quando a
    // página não pode sair sozinha.
    let back_keys = gtk::ShortcutController::new();
    back_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    for accel in ["Escape", "<Alt>Left"] {
        back_keys.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(accel),
            Some(gtk::NamedAction::new("page.leave")),
        ));
    }
    nav_page.add_controller(back_keys);
    for (b, action) in [
        (&prev, "page.prev"),
        (&next, "page.next"),
        (&undo_button, "page.undo"),
        (&zoom_in, "page.zoom-in"),
        (&zoom_out, "page.zoom-out"),
        (&zoom_label, "page.zoom-fit"),
        (&add_text, "page.add-text"),
        (&add_date, "page.add-date"),
        (&add_image, "page.add-image"),
        (&add_highlight, "page.add-highlight"),
        (&add_frame, "page.add-frame"),
    ] {
        b.set_action_name(Some(action));
    }
    save.set_action_name(Some("page.run"));
    for b in [&open, &choose, &error_choose] {
        b.set_action_name(Some("page.open"));
    }

    // Painel → anotação selecionada
    let a = Rc::downgrade(&ann);
    text.buffer().connect_changed(move |buf| {
        if let Some(a) = a.upgrade() {
            let value = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            a.edit(|item| {
                if let Kind::Text { text, .. } = &mut item.kind {
                    *text = value.clone();
                }
            });
        }
    });
    let a = Rc::downgrade(&ann);
    font_size.connect_value_notify(move |row| {
        if let Some(a) = a.upgrade() {
            let v = row.value() as f32;
            a.edit(|item| {
                if let Kind::Text { size, .. } = &mut item.kind {
                    *size = v;
                }
            });
        }
    });
    let a = Rc::downgrade(&ann);
    bold.connect_active_notify(move |row| {
        if let Some(a) = a.upgrade() {
            let v = row.is_active();
            a.edit(|item| {
                if let Kind::Text { bold, .. } = &mut item.kind {
                    *bold = v;
                }
            });
        }
    });
    let a = Rc::downgrade(&ann);
    color.connect_selected_notify(move |row| {
        if let Some(a) = a.upgrade() {
            let c = COLORS.get(row.selected() as usize).map_or(COLORS[BLACK].1, |(_, c)| *c);
            a.edit(|item| match &mut item.kind {
                Kind::Text { color, .. } | Kind::Highlight { color } | Kind::Frame { color, .. } => *color = c,
                Kind::Image { .. } => {}
            });
        }
    });
    let a = Rc::downgrade(&ann);
    width.connect_value_notify(move |row| {
        if let Some(a) = a.upgrade() {
            let v = row.value() as f32 * MM;
            a.edit(|item| {
                if matches!(item.kind, Kind::Image { .. }) && item.w > 0.0 {
                    item.h *= v / item.w;
                }
                item.w = v;
            });
            a.fill_size_rows();
        }
    });
    let a = Rc::downgrade(&ann);
    height.connect_value_notify(move |row| {
        if let Some(a) = a.upgrade() {
            let v = row.value() as f32 * MM;
            a.edit(|item| item.h = v);
        }
    });
    let a = Rc::downgrade(&ann);
    line.connect_value_notify(move |row| {
        if let Some(a) = a.upgrade() {
            let v = row.value() as f32;
            a.edit(|item| {
                if let Kind::Frame { line_width, .. } = &mut item.kind {
                    *line_width = v;
                }
            });
        }
    });
    let a = Rc::downgrade(&ann);
    remove_white.connect_active_notify(move |row| {
        if let Some(a) = a.upgrade() {
            let v = row.is_active();
            a.edit(|item| {
                if let Kind::Image { remove_white, .. } = &mut item.kind {
                    *remove_white = v;
                }
            });
        }
    });
    let a = Rc::downgrade(&ann);
    all_pages.connect_active_notify(move |row| {
        if let Some(a) = a.upgrade() {
            let v = row.is_active();
            a.edit(|item| {
                item.all_pages = v;
                if !v {
                    item.page = a.page.get();
                }
            });
        }
    });
    let a = Rc::downgrade(&ann);
    delete.connect_clicked(move |_| {
        if let Some(a) = a.upgrade() {
            a.delete_selected();
        }
    });

    let a = Rc::downgrade(&ann);
    widgets::add_file_drop(&nav_page, move |files| {
        let Some(a) = a.upgrade() else { return };
        if let Some(f) = files.iter().find(|f| catalog::extension_ok(Accepts::Pdf, f)).cloned() {
            a.confirm_discard(move |a| a.load(f.clone()));
        } else if a.input.borrow().is_some()
            && let Some(img) = files.into_iter().find(|f| catalog::extension_ok(Accepts::Images, f))
        {
            a.add_image(img);
        }
    });
    let a = Rc::downgrade(&ann);
    app.nav.connect_popped(move |_, popped| {
        if let Some(a) = a.upgrade()
            && a.nav_page.upgrade().as_ref() == Some(popped)
        {
            a.render_gen.set(a.render_gen.get() + 1);
            a.pages.borrow_mut().clear();
            a.images.borrow_mut().clear();
        }
    });

    if let Some(f) = files.into_iter().next() {
        ann.load(f);
    }
    let keep = ann.clone();
    nav_page.connect_destroy(move |_| {
        let _ = &keep;
    });
    nav_page
}

impl Annotator {
    fn window(&self) -> Option<adw::ApplicationWindow> {
        self.app.upgrade().map(|a| a.window.clone())
    }

    fn toast(&self, text: &str) {
        if let Some(app) = self.app.upgrade() {
            app.toast(text);
        }
    }

    fn pick(self: &Rc<Self>) {
        self.confirm_discard(|a| a.pick_now());
    }

    fn pick_now(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let a = self.clone();
        glib::spawn_future_local(async move {
            if let Some(f) = widgets::pick_files(&win, Some(Accepts::Pdf), false).await.into_iter().next() {
                a.load(f);
            }
        });
    }

    fn load(self: &Rc<Self>, path: PathBuf) {
        self.stack.set_visible_child_name("carregando");
        self.action_bar.set_revealed(false);
        self.save.set_sensitive(false);
        let a = self.clone();
        glib::spawn_future_local(async move {
            // PDF com senha: pede a senha e abre uma cópia desbloqueada.
            let Some(app) = a.app.upgrade() else { return };
            let Some(path) = super::unlock::prepare(&app, vec![path]).await.into_iter().next() else {
                a.stack.set_visible_child_name(if a.input.borrow().is_some() { "editor" } else { "vazio" });
                a.action_bar.set_revealed(a.input.borrow().is_some());
                a.save.set_sensitive(a.input.borrow().is_some());
                return;
            };
            let p = path.clone();
            let result = widgets::blocking(move || -> OpResult<Vec<(f32, f32)>> {
                let cancel = CancelToken::new();
                let n = qpdf::page_count(&p, &cancel)?;
                poppler::page_sizes(&p, n, &cancel)
            })
            .await;
            match result {
                Ok(sizes) if !sizes.is_empty() => a.start(path, sizes),
                Ok(_) => a.show_load_error(&OpError::OutputInvalid("o documento não tem páginas".into())),
                Err(e) => a.show_load_error(&e),
            }
        });
    }

    fn show_load_error(&self, e: &OpError) {
        self.error_page.set_title(e.title());
        self.error_page.set_description(Some(&glib::markup_escape_text(&e.to_string())));
        self.stack.set_visible_child_name("erro");
    }

    fn start(self: &Rc<Self>, path: PathBuf, sizes: Vec<(f32, f32)>) {
        if let Some(page) = self.nav_page.upgrade() {
            page.set_title(&format!("Anotar — {}", display_name(&path)));
        }
        *self.dest_dir.borrow_mut() = super::unlock::original(&path).parent().map(Path::to_path_buf);
        *self.sizes.borrow_mut() = sizes;
        *self.input.borrow_mut() = Some(path);
        self.items.borrow_mut().clear();
        self.saved.borrow_mut().clear();
        self.undo.borrow_mut().clear();
        self.pages.borrow_mut().clear();
        self.selected.set(None);
        self.zoom.set(None);
        self.page.set(0);
        self.stack.set_visible_child_name("editor");
        self.action_bar.set_revealed(true);
        self.go_to(1);
        self.refresh_panel();
        self.canvas.grab_focus();
    }

    fn total(&self) -> u32 {
        self.sizes.borrow().len() as u32
    }

    fn page_size(&self) -> (f32, f32) {
        self.sizes.borrow().get(self.page.get() as usize - 1).copied().unwrap_or(pdfwriter::A4_POINTS)
    }

    fn go_to(self: &Rc<Self>, page: u32) {
        let total = self.total();
        if page == 0 || page > total || page == self.page.get() {
            return;
        }
        self.page.set(page);
        if let Some(i) = self.selected.get()
            && !self.items.borrow()[i].on_page(page)
        {
            self.selected.set(None);
            self.refresh_panel();
        }
        self.page_label.set_text(&format!("Página {page} de {total}"));
        self.prev.set_sensitive(page > 1);
        self.next.set_sensitive(page < total);
        self.apply_zoom();
        self.render_page(page);
        self.canvas.queue_draw();
    }

    fn render_page(self: &Rc<Self>, page: u32) {
        let side = self.wanted_side();
        let have = self.pages.borrow().get(&page).map_or(0, |s| s.width().max(s.height()) as u32);
        if have as f64 >= side as f64 * 0.9 {
            return;
        }
        let Some(path) = self.input.borrow().clone() else { return };
        let generation = self.render_gen.get() + 1;
        self.render_gen.set(generation);
        let a = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let pixels = widgets::blocking(move || -> Option<image::RgbaImage> {
                let dir = JobDir::new().ok()?;
                let files = poppler::thumbnails(&path, page, page, side, dir.path(), &CancelToken::new()).ok()?;
                image::open(files.first()?).ok().map(|i| i.into_rgba8())
            })
            .await;
            let Some(a) = a.upgrade() else { return };
            if a.render_gen.get() != generation && a.page.get() != page {
                return;
            }
            if let Some(surface) = pixels.and_then(surface_from_rgba) {
                let mut pages = a.pages.borrow_mut();
                if pages.len() > 8 {
                    pages.clear();
                }
                // Só a página atual fica guardada em alta resolução.
                if side > RENDER_SIDE {
                    pages.retain(|_, s| s.width().max(s.height()) as u32 <= RENDER_SIDE);
                }
                // Uma renderização menor que chegue atrasada não substitui a maior.
                let bigger = pages.get(&page).is_none_or(|old| old.width() < surface.width());
                if bigger {
                    pages.insert(page, surface);
                }
            }
            a.canvas.queue_draw();
        });
    }

    fn image_surface(self: &Rc<Self>, path: &Path, remove_white: bool) -> Option<cairo::ImageSurface> {
        let key = (path.to_path_buf(), remove_white);
        if let Some(entry) = self.images.borrow().get(&key) {
            return entry.clone();
        }
        self.images.borrow_mut().insert(key.clone(), None);
        let a = Rc::downgrade(self);
        let p = path.to_path_buf();
        glib::spawn_future_local(async move {
            let pixels = widgets::blocking(move || pdfwriter::decode_rgba(&p, IMAGE_SIDE, remove_white).ok()).await;
            let Some(a) = a.upgrade() else { return };
            a.images.borrow_mut().insert(key, pixels.and_then(surface_from_rgba));
            a.canvas.queue_draw();
        });
        None
    }

    /// Pixels por ponto com a página ajustada à área visível.
    fn fit_scale(&self) -> f64 {
        let (pw, ph) = self.page_size();
        let (w, h) = (self.view.width() as f64, self.view.height() as f64);
        ((w - 2.0 * MARGIN) / pw as f64).min((h - 2.0 * MARGIN) / ph as f64).max(0.05)
    }

    /// Posição e escala da página na área de desenho: (x, y, pixels por ponto).
    fn layout(&self) -> (f64, f64, f64) {
        let (pw, ph) = self.page_size();
        let (w, h) = (self.canvas.width() as f64, self.canvas.height() as f64);
        let scale = match self.zoom.get() {
            Some(s) => s,
            None => ((w - 2.0 * MARGIN) / pw as f64).min((h - 2.0 * MARGIN) / ph as f64).max(0.05),
        };
        (((w - pw as f64 * scale) / 2.0).max(MARGIN), ((h - ph as f64 * scale) / 2.0).max(MARGIN), scale)
    }

    /// Ctrl+roda e pinça aproximam em volta do ponteiro; arrastar fora das
    /// anotações move a vista quando a página está aproximada.
    fn connect_view_gestures(self: &Rc<Self>) {
        let pointer = Rc::new(Cell::new(None::<(f64, f64)>));
        let motion = gtk::EventControllerMotion::new();
        let p = pointer.clone();
        motion.connect_motion(move |_, x, y| p.set(Some((x, y))));
        let p = pointer.clone();
        motion.connect_leave(move |_| p.set(None));
        self.view.add_controller(motion);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let a = Rc::downgrade(self);
        scroll.connect_scroll(move |c, _, dy| {
            let Some(a) = a.upgrade() else { return glib::Propagation::Proceed };
            if !c.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            a.zoom_by(if dy < 0.0 { 1.1 } else { 1.0 / 1.1 }, pointer.get());
            glib::Propagation::Stop
        });
        self.view.add_controller(scroll);

        let pinch = gtk::GestureZoom::new();
        let start = Rc::new(Cell::new(1.0));
        let (a, s) = (Rc::downgrade(self), start.clone());
        pinch.connect_begin(move |_, _| {
            if let Some(a) = a.upgrade() {
                s.set(a.zoom.get().unwrap_or_else(|| a.fit_scale()));
            }
        });
        let a = Rc::downgrade(self);
        pinch.connect_scale_changed(move |g, scale| {
            if let Some(a) = a.upgrade() {
                a.set_zoom(Some(start.get() * scale), g.bounding_box_center());
            }
        });
        self.view.add_controller(pinch);

        // A vista não se move com a rolagem (a área de desenho sim), então o
        // deslocamento do arraste é medido nela.
        let pan = gtk::GestureDrag::new();
        let a = Rc::downgrade(self);
        pan.connect_drag_begin(move |_, _, _| {
            let Some(a) = a.upgrade() else { return };
            // O arraste de uma anotação (tratado antes, na área de desenho) tem prioridade.
            if a.zoom.get().is_some() && a.drag.borrow().is_none() {
                a.pan.set(Some((a.view.hadjustment().value(), a.view.vadjustment().value())));
                a.canvas.set_cursor_from_name(Some("grabbing"));
            }
        });
        let a = Rc::downgrade(self);
        pan.connect_drag_update(move |_, dx, dy| {
            if let Some(a) = a.upgrade()
                && let Some((x, y)) = a.pan.get()
            {
                a.view.hadjustment().set_value(x - dx);
                a.view.vadjustment().set_value(y - dy);
            }
        });
        let a = Rc::downgrade(self);
        pan.connect_drag_end(move |_, _, _| {
            if let Some(a) = a.upgrade()
                && a.pan.take().is_some()
            {
                a.canvas.set_cursor_from_name(Some("default"));
            }
        });
        self.view.add_controller(pan);

        for horizontal in [true, false] {
            let adj = if horizontal { self.view.hadjustment() } else { self.view.vadjustment() };
            let a = Rc::downgrade(self);
            adj.connect_upper_notify(move |adj| {
                let Some(a) = a.upgrade() else { return };
                if let Some(((fx, fy), (px, py))) = a.anchor.get() {
                    let (f, p) = if horizontal { (fx, px) } else { (fy, py) };
                    adj.set_value(f * adj.upper() - p);
                }
            });
        }
    }

    fn zoom_by(self: &Rc<Self>, factor: f64, at: Option<(f64, f64)>) {
        if self.input.borrow().is_none() {
            return;
        }
        let current = self.zoom.get().unwrap_or_else(|| self.fit_scale());
        self.set_zoom(Some(current * factor), at);
    }

    /// Muda o zoom (pixels por ponto; `None` = ajustada), mantendo parado o
    /// ponto `at` da vista (padrão: o centro).
    fn set_zoom(self: &Rc<Self>, scale: Option<f64>, at: Option<(f64, f64)>) {
        let fit = self.fit_scale();
        let max = fit * super::page_zoom::MAX_ZOOM;
        let scale = scale.map(|s| s.min(max)).filter(|s| *s > fit * 1.001);
        let old = self.zoom.replace(scale);
        let (h, v) = (self.view.hadjustment(), self.view.vadjustment());
        let (px, py) = at.unwrap_or((h.page_size() / 2.0, v.page_size() / 2.0));
        let frac =
            |adj: &gtk::Adjustment, p: f64| if adj.upper() > 0.0 { (adj.value() + p) / adj.upper() } else { 0.5 };
        if old != scale {
            self.anchor.set(Some(((frac(&h, px), frac(&v, py)), (px, py))));
            let a = Rc::downgrade(self);
            glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                if let Some(a) = a.upgrade() {
                    a.anchor.set(None);
                }
            });
        }
        self.apply_zoom();
    }

    /// Tamanho da área de desenho, rótulo e resolução conforme o zoom.
    fn apply_zoom(self: &Rc<Self>) {
        let (pw, ph) = self.page_size();
        match self.zoom.get() {
            Some(s) => {
                self.canvas.set_content_width((pw as f64 * s + 2.0 * MARGIN).ceil() as i32);
                self.canvas.set_content_height((ph as f64 * s + 2.0 * MARGIN).ceil() as i32);
            }
            None => {
                self.canvas.set_content_width(0);
                self.canvas.set_content_height(0);
            }
        }
        let relative = self.zoom.get().map_or(1.0, |s| s / self.fit_scale());
        self.zoom_label.set_label(&super::page_zoom::zoom_label(relative));
        self.zoom_out.set_sensitive(self.zoom.get().is_some());
        self.zoom_in.set_sensitive(relative < super::page_zoom::MAX_ZOOM - 1e-3);
        self.canvas.queue_draw();
        // Mais pixels quando o zoom parar de mudar.
        let a = Rc::downgrade(self);
        let zoom = self.zoom.get();
        glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            if let Some(a) = a.upgrade()
                && a.zoom.get() == zoom
            {
                a.render_page(a.page.get());
            }
        });
    }

    /// Lado da página renderizada que o zoom atual pede.
    fn wanted_side(&self) -> u32 {
        let (pw, ph) = self.page_size();
        let scale = self.zoom.get().unwrap_or_else(|| self.fit_scale()) * self.canvas.scale_factor().max(1) as f64;
        ((pw.max(ph) as f64 * scale).ceil() as u32).clamp(RENDER_SIDE, MAX_RENDER_SIDE)
    }

    /// Rola a vista para a anotação selecionada aparecer inteira (zoom).
    fn reveal_selected(&self) {
        let (Some(i), Some(_)) = (self.selected.get(), self.zoom.get()) else { return };
        let Some(item) = self.items.borrow().get(i).cloned() else { return };
        let (ox, oy, s) = self.layout();
        let (x, y, w, h) = item.rect_on(self.page_size());
        let reveal = |adj: gtk::Adjustment, start: f64, len: f64| {
            let (lo, hi) = (start - MARGIN, start + len + MARGIN);
            if lo < adj.value() {
                adj.set_value(lo);
            } else if hi > adj.value() + adj.page_size() {
                adj.set_value((hi - adj.page_size()).min(lo));
            }
        };
        reveal(self.view.hadjustment(), ox + x as f64 * s, w as f64 * s);
        reveal(self.view.vadjustment(), oy + y as f64 * s, h as f64 * s);
    }

    fn draw(self: &Rc<Self>, cr: &cairo::Context, _w: f64, _h: f64) {
        if self.input.borrow().is_none() {
            return;
        }
        let (pw, ph) = self.page_size();
        let (ox, oy, s) = self.layout();
        let (pw_px, ph_px) = (pw as f64 * s, ph as f64 * s);
        // Papel com sombra discreta.
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.18);
        cr.rectangle(ox + 2.0, oy + 3.0, pw_px, ph_px);
        let _ = cr.fill();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.rectangle(ox, oy, pw_px, ph_px);
        let _ = cr.fill();
        let page = self.page.get();
        if let Some(surface) = self.pages.borrow().get(&page) {
            let _ = cr.save();
            cr.translate(ox, oy);
            cr.scale(pw_px / surface.width() as f64, ph_px / surface.height() as f64);
            let _ = cr.set_source_surface(surface, 0.0, 0.0);
            cr.source().set_filter(cairo::Filter::Good);
            let _ = cr.paint();
            let _ = cr.restore();
        } else {
            cr.set_source_rgb(0.45, 0.45, 0.45);
            cr.set_font_size(14.0);
            let label = "Carregando página…";
            if let Ok(ext) = cr.text_extents(label) {
                cr.move_to(ox + (pw_px - ext.width()) / 2.0, oy + ph_px / 2.0);
                let _ = cr.show_text(label);
            }
        }

        let items = self.items.borrow().clone();
        let _ = cr.save();
        cr.rectangle(ox, oy, pw_px, ph_px);
        cr.clip();
        cr.translate(ox, oy);
        cr.scale(s, s);
        for item in items.iter().filter(|i| i.on_page(page)) {
            self.draw_item(cr, item, (pw, ph));
        }
        let _ = cr.restore();

        // Contornos: tracejado leve em todas, destaque na selecionada.
        let selected = self.selected.get();
        let accent = self.canvas.color();
        for (i, item) in items.iter().enumerate().filter(|(_, i)| i.on_page(page)) {
            let (x, y, w, h) = item.rect_on((pw, ph));
            let (x, y, w, h) = (ox + x as f64 * s, oy + y as f64 * s, (w as f64 * s).max(6.0), (h as f64 * s).max(6.0));
            if Some(i) == selected {
                cr.set_source_rgb(0.21, 0.52, 0.89);
                cr.set_line_width(1.5);
                cr.set_dash(&[], 0.0);
                cr.rectangle(x - 3.0, y - 3.0, w + 6.0, h + 6.0);
                let _ = cr.stroke();
                cr.rectangle(x + w + 3.0 - HANDLE / 2.0, y + h + 3.0 - HANDLE / 2.0, HANDLE, HANDLE);
                let _ = cr.fill_preserve();
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            } else {
                cr.set_source_rgba(0.3, 0.3, 0.3, 0.6);
                cr.set_line_width(1.0);
                cr.set_dash(&[3.0, 3.0], 0.0);
                cr.rectangle(x - 2.0, y - 2.0, w + 4.0, h + 4.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
            }
        }
        if self.canvas.has_focus() && selected.is_none() && !items.is_empty() {
            cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, 0.25);
            cr.set_line_width(2.0);
            cr.rectangle(ox - 4.0, oy - 4.0, pw_px + 8.0, ph_px + 8.0);
            let _ = cr.stroke();
        }
    }

    /// Desenha uma anotação em pontos, origem no canto superior esquerdo.
    fn draw_item(self: &Rc<Self>, cr: &cairo::Context, item: &Annotation, size: (f32, f32)) {
        let (x, y, w, h) = item.rect_on(size);
        let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
        let _ = cr.save();
        match &item.kind {
            Kind::Text { text, size, bold, color } => {
                cr.set_source_rgba(color.0 as f64, color.1 as f64, color.2 as f64, item.opacity as f64);
                let weight = if *bold { cairo::FontWeight::Bold } else { cairo::FontWeight::Normal };
                cr.select_font_face("Helvetica", cairo::FontSlant::Normal, weight);
                cr.set_font_size(*size as f64);
                for (i, line) in text.trim_end().lines().enumerate() {
                    cr.move_to(x, y + (ASCENT + i as f32 * LEADING) as f64 * *size as f64);
                    let _ = cr.show_text(line);
                }
            }
            Kind::Image { path, remove_white } => match self.image_surface(path, *remove_white) {
                Some(surface) => {
                    cr.translate(x, y);
                    cr.scale(w / surface.width() as f64, h / surface.height() as f64);
                    let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                    cr.source().set_filter(cairo::Filter::Good);
                    let _ = cr.paint_with_alpha(item.opacity as f64);
                }
                None => {
                    cr.set_source_rgba(0.5, 0.5, 0.5, 0.2);
                    cr.rectangle(x, y, w, h);
                    let _ = cr.fill();
                }
            },
            Kind::Highlight { color } => {
                cr.set_operator(cairo::Operator::Multiply);
                cr.set_source_rgba(color.0 as f64, color.1 as f64, color.2 as f64, item.opacity as f64);
                cr.rectangle(x, y, w, h);
                let _ = cr.fill();
            }
            Kind::Frame { color, line_width } => {
                let lw = *line_width as f64;
                cr.set_source_rgba(color.0 as f64, color.1 as f64, color.2 as f64, item.opacity as f64);
                cr.set_line_width(lw);
                cr.rectangle(x + lw / 2.0, y + lw / 2.0, (w - lw).max(0.0), (h - lw).max(0.0));
                let _ = cr.stroke();
            }
        }
        let _ = cr.restore();
    }

    /// Anotação da página atual sob o ponto (em pixels), a de cima primeiro.
    fn hit(&self, px: f64, py: f64) -> Option<(usize, DragMode)> {
        let (ox, oy, s) = self.layout();
        let size = self.page_size();
        let page = self.page.get();
        let items = self.items.borrow();
        // A alça da selecionada tem prioridade.
        if let Some(i) = self.selected.get()
            && let Some(item) = items.get(i)
            && item.on_page(page)
        {
            let (x, y, w, h) = item.rect_on(size);
            let (hx, hy) = (ox + (x + w) as f64 * s + 3.0, oy + (y + h) as f64 * s + 3.0);
            if (px - hx).abs() <= HANDLE && (py - hy).abs() <= HANDLE {
                return Some((i, DragMode::Resize));
            }
        }
        items.iter().enumerate().rev().filter(|(_, it)| it.on_page(page)).find_map(|(i, item)| {
            let (x, y, w, h) = item.rect_on(size);
            let (x0, y0) = (ox + x as f64 * s - 4.0, oy + y as f64 * s - 4.0);
            let (x1, y1) = (x0 + (w as f64 * s).max(6.0) + 8.0, y0 + (h as f64 * s).max(6.0) + 8.0);
            (px >= x0 && px <= x1 && py >= y0 && py <= y1).then_some((i, DragMode::Move))
        })
    }

    fn update_cursor(&self, x: f64, y: f64) {
        let name = match self.hit(x, y) {
            Some((_, DragMode::Resize)) => "nwse-resize",
            Some((_, DragMode::Move)) => "move",
            None => "default",
        };
        self.canvas.set_cursor_from_name(Some(name));
    }

    fn drag_begin(self: &Rc<Self>, x: f64, y: f64) {
        self.canvas.grab_focus();
        match self.hit(x, y) {
            Some((index, mode)) => {
                let start = self.items.borrow()[index].clone();
                *self.drag.borrow_mut() = Some(Drag { index, mode, start, moved: false });
                self.select(Some(index));
            }
            None => {
                self.drag.borrow_mut().take();
                self.select(None);
            }
        }
    }

    fn drag_update(&self, dx: f64, dy: f64) {
        let Some(mut drag) = self.drag.borrow().clone() else { return };
        if !drag.moved {
            if dx.abs() < 2.0 && dy.abs() < 2.0 {
                return;
            }
            self.remember();
            drag.moved = true;
            *self.drag.borrow_mut() = Some(drag.clone());
        }
        let (_, _, s) = self.layout();
        let (dxp, dyp) = ((dx / s) as f32, (dy / s) as f32);
        let size = self.page_size();
        let mut item = drag.start.clone();
        match drag.mode {
            DragMode::Move => {
                item.x += dxp / size.0;
                item.y += dyp / size.1;
            }
            DragMode::Resize => resize(&mut item, dxp, dyp),
        }
        keep_inside(&mut item, size);
        self.items.borrow_mut()[drag.index] = item;
        self.canvas.queue_draw();
    }

    fn key(self: &Rc<Self>, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> glib::Propagation {
        use gtk::gdk::Key;
        let shift = state.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let step = if shift { 12.0 } else { 2.0 };
        let (dx, dy) = match key {
            Key::Left | Key::KP_Left => (-step, 0.0),
            Key::Right | Key::KP_Right => (step, 0.0),
            Key::Up | Key::KP_Up => (0.0, -step),
            Key::Down | Key::KP_Down => (0.0, step),
            _ => (0.0, 0.0),
        };
        if dx != 0.0 || dy != 0.0 {
            return match self.selected.get() {
                Some(_) => {
                    self.remember();
                    let size = self.page_size();
                    self.edit_structural(|item| {
                        item.x += dx / size.0;
                        item.y += dy / size.1;
                        keep_inside(item, size);
                    });
                    self.reveal_selected();
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            };
        }
        match key {
            Key::plus | Key::equal | Key::KP_Add | Key::minus | Key::KP_Subtract if self.selected.get().is_some() => {
                let grow = matches!(key, Key::plus | Key::equal | Key::KP_Add);
                let size = self.page_size();
                self.remember();
                self.edit_structural(|item| {
                    let (w, h) = item.size();
                    let factor = if grow { 0.1 } else { -0.1 };
                    resize(item, w * factor, h * factor);
                    keep_inside(item, size);
                });
                self.refresh_panel();
                glib::Propagation::Stop
            }
            Key::Delete | Key::KP_Delete | Key::BackSpace if self.selected.get().is_some() => {
                self.delete_selected();
                glib::Propagation::Stop
            }
            Key::Tab | Key::ISO_Left_Tab => {
                let page = self.page.get();
                let on_page: Vec<usize> =
                    self.items.borrow().iter().enumerate().filter(|(_, i)| i.on_page(page)).map(|(i, _)| i).collect();
                let back = key == Key::ISO_Left_Tab || shift;
                let pos = self.selected.get().and_then(|s| on_page.iter().position(|&i| i == s));
                let next = match (pos, back) {
                    (None, false) => on_page.first().copied(),
                    (None, true) => on_page.last().copied(),
                    (Some(p), false) => on_page.get(p + 1).copied(),
                    (Some(p), true) => p.checked_sub(1).and_then(|p| on_page.get(p).copied()),
                };
                match next {
                    Some(i) => {
                        self.select(Some(i));
                        glib::Propagation::Stop
                    }
                    // Fim da lista: o foco segue para o próximo controle.
                    None => {
                        self.select(None);
                        glib::Propagation::Proceed
                    }
                }
            }
            Key::Page_Up | Key::KP_Page_Up => {
                self.go_to(self.page.get().saturating_sub(1));
                glib::Propagation::Stop
            }
            Key::Page_Down | Key::KP_Page_Down => {
                self.go_to(self.page.get() + 1);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    fn remember(&self) {
        let snapshot = self.items.borrow().clone();
        let mut undo = self.undo.borrow_mut();
        undo.push(snapshot);
        if undo.len() > 100 {
            undo.remove(0);
        }
        self.undo_button.set_sensitive(true);
    }

    fn undo(self: &Rc<Self>) {
        let Some(prev) = self.undo.borrow_mut().pop() else { return };
        let len = prev.len();
        *self.items.borrow_mut() = prev;
        if self.selected.get().is_some_and(|i| i >= len) {
            self.selected.set(None);
        }
        self.refresh_panel();
        self.canvas.queue_draw();
    }

    /// Altera a anotação selecionada a partir do painel.
    fn edit(self: &Rc<Self>, f: impl FnOnce(&mut Annotation)) {
        if self.filling.get() {
            return;
        }
        self.edit_structural(f);
        self.refresh_list();
        self.update_warning();
    }

    fn edit_structural(&self, f: impl FnOnce(&mut Annotation)) {
        let Some(i) = self.selected.get() else { return };
        if let Some(item) = self.items.borrow_mut().get_mut(i) {
            f(item);
        }
        self.update_dirty();
        self.canvas.queue_draw();
    }

    fn dirty(&self) -> bool {
        *self.items.borrow() != *self.saved.borrow()
    }

    fn update_dirty(&self) {
        if let Some(page) = self.nav_page.upgrade() {
            page.set_can_pop(!self.dirty());
        }
    }

    /// Volta à tela anterior, confirmando se há anotações não salvas.
    fn leave(self: &Rc<Self>) {
        // Na página, Esc primeiro só desfaz a seleção.
        if self.canvas.has_focus() && self.selected.get().is_some() {
            self.select(None);
            return;
        }
        self.confirm_discard(|a| {
            a.items.borrow_mut().clear();
            a.update_dirty();
            if let Some(app) = a.app.upgrade() {
                app.nav.pop();
            }
        });
    }

    /// Executa `then` direto, ou depois de o usuário aceitar descartar as
    /// anotações não salvas.
    fn confirm_discard(self: &Rc<Self>, then: impl Fn(&Rc<Self>) + 'static) {
        let Some(app) = self.app.upgrade() else { return };
        if !self.dirty() {
            then(self);
            return;
        }
        let n = self.items.borrow().len();
        let dialog = adw::AlertDialog::new(
            Some("Descartar as anotações?"),
            Some(&format!(
                "{n} anotaç{} ainda não fo{} salva{}. O PDF original não foi alterado.",
                if n == 1 { "ão" } else { "ões" },
                if n == 1 { "i" } else { "ram" },
                if n == 1 { "" } else { "s" }
            )),
        );
        dialog.add_response("cancel", "Continuar anotando");
        dialog.add_response("discard", "Descartar");
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let a = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response == "discard"
                && let Some(a) = a.upgrade()
            {
                then(&a);
            }
        });
        dialog.present(Some(&app.window));
    }

    fn push_item(self: &Rc<Self>, item: Annotation) {
        self.remember();
        let index = {
            let mut items = self.items.borrow_mut();
            items.push(item);
            items.len() - 1
        };
        self.select(Some(index));
        self.canvas.grab_focus();
        if let Some(Annotation { kind: Kind::Text { .. }, .. }) = self.items.borrow().get(index) {
            // Já deixa o texto pronto para ser substituído.
            let buf = self.props.text.buffer();
            buf.select_range(&buf.start_iter(), &buf.end_iter());
            self.props.text.grab_focus();
        }
    }

    /// Nova anotação centrada na página atual.
    fn centered(&self, kind: Kind, w: f32, h: f32) -> Annotation {
        let (pw, ph) = self.page_size();
        let mut item = Annotation { kind, page: self.page.get(), all_pages: false, x: 0.0, y: 0.0, w, h, opacity: 1.0 };
        let (bw, bh) = item.size();
        item.x = (0.5 - bw / 2.0 / pw).max(0.0);
        item.y = (0.5 - bh / 2.0 / ph).max(0.0);
        item
    }

    fn add_text(self: &Rc<Self>, text: &str) {
        if self.input.borrow().is_none() {
            return;
        }
        let kind = Kind::Text { text: text.into(), size: 14.0, bold: false, color: COLORS[BLACK].1 };
        let item = self.centered(kind, 0.0, 0.0);
        self.push_item(item);
    }

    fn add_shape(self: &Rc<Self>, highlight: bool) {
        if self.input.borrow().is_none() {
            return;
        }
        let item = if highlight {
            self.centered(Kind::Highlight { color: COLORS[YELLOW].1 }, 60.0 * MM, 6.0 * MM)
        } else {
            self.centered(Kind::Frame { color: COLORS[RED].1, line_width: 2.0 }, 60.0 * MM, 25.0 * MM)
        };
        self.push_item(item);
    }

    fn pick_image(self: &Rc<Self>) {
        if self.input.borrow().is_none() {
            return;
        }
        let Some(win) = self.window() else { return };
        let a = self.clone();
        glib::spawn_future_local(async move {
            if let Some(f) = widgets::pick_files(&win, Some(Accepts::Images), false).await.into_iter().next() {
                a.add_image(f);
            }
        });
    }

    fn add_image(self: &Rc<Self>, path: PathBuf) {
        let a = self.clone();
        glib::spawn_future_local(async move {
            let p = path.clone();
            match widgets::blocking(move || pdfwriter::display_dimensions(&p)).await {
                Ok((iw, ih)) => {
                    let (pw, _) = a.page_size();
                    // Largura de uma assinatura (5 cm), sem passar de 60% da página.
                    let w = (50.0 * MM).min(pw * 0.6);
                    let h = w * ih as f32 / iw.max(1) as f32;
                    let mut item = a.centered(Kind::Image { path, remove_white: true }, w, h);
                    // Assinaturas costumam ir na parte de baixo.
                    item.y = item.y.max(0.62);
                    keep_inside(&mut item, a.page_size());
                    a.push_item(item);
                }
                Err(e) => a.toast(&e.to_string()),
            }
        });
    }

    fn delete_selected(self: &Rc<Self>) {
        let Some(i) = self.selected.get() else { return };
        self.remember();
        self.items.borrow_mut().remove(i);
        self.selected.set(None);
        self.refresh_panel();
        self.canvas.queue_draw();
        self.canvas.grab_focus();
        self.toast("Anotação removida. Ctrl+Z desfaz.");
    }

    fn select(self: &Rc<Self>, index: Option<usize>) {
        self.selected.set(index);
        self.refresh_panel();
        // Pelo teclado ou pela lista, a vista acompanha; com o mouse, não
        // rola embaixo do ponteiro.
        if self.drag.borrow().is_none() {
            self.reveal_selected();
        }
        self.canvas.queue_draw();
    }

    fn fill_size_rows(&self) {
        let Some(item) = self.selected.get().and_then(|i| self.items.borrow().get(i).cloned()) else { return };
        let was = self.filling.replace(true);
        self.props.width.set_value((item.w / MM).round() as f64);
        self.props.height.set_value((item.h / MM).round() as f64);
        self.filling.set(was);
    }

    /// Mostra no painel os campos da anotação selecionada.
    fn refresh_panel(self: &Rc<Self>) {
        self.refresh_list();
        self.update_dirty();
        self.undo_button.set_sensitive(!self.undo.borrow().is_empty());
        self.save.set_sensitive(!self.items.borrow().is_empty() && !self.saving.get());
        let p = &self.props;
        let Some(item) = self.selected.get().and_then(|i| self.items.borrow().get(i).cloned()) else {
            p.group.set_visible(false);
            return;
        };
        self.filling.set(true);
        p.group.set_visible(true);
        let is_text = matches!(item.kind, Kind::Text { .. });
        let is_image = matches!(item.kind, Kind::Image { .. });
        p.text_box.set_visible(is_text);
        p.font_size.set_visible(is_text);
        p.bold.set_visible(is_text);
        p.color.set_visible(!is_image);
        p.width.set_visible(!is_text);
        p.height.set_visible(!is_text && !is_image);
        p.line.set_visible(matches!(item.kind, Kind::Frame { .. }));
        p.remove_white.set_visible(is_image);
        p.group.set_title(match item.kind {
            Kind::Text { .. } => "Texto selecionado",
            Kind::Image { .. } => "Imagem selecionada",
            Kind::Highlight { .. } => "Destaque selecionado",
            Kind::Frame { .. } => "Moldura selecionada",
        });
        match &item.kind {
            Kind::Text { text, size, bold, color } => {
                let buf = p.text.buffer();
                if buf.text(&buf.start_iter(), &buf.end_iter(), false) != text.as_str() {
                    buf.set_text(text);
                }
                p.font_size.set_value(*size as f64);
                p.bold.set_active(*bold);
                p.color.set_selected(color_index(*color));
            }
            Kind::Image { remove_white, .. } => p.remove_white.set_active(*remove_white),
            Kind::Highlight { color } => p.color.set_selected(color_index(*color)),
            Kind::Frame { color, line_width } => {
                p.color.set_selected(color_index(*color));
                p.line.set_value(*line_width as f64);
            }
        }
        p.width.set_value((item.w / MM).round() as f64);
        p.height.set_value((item.h / MM).round() as f64);
        p.all_pages.set_active(item.all_pages);
        self.filling.set(false);
        self.update_warning();
    }

    fn update_warning(&self) {
        let p = &self.props;
        let text = self.selected.get().and_then(|i| match &self.items.borrow().get(i)?.kind {
            Kind::Text { text, .. } => Some(text.clone()),
            _ => None,
        });
        let msg = text.and_then(|t| {
            let (_, missing) = pdftoolbox::stamp::encode(&t);
            if t.trim().is_empty() {
                Some("Digite o texto da anotação.".to_string())
            } else if !missing.is_empty() {
                Some(operations::unsupported_chars(&missing))
            } else {
                None
            }
        });
        p.warning.set_visible(msg.is_some());
        p.warning.set_text(msg.as_deref().unwrap_or(""));
    }

    fn refresh_list(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let items = self.items.borrow().clone();
        self.list_group.set_visible(!items.is_empty());
        self.list_group.set_title(&format!("Anotações ({})", items.len()));
        for (i, item) in items.iter().enumerate() {
            let row =
                adw::ActionRow::builder().title(glib::markup_escape_text(&describe(item))).activatable(true).build();
            if Some(i) == self.selected.get() {
                row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            let a = Rc::downgrade(self);
            row.connect_activated(move |_| {
                let Some(a) = a.upgrade() else { return };
                let page =
                    a.items.borrow().get(i).map(|it| if it.on_page(a.page.get()) { a.page.get() } else { it.page });
                if let Some(page) = page {
                    a.go_to(page);
                    a.select(Some(i));
                    a.canvas.grab_focus();
                }
            });
            self.list.append(&row);
        }
    }

    fn save_as(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let a = self.clone();
        glib::spawn_future_local(async move {
            let initial = a.dest_dir.borrow().clone();
            if let Some(dir) = widgets::pick_folder(&win, initial.as_deref()).await {
                a.save_to(Some(dir));
            }
        });
    }

    fn save_to(self: &Rc<Self>, dir: Option<PathBuf>) {
        if self.saving.get() {
            return;
        }
        let (Some(app), Some(input)) = (self.app.upgrade(), self.input.borrow().clone()) else { return };
        let Some(dir) = dir.or_else(|| self.dest_dir.borrow().clone()) else { return };
        let items = self.items.borrow().clone();
        if let Err(e) = annotate::validate(&items, self.total()) {
            self.toast(&e.to_string());
            return;
        }
        let dest = Destination { dir, name: None };
        let request = Request::Annotate { input: input.clone(), items };
        let handle = jobs::spawn(move |ctx| operations::execute(request, &dest, ctx));
        let task = app.tasks.add(&format!("Anotar — {}", display_name(&input)), handle.cancel.clone());
        self.saving.set(true);
        self.save.set_sensitive(false);
        self.save.set_label("Salvando…");
        let a = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(ev) = handle.events.recv().await {
                match ev {
                    JobEvent::Queued => {}
                    JobEvent::Progress(f, t) => task.progress(f, &t),
                    JobEvent::Finished(result) => {
                        a.saving.set(false);
                        a.save.set_label("Salvar");
                        a.refresh_panel();
                        a.finished(result, &task);
                        break;
                    }
                }
            }
        });
    }

    fn finished(&self, result: OpResult<operations::Outcome>, task: &super::tasks::TaskRow) {
        let Some(app) = self.app.upgrade() else { return };
        match result {
            Ok(outcome) if !outcome.outputs.is_empty() => {
                let path = outcome.outputs[0].path.clone();
                task.finish(&format!("Concluído: {}", display_name(&path)), Some(path.clone()));
                *self.saved.borrow_mut() = self.items.borrow().clone();
                self.update_dirty();
                let toast = adw::Toast::builder()
                    .title(glib::markup_escape_text(&format!("Salvo como “{}”", display_name(&path))))
                    .button_label("Abrir")
                    .timeout(6)
                    .build();
                let win = app.window.clone();
                toast.connect_button_clicked(move |_| widgets::open_file(&win, &path));
                app.toasts.add_toast(toast);
            }
            Ok(_) => task.finish("Nenhum arquivo criado", None),
            Err(e) => {
                task.finish(e.title(), None);
                let dialog = adw::AlertDialog::new(Some(e.title()), Some(&e.to_string()));
                dialog.add_response("ok", "Fechar");
                dialog.present(Some(&app.window));
            }
        }
    }
}

/// Aumenta (ou diminui) a caixa em `dx` × `dy` pontos. Texto muda o tamanho
/// da letra; imagem mantém a proporção.
fn resize(item: &mut Annotation, dx: f32, dy: f32) {
    let (w, h) = item.size();
    match &mut item.kind {
        Kind::Text { size, .. } => {
            if w > 0.0 {
                *size = (*size * (w + dx) / w).clamp(4.0, 144.0).round();
            }
        }
        Kind::Image { .. } => {
            let new_w = (w + dx).max(10.0);
            item.h = h * new_w / w.max(0.01);
            item.w = new_w;
        }
        Kind::Highlight { .. } | Kind::Frame { .. } => {
            item.w = (w + dx).max(4.0);
            item.h = (h + dy).max(4.0);
        }
    }
}

/// Mantém a caixa dentro da página (quando ela cabe).
fn keep_inside(item: &mut Annotation, (pw, ph): (f32, f32)) {
    let (w, h) = item.size();
    item.x = item.x.clamp(0.0, (1.0 - w / pw).max(0.0));
    item.y = item.y.clamp(0.0, (1.0 - h / ph).max(0.0));
}
