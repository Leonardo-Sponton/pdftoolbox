//! Miniatura de página: ocupa exatamente o formato da página (cabendo em
//! `max_w` × `max_h`), já girada, para que fundo e sombra pareçam papel.
//! Sem textura, mostra um retângulo A4 vazio enquanto a página carrega.

use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, prelude::*};

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Default)]
    pub struct PageThumb {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub max: Cell<(i32, i32)>,
        pub rotation: Cell<i32>,
        /// Proporção do retângulo vazio exibido antes de a textura chegar.
        pub placeholder: Cell<(f64, f64)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PageThumb {
        const NAME: &'static str = "PdfToolboxPageThumb";
        type Type = super::PageThumb;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("pagethumb");
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for PageThumb {}

    impl PageThumb {
        /// Tamanho exibido: proporção da página (girada), cabendo no máximo.
        pub fn display_size(&self) -> (i32, i32) {
            let (max_w, max_h) = self.max.get();
            let (mut w, mut h) = match self.texture.borrow().as_ref() {
                Some(t) => (t.width() as f64, t.height() as f64),
                None => match self.placeholder.get() {
                    (0.0, _) | (_, 0.0) => (210.0, 297.0),
                    p => p,
                },
            };
            if self.rotation.get().rem_euclid(180) == 90 {
                std::mem::swap(&mut w, &mut h);
            }
            let scale = (max_w as f64 / w).min(max_h as f64 / h);
            (((w * scale).round() as i32).max(1), ((h * scale).round() as i32).max(1))
        }
    }

    impl WidgetImpl for PageThumb {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let (w, h) = self.display_size();
            let v = if orientation == gtk::Orientation::Horizontal { w } else { h };
            (v, v, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(texture) = self.texture.borrow().clone() else { return };
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            let angle = self.rotation.get().rem_euclid(360);
            // Retângulo da página antes da rotação, centrado na origem.
            let (pw, ph) = if angle % 180 == 90 { (h, w) } else { (w, h) };
            snapshot.save();
            snapshot.translate(&graphene::Point::new(w / 2.0, h / 2.0));
            snapshot.rotate(angle as f32);
            snapshot.append_scaled_texture(
                &texture,
                gtk::gsk::ScalingFilter::Trilinear,
                &graphene::Rect::new(-pw / 2.0, -ph / 2.0, pw, ph),
            );
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct PageThumb(ObjectSubclass<imp::PageThumb>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PageThumb {
    pub fn new(max_w: i32, max_h: i32) -> Self {
        let thumb: Self = glib::Object::new();
        thumb.imp().max.set((max_w, max_h));
        thumb.set_halign(gtk::Align::Center);
        thumb.set_valign(gtk::Align::Center);
        thumb.add_css_class("miniatura-vazia");
        thumb
    }

    pub fn set_texture(&self, texture: Option<&gdk::Texture>) {
        *self.imp().texture.borrow_mut() = texture.cloned();
        self.set_css_classes(if texture.is_some() { &["miniatura"] } else { &["miniatura-vazia"] });
        self.queue_resize();
    }

    /// Formato do retângulo vazio (padrão: A4 retrato).
    pub fn set_placeholder(&self, w: f64, h: f64) {
        self.imp().placeholder.set((w, h));
        self.queue_resize();
    }

    pub fn has_texture(&self) -> bool {
        self.imp().texture.borrow().is_some()
    }

    pub fn set_rotation(&self, degrees: i32) {
        if self.imp().rotation.replace(degrees.rem_euclid(360)) != degrees.rem_euclid(360) {
            self.queue_resize();
        }
    }

    pub fn rotation(&self) -> i32 {
        self.imp().rotation.get()
    }

    pub fn set_max(&self, max_w: i32, max_h: i32) {
        self.imp().max.set((max_w, max_h));
        self.queue_resize();
    }
}
