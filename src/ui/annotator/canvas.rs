//! Área de desenho do Anotar: a página renderizada com as anotações por
//! cima (mesmas medidas da operação), seleção e arraste com o mouse,
//! teclado e zoom.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{cairo, gdk, glib};

use pdftoolbox::annotate::{ASCENT, LEADING};
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::{Annotation, AnnotationKind as Kind};
use pdftoolbox::pdfwriter;

use super::super::page_zoom::{self, connect_zoom_input};
use super::super::{thumbs, widgets};
use super::Annotator;

/// Lado maior da página renderizada para a tela, em pixels.
const RENDER_SIDE: u32 = 1400;
/// Maior lado renderizado ao aproximar.
const MAX_RENDER_SIDE: u32 = 5000;
/// Passo dos botões e teclas de zoom.
pub(super) const ZOOM_STEP: f64 = 1.25;
/// Lado maior da prévia de uma imagem inserida.
const IMAGE_SIDE: u32 = 900;
/// Margem em volta da página, em pixels.
const MARGIN: f64 = 24.0;
/// Tamanho da alça de redimensionar, em pixels.
const HANDLE: f64 = 10.0;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum DragMode {
    Move,
    Resize,
}

/// Arraste de uma anotação em andamento.
#[derive(Clone)]
pub(super) struct Drag {
    index: usize,
    mode: DragMode,
    /// A anotação antes do arraste: o deslocamento é aplicado a ela.
    start: Annotation,
    /// Já passou do limiar de movimento (e entrou no histórico).
    moved: bool,
}

/// Converte RGBA (não pré-multiplicado) para uma superfície ARGB32 do cairo.
fn surface_from_rgba(img: image::RgbaImage) -> Option<cairo::ImageSurface> {
    let (w, h) = img.dimensions();
    let stride = cairo::Format::ARgb32.stride_for_width(w).ok()? as usize;
    let mut data = vec![0u8; stride * h as usize];
    for (y, row) in img.rows().enumerate() {
        for (x, p) in row.enumerate() {
            let [r, g, b, a] = p.0;
            let pm = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
            let px = u32::from(a) << 24 | u32::from(pm(r)) << 16 | u32::from(pm(g)) << 8 | u32::from(pm(b));
            let i = y * stride + x * 4;
            data[i..i + 4].copy_from_slice(&px.to_ne_bytes());
        }
    }
    cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, w as i32, h as i32, stride as i32).ok()
}

/// Pixels por ponto para a página `page` (pontos) caber em `area` (pixels).
fn fit_scale((pw, ph): (f32, f32), (w, h): (f64, f64)) -> f64 {
    ((w - 2.0 * MARGIN) / f64::from(pw)).min((h - 2.0 * MARGIN) / f64::from(ph)).max(0.05)
}

fn set_rgba(cr: &cairo::Context, (r, g, b): (f32, f32, f32), alpha: f32) {
    cr.set_source_rgba(f64::from(r), f64::from(g), f64::from(b), f64::from(alpha));
}

/// Aumenta (ou diminui) a caixa em `dx` × `dy` pontos. Texto muda o tamanho
/// da letra; imagem mantém a proporção.
pub(super) fn resize(item: &mut Annotation, dx: f32, dy: f32) {
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
pub(super) fn keep_inside(item: &mut Annotation, (pw, ph): (f32, f32)) {
    let (w, h) = item.size();
    item.x = item.x.clamp(0.0, (1.0 - w / pw).max(0.0));
    item.y = item.y.clamp(0.0, (1.0 - h / ph).max(0.0));
}

impl Annotator {
    /// Desenho, mouse, teclado e zoom na página.
    pub(super) fn connect_canvas(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.canvas.set_draw_func(move |_, cr, _, _| {
            if let Some(a) = weak.upgrade() {
                a.draw(cr);
            }
        });
        let drag = gtk::GestureDrag::new();
        drag.connect_drag_begin(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |_, x, y| a.drag_begin(x, y)
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |_, dx, dy| a.drag_update(dx, dy)
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |_, _, _| {
                a.drag.borrow_mut().take();
                a.refresh_panel();
            }
        ));
        self.canvas.add_controller(drag);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |_, x, y| a.update_cursor(x, y)
        ));
        self.canvas.add_controller(motion);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = a)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| a.key(key, state)
        ));
        self.canvas.add_controller(keys);
        self.canvas.connect_has_focus_notify(|canvas| canvas.queue_draw());
        self.connect_view_gestures();
    }

    /// Renderiza a página com resolução para o zoom atual, se a guardada
    /// não bastar. Só a página atual fica guardada em alta resolução.
    pub(super) fn render_page(self: &Rc<Self>, page: u32) {
        let side = self.wanted_side();
        let have = self.pages.borrow().get(&page).map_or(0, |s| s.width().max(s.height()) as u32);
        if f64::from(have) >= f64::from(side) * 0.9 {
            return;
        }
        let Some(path) = self.input.borrow().clone() else { return };
        let generation = self.render_gen.get() + 1;
        self.render_gen.set(generation);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let pixels = widgets::blocking(move || {
                thumbs::render_page(&path, page, side, &CancelToken::new()).map(image::DynamicImage::into_rgba8)
            })
            .await;
            let Some(a) = weak.upgrade() else { return };
            if a.render_gen.get() != generation && a.page.get() != page {
                return;
            }
            if let Some(surface) = pixels.and_then(surface_from_rgba) {
                let mut pages = a.pages.borrow_mut();
                if pages.len() > 8 {
                    pages.clear();
                }
                if side > RENDER_SIDE {
                    pages.retain(|_, s| s.width().max(s.height()) as u32 <= RENDER_SIDE);
                }
                // Uma renderização menor que chegue atrasada não substitui a maior.
                if pages.get(&page).is_none_or(|old| old.width() < surface.width()) {
                    pages.insert(page, surface);
                }
            }
            a.canvas.queue_draw();
        });
    }

    /// Prévia de uma imagem inserida; decodificada em segundo plano na
    /// primeira vez (até lá, `None` e um retângulo cinza).
    fn image_surface(self: &Rc<Self>, path: &Path, remove_white: bool) -> Option<cairo::ImageSurface> {
        let key = (path.to_path_buf(), remove_white);
        if let Some(entry) = self.images.borrow().get(&key) {
            return entry.clone();
        }
        self.images.borrow_mut().insert(key.clone(), None);
        let weak = Rc::downgrade(self);
        let p = path.to_path_buf();
        glib::spawn_future_local(async move {
            let pixels = widgets::blocking(move || pdfwriter::decode_rgba(&p, IMAGE_SIDE, remove_white).ok()).await;
            let Some(a) = weak.upgrade() else { return };
            a.images.borrow_mut().insert(key, pixels.and_then(surface_from_rgba));
            a.canvas.queue_draw();
        });
        None
    }

    /// Pixels por ponto com a página ajustada à área visível.
    fn view_fit_scale(&self) -> f64 {
        fit_scale(self.page_size(), (f64::from(self.view.width()), f64::from(self.view.height())))
    }

    /// Posição e escala da página na área de desenho: (x, y, pixels por ponto).
    fn layout(&self) -> (f64, f64, f64) {
        let (pw, ph) = self.page_size();
        let (w, h) = (f64::from(self.canvas.width()), f64::from(self.canvas.height()));
        let scale = self.zoom.get().unwrap_or_else(|| fit_scale((pw, ph), (w, h)));
        (((w - f64::from(pw) * scale) / 2.0).max(MARGIN), ((h - f64::from(ph) * scale) / 2.0).max(MARGIN), scale)
    }

    /// Ctrl+roda e pinça aproximam em volta do ponteiro; arrastar fora das
    /// anotações move a vista quando a página está aproximada.
    fn connect_view_gestures(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let current = move || {
            let a = weak.upgrade()?;
            a.input.borrow().is_some().then(|| a.zoom.get().unwrap_or_else(|| a.view_fit_scale()))
        };
        let weak = Rc::downgrade(self);
        let zoom_to = move |scale, at| {
            if let Some(a) = weak.upgrade() {
                a.set_zoom(Some(scale), at);
            }
        };
        connect_zoom_input(&self.view, current, zoom_to);

        // A vista não se move com a rolagem (a área de desenho sim), então o
        // deslocamento do arraste é medido nela.
        let pan = gtk::GestureDrag::new();
        pan.connect_drag_begin(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |_, _, _| {
                // O arraste de uma anotação (tratado antes, na área de desenho) tem prioridade.
                if a.zoom.get().is_some() && a.drag.borrow().is_none() {
                    a.pan.set(Some((a.view.hadjustment().value(), a.view.vadjustment().value())));
                    a.canvas.set_cursor_from_name(Some("grabbing"));
                }
            }
        ));
        pan.connect_drag_update(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |_, dx, dy| {
                if let Some((x, y)) = a.pan.get() {
                    a.view.hadjustment().set_value(x - dx);
                    a.view.vadjustment().set_value(y - dy);
                }
            }
        ));
        pan.connect_drag_end(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |_, _, _| {
                if a.pan.take().is_some() {
                    a.canvas.set_cursor_from_name(Some("default"));
                }
            }
        ));
        self.view.add_controller(pan);
    }

    pub(super) fn zoom_by(self: &Rc<Self>, factor: f64) {
        if self.input.borrow().is_none() {
            return;
        }
        let current = self.zoom.get().unwrap_or_else(|| self.view_fit_scale());
        self.set_zoom(Some(current * factor), None);
    }

    /// Muda o zoom (pixels por ponto; `None` = ajustada), mantendo parado o
    /// ponto `at` da vista (padrão: o centro).
    pub(super) fn set_zoom(self: &Rc<Self>, scale: Option<f64>, at: Option<(f64, f64)>) {
        let fit = self.view_fit_scale();
        let max = fit * page_zoom::MAX_ZOOM;
        let scale = scale.map(|s| s.min(max)).filter(|s| *s > fit * 1.001);
        if self.zoom.replace(scale) != scale {
            self.anchor.hold(&self.view, at);
        }
        self.apply_zoom();
    }

    /// Tamanho da área de desenho, rótulo e resolução conforme o zoom.
    pub(super) fn apply_zoom(self: &Rc<Self>) {
        let (pw, ph) = self.page_size();
        let (w, h) = match self.zoom.get() {
            Some(s) => {
                ((f64::from(pw) * s + 2.0 * MARGIN).ceil() as i32, (f64::from(ph) * s + 2.0 * MARGIN).ceil() as i32)
            }
            None => (0, 0),
        };
        self.canvas.set_content_width(w);
        self.canvas.set_content_height(h);
        let relative = self.zoom.get().map_or(1.0, |s| s / self.view_fit_scale());
        self.zoom_controls.show(relative, self.zoom.get().is_some(), relative < page_zoom::MAX_ZOOM - 1e-3);
        self.canvas.queue_draw();
        // Mais pixels quando o zoom parar de mudar.
        let weak = Rc::downgrade(self);
        let zoom = self.zoom.get();
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(a) = weak.upgrade()
                && a.zoom.get() == zoom
            {
                a.render_page(a.page.get());
            }
        });
    }

    /// Lado da página renderizada que o zoom atual pede.
    fn wanted_side(&self) -> u32 {
        let (pw, ph) = self.page_size();
        let device = f64::from(self.canvas.scale_factor().max(1));
        let scale = self.zoom.get().unwrap_or_else(|| self.view_fit_scale()) * device;
        ((f64::from(pw.max(ph)) * scale).ceil() as u32).clamp(RENDER_SIDE, MAX_RENDER_SIDE)
    }

    /// Rola a vista para a anotação selecionada aparecer inteira (zoom).
    pub(super) fn reveal_selected(&self) {
        if self.zoom.get().is_none() {
            return;
        }
        let Some(item) = self.selected_item() else { return };
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
        reveal(self.view.hadjustment(), ox + f64::from(x) * s, f64::from(w) * s);
        reveal(self.view.vadjustment(), oy + f64::from(y) * s, f64::from(h) * s);
    }

    fn draw(self: &Rc<Self>, cr: &cairo::Context) {
        if self.input.borrow().is_none() {
            return;
        }
        let (pw, ph) = self.page_size();
        let (ox, oy, s) = self.layout();
        let (pw_px, ph_px) = (f64::from(pw) * s, f64::from(ph) * s);
        // Os erros do cairo só afetam este quadro; o próximo redesenha tudo.
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
            cr.scale(pw_px / f64::from(surface.width()), ph_px / f64::from(surface.height()));
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
        for (i, item) in items.iter().enumerate().filter(|(_, i)| i.on_page(page)) {
            let (x, y, w, h) = item.rect_on((pw, ph));
            let (x, y) = (ox + f64::from(x) * s, oy + f64::from(y) * s);
            let (w, h) = ((f64::from(w) * s).max(6.0), (f64::from(h) * s).max(6.0));
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
        // Foco do teclado na página sem anotação selecionada: moldura suave.
        if self.canvas.has_focus() && selected.is_none() && !items.is_empty() {
            let accent = self.canvas.color();
            set_rgba(cr, (accent.red(), accent.green(), accent.blue()), 0.25);
            cr.set_line_width(2.0);
            cr.rectangle(ox - 4.0, oy - 4.0, pw_px + 8.0, ph_px + 8.0);
            let _ = cr.stroke();
        }
    }

    /// Desenha uma anotação em pontos, origem no canto superior esquerdo.
    fn draw_item(self: &Rc<Self>, cr: &cairo::Context, item: &Annotation, size: (f32, f32)) {
        let (x, y, w, h) = item.rect_on(size);
        let (x, y, w, h) = (f64::from(x), f64::from(y), f64::from(w), f64::from(h));
        let _ = cr.save();
        match &item.kind {
            Kind::Text { text, size, bold, color } => {
                set_rgba(cr, *color, item.opacity);
                let weight = if *bold { cairo::FontWeight::Bold } else { cairo::FontWeight::Normal };
                cr.select_font_face("Helvetica", cairo::FontSlant::Normal, weight);
                cr.set_font_size(f64::from(*size));
                for (i, line) in text.trim_end().lines().enumerate() {
                    cr.move_to(x, y + f64::from(ASCENT + i as f32 * LEADING) * f64::from(*size));
                    let _ = cr.show_text(line);
                }
            }
            Kind::Image { path, remove_white } => match self.image_surface(path, *remove_white) {
                Some(surface) => {
                    cr.translate(x, y);
                    cr.scale(w / f64::from(surface.width()), h / f64::from(surface.height()));
                    let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                    cr.source().set_filter(cairo::Filter::Good);
                    let _ = cr.paint_with_alpha(f64::from(item.opacity));
                }
                None => {
                    cr.set_source_rgba(0.5, 0.5, 0.5, 0.2);
                    cr.rectangle(x, y, w, h);
                    let _ = cr.fill();
                }
            },
            Kind::Highlight { color } => {
                cr.set_operator(cairo::Operator::Multiply);
                set_rgba(cr, *color, item.opacity);
                cr.rectangle(x, y, w, h);
                let _ = cr.fill();
            }
            Kind::Frame { color, line_width } => {
                let lw = f64::from(*line_width);
                set_rgba(cr, *color, item.opacity);
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
            let (hx, hy) = (ox + f64::from(x + w) * s + 3.0, oy + f64::from(y + h) * s + 3.0);
            if (px - hx).abs() <= HANDLE && (py - hy).abs() <= HANDLE {
                return Some((i, DragMode::Resize));
            }
        }
        items.iter().enumerate().rev().filter(|(_, it)| it.on_page(page)).find_map(|(i, item)| {
            let (x, y, w, h) = item.rect_on(size);
            let (x0, y0) = (ox + f64::from(x) * s - 4.0, oy + f64::from(y) * s - 4.0);
            let (x1, y1) = (x0 + (f64::from(w) * s).max(6.0) + 8.0, y0 + (f64::from(h) * s).max(6.0) + 8.0);
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
                self.drag.replace(Some(Drag { index, mode, start, moved: false }));
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
            self.drag.replace(Some(drag.clone()));
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

    fn key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        use gdk::Key;
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let step = if shift { 12.0 } else { 2.0 };
        let arrow = match key {
            Key::Left | Key::KP_Left => Some((-step, 0.0)),
            Key::Right | Key::KP_Right => Some((step, 0.0)),
            Key::Up | Key::KP_Up => Some((0.0, -step)),
            Key::Down | Key::KP_Down => Some((0.0, step)),
            _ => None,
        };
        if let Some((dx, dy)) = arrow {
            if self.selected.get().is_none() {
                return glib::Propagation::Proceed;
            }
            self.remember();
            let size = self.page_size();
            self.edit_structural(|item| {
                item.x += dx / size.0;
                item.y += dy / size.1;
                keep_inside(item, size);
            });
            self.reveal_selected();
            return glib::Propagation::Stop;
        }
        let has_selection = self.selected.get().is_some();
        match key {
            Key::plus | Key::equal | Key::KP_Add | Key::minus | Key::KP_Subtract if has_selection => {
                let factor = if matches!(key, Key::plus | Key::equal | Key::KP_Add) { 0.1 } else { -0.1 };
                let size = self.page_size();
                self.remember();
                self.edit_structural(|item| {
                    let (w, h) = item.size();
                    resize(item, w * factor, h * factor);
                    keep_inside(item, size);
                });
                self.refresh_panel();
                glib::Propagation::Stop
            }
            Key::Delete | Key::KP_Delete | Key::BackSpace if has_selection => {
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
                self.select(next);
                // Fim da lista: o foco segue para o próximo controle.
                if next.is_some() { glib::Propagation::Stop } else { glib::Propagation::Proceed }
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
}
