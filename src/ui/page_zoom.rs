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

use pdftoolbox::error::display_name;

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

/// Ponto a manter parado durante o zoom: fração do conteúdo e posição na
/// vista, (x, y) em cada um.
pub type ZoomAnchor = ((f64, f64), (f64, f64));

/// “Ajustada”, “1,5×”, “2×”.
pub fn zoom_label(z: f64) -> String {
    if z <= 1.0 + 1e-6 {
        "Ajustada".into()
    } else if (z - z.round()).abs() < 0.05 {
        format!("{}×", z.round())
    } else {
        format!("{:.1}×", z).replace('.', ",")
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
    /// Ponto a manter sob o ponteiro depois do zoom: fração do conteúdo e
    /// posição na vista, por eixo.
    anchor: Cell<Option<ZoomAnchor>>,
    scroller: gtk::ScrolledWindow,
    zoom_label: gtk::Button,
    zoom_out: gtk::Button,
    zoom_in: gtk::Button,
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

    let title = adw::WindowTitle::new("", &display_name(&path));
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    let prev = super::widgets::icon_button("go-previous-symbolic", "Página anterior (←)");
    let next = super::widgets::icon_button("go-next-symbolic", "Próxima página (→)");
    header.pack_start(&prev);
    header.pack_start(&next);
    let zoom_out = super::widgets::icon_button("zoom-out-symbolic", "Afastar (−)");
    let zoom_in = super::widgets::icon_button("zoom-in-symbolic", "Aproximar (+)");
    let zoom_label = gtk::Button::builder().label(zoom_label(1.0)).tooltip_text("Ajustar à janela (0)").build();
    zoom_label.add_css_class("flat");
    zoom_label.add_css_class("numeric");
    let zoom_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    zoom_box.add_css_class("linked");
    zoom_box.append(&zoom_out);
    zoom_box.append(&zoom_label);
    zoom_box.append(&zoom_in);
    header.pack_end(&zoom_box);

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
        anchor: Cell::new(None),
        scroller: scroller.clone(),
        zoom_label: zoom_label.clone(),
        zoom_out: zoom_out.clone(),
        zoom_in: zoom_in.clone(),
        pending: Cell::new(None),
        spinner,
        title,
        prev: prev.clone(),
        next: next.clone(),
    });
    for (button, factor) in [(&zoom_out, 1.0 / STEP), (&zoom_in, STEP)] {
        let z = Rc::downgrade(&zoom);
        button.connect_clicked(move |_| {
            if let Some(z) = z.upgrade() {
                z.zoom_by(factor, None);
            }
        });
    }
    let z = Rc::downgrade(&zoom);
    zoom_label.connect_clicked(move |_| {
        if let Some(z) = z.upgrade() {
            z.set_zoom(1.0, None);
        }
    });
    zoom.connect_view_gestures();
    for (button, delta) in [(&prev, -1isize), (&next, 1)] {
        let z = Rc::downgrade(&zoom);
        button.connect_clicked(move |_| {
            if let Some(z) = z.upgrade() {
                z.step(delta);
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (z, d) = (Rc::downgrade(&zoom), dialog.downgrade());
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(z) = z.upgrade() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Left | gdk::Key::Page_Up | gdk::Key::Up => z.step(-1),
            gdk::Key::Right | gdk::Key::Page_Down | gdk::Key::Down => z.step(1),
            gdk::Key::Home => z.go(0),
            gdk::Key::End => z.go(z.items.len() - 1),
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => z.zoom_by(STEP, None),
            gdk::Key::minus | gdk::Key::KP_Subtract => z.zoom_by(1.0 / STEP, None),
            gdk::Key::_0 | gdk::Key::KP_0 => z.set_zoom(1.0, None),
            gdk::Key::space => {
                if let Some(d) = d.upgrade() {
                    d.close();
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
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
        // Posição do ponteiro na vista, para aproximar em volta dela.
        let pointer = Rc::new(Cell::new(None::<(f64, f64)>));
        let motion = gtk::EventControllerMotion::new();
        let p = pointer.clone();
        motion.connect_motion(move |_, x, y| p.set(Some((x, y))));
        let p = pointer.clone();
        motion.connect_leave(move |_| p.set(None));
        self.scroller.add_controller(motion);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        // Antes da rolagem da própria ScrolledWindow.
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let z = Rc::downgrade(self);
        scroll.connect_scroll(move |c, _, dy| {
            let Some(z) = z.upgrade() else { return glib::Propagation::Proceed };
            if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            z.zoom_by(if dy < 0.0 { 1.1 } else { 1.0 / 1.1 }, pointer.get());
            glib::Propagation::Stop
        });
        self.scroller.add_controller(scroll);

        let pinch = gtk::GestureZoom::new();
        let start = Rc::new(Cell::new(1.0));
        let (z, s) = (Rc::downgrade(self), start.clone());
        pinch.connect_begin(move |_, _| {
            if let Some(z) = z.upgrade() {
                s.set(z.zoom.get());
            }
        });
        let z = Rc::downgrade(self);
        pinch.connect_scale_changed(move |g, scale| {
            if let Some(z) = z.upgrade() {
                z.set_zoom(start.get() * scale, g.bounding_box_center());
            }
        });
        self.scroller.add_controller(pinch);

        let pan = gtk::GestureDrag::new();
        let origin = Rc::new(Cell::new((0.0, 0.0)));
        let (z, o) = (Rc::downgrade(self), origin.clone());
        pan.connect_drag_begin(move |_, _, _| {
            if let Some(z) = z.upgrade() {
                o.set((z.scroller.hadjustment().value(), z.scroller.vadjustment().value()));
                if z.zoom.get() > 1.0 {
                    z.scroller.set_cursor_from_name(Some("grabbing"));
                }
            }
        });
        let z = Rc::downgrade(self);
        pan.connect_drag_update(move |_, dx, dy| {
            if let Some(z) = z.upgrade() {
                let (x, y) = origin.get();
                z.scroller.hadjustment().set_value(x - dx);
                z.scroller.vadjustment().set_value(y - dy);
            }
        });
        let z = Rc::downgrade(self);
        pan.connect_drag_end(move |_, _, _| {
            if let Some(z) = z.upgrade() {
                z.update_cursor();
            }
        });
        self.scroller.add_controller(pan);

        // Depois do zoom, o tamanho do conteúdo muda: mantém o ponto ancorado.
        for horizontal in [true, false] {
            let adj = if horizontal { self.scroller.hadjustment() } else { self.scroller.vadjustment() };
            let z = Rc::downgrade(self);
            adj.connect_upper_notify(move |adj| {
                let Some(z) = z.upgrade() else { return };
                if let Some(((fx, fy), (px, py))) = z.anchor.get() {
                    let (f, p) = if horizontal { (fx, px) } else { (fy, py) };
                    adj.set_value(f * adj.upper() - p);
                }
            });
        }
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
        let (h, v) = (self.scroller.hadjustment(), self.scroller.vadjustment());
        let (px, py) = at.unwrap_or((h.page_size() / 2.0, v.page_size() / 2.0));
        let frac =
            |adj: &gtk::Adjustment, p: f64| if adj.upper() > 0.0 { (adj.value() + p) / adj.upper() } else { 0.5 };
        if (old - z).abs() > 1e-9 {
            self.anchor.set(Some(((frac(&h, px), frac(&v, py)), (px, py))));
            // A âncora vale só para o próximo layout.
            let a = Rc::downgrade(self);
            glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                if let Some(a) = a.upgrade() {
                    a.anchor.set(None);
                }
            });
        }
        self.thumb.set_max((self.fit.0 as f64 * z) as i32, (self.fit.1 as f64 * z) as i32);
        self.zoom_label.set_label(&zoom_label(z));
        self.zoom_out.set_sensitive(z > 1.0);
        self.zoom_in.set_sensitive(z < MAX_ZOOM);
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
