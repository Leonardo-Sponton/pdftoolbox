//! Página ampliada: abre uma página da grade em tamanho grande para conferir
//! detalhes antes de decidir (remover, extrair, girar…). ← e → passam pelas
//! páginas na ordem da grade; Esc ou espaço fecham.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use pdftoolbox::error::display_name;

use super::page_thumb::PageThumb;
use super::thumbs;

/// Lado maior da página renderizada, em pixels.
const RENDER: u32 = 1600;

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

    // Altura útil: descontando a barra de título e as margens.
    let thumb = PageThumb::new(width - 32, height - 47 - 32);
    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let overlay = gtk::Overlay::builder().child(&thumb).vexpand(true).hexpand(true).build();
    overlay.add_overlay(&spinner);
    overlay.set_margin_top(16);
    overlay.set_margin_bottom(16);
    overlay.set_margin_start(16);
    overlay.set_margin_end(16);
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
        spinner,
        title,
        prev: prev.clone(),
        next: next.clone(),
    });
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
                z.rendered.borrow_mut().insert(page, t.clone());
            }
            // O usuário pode já ter passado para outra página.
            let current = &z.items[z.index.get()];
            if current.texture.is_none() && current.page == page {
                z.spinner.set_visible(false);
                if texture.is_some() {
                    z.thumb.set_texture(texture.as_ref());
                }
            }
        });
    }
}
