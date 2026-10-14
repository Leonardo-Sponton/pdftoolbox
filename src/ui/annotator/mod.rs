//! Anotar e assinar: a página aparece grande; textos, assinaturas (imagens),
//! destaques e molduras são posicionados arrastando com o mouse ou pelo
//! teclado. A prévia é desenhada na hora com as mesmas medidas da operação
//! (fonte Helvetica e caixas de `pdftoolbox::annotate`); o resultado é gravado
//! pela camada sobreposta do qpdf (`Request::Annotate`), sem rasterizar.
//!
//! A página começa ajustada à área de desenho e pode ser aproximada
//! (Ctrl+roda, pinça, Ctrl+mais/menos, Ctrl+0 volta a ajustar); aproximada,
//! arrastar fora das anotações move a vista.
//!
//! - `canvas`: desenho, mouse, teclado e zoom;
//! - `panel`: painel lateral (adicionar, propriedades, lista).

mod canvas;
mod panel;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{cairo, gio, glib};

use pdftoolbox::catalog::{self, Accepts, ToolId};
use pdftoolbox::engines::{poppler, qpdf};
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::{Annotation, AnnotationKind as Kind, Request};
use pdftoolbox::pdfwriter::{self, MM};
use pdftoolbox::{OpError, OpResult, annotate, human};

use super::editor::{self, EditorChrome, EmptyState, History};
use super::page_zoom::{ZoomAnchor, ZoomControls};
use super::{AppState, unlock, widgets};
use canvas::{Drag, ZOOM_STEP, keep_inside};
use panel::{BLACK, COLORS, Props, RED, YELLOW};

struct Annotator {
    app: Weak<AppState>,
    chrome: EditorChrome,
    input: RefCell<Option<PathBuf>>,
    /// Tamanho exibido de cada página, em pontos.
    sizes: RefCell<Vec<(f32, f32)>>,
    page: Cell<u32>,
    items: RefCell<Vec<Annotation>>,
    selected: Cell<Option<usize>>,
    history: History<Vec<Annotation>>,
    /// Páginas renderizadas.
    pages: RefCell<HashMap<u32, cairo::ImageSurface>>,
    /// Prévias das imagens inseridas (`None`: carregando ou ilegível).
    images: RefCell<HashMap<(PathBuf, bool), Option<cairo::ImageSurface>>>,
    /// Invalida renderizações que chegam depois de trocar de página.
    render_gen: Cell<u64>,
    drag: RefCell<Option<Drag>>,
    /// Preenchendo o painel por código: ignora os sinais de alteração.
    filling: Cell<bool>,
    canvas: gtk::DrawingArea,
    /// Rolagem em volta da página (quando aproximada).
    view: gtk::ScrolledWindow,
    /// Pixels por ponto escolhidos pelo zoom; `None` = ajustada à área.
    zoom: Cell<Option<f64>>,
    zoom_controls: ZoomControls,
    /// Ponto a manter sob o ponteiro depois do zoom.
    anchor: ZoomAnchor,
    /// Arrastando a vista: posição das barras de rolagem no início.
    pan: Cell<Option<(f64, f64)>>,
    page_label: gtk::Label,
    prev: gtk::Button,
    next: gtk::Button,
    undo_button: gtk::Button,
    props: Props,
    list: gtk::ListBox,
    list_group: adw::PreferencesGroup,
    dest_dir: RefCell<Option<PathBuf>>,
    saving: Cell<bool>,
    /// Anotações como estavam no último salvamento (para avisar ao sair).
    saved: RefCell<Vec<Annotation>>,
}

/// Barra inferior: zoom, páginas e desfazer.
struct ActionBar {
    bar: gtk::ActionBar,
    zoom: ZoomControls,
    page_label: gtk::Label,
    prev: gtk::Button,
    next: gtk::Button,
    undo: gtk::Button,
}

fn action_bar() -> ActionBar {
    let bar = gtk::ActionBar::new();
    let prev = widgets::icon_button("go-previous-symbolic", "Página anterior (Page Up)");
    let next = widgets::icon_button("go-next-symbolic", "Próxima página (Page Down)");
    let page_label = gtk::Label::new(None);
    page_label.add_css_class("numeric");
    let nav_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    nav_box.append(&prev);
    nav_box.append(&page_label);
    nav_box.append(&next);
    bar.set_center_widget(Some(&nav_box));
    let undo = widgets::icon_button("edit-undo-symbolic", "Desfazer (Ctrl+Z)");
    bar.pack_end(&undo);
    let zoom = ZoomControls::new("Afastar (Ctrl+−)", "Ajustar à área (Ctrl+0)", "Aproximar (Ctrl++)");
    bar.pack_start(&zoom.widget);
    for (b, action) in [
        (&prev, "page.prev"),
        (&next, "page.next"),
        (&undo, "page.undo"),
        (&zoom.zoom_in, "page.zoom-in"),
        (&zoom.zoom_out, "page.zoom-out"),
        (&zoom.fit, "page.zoom-fit"),
    ] {
        b.set_action_name(Some(action));
    }
    ActionBar { bar, zoom, page_label, prev, next, undo }
}

pub fn build(app: &Rc<AppState>, files: Vec<PathBuf>) -> adw::NavigationPage {
    let info = catalog::info(ToolId::Annotate);
    let canvas = gtk::DrawingArea::builder().hexpand(true).vexpand(true).focusable(true).can_focus(true).build();
    canvas.update_property(&[gtk::accessible::Property::Label(
        "Página do documento. Setas movem a anotação selecionada, Shift+setas movem mais, mais e menos mudam o \
         tamanho, Delete remove, Tab passa para a próxima anotação, Page Up e Page Down trocam de página.",
    )]);
    // Viewport própria: sem `scroll-to-focus`, focar a página (que é maior
    // que a vista com zoom) não rola para o canto dela.
    let viewport = gtk::Viewport::builder().child(&canvas).scroll_to_focus(false).build();
    let view = gtk::ScrolledWindow::builder().child(&viewport).hexpand(true).vexpand(true).build();
    let panel = panel::build();
    let editor_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    editor_box.append(&view);
    editor_box.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    editor_box.append(&panel.widget);

    let bar = action_bar();
    let empty = EmptyState {
        icon: info.icon,
        title: "Escolha um PDF para anotar",
        description: "Ou arraste o arquivo para esta janela. Você poderá escrever textos, destacar trechos e \
                      colocar a imagem da sua assinatura.",
        open_action: "page.open",
    };
    let (chrome, nav_page) = EditorChrome::new(info, &empty, &editor_box, bar.bar);

    let ann = Rc::new(Annotator {
        app: Rc::downgrade(app),
        chrome,
        input: RefCell::new(None),
        sizes: RefCell::new(Vec::new()),
        page: Cell::new(1),
        items: RefCell::new(Vec::new()),
        selected: Cell::new(None),
        history: History::new(),
        pages: RefCell::new(HashMap::new()),
        images: RefCell::new(HashMap::new()),
        render_gen: Cell::new(0),
        drag: RefCell::new(None),
        filling: Cell::new(false),
        canvas,
        anchor: ZoomAnchor::attach(&view),
        view,
        zoom: Cell::new(None),
        zoom_controls: bar.zoom,
        pan: Cell::new(None),
        page_label: bar.page_label,
        prev: bar.prev,
        next: bar.next,
        undo_button: bar.undo,
        props: panel.props,
        list: panel.list,
        list_group: panel.list_group,
        dest_dir: RefCell::new(None),
        saving: Cell::new(false),
        saved: RefCell::new(Vec::new()),
    });
    ann.connect_canvas();
    ann.connect_panel();

    // Ações e atalhos
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: fn(&Rc<Annotator>)| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(glib::clone!(
            #[weak]
            ann,
            move |_, _| f(&ann)
        ));
        group.add_action(&action);
    };
    add("open", |a| a.confirm_discard(|a| a.pick()));
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
    add("zoom-in", |a| a.zoom_by(ZOOM_STEP));
    add("zoom-out", |a| a.zoom_by(1.0 / ZOOM_STEP));
    add("zoom-fit", |a| a.set_zoom(None, None));
    nav_page.insert_action_group("page", Some(&group));
    let shortcuts = widgets::shortcuts(&[
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
    ]);
    shortcuts.set_scope(gtk::ShortcutScope::Global);
    nav_page.add_controller(shortcuts);

    widgets::add_file_drop(
        &nav_page,
        glib::clone!(
            #[weak]
            ann,
            move |files| {
                if let Some(f) = files.iter().find(|f| catalog::extension_ok(Accepts::Pdf, f)).cloned() {
                    ann.confirm_discard(move |a| a.load(f.clone()));
                } else if ann.input.borrow().is_some()
                    && let Some(img) = files.into_iter().find(|f| catalog::extension_ok(Accepts::Images, f))
                {
                    ann.add_image(img);
                }
            }
        ),
    );
    // Ao sair da página, libera as páginas e imagens renderizadas.
    app.nav.connect_popped(glib::clone!(
        #[weak]
        ann,
        move |_, popped| {
            if ann.chrome.is_page(popped) {
                ann.render_gen.set(ann.render_gen.get() + 1);
                ann.pages.borrow_mut().clear();
                ann.images.borrow_mut().clear();
            }
        }
    ));

    if let Some(f) = files.into_iter().next() {
        ann.load(f);
    }
    widgets::keep_alive(&nav_page, ann);
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

    fn has_document(&self) -> bool {
        self.input.borrow().is_some()
    }

    fn selected_item(&self) -> Option<Annotation> {
        self.selected.get().and_then(|i| self.items.borrow().get(i).cloned())
    }

    fn pick(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let a = self.clone();
        glib::spawn_future_local(async move {
            if let Some(f) = widgets::pick_files(&win, Some(Accepts::Pdf), false).await.into_iter().next() {
                a.load(f);
            }
        });
    }

    fn load(self: &Rc<Self>, path: PathBuf) {
        self.chrome.show_loading();
        let a = self.clone();
        glib::spawn_future_local(async move {
            // PDF com senha: pede a senha e abre uma cópia desbloqueada.
            let Some(app) = a.app.upgrade() else { return };
            let Some(path) = unlock::prepare(&app, vec![path]).await.into_iter().next() else {
                a.chrome.restore(a.has_document());
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
                Ok(_) => a.chrome.show_error(&OpError::OutputInvalid("o documento não tem páginas".into())),
                Err(e) => a.chrome.show_error(&e),
            }
        });
    }

    fn start(self: &Rc<Self>, path: PathBuf, sizes: Vec<(f32, f32)>) {
        self.chrome.set_title(&format!("Anotar — {}", human::file_name(&path)));
        self.dest_dir.replace(unlock::original(&path).parent().map(Path::to_path_buf));
        self.sizes.replace(sizes);
        self.input.replace(Some(path));
        self.items.borrow_mut().clear();
        self.saved.borrow_mut().clear();
        self.history.clear();
        self.pages.borrow_mut().clear();
        self.selected.set(None);
        self.zoom.set(None);
        self.page.set(0);
        self.chrome.show_content();
        self.go_to(1);
        self.refresh_panel();
        self.canvas.grab_focus();
    }

    fn total(&self) -> u32 {
        self.sizes.borrow().len() as u32
    }

    /// Tamanho exibido da página atual, em pontos.
    fn page_size(&self) -> (f32, f32) {
        let index = self.page.get().saturating_sub(1) as usize;
        self.sizes.borrow().get(index).copied().unwrap_or(pdfwriter::A4_POINTS)
    }

    fn go_to(self: &Rc<Self>, page: u32) {
        let total = self.total();
        if page == 0 || page > total || page == self.page.get() {
            return;
        }
        self.page.set(page);
        if self.selected_item().is_some_and(|item| !item.on_page(page)) {
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

    fn remember(&self) {
        self.history.push(self.items.borrow().clone());
        self.undo_button.set_sensitive(true);
    }

    fn undo(self: &Rc<Self>) {
        let Some(previous) = self.history.pop() else { return };
        let len = previous.len();
        self.items.replace(previous);
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

    /// Altera a anotação selecionada sem atualizar o painel (arraste, teclado).
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
        self.chrome.set_dirty(self.dirty());
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
        let body = if n == 1 {
            "1 anotação ainda não foi salva. O PDF original não foi alterado.".to_string()
        } else {
            format!("{n} anotações ainda não foram salvas. O PDF original não foi alterado.")
        };
        let weak = Rc::downgrade(self);
        editor::confirm_discard(&app, "Descartar as anotações?", &body, "Continuar anotando", move || {
            if let Some(a) = weak.upgrade() {
                then(&a);
            }
        });
    }

    fn push_item(self: &Rc<Self>, item: Annotation) {
        self.remember();
        let is_text = matches!(item.kind, Kind::Text { .. });
        let index = {
            let mut items = self.items.borrow_mut();
            items.push(item);
            items.len() - 1
        };
        self.select(Some(index));
        self.canvas.grab_focus();
        if is_text {
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
        if !self.has_document() {
            return;
        }
        let kind = Kind::Text { text: text.into(), size: 14.0, bold: false, color: COLORS[BLACK].1 };
        let item = self.centered(kind, 0.0, 0.0);
        self.push_item(item);
    }

    fn add_shape(self: &Rc<Self>, highlight: bool) {
        if !self.has_document() {
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
        if !self.has_document() {
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
        // As marcas gravadas (podem mudar durante a gravação).
        let items = self.items.borrow().clone();
        if let Err(e) = annotate::validate(&items, self.total()) {
            self.toast(&e.to_string());
            return;
        }
        let title = format!("Anotar — {}", human::file_name(&input));
        let request = Request::Annotate { input, items: items.clone() };
        self.saving.set(true);
        self.chrome.set_saving(true);
        let a = self.clone();
        editor::save(&app, &title, request, dir, move |saved| {
            a.saving.set(false);
            a.chrome.set_saving(false);
            if saved {
                a.saved.replace(items);
            }
            a.refresh_panel();
        });
    }
}
