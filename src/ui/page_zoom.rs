//! Página ampliada: abre uma página da grade em tamanho grande para conferir
//! detalhes antes de decidir (remover, extrair, girar…). ← e → passam pelas
//! páginas na ordem da grade; Esc ou espaço fecham. A página começa ajustada
//! à janela e pode ser aproximada até `MAX_ZOOM` vezes (+, −, 0, Ctrl+roda,
//! pinça); aproximada, arrastar move a vista. A página é renderizada de novo,
//! com mais pixels, quando a ampliação passa da resolução que já existe.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use pdftoolbox::human;

use super::page_thumb::PageThumb;
use super::thumbs;

/// Lado maior da página renderizada, em pixels (ajustada à janela).
const RENDER: u32 = 1600;
/// Maior lado renderizado ao aproximar (limita a memória: ~50 MB por página).
const MAX_RENDER: u32 = 5000;
/// Maior ampliação, em relação à página ajustada à janela.
pub const MAX_ZOOM: f64 = 6.0;
/// Passo das teclas e botões de zoom.
const STEP: f64 = 1.25;

/// Mantém parado um ponto da vista enquanto o conteúdo de uma
/// `ScrolledWindow` muda de tamanho (zoom): depois do layout, as barras de
/// rolagem são acertadas para o ponto continuar sob o ponteiro.
#[derive(Clone)]
pub struct ZoomAnchor {
    point: Rc<Cell<Option<AnchorPoint>>>,
}

/// Ponto ancorado, (x, y) em cada campo.
#[derive(Clone, Copy)]
struct AnchorPoint {
    /// Fração do conteúdo sob o ponto.
    fraction: (f64, f64),
    /// Posição do ponto na vista, em pixels.
    offset: (f64, f64),
}

impl ZoomAnchor {
    pub fn attach(view: &gtk::ScrolledWindow) -> ZoomAnchor {
        let anchor = ZoomAnchor { point: Rc::default() };
        for horizontal in [true, false] {
            let adj = if horizontal { view.hadjustment() } else { view.vadjustment() };
            let point = anchor.point.clone();
            adj.connect_upper_notify(move |adj| {
                if let Some(AnchorPoint { fraction: (fx, fy), offset: (px, py) }) = point.get() {
                    let (f, p) = if horizontal { (fx, px) } else { (fy, py) };
                    adj.set_value(f * adj.upper() - p);
                }
            });
        }
        anchor
    }

    /// Fixa o ponto `at` da vista (padrão: o centro) até o próximo layout.
    pub fn hold(&self, view: &gtk::ScrolledWindow, at: Option<(f64, f64)>) {
        let (h, v) = (view.hadjustment(), view.vadjustment());
        let (px, py) = at.unwrap_or((h.page_size() / 2.0, v.page_size() / 2.0));
        let frac =
            |adj: &gtk::Adjustment, p: f64| if adj.upper() > 0.0 { (adj.value() + p) / adj.upper() } else { 0.5 };
        self.point.set(Some(AnchorPoint { fraction: (frac(&h, px), frac(&v, py)), offset: (px, py) }));
        let point = self.point.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || point.set(None));
    }
}

/// Ctrl+roda e pinça em `view` mudam a escala em volta do ponteiro (ou do
/// centro da pinça). `current` dá a escala atual (`None`: nada a ampliar);
/// `zoom_to` aplica a nova escala mantendo parado o ponto dado.
pub fn connect_zoom_input(
    view: &gtk::ScrolledWindow,
    current: impl Fn() -> Option<f64> + 'static,
    zoom_to: impl Fn(f64, Option<(f64, f64)>) + 'static,
) {
    let (current, zoom_to) = (Rc::new(current), Rc::new(zoom_to));
    // Posição do ponteiro na vista, para aproximar em volta dela.
    let pointer = Rc::new(Cell::new(None::<(f64, f64)>));
    let motion = gtk::EventControllerMotion::new();
    let p = pointer.clone();
    motion.connect_motion(move |_, x, y| p.set(Some((x, y))));
    let p = pointer.clone();
    motion.connect_leave(move |_| p.set(None));
    view.add_controller(motion);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    // Antes da rolagem da própria ScrolledWindow.
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (c, z) = (current.clone(), zoom_to.clone());
    scroll.connect_scroll(move |controller, _, dy| {
        if !controller.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
            return glib::Propagation::Proceed;
        }
        if let Some(scale) = c() {
            z(scale * if dy < 0.0 { 1.1 } else { 1.0 / 1.1 }, pointer.get());
        }
        glib::Propagation::Stop
    });
    view.add_controller(scroll);

    let pinch = gtk::GestureZoom::new();
    let start = Rc::new(Cell::new(None::<f64>));
    let s = start.clone();
    pinch.connect_begin(move |_, _| s.set(current()));
    pinch.connect_scale_changed(move |g, scale| {
        if let Some(begin) = start.get() {
            zoom_to(begin * scale, g.bounding_box_center());
        }
    });
    view.add_controller(pinch);
}

/// Botões “Afastar”, escala atual (volta a ajustar) e “Aproximar”.
pub struct ZoomControls {
    pub widget: gtk::Box,
    pub zoom_out: gtk::Button,
    pub fit: gtk::Button,
    pub zoom_in: gtk::Button,
}

impl ZoomControls {
    pub fn new(out_tooltip: &str, fit_tooltip: &str, in_tooltip: &str) -> ZoomControls {
        let zoom_out = super::widgets::icon_button("zoom-out-symbolic", out_tooltip);
        let zoom_in = super::widgets::icon_button("zoom-in-symbolic", in_tooltip);
        let fit = gtk::Button::builder().label(zoom_label(1.0)).tooltip_text(fit_tooltip).build();
        fit.add_css_class("flat");
        fit.add_css_class("numeric");
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.add_css_class("linked");
        widget.append(&zoom_out);
        widget.append(&fit);
        widget.append(&zoom_in);
        ZoomControls { widget, zoom_out, fit, zoom_in }
    }

    /// Mostra a ampliação `relative` (1 = ajustada) e o que ainda é possível.
    pub fn show(&self, relative: f64, can_zoom_out: bool, can_zoom_in: bool) {
        self.fit.set_label(&zoom_label(relative));
        self.zoom_out.set_sensitive(can_zoom_out);
        self.zoom_in.set_sensitive(can_zoom_in);
    }
}

/// “Ajustada”, “1,5×”, “2×”.
pub fn zoom_label(z: f64) -> String {
    if z <= 1.0 + 1e-6 {
        "Ajustada".into()
    } else if (z - z.round()).abs() < 0.05 {
        format!("{}×", z.round())
    } else {
        format!("{}×", human::decimal(z, 1))
    }
}

/// Uma página da sequência mostrada.
pub struct ZoomItem {
    /// Página do PDF; ignorada quando `texture` é dada.
    pub page: u32,
    /// Rotação adicional (a planejada na ferramenta), em graus.
    pub rotation: i32,
    /// Título, como “Página 3 de 12”.
    pub label: String,
    /// Imagem pronta (ex.: página em branco), sem renderizar o PDF.
    pub texture: Option<gdk::Texture>,
}

struct Zoom {
    path: PathBuf,
    items: Vec<ZoomItem>,
    index: Cell<usize>,
    rendered: RefCell<HashMap<u32, gdk::Texture>>,
    thumb: PageThumb,
    /// Tamanho da página ajustada à janela.
    fit: (i32, i32),
    zoom: Cell<f64>,
    /// Ponto a manter sob o ponteiro depois do zoom.
    anchor: ZoomAnchor,
    scroller: gtk::ScrolledWindow,
    controls: ZoomControls,
    /// Renderização maior em andamento: (página, lado).
    pending: Cell<Option<(u32, u32)>>,
    spinner: gtk::Spinner,
    title: adw::WindowTitle,
    prev: gtk::Button,
    next: gtk::Button,
}

pub fn show(parent: &impl IsA<gtk::Widget>, path: PathBuf, items: Vec<ZoomItem>, start: usize) {
    if items.is_empty() {
        return;
    }
    let (win_w, win_h) =
        parent.as_ref().root().map(|r| (r.width(), r.height())).filter(|(w, h)| *w > 0 && *h > 0).unwrap_or((980, 720));
    let width = ((win_w as f64 * 0.92) as i32).min(1100);
    let height = (win_h as f64 * 0.94) as i32;

    let title = adw::WindowTitle::new("", &human::file_name(&path));
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    let prev = super::widgets::icon_button("go-previous-symbolic", "Página anterior (←)");
    let next = super::widgets::icon_button("go-next-symbolic", "Próxima página (→)");
    header.pack_start(&prev);
    header.pack_start(&next);
    let controls = ZoomControls::new("Afastar (−)", "Ajustar à janela (0)", "Aproximar (+)");
    header.pack_end(&controls.widget);

    // Altura útil: descontando a barra de título e as margens.
    let fit = (width - 32, height - 47 - 32);
    let thumb = PageThumb::new(fit.0, fit.1);
    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    thumb.set_margin_top(16);
    thumb.set_margin_bottom(16);
    thumb.set_margin_start(16);
    thumb.set_margin_end(16);
    let scroller = gtk::ScrolledWindow::builder().child(&thumb).vexpand(true).hexpand(true).build();
    let overlay = gtk::Overlay::builder().child(&scroller).vexpand(true).hexpand(true).build();
    overlay.add_overlay(&spinner);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&overlay));
    let dialog = adw::Dialog::builder().content_width(width).content_height(height).child(&toolbar).build();

    let zoom = Rc::new(Zoom {
        path,
        items,
        index: Cell::new(start),
        rendered: RefCell::new(HashMap::new()),
        thumb,
        fit,
        zoom: Cell::new(1.0),
        anchor: ZoomAnchor::attach(&scroller),
        scroller: scroller.clone(),
        controls,
        pending: Cell::new(None),
        spinner,
        title,
        prev: prev.clone(),
        next: next.clone(),
    });
    for (button, factor) in [(&zoom.controls.zoom_out, 1.0 / STEP), (&zoom.controls.zoom_in, STEP)] {
        button.connect_clicked(glib::clone!(
            #[weak]
            zoom,
            move |_| zoom.zoom_by(factor, None)
        ));
    }
    zoom.controls.fit.connect_clicked(glib::clone!(
        #[weak]
        zoom,
        move |_| zoom.set_zoom(1.0, None)
    ));
    zoom.connect_view_gestures();
    for (button, delta) in [(&prev, -1isize), (&next, 1)] {
        button.connect_clicked(glib::clone!(
            #[weak]
            zoom,
            move |_| zoom.step(delta)
        ));
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[weak(rename_to = z)]
        zoom,
        #[weak]
        dialog,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            match key {
                gdk::Key::Left | gdk::Key::Page_Up | gdk::Key::Up => z.step(-1),
                gdk::Key::Right | gdk::Key::Page_Down | gdk::Key::Down => z.step(1),
                gdk::Key::Home => z.go(0),
                gdk::Key::End => z.go(z.items.len() - 1),
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => z.zoom_by(STEP, None),
                gdk::Key::minus | gdk::Key::KP_Subtract => z.zoom_by(1.0 / STEP, None),
                gdk::Key::_0 | gdk::Key::KP_0 => z.set_zoom(1.0, None),
                gdk::Key::space => {
                    dialog.close();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    ));
    dialog.add_controller(keys);
    zoom.go(start);
    dialog.present(Some(parent));
    // O estado vive enquanto o diálogo existir.
    dialog.connect_closed(move |_| {
        let _ = &zoom;
    });
}

impl Zoom {
    /// Ctrl+roda e pinça aproximam em volta do ponteiro; arrastar move a vista.
    fn connect_view_gestures(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let current = move || weak.upgrade().map(|z| z.zoom.get());
        let weak = Rc::downgrade(self);
        let zoom_to = move |scale, at| {
            if let Some(z) = weak.upgrade() {
                z.set_zoom(scale, at);
            }
        };
        connect_zoom_input(&self.scroller, current, zoom_to);

        let pan = gtk::GestureDrag::new();
        let origin = Rc::new(Cell::new((0.0, 0.0)));
        let o = origin.clone();
        pan.connect_drag_begin(glib::clone!(
            #[weak(rename_to = z)]
            self,
            move |_, _, _| {
                o.set((z.scroller.hadjustment().value(), z.scroller.vadjustment().value()));
                if z.zoom.get() > 1.0 {
                    z.scroller.set_cursor_from_name(Some("grabbing"));
                }
            }
        ));
        pan.connect_drag_update(glib::clone!(
            #[weak(rename_to = z)]
            self,
            move |_, dx, dy| {
                let (x, y) = origin.get();
                z.scroller.hadjustment().set_value(x - dx);
                z.scroller.vadjustment().set_value(y - dy);
            }
        ));
        pan.connect_drag_end(glib::clone!(
            #[weak(rename_to = z)]
            self,
            move |_, _, _| z.update_cursor()
        ));
        self.scroller.add_controller(pan);
    }

    fn update_cursor(&self) {
        self.scroller.set_cursor_from_name(Some(if self.zoom.get() > 1.0 { "grab" } else { "default" }));
    }

    fn zoom_by(self: &Rc<Self>, factor: f64, at: Option<(f64, f64)>) {
        self.set_zoom(self.zoom.get() * factor, at);
    }

    /// Muda a ampliação mantendo parado o ponto `at` da vista (padrão: o centro).
    fn set_zoom(self: &Rc<Self>, z: f64, at: Option<(f64, f64)>) {
        let z = z.clamp(1.0, MAX_ZOOM);
        let old = self.zoom.replace(z);
        if (old - z).abs() > 1e-9 {
            self.anchor.hold(&self.scroller, at);
        }
        self.thumb.set_max((f64::from(self.fit.0) * z) as i32, (f64::from(self.fit.1) * z) as i32);
        self.controls.show(z, z > 1.0, z < MAX_ZOOM);
        self.update_cursor();
        self.sharpen_later();
    }

    /// Lado em pixels que a página precisa ter para não ficar borrada.
    fn wanted_side(&self) -> u32 {
        let scale = self.scroller.scale_factor().max(1) as f64;
        let side = self.fit.0.max(self.fit.1) as f64 * self.zoom.get() * scale;
        (side.ceil() as u32).clamp(RENDER, MAX_RENDER)
    }

    /// Renderiza de novo, maior, quando a ampliação parar de mudar.
    fn sharpen_later(self: &Rc<Self>) {
        let z = Rc::downgrade(self);
        let zoom = self.zoom.get();
        glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            if let Some(z) = z.upgrade()
                && z.zoom.get() == zoom
            {
                z.sharpen();
            }
        });
    }

    fn sharpen(self: &Rc<Self>) {
        let item = &self.items[self.index.get()];
        if item.texture.is_some() {
            return;
        }
        let (page, side) = (item.page, self.wanted_side());
        let have = self.rendered.borrow().get(&page).map_or(0, |t| t.width().max(t.height()) as u32);
        if have as f64 >= side as f64 * 0.9 || self.pending.get().is_some_and(|(p, s)| p == page && s >= side) {
            return;
        }
        self.pending.set(Some((page, side)));
        let (z, path) = (Rc::downgrade(self), self.path.clone());
        glib::spawn_future_local(async move {
            let texture = thumbs::large_page(&path, page, side).await;
            let Some(z) = z.upgrade() else { return };
            z.pending.set(None);
            let Some(t) = texture else { return };
            {
                // Só uma página guardada em alta resolução por vez.
                let mut rendered = z.rendered.borrow_mut();
                rendered.retain(|_, t| t.width().max(t.height()) as u32 <= RENDER);
                rendered.insert(page, t.clone());
            }
            let current = &z.items[z.index.get()];
            if current.texture.is_none() && current.page == page {
                z.thumb.set_texture(Some(&t));
            }
        });
    }

    fn step(self: &Rc<Self>, delta: isize) {
        let i = self.index.get() as isize + delta;
        if i >= 0 && (i as usize) < self.items.len() {
            self.go(i as usize);
        }
    }

    fn go(self: &Rc<Self>, i: usize) {
        self.index.set(i);
        let item = &self.items[i];
        self.title.set_title(&item.label);
        self.prev.set_sensitive(i > 0);
        self.next.set_sensitive(i + 1 < self.items.len());
        self.thumb.set_rotation(item.rotation);
        self.thumb.update_property(&[gtk::accessible::Property::Label(&item.label)]);
        let ready = item.texture.clone().or_else(|| self.rendered.borrow().get(&item.page).cloned());
        if let Some(t) = ready {
            self.spinner.set_visible(false);
            self.thumb.set_texture(Some(&t));
            self.sharpen();
            return;
        }
        // Enquanto renderiza, mostra a miniatura (se houver) já no formato certo.
        self.thumb.set_texture(thumbs::cached(&self.path, item.page).as_ref());
        self.spinner.set_visible(true);
        let (z, page, path) = (Rc::downgrade(self), item.page, self.path.clone());
        glib::spawn_future_local(async move {
            let texture = thumbs::large_page(&path, page, RENDER).await;
            let Some(z) = z.upgrade() else { return };
            if let Some(t) = &texture {
                z.rendered.borrow_mut().entry(page).or_insert_with(|| t.clone());
            }
            // O usuário pode já ter passado para outra página.
            let current = &z.items[z.index.get()];
            if current.texture.is_none() && current.page == page {
                z.spinner.set_visible(false);
                if texture.is_some() {
                    z.thumb.set_texture(texture.as_ref());
                    z.sharpen();
                }
            }
        });
    }
}
