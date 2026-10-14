//! Painel lateral do Anotar: botões para adicionar, campos da anotação
//! selecionada e a lista de anotações.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use pdftoolbox::human;
use pdftoolbox::i18n::{n_, tr};
use pdftoolbox::operations::{self, Annotation, AnnotationKind as Kind};
use pdftoolbox::pdfwriter::MM;

use super::Annotator;

pub(super) const COLORS: [(&str, (f32, f32, f32)); 7] = [
    (n_("Preto"), (0.0, 0.0, 0.0)),
    (n_("Azul"), (0.05, 0.2, 0.65)),
    (n_("Vermelho"), (0.8, 0.1, 0.1)),
    (n_("Verde"), (0.1, 0.5, 0.2)),
    (n_("Amarelo"), (1.0, 0.88, 0.2)),
    (n_("Rosa"), (1.0, 0.6, 0.8)),
    (n_("Azul-claro"), (0.55, 0.85, 1.0)),
];
pub(super) const BLACK: usize = 0;
pub(super) const RED: usize = 2;
pub(super) const YELLOW: usize = 4;

/// Campos do painel da anotação selecionada.
pub(super) struct Props {
    pub group: adw::PreferencesGroup,
    pub text: gtk::TextView,
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
    delete: gtk::Button,
}

/// Painel lateral montado: o widget, os campos e a lista de anotações.
pub(super) struct Panel {
    pub widget: gtk::ScrolledWindow,
    pub props: Props,
    pub list: gtk::ListBox,
    pub list_group: adw::PreferencesGroup,
}

fn add_button(icon: &str, label: &str, tooltip: &str, action: &str) -> gtk::Button {
    let content = adw::ButtonContent::builder().icon_name(icon).label(label).build();
    gtk::Button::builder().child(&content).tooltip_text(tooltip).action_name(action).build()
}

fn spin(title: &str, subtitle: &str, (min, max, step): (f64, f64, f64), digits: u32) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_digits(digits);
    row
}

fn caption(text: &str) -> gtk::Label {
    let label = gtk::Label::builder().label(text).wrap(true).xalign(0.0).build();
    label.add_css_class("dim-label");
    label.add_css_class("caption");
    label
}

fn color_index(c: (f32, f32, f32)) -> u32 {
    COLORS.iter().position(|(_, rgb)| *rgb == c).unwrap_or(BLACK) as u32
}

/// Texto da linha da anotação na lista.
fn describe(item: &Annotation) -> String {
    let what = match &item.kind {
        Kind::Text { text, .. } => {
            let first = text.lines().next().unwrap_or("").trim();
            let short: String = first.chars().take(28).collect();
            let more = if first.chars().count() > 28 || text.trim().lines().count() > 1 { "…" } else { "" };
            tr!("Texto “{text}”", text = format!("{short}{more}"))
        }
        Kind::Image { path, .. } => tr!("Imagem {file}", file = human::file_name(path)),
        Kind::Highlight { .. } => tr!("Destaque").into(),
        Kind::Frame { .. } => tr!("Moldura").into(),
    };
    let wheres =
        if item.all_pages { tr!("todas as páginas").to_string() } else { tr!("pág. {page}", page = item.page) };
    format!("{what} · {wheres}")
}

/// Grupo “Adicionar”: os botões ativam as ações `page.add-*`.
fn add_group() -> adw::PreferencesGroup {
    let text =
        add_button("pdftoolbox-anotar-symbolic", tr!("Texto"), tr!("Escrever um texto na página"), "page.add-text");
    let date =
        add_button("x-office-calendar-symbolic", tr!("Data de hoje"), tr!("Escrever a data de hoje"), "page.add-date");
    let highlight = add_button(
        "pdftoolbox-marca-texto-symbolic",
        tr!("Destaque"),
        tr!("Marca-texto sobre um trecho"),
        "page.add-highlight",
    );
    let frame =
        add_button("pdftoolbox-moldura-symbolic", tr!("Moldura"), tr!("Retângulo com contorno"), "page.add-frame");
    let image = add_button(
        "pdftoolbox-imagens-para-pdf-symbolic",
        tr!("Assinatura ou imagem…"),
        tr!("Inserir uma imagem, como a foto ou digitalização da sua assinatura"),
        "page.add-image",
    );
    // Grade simples: num GtkFlowBox o foco do teclado para na célula, e
    // Enter não chega ao botão.
    let grid = gtk::Grid::builder().column_spacing(6).row_spacing(6).column_homogeneous(true).build();
    for (i, b) in [&text, &date, &highlight, &frame].into_iter().enumerate() {
        grid.attach(b, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    grid.attach(&image, 0, 2, 2, 1);
    let group = adw::PreferencesGroup::builder()
        .title(tr!("Adicionar"))
        .description(tr!("Entra no centro da página atual; depois arraste para posicionar."))
        .build();
    group.add(&grid);
    group
}

fn props() -> Props {
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .accepts_tab(false)
        .height_request(72)
        .build();
    text.update_property(&[gtk::accessible::Property::Label(tr!("Texto da anotação"))]);
    let warning = gtk::Label::builder().wrap(true).xalign(0.0).visible(false).margin_bottom(12).build();
    warning.add_css_class("error");
    warning.add_css_class("caption");
    let delete = gtk::Button::builder()
        .child(&adw::ButtonContent::builder().icon_name("user-trash-symbolic").label(tr!("Remover anotação")).build())
        .tooltip_text(tr!("Remover a anotação selecionada (Delete)"))
        .halign(gtk::Align::Start)
        .margin_top(12)
        .build();
    delete.add_css_class("destructive-action");
    Props {
        group: adw::PreferencesGroup::builder().title(tr!("Anotação selecionada")).visible(false).build(),
        text_box: gtk::Frame::builder().child(&text).margin_bottom(12).build(),
        text,
        warning,
        font_size: spin(tr!("Tamanho da letra"), tr!("Em pontos"), (4.0, 144.0, 1.0), 0),
        bold: adw::SwitchRow::builder().title(tr!("Negrito")).build(),
        color: adw::ComboRow::builder()
            .title(tr!("Cor"))
            .model(&gtk::StringList::new(&COLORS.map(|(name, _)| tr(name))))
            .build(),
        width: spin(tr!("Largura"), tr!("Em milímetros"), (2.0, 600.0, 1.0), 0),
        height: spin(tr!("Altura"), tr!("Em milímetros"), (2.0, 600.0, 1.0), 0),
        line: spin(tr!("Espessura do contorno"), tr!("Em pontos"), (0.5, 12.0, 0.5), 1),
        remove_white: adw::SwitchRow::builder()
            .title(tr!("Remover fundo branco"))
            .subtitle(tr!("Deixa transparente o papel em volta da assinatura"))
            .build(),
        all_pages: adw::SwitchRow::builder()
            .title(tr!("Repetir em todas as páginas"))
            .subtitle(tr!("Útil para rubricas e carimbos"))
            .build(),
        delete,
    }
}

pub(super) fn build() -> Panel {
    let props = props();
    let props_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    props_box.append(&props.text_box);
    props_box.append(&props.warning);
    let rows = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    rows.add_css_class("boxed-list");
    for row in [
        props.font_size.upcast_ref::<gtk::Widget>(),
        props.bold.upcast_ref(),
        props.color.upcast_ref(),
        props.width.upcast_ref(),
        props.height.upcast_ref(),
        props.line.upcast_ref(),
        props.remove_white.upcast_ref(),
        props.all_pages.upcast_ref(),
    ] {
        rows.append(row);
    }
    props_box.append(&rows);
    props_box.append(&props.delete);
    props.group.add(&props_box);

    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list.add_css_class("boxed-list");
    let list_group = adw::PreferencesGroup::builder().title(tr!("Anotações")).visible(false).build();
    list_group.add(&list);

    let side = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    side.append(&add_group());
    side.append(&caption(tr!(
        "Clique numa anotação para selecioná-la. Arraste para mover; arraste o canto inferior direito para \
         mudar o tamanho."
    )));
    side.append(&props.group);
    side.append(&list_group);
    side.append(&caption(tr!(
        "A assinatura entra como imagem: é uma assinatura visual, sem a validade jurídica de uma assinatura \
         digital com certificado."
    )));
    let widget = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&side)
        .width_request(360)
        .hexpand(false)
        .build();
    Panel { widget, props, list, list_group }
}

impl Annotator {
    /// Painel → anotação selecionada.
    pub(super) fn connect_panel(self: &Rc<Self>) {
        let p = &self.props;
        p.text.buffer().connect_changed(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |buf| {
                let value = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
                a.edit(|item| {
                    if let Kind::Text { text, .. } = &mut item.kind {
                        text.clone_from(&value);
                    }
                });
            }
        ));
        p.font_size.connect_value_notify(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |row| {
                let v = row.value() as f32;
                a.edit(|item| {
                    if let Kind::Text { size, .. } = &mut item.kind {
                        *size = v;
                    }
                });
            }
        ));
        p.bold.connect_active_notify(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |row| {
                let v = row.is_active();
                a.edit(|item| {
                    if let Kind::Text { bold, .. } = &mut item.kind {
                        *bold = v;
                    }
                });
            }
        ));
        p.color.connect_selected_notify(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |row| {
                let c = COLORS.get(row.selected() as usize).map_or(COLORS[BLACK].1, |(_, c)| *c);
                a.edit(|item| match &mut item.kind {
                    Kind::Text { color, .. } | Kind::Highlight { color } | Kind::Frame { color, .. } => *color = c,
                    Kind::Image { .. } => {}
                });
            }
        ));
        p.width.connect_value_notify(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |row| {
                let v = row.value() as f32 * MM;
                a.edit(|item| {
                    // Imagens mantêm a proporção.
                    if matches!(item.kind, Kind::Image { .. }) && item.w > 0.0 {
                        item.h *= v / item.w;
                    }
                    item.w = v;
                });
                a.fill_size_rows();
            }
        ));
        p.height.connect_value_notify(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |row| {
                let v = row.value() as f32 * MM;
                a.edit(|item| item.h = v);
            }
        ));
        p.line.connect_value_notify(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |row| {
                let v = row.value() as f32;
                a.edit(|item| {
                    if let Kind::Frame { line_width, .. } = &mut item.kind {
                        *line_width = v;
                    }
                });
            }
        ));
        p.remove_white.connect_active_notify(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |row| {
                let v = row.is_active();
                a.edit(|item| {
                    if let Kind::Image { remove_white, .. } = &mut item.kind {
                        *remove_white = v;
                    }
                });
            }
        ));
        p.all_pages.connect_active_notify(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |row| {
                let v = row.is_active();
                let page = a.page.get();
                a.edit(|item| {
                    item.all_pages = v;
                    if !v {
                        item.page = page;
                    }
                });
            }
        ));
        p.delete.connect_clicked(glib::clone!(
            #[weak(rename_to = a)]
            self,
            move |_| a.delete_selected()
        ));
    }

    /// Largura e altura da anotação selecionada, sem disparar edições.
    pub(super) fn fill_size_rows(&self) {
        let Some(item) = self.selected_item() else { return };
        let was = self.filling.replace(true);
        self.props.width.set_value(f64::from((item.w / MM).round()));
        self.props.height.set_value(f64::from((item.h / MM).round()));
        self.filling.set(was);
    }

    /// Mostra no painel os campos da anotação selecionada.
    pub(super) fn refresh_panel(self: &Rc<Self>) {
        self.refresh_list();
        self.update_dirty();
        self.undo_button.set_sensitive(!self.history.is_empty());
        self.chrome.save.set_sensitive(!self.items.borrow().is_empty() && !self.saving.get());
        let p = &self.props;
        let Some(item) = self.selected_item() else {
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
            Kind::Text { .. } => tr!("Texto selecionado"),
            Kind::Image { .. } => tr!("Imagem selecionada"),
            Kind::Highlight { .. } => tr!("Destaque selecionado"),
            Kind::Frame { .. } => tr!("Moldura selecionada"),
        });
        match &item.kind {
            Kind::Text { text, size, bold, color } => {
                let buf = p.text.buffer();
                if buf.text(&buf.start_iter(), &buf.end_iter(), false) != text.as_str() {
                    buf.set_text(text);
                }
                p.font_size.set_value(f64::from(*size));
                p.bold.set_active(*bold);
                p.color.set_selected(color_index(*color));
            }
            Kind::Image { remove_white, .. } => p.remove_white.set_active(*remove_white),
            Kind::Highlight { color } => p.color.set_selected(color_index(*color)),
            Kind::Frame { color, line_width } => {
                p.color.set_selected(color_index(*color));
                p.line.set_value(f64::from(*line_width));
            }
        }
        p.width.set_value(f64::from((item.w / MM).round()));
        p.height.set_value(f64::from((item.h / MM).round()));
        p.all_pages.set_active(item.all_pages);
        self.filling.set(false);
        self.update_warning();
    }

    /// Aviso de texto vazio ou com caracteres que a fonte não tem.
    pub(super) fn update_warning(&self) {
        let text = self.selected_item().and_then(|item| match item.kind {
            Kind::Text { text, .. } => Some(text),
            _ => None,
        });
        let msg = text.and_then(|t| {
            let (_, missing) = pdftoolbox::stamp::encode(&t);
            if t.trim().is_empty() {
                Some(tr!("Digite o texto da anotação.").to_string())
            } else if !missing.is_empty() {
                Some(operations::unsupported_chars(&missing))
            } else {
                None
            }
        });
        self.props.warning.set_visible(msg.is_some());
        self.props.warning.set_text(msg.as_deref().unwrap_or(""));
    }

    /// Lista das anotações; ativar uma leva à página dela e a seleciona.
    pub(super) fn refresh_list(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let items = self.items.borrow().clone();
        self.list_group.set_visible(!items.is_empty());
        self.list_group.set_title(&tr!("Anotações ({n})", n = items.len()));
        for (i, item) in items.iter().enumerate() {
            let row =
                adw::ActionRow::builder().title(glib::markup_escape_text(&describe(item))).activatable(true).build();
            if Some(i) == self.selected.get() {
                row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            row.connect_activated(glib::clone!(
                #[weak(rename_to = a)]
                self,
                move |_| {
                    let current = a.page.get();
                    let page = a.items.borrow().get(i).map(|it| if it.on_page(current) { current } else { it.page });
                    if let Some(page) = page {
                        a.go_to(page);
                        a.select(Some(i));
                        a.canvas.grab_focus();
                    }
                }
            ));
            self.list.append(&row);
        }
    }
}
