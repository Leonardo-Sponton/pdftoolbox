//! Pré-visualizações das ferramentas: grade de páginas marcadas conforme o
//! plano (`pdftoolbox::preview`) e folhas de exemplo de Imagens para PDF.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;

use pdftoolbox::error::display_name;
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::{ImageLayout, PageSize};
use pdftoolbox::pdfwriter;
use pdftoolbox::preview::{MarkKind, Plan};

use super::page_thumb::PageThumb;
use super::page_zoom::{self, ZoomItem};
use super::thumbs;

const CELL: i32 = 104;
/// Altura de uma linha da grade: célula, número da página e margens.
const ROW: i32 = CELL + 36;
const CHUNK: u32 = 24;

struct PageCell {
    child: gtk::FlowBoxChild,
    frame: gtk::Box,
    thumb: PageThumb,
    badge: gtk::Label,
}

type Callback = Box<dyn Fn()>;
type PageCallback = Box<dyn Fn(u32)>;

pub struct PagePreview {
    pub group: adw::PreferencesGroup,
    summary: gtk::Label,
    hint: gtk::Label,
    message: gtk::Label,
    flow: gtk::FlowBox,
    scroll: gtk::ScrolledWindow,
    chooser: gtk::DropDown,
    files: RefCell<Vec<PathBuf>>,
    cells: RefCell<Vec<PageCell>>,
    path: RefCell<Option<PathBuf>>,
    total: Cell<Option<u32>>,
    loader: RefCell<Option<CancelToken>>,
    generation: Cell<u64>,
    updating: Cell<bool>,
    /// Marcação do último plano válido, para rolar só até páginas novas.
    last_marked: RefCell<Vec<bool>>,
    on_loaded: RefCell<Option<Callback>>,
    on_click: RefCell<Option<PageCallback>>,
}

impl Drop for PagePreview {
    fn drop(&mut self) {
        if let Some(t) = self.loader.borrow_mut().take() {
            t.cancel();
        }
    }
}

impl PagePreview {
    /// `clickable`: clicar numa página chama `on_click` (marcar/desmarcar).
    pub fn new(clickable: bool) -> Rc<PagePreview> {
        let summary = gtk::Label::builder().wrap(true).xalign(0.0).build();
        summary.add_css_class("pv-resumo");
        summary.set_accessible_role(gtk::AccessibleRole::Status);
        let zoom_hint = "Para ampliar uma página, clique nela com o botão direito ou use a tecla espaço.";
        let hint = gtk::Label::builder()
            .label(
                "Clique numa página para marcá-la ou desmarcá-la. Para ampliar, use o botão direito ou a tecla espaço.",
            )
            .wrap(true)
            .xalign(0.0)
            .build();
        if !clickable {
            hint.set_label(zoom_hint);
        }
        hint.add_css_class("pv-legenda");
        let message = gtk::Label::builder().wrap(true).xalign(0.0).visible(false).build();
        message.add_css_class("dim-label");
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(2)
            .max_children_per_line(12)
            .column_spacing(4)
            .row_spacing(4)
            .activate_on_single_click(true)
            .valign(gtk::Align::Start)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(380)
            .child(&flow)
            .build();
        scroll.add_css_class("card");
        let chooser = gtk::DropDown::builder().valign(gtk::Align::Center).visible(false).build();
        chooser.set_tooltip_text(Some("Arquivo mostrado na pré-visualização"));
        let group = adw::PreferencesGroup::builder().title("Pré-visualização").header_suffix(&chooser).build();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.append(&summary);
        inner.append(&hint);
        inner.append(&message);
        inner.append(&scroll);
        group.add(&inner);

        let preview = Rc::new(PagePreview {
            group,
            summary,
            hint,
            message,
            flow: flow.clone(),
            scroll,
            chooser: chooser.clone(),
            files: RefCell::new(Vec::new()),
            cells: RefCell::new(Vec::new()),
            path: RefCell::new(None),
            total: Cell::new(None),
            loader: RefCell::new(None),
            generation: Cell::new(0),
            updating: Cell::new(false),
            last_marked: RefCell::new(Vec::new()),
            on_loaded: RefCell::new(None),
            on_click: RefCell::new(None),
        });
        if clickable {
            let p = Rc::downgrade(&preview);
            flow.connect_child_activated(move |_, child| {
                if let Some(p) = p.upgrade()
                    && let Some(cb) = p.on_click.borrow().as_ref()
                {
                    cb(child.index() as u32 + 1);
                }
            });
        }
        // Ampliar: espaço (antes do FlowBox, que o usaria para ativar) e botão direito.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let p = Rc::downgrade(&preview);
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gtk::gdk::Key::space
                && mods.is_empty()
                && let Some(p) = p.upgrade()
                && let Some(child) = p.flow.focus_child().and_downcast::<gtk::FlowBoxChild>()
            {
                p.zoom(child.index() as usize);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        flow.add_controller(keys);
        let right = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
        let p = Rc::downgrade(&preview);
        right.connect_pressed(move |g, _, x, y| {
            if let Some(p) = p.upgrade()
                && let Some(child) = p.flow.child_at_pos(x as i32, y as i32)
            {
                g.set_state(gtk::EventSequenceState::Claimed);
                p.zoom(child.index() as usize);
            }
        });
        flow.add_controller(right);
        let p = Rc::downgrade(&preview);
        chooser.connect_selected_notify(move |d| {
            if let Some(p) = p.upgrade()
                && !p.updating.get()
            {
                let path = p.files.borrow().get(d.selected() as usize).cloned();
                p.load(path);
            }
        });
        preview.load(None);
        preview
    }

    /// Abre a página `index` (0 = primeira) ampliada, com a rotação planejada.
    fn zoom(&self, index: usize) {
        let (Some(path), Some(total)) = (self.current(), self.total.get()) else { return };
        let items = self
            .cells
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let rotation = c.thumb.rotation();
                let mut label = format!("Página {} de {total}", i + 1);
                if rotation != 0 {
                    label.push_str(&format!(" · girada {rotation}°"));
                }
                ZoomItem { page: i as u32 + 1, rotation, label, texture: None }
            })
            .collect();
        page_zoom::show(&self.group, path, items, index);
    }

    pub fn connect_loaded(&self, f: impl Fn() + 'static) {
        *self.on_loaded.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_page_clicked(&self, f: impl Fn(u32) + 'static) {
        *self.on_click.borrow_mut() = Some(Box::new(f));
    }

    /// Total de páginas do arquivo mostrado (quando já lido).
    pub fn total(&self) -> Option<u32> {
        self.total.get()
    }

    /// Arquivo mostrado na grade.
    pub fn current(&self) -> Option<PathBuf> {
        self.path.borrow().clone()
    }

    /// Arquivos disponíveis; mantém o arquivo mostrado se ele continuar na lista.
    pub fn set_files(self: &Rc<Self>, files: &[PathBuf]) {
        *self.files.borrow_mut() = files.to_vec();
        let names: Vec<String> = files.iter().map(|f| display_name(f)).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let current = self.path.borrow().clone();
        let index = current.and_then(|c| files.iter().position(|f| *f == c)).unwrap_or(0);
        self.updating.set(true);
        self.chooser.set_model(Some(&gtk::StringList::new(&refs)));
        self.chooser.set_selected(index as u32);
        self.updating.set(false);
        self.chooser.set_visible(files.len() > 1);
        self.load(files.get(index).cloned());
    }

    fn load(self: &Rc<Self>, path: Option<PathBuf>) {
        if *self.path.borrow() == path && (self.total.get().is_some() || self.loader.borrow().is_some()) {
            return;
        }
        if let Some(t) = self.loader.borrow_mut().take() {
            t.cancel();
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        *self.path.borrow_mut() = path.clone();
        self.total.set(None);
        self.last_marked.borrow_mut().clear();
        let Some(path) = path else {
            self.set_cells(0);
            self.show_message(Some("Adicione um arquivo para ver as páginas aqui."));
            self.summary.set_visible(false);
            self.notify_loaded();
            return;
        };
        self.show_message(Some("Lendo o documento…"));
        let cancel = CancelToken::new();
        *self.loader.borrow_mut() = Some(cancel.clone());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let facts = thumbs::facts(&path).await;
            let Some(p) = weak.upgrade() else { return };
            if p.generation.get() != generation {
                return;
            }
            if facts.locked {
                p.set_cells(0);
                p.show_message(Some(
                    "🔒 Este PDF está protegido por senha, então as páginas não podem ser mostradas. \
                     Use “Remover senha” com a senha do documento e abra a cópia desbloqueada.",
                ));
                p.notify_loaded();
                return;
            }
            let total = match (facts.problem, facts.pages) {
                (Some(problem), _) => {
                    p.set_cells(0);
                    p.show_message(Some(&format!("⚠ {problem}. Não é possível mostrar as páginas.")));
                    p.notify_loaded();
                    return;
                }
                (None, Some(n)) => n,
                (None, None) => {
                    p.set_cells(0);
                    p.show_message(Some("Não foi possível ler as páginas."));
                    p.notify_loaded();
                    return;
                }
            };
            p.total.set(Some(total));
            p.set_cells(total);
            // Duas linhas e meia visíveis quando houver páginas para isso; senão,
            // ao menos uma linha inteira (a altura natural do FlowBox, medida
            // antes das miniaturas, cortava as células pela metade).
            p.scroll.set_min_content_height(if total > 6 { 300 } else { ROW });
            p.show_message(None);
            p.summary.set_visible(true);
            p.notify_loaded();
            drop(p);
            stream_thumbnails(weak, path, total, generation, cancel).await;
        });
    }

    fn notify_loaded(&self) {
        if let Some(cb) = self.on_loaded.borrow().as_ref() {
            cb();
        }
    }

    fn show_message(&self, text: Option<&str>) {
        self.message.set_visible(text.is_some());
        self.message.set_text(text.unwrap_or_default());
        self.scroll.set_visible(text.is_none());
        self.hint.set_sensitive(text.is_none());
    }

    /// Ajusta a quantidade de células, reaproveitando as existentes.
    fn set_cells(&self, n: u32) {
        let mut cells = self.cells.borrow_mut();
        while cells.len() > n as usize {
            let cell = cells.pop().unwrap();
            self.flow.remove(&cell.child);
        }
        for cell in cells.iter() {
            cell.thumb.set_texture(None);
        }
        while cells.len() < n as usize {
            let page = cells.len() as u32 + 1;
            let cell = make_cell(page);
            self.flow.append(&cell.child);
            cells.push(cell);
        }
        let path = self.path.borrow().clone();
        if let Some(path) = path {
            for (i, cell) in cells.iter().enumerate() {
                if let Some(t) = thumbs::cached(&path, i as u32 + 1) {
                    cell.thumb.set_texture(Some(&t));
                }
            }
        }
    }

    /// Aplica o plano às miniaturas. `Err` mostra o problema no lugar do resumo.
    /// `follow`: rolar até páginas recém-marcadas que estejam fora de vista
    /// (ao digitar). Cliques nunca rolam: o usuário já está olhando a página.
    pub fn apply(&self, plan: Result<&Plan, String>, idle_text: Option<&str>, follow: bool) {
        let cells = self.cells.borrow();
        let plan = match plan {
            Ok(plan) => {
                self.summary.set_text(&plan.summary);
                self.summary.remove_css_class("texto-erro");
                Some(plan)
            }
            Err(e) => {
                self.summary.set_text(&format!("⚠ {e}"));
                self.summary.add_css_class("texto-erro");
                None
            }
        };
        if let (None, Some(text)) = (plan, idle_text) {
            self.summary.set_text(text);
            self.summary.remove_css_class("texto-erro");
        }
        for (i, cell) in cells.iter().enumerate() {
            let mark = plan.and_then(|p| p.marks.get(i));
            for c in ["pv-selecionada", "pv-removida", "pv-fora", "pv-esmaecida"] {
                cell.frame.remove_css_class(c);
            }
            for g in 0..8 {
                cell.frame.remove_css_class(&format!("pv-grupo-{g}"));
            }
            let Some(mark) = mark else {
                cell.badge.set_visible(false);
                cell.thumb.set_rotation(0);
                continue;
            };
            match &mark.kind {
                MarkKind::Normal => {}
                MarkKind::Selected => cell.frame.add_css_class("pv-selecionada"),
                MarkKind::Removed => cell.frame.add_css_class("pv-removida"),
                MarkKind::Excluded => cell.frame.add_css_class("pv-fora"),
                MarkKind::Group(g) => cell.frame.add_css_class(&format!("pv-grupo-{}", g % 8)),
            }
            cell.badge.set_visible(mark.badge.is_some());
            cell.badge.set_text(mark.badge.as_deref().unwrap_or_default());
            cell.thumb.set_rotation(mark.rotation);
            let mut desc = format!("Página {}", i + 1);
            match &mark.kind {
                MarkKind::Selected if mark.rotation != 0 => desc.push_str(&format!(", será girada {}°", mark.rotation)),
                MarkKind::Selected => desc.push_str(", selecionada"),
                MarkKind::Removed => desc.push_str(", será removida"),
                MarkKind::Excluded => desc.push_str(", fica de fora"),
                MarkKind::Group(g) => desc.push_str(&format!(", vai para o arquivo {}", g + 1)),
                MarkKind::Normal => {}
            }
            cell.child.update_property(&[gtk::accessible::Property::Label(&desc)]);
        }
        // Planos inválidos ou vazios não mexem na rolagem nem na referência.
        let Some(plan) = plan else { return };
        let new_pages = plan.newly_marked(&self.last_marked.borrow());
        *self.last_marked.borrow_mut() = plan.marked();
        if !follow || new_pages.is_empty() {
            return;
        }
        let targets: Vec<gtk::FlowBoxChild> =
            new_pages.iter().filter_map(|p| cells.get(*p as usize - 1)).map(|c| c.child.clone()).collect();
        let scroll = self.scroll.clone();
        glib::idle_add_local_once(move || {
            let Some(viewport) = scroll.child() else { return };
            let adj = scroll.vadjustment();
            let (top, bottom) = (adj.value(), adj.value() + adj.page_size());
            let bounds: Vec<_> = targets.iter().filter_map(|c| c.compute_bounds(&viewport)).collect();
            // Alguma página nova já está à vista: deixa a grade onde está.
            if bounds.iter().any(|b| b.y() as f64 >= top && (b.y() + b.height()) as f64 <= bottom) {
                return;
            }
            if let Some(first) = bounds.first() {
                adj.set_value((first.y() as f64 - 8.0).max(0.0));
            }
        });
    }
}

fn make_cell(page: u32) -> PageCell {
    let thumb = PageThumb::new(CELL - 12, CELL - 12);
    let holder = gtk::Box::builder().width_request(CELL - 8).height_request(CELL - 8).build();
    holder.set_halign(gtk::Align::Center);
    thumb.set_hexpand(true);
    holder.append(&thumb);
    let badge = gtk::Label::builder().halign(gtk::Align::End).valign(gtk::Align::Start).visible(false).build();
    badge.add_css_class("pv-selo");
    let overlay = gtk::Overlay::builder().child(&holder).build();
    overlay.add_overlay(&badge);
    let number = gtk::Label::new(Some(&page.to_string()));
    number.add_css_class("pv-numero");
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 4);
    frame.add_css_class("pv-celula");
    frame.append(&overlay);
    frame.append(&number);
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&frame));
    child.update_property(&[gtk::accessible::Property::Label(&format!("Página {page}"))]);
    PageCell { child, frame, thumb, badge }
}

/// Carrega as miniaturas em blocos, mostrando cada bloco assim que fica pronto.
async fn stream_thumbnails(weak: Weak<PagePreview>, path: PathBuf, total: u32, generation: u64, cancel: CancelToken) {
    let mut first = 1;
    while first <= total && !cancel.is_cancelled() {
        let last = (first + CHUNK - 1).min(total);
        let pages = thumbs::pdf_pages(&path, first, last, &cancel).await;
        let Some(p) = weak.upgrade() else { return };
        if p.generation.get() != generation {
            return;
        }
        let cells = p.cells.borrow();
        for (page, texture) in pages {
            if let Some(cell) = cells.get(page as usize - 1) {
                cell.thumb.set_texture(Some(&texture));
            }
        }
        first = last + 1;
    }
}

/// Folhas de exemplo de Imagens para PDF: mostram onde cada imagem ficará na
/// página, com a mesma regra de posicionamento da conversão.
pub struct LayoutPreview {
    pub group: adw::PreferencesGroup,
    flow: gtk::FlowBox,
    more: gtk::Label,
}

const SHEET: f32 = 150.0;
const MAX_SHEETS: usize = 8;

impl LayoutPreview {
    pub fn new() -> LayoutPreview {
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(1)
            .max_children_per_line(8)
            .column_spacing(16)
            .row_spacing(12)
            .can_focus(false)
            .build();
        let more = gtk::Label::builder().xalign(0.0).visible(false).build();
        more.add_css_class("pv-legenda");
        let group = adw::PreferencesGroup::builder()
            .title("Pré-visualização")
            .description("Onde cada imagem ficará na página. Imagens mais largas que altas ganham página deitada.")
            .build();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.append(&flow);
        inner.append(&more);
        group.add(&inner);
        group.set_visible(false);
        LayoutPreview { group, flow, more }
    }

    /// `images`: caminho e dimensões exibidas (px) de cada imagem, na ordem.
    pub fn update(&self, images: &[(PathBuf, Option<(f32, f32)>)], layout: ImageLayout) {
        while let Some(c) = self.flow.first_child() {
            self.flow.remove(&c);
        }
        self.group.set_visible(!images.is_empty());
        for (i, (path, dims)) in images.iter().take(MAX_SHEETS).enumerate() {
            self.flow.append(&sheet(i + 1, path, *dims, layout));
        }
        let extra = images.len().saturating_sub(MAX_SHEETS);
        self.more.set_visible(extra > 0);
        self.more.set_text(&format!("e mais {extra} página{} com a mesma regra.", if extra == 1 { "" } else { "s" }));
    }
}

fn size_name(layout: ImageLayout, landscape: bool) -> String {
    let base = match layout.size {
        PageSize::A4 => "A4",
        PageSize::Letter => "Carta",
        PageSize::FitImage => return "tamanho da imagem".into(),
    };
    format!("{base} {}", if landscape { "paisagem" } else { "retrato" })
}

fn sheet(n: usize, path: &Path, dims: Option<(f32, f32)>, layout: ImageLayout) -> gtk::Widget {
    let (dw, dh) = dims.unwrap_or((3.0, 4.0));
    let pl = pdfwriter::place(layout, dw, dh);
    let scale = SHEET / pl.page_w.max(pl.page_h);
    let (pw, ph) = ((pl.page_w * scale).round() as i32, (pl.page_h * scale).round() as i32);
    let fixed = gtk::Fixed::builder().width_request(pw).height_request(ph).build();
    fixed.add_css_class("folha-previa");
    let (iw, ih) = ((pl.w * scale).round().max(1.0) as i32, (pl.h * scale).round().max(1.0) as i32);
    let thumb = thumbs::picture(path, iw, ih);
    thumb.set_placeholder(iw as f64, ih as f64);
    // PDF tem origem embaixo; na tela, em cima.
    fixed.put(&thumb, (pl.x * scale) as f64, ((pl.page_h - pl.y - pl.h) * scale) as f64);
    let caption = gtk::Label::new(Some(&format!("{n} · {}", size_name(layout, pl.page_w > pl.page_h))));
    caption.add_css_class("pv-legenda");
    let holder = gtk::Box::builder().width_request(SHEET as i32).height_request(SHEET as i32).build();
    fixed.set_halign(gtk::Align::Center);
    fixed.set_valign(gtk::Align::End);
    fixed.set_hexpand(true);
    holder.append(&fixed);
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.append(&holder);
    b.append(&caption);
    b.set_tooltip_text(Some(&display_name(path)));
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&b));
    child.set_focusable(false);
    child.upcast()
}

/// “Como vai ficar”: uma página renderizada com o carimbo aplicado pelas
/// mesmas funções da operação. Atualiza com atraso curto enquanto o usuário
/// digita, e descarta resultados que chegam fora de ordem.
pub struct StampPreview {
    pub group: adw::PreferencesGroup,
    thumb: PageThumb,
    caption: gtk::Label,
    spinner: gtk::Spinner,
    generation: Cell<u64>,
    timer: RefCell<Option<glib::SourceId>>,
}

type LayerFn = Box<dyn FnOnce((f32, f32)) -> pdfwriter::OverlayPage + Send>;

impl StampPreview {
    pub fn new() -> Rc<StampPreview> {
        let thumb = PageThumb::new(300, 380);
        thumb.set_margin_top(12);
        thumb.set_margin_bottom(4);
        let spinner = gtk::Spinner::builder().halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let overlay = gtk::Overlay::builder().child(&thumb).build();
        overlay.add_overlay(&spinner);
        let caption = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).build();
        caption.add_css_class("pv-legenda");
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.append(&overlay);
        inner.append(&caption);
        let group = adw::PreferencesGroup::builder()
            .title("Como vai ficar")
            .description("A página é renderizada com o resultado real, antes de salvar.")
            .build();
        group.add(&inner);
        group.set_visible(false);
        Rc::new(StampPreview { group, thumb, caption, spinner, generation: Cell::new(0), timer: RefCell::new(None) })
    }

    pub fn hide(&self) {
        self.generation.set(self.generation.get() + 1);
        if let Some(t) = self.timer.borrow_mut().take() {
            t.remove();
        }
        self.group.set_visible(false);
    }

    /// Mostra `page` de `path` com a camada que `layer` produzir.
    pub fn show(self: &Rc<Self>, path: &Path, page: u32, caption: &str, layer: LayerFn) {
        self.group.set_visible(true);
        self.caption.set_text(caption);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        if let Some(t) = self.timer.borrow_mut().take() {
            t.remove();
        }
        self.spinner.set_visible(true);
        self.spinner.start();
        let this = Rc::downgrade(self);
        let path = path.to_path_buf();
        let mut layer = Some(layer);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(220), move || {
            let Some(me) = this.upgrade() else { return };
            me.timer.borrow_mut().take();
            let Some(layer) = layer.take() else { return };
            let this = Rc::downgrade(&me);
            glib::spawn_future_local(async move {
                let texture = thumbs::stamped_page(&path, page, layer, 760).await;
                let Some(me) = this.upgrade() else { return };
                if me.generation.get() != generation {
                    return;
                }
                me.spinner.stop();
                me.spinner.set_visible(false);
                match texture {
                    Some(t) => me.thumb.set_texture(Some(&t)),
                    None => me.caption.set_text("Não foi possível gerar a prévia desta página."),
                }
            });
        });
        *self.timer.borrow_mut() = Some(id);
    }
}

/// Prévia do texto extraído da primeira página.
pub struct TextPreview {
    pub group: adw::PreferencesGroup,
    text: gtk::Label,
    scroll: gtk::ScrolledWindow,
    note: gtk::Label,
    generation: Cell<u64>,
}

impl TextPreview {
    pub fn new() -> Rc<TextPreview> {
        let text = gtk::Label::builder()
            .xalign(0.0)
            .yalign(0.0)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(12)
            .margin_end(12)
            .build();
        text.add_css_class("monospace");
        text.add_css_class("caption");
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(120)
            .max_content_height(280)
            .propagate_natural_height(true)
            .child(&text)
            .build();
        scroll.add_css_class("card");
        let note = gtk::Label::builder().wrap(true).xalign(0.0).visible(false).build();
        note.add_css_class("texto-aviso");
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner.append(&note);
        inner.append(&scroll);
        let group = adw::PreferencesGroup::builder().title("Prévia do texto").build();
        group.add(&inner);
        group.set_visible(false);
        Rc::new(TextPreview { group, text, scroll, note, generation: Cell::new(0) })
    }

    pub fn update(self: &Rc<Self>, path: Option<&Path>, layout: bool) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let Some(path) = path.map(Path::to_path_buf) else {
            self.group.set_visible(false);
            return;
        };
        self.group.set_visible(true);
        self.group.set_description(Some(&glib::markup_escape_text(&format!(
            "Como o texto de “{}” ficará no arquivo .txt.",
            display_name(&path)
        ))));
        self.text.set_text("Lendo…");
        self.scroll.set_visible(true);
        self.note.set_visible(false);
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let text = thumbs::first_text_page(&path, layout).await;
            let Some(me) = this.upgrade() else { return };
            if me.generation.get() != generation {
                return;
            }
            match text {
                Some(Some((page, t))) => {
                    let start = match page {
                        1 => String::new(),
                        2 => " A página 1 não tem texto selecionável (capa ou imagem?).".into(),
                        3 => " As páginas 1 e 2 não têm texto selecionável (capa ou imagens?).".into(),
                        n => format!(" As páginas 1 a {} não têm texto selecionável (capa ou imagens?).", n - 1),
                    };
                    me.group.set_description(Some(&glib::markup_escape_text(&format!(
                        "Página {page} de “{}”, como ficará no arquivo .txt.{start}",
                        display_name(&path)
                    ))));
                    const MAX_LINES: usize = 60;
                    let lines: Vec<&str> = t.trim_end().lines().collect();
                    let mut shown = lines.iter().take(MAX_LINES).copied().collect::<Vec<_>>().join("\n");
                    if lines.len() > MAX_LINES {
                        shown.push_str(&format!("\n… (mais {} linhas)", lines.len() - MAX_LINES));
                    }
                    me.text.set_text(&shown);
                }
                Some(None) => {
                    me.text.set_text("");
                    me.scroll.set_visible(false);
                    me.note.set_text(
                        "Nenhuma página tem texto selecionável — o PDF parece ser uma digitalização (imagem). \
                         Use antes a ferramenta “OCR — reconhecer texto” e extraia o texto do PDF que ela criar.",
                    );
                    me.note.set_visible(true);
                }
                None => {
                    me.text.set_text("");
                    me.scroll.set_visible(false);
                    me.note.set_text("Não foi possível ler o texto deste arquivo (protegido por senha ou danificado).");
                    me.note.set_visible(true);
                }
            }
        });
    }
}
