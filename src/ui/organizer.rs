//! Organizar páginas: miniaturas, seleção múltipla, arrastar para reordenar,
//! girar, duplicar, excluir, inserir páginas em branco, ampliar e desfazer.
//! O resultado é gravado pela operação estrutural `Request::Organize` (sem
//! rasterizar).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use pdftoolbox::OpResult;
use pdftoolbox::catalog::{self, Accepts, ToolId};
use pdftoolbox::engines::{poppler, qpdf};
use pdftoolbox::human;
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::{PageItem, Request};
use pdftoolbox::storage::JobDir;

use super::editor::{self, EditorChrome, EmptyState, History};
use super::page_zoom::{self, ZoomItem};
use super::{AppState, thumbs, unlock, widgets};

/// Lado das miniaturas, em pixels.
const THUMB: i32 = 160;
/// Miniaturas renderizadas por chamada do pdftoppm.
const CHUNK: u32 = 12;

/// Célula da grade. As células são reaproveitadas (o índice é estável);
/// recriar a grade com `remove_all()` deixa o cursor do GtkFlowBox inválido
/// e derruba o programa na próxima tecla de navegação.
struct PageCell {
    child: gtk::FlowBoxChild,
    picture: gtk::Picture,
    label: gtk::Label,
    rotation: gtk::Label,
}

struct Organizer {
    app: Weak<AppState>,
    chrome: EditorChrome,
    input: RefCell<Option<PathBuf>>,
    total: Cell<u32>,
    items: RefCell<Vec<PageItem>>,
    /// Ordem gravada por último (ou a do arquivo aberto): diferente de
    /// `items` = alterações não salvas.
    saved: RefCell<Vec<PageItem>>,
    history: History<Vec<PageItem>>,
    /// Folhas brancas já criadas, por tamanho em pixels.
    blanks: RefCell<HashMap<(u32, u32), gdk::Texture>>,
    textures: RefCell<HashMap<u32, gdk::Texture>>,
    cells: RefCell<Vec<PageCell>>,
    thumbs_dir: RefCell<Option<JobDir>>,
    loader: RefCell<Option<CancelToken>>,
    flow: gtk::FlowBox,
    count: gtk::Label,
    undo_button: gtk::Button,
    dest_dir: RefCell<Option<PathBuf>>,
    saving: Cell<bool>,
}

/// Barra inferior: botões (com as ações que ativam) e o contador.
fn action_bar() -> (gtk::ActionBar, gtk::Label, gtk::Button) {
    let action_bar = gtk::ActionBar::new();
    let button = |icon: &str, tooltip: &str, action: &str| {
        let b = widgets::icon_button(icon, tooltip);
        b.set_action_name(Some(action));
        b
    };
    for b in [
        button("object-rotate-left-symbolic", "Girar seleção à esquerda (Ctrl+Shift+R)", "page.rotate-left"),
        button("object-rotate-right-symbolic", "Girar seleção à direita (Ctrl+R)", "page.rotate-right"),
        button("edit-copy-symbolic", "Duplicar seleção (Ctrl+D)", "page.duplicate"),
        button("user-trash-symbolic", "Excluir seleção (Delete)", "page.delete"),
        button(
            "document-new-symbolic",
            "Inserir página em branco depois da seleção, do tamanho da página anterior (Ctrl+B)",
            "page.insert-blank",
        ),
    ] {
        action_bar.pack_start(&b);
    }
    action_bar.pack_start(&gtk::Separator::new(gtk::Orientation::Vertical));
    action_bar.pack_start(&button("go-previous-symbolic", "Mover seleção para trás (Ctrl+←)", "page.move-left"));
    action_bar.pack_start(&button("go-next-symbolic", "Mover seleção para frente (Ctrl+→)", "page.move-right"));
    action_bar.pack_start(&gtk::Separator::new(gtk::Orientation::Vertical));
    action_bar.pack_start(&button("zoom-in-symbolic", "Ampliar a página (espaço ou clique duplo)", "page.zoom"));
    let count = gtk::Label::new(None);
    count.add_css_class("dim-label");
    count.set_ellipsize(gtk::pango::EllipsizeMode::End);
    action_bar.set_center_widget(Some(&count));
    action_bar.pack_end(&button("edit-select-all-symbolic", "Selecionar todas (Ctrl+A)", "page.select-all"));
    let undo_button = button("edit-undo-symbolic", "Desfazer (Ctrl+Z)", "page.undo");
    action_bar.pack_end(&undo_button);
    (action_bar, count, undo_button)
}

pub fn build(app: &Rc<AppState>, files: Vec<PathBuf>) -> adw::NavigationPage {
    let info = catalog::info(ToolId::Organize);
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Multiple)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(12)
        .column_spacing(8)
        .row_spacing(8)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .valign(gtk::Align::Start)
        .activate_on_single_click(false)
        .build();
    flow.update_property(&[gtk::accessible::Property::Label("Páginas do documento")]);
    let grid_scroll =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&flow).vexpand(true).build();
    let (action_bar, count, undo_button) = action_bar();
    let empty = EmptyState {
        icon: "view-grid-symbolic",
        title: "Escolha um PDF para organizar",
        description: "Ou arraste o arquivo para esta janela. Você poderá reordenar, girar, duplicar, excluir e \
                      inserir páginas em branco antes de salvar.",
        open_action: "page.add",
    };
    let (chrome, nav_page) = EditorChrome::new(info, &empty, &grid_scroll, action_bar);

    let org = Rc::new(Organizer {
        app: Rc::downgrade(app),
        chrome,
        input: RefCell::new(None),
        total: Cell::new(0),
        items: RefCell::new(Vec::new()),
        saved: RefCell::new(Vec::new()),
        history: History::new(),
        blanks: RefCell::new(HashMap::new()),
        textures: RefCell::new(HashMap::new()),
        cells: RefCell::new(Vec::new()),
        thumbs_dir: RefCell::new(None),
        loader: RefCell::new(None),
        flow: flow.clone(),
        count,
        undo_button,
        dest_dir: RefCell::new(None),
        saving: Cell::new(false),
    });

    // Ações da página e atalhos
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: fn(&Rc<Organizer>)| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(glib::clone!(
            #[weak]
            org,
            move |_, _| f(&org)
        ));
        group.add_action(&action);
    };
    add("add", |o| o.confirm_discard(|o| o.pick()));
    add("leave", |o| o.leave());
    add("insert-blank", |o| o.insert_blank());
    add("zoom", |o| o.zoom(None));
    add("run", |o| o.save_to(None));
    add("save-as", |o| o.save_as());
    add("rotate-right", |o| o.rotate(90));
    add("rotate-left", |o| o.rotate(-90));
    add("duplicate", |o| o.duplicate());
    add("delete", |o| o.delete());
    add("undo", |o| o.undo());
    add("move-left", |o| o.nudge(-1));
    add("move-right", |o| o.nudge(1));
    add("select-all", |o| o.flow.select_all());
    nav_page.insert_action_group("page", Some(&group));
    let shortcuts = widgets::shortcuts(&[
        ("<Control>s", "page.run"),
        ("<Control>r", "page.rotate-right"),
        ("<Control><Shift>r", "page.rotate-left"),
        ("<Control>d", "page.duplicate"),
        ("Delete", "page.delete"),
        ("<Control>z", "page.undo"),
        ("<Control>a", "page.select-all"),
        ("<Control>b", "page.insert-blank"),
        ("Insert", "page.insert-blank"),
    ]);
    shortcuts.set_scope(gtk::ShortcutScope::Global);
    nav_page.add_controller(shortcuts);
    // Ctrl+setas precisam ser capturadas antes das teclas internas do FlowBox,
    // que as usam para mover o cursor sem alterar a seleção. Espaço amplia a
    // página (o FlowBox o usaria para selecionar; Enter continua).
    let moves = widgets::shortcuts(&[
        ("<Control>Left", "page.move-left"),
        ("<Control>Right", "page.move-right"),
        ("space", "page.zoom"),
    ]);
    moves.set_propagation_phase(gtk::PropagationPhase::Capture);
    flow.add_controller(moves);

    // Clique duplo (ou Enter) numa página: ampliar.
    flow.connect_child_activated(glib::clone!(
        #[weak]
        org,
        move |_, child| org.zoom(Some(child.index() as usize))
    ));
    flow.connect_selected_children_changed(glib::clone!(
        #[weak]
        org,
        move |_| org.update_count()
    ));
    widgets::add_file_drop(
        &nav_page,
        glib::clone!(
            #[weak]
            org,
            move |files| {
                if let Some(f) = files.into_iter().find(|f| catalog::extension_ok(Accepts::Pdf, f)) {
                    org.confirm_discard(move |o| o.load(f.clone()));
                }
            }
        ),
    );
    // Ao sair da página, interrompe as miniaturas e apaga os temporários.
    app.nav.connect_popped(glib::clone!(
        #[weak]
        org,
        move |_, popped| {
            if org.chrome.is_page(popped) {
                org.stop_loader();
                org.thumbs_dir.borrow_mut().take();
            }
        }
    ));

    org.update_count();
    if let Some(f) = files.into_iter().next() {
        org.load(f);
    }
    widgets::keep_alive(&nav_page, org);
    nav_page
}

impl Organizer {
    fn window(&self) -> Option<adw::ApplicationWindow> {
        self.app.upgrade().map(|a| a.window.clone())
    }

    fn toast(&self, text: &str) {
        if let Some(app) = self.app.upgrade() {
            app.toast(text);
        }
    }

    fn pick(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let org = self.clone();
        glib::spawn_future_local(async move {
            if let Some(f) = widgets::pick_files(&win, Some(Accepts::Pdf), false).await.into_iter().next() {
                org.load(f);
            }
        });
    }

    fn stop_loader(&self) {
        if let Some(t) = self.loader.borrow_mut().take() {
            t.cancel();
        }
    }

    fn load(self: &Rc<Self>, path: PathBuf) {
        self.stop_loader();
        self.chrome.show_loading();
        let org = self.clone();
        glib::spawn_future_local(async move {
            // PDF com senha: pede a senha e abre uma cópia desbloqueada.
            let Some(app) = org.app.upgrade() else { return };
            let Some(path) = unlock::prepare(&app, vec![path]).await.into_iter().next() else {
                org.chrome.restore(org.input.borrow().is_some());
                return;
            };
            let p = path.clone();
            let result = widgets::blocking(move || -> OpResult<(u32, JobDir)> {
                let n = qpdf::page_count(&p, &CancelToken::new())?;
                Ok((n, JobDir::new()?))
            })
            .await;
            match result {
                Ok((n, dir)) => org.start(path, n, dir),
                Err(e) => org.chrome.show_error(&e),
            }
        });
    }

    fn start(self: &Rc<Self>, path: PathBuf, total: u32, dir: JobDir) {
        self.chrome.set_title(&format!("Organizar — {}", human::file_name(&path)));
        self.dest_dir.replace(unlock::original(&path).parent().map(Path::to_path_buf));
        self.total.set(total);
        self.items.replace((1..=total).map(|page| PageItem { page, rotation: 0 }).collect());
        self.saved.replace(self.items.borrow().clone());
        self.history.clear();
        self.textures.borrow_mut().clear();
        let thumbs = dir.path().to_path_buf();
        self.thumbs_dir.replace(Some(dir));
        self.input.replace(Some(path.clone()));
        self.rebuild(&[]);
        self.chrome.show_content();
        self.chrome.save.set_sensitive(true);
        self.load_thumbnails(path, total, thumbs);
    }

    /// Miniaturas em blocos, numa thread própria, com cancelamento.
    fn load_thumbnails(self: &Rc<Self>, path: PathBuf, total: u32, dir: PathBuf) {
        let cancel = CancelToken::new();
        self.loader.replace(Some(cancel.clone()));
        let (tx, rx) = async_channel::unbounded::<Vec<(u32, PathBuf)>>();
        std::thread::spawn(move || {
            let mut first = 1;
            while first <= total && !cancel.is_cancelled() {
                let last = (first + CHUNK - 1).min(total);
                let Ok(files) = poppler::thumbnails(&path, first, last, THUMB as u32, &dir, &cancel) else { break };
                if tx.send_blocking((first..=last).zip(files).collect()).is_err() {
                    break;
                }
                first = last + 1;
            }
        });
        let org = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(batch) = rx.recv().await {
                let Some(org) = org.upgrade() else { break };
                for (page, file) in batch {
                    if let Ok(tex) = gdk::Texture::from_filename(&file) {
                        org.textures.borrow_mut().insert(page, tex);
                    }
                    let _ = std::fs::remove_file(file);
                }
                org.refresh_pictures();
            }
        });
    }

    /// Imagem da posição `pos`: a miniatura da página ou, para uma página em
    /// branco, uma folha branca no formato da vizinha que lhe dá o tamanho
    /// (a mesma regra de `operations::blank_sizes`).
    fn texture_at(&self, items: &[PageItem], pos: usize) -> Option<gdk::Texture> {
        let item = items[pos];
        if !item.is_blank() {
            return self.textures.borrow().get(&item.page).cloned();
        }
        let neighbour =
            items[..pos].iter().rev().find(|it| !it.is_blank()).or(items[pos + 1..].iter().find(|it| !it.is_blank()));
        let (mut w, mut h) = (113, THUMB as u32);
        if let Some(n) = neighbour
            && let Some(t) = self.textures.borrow().get(&n.page)
        {
            (w, h) = (t.width() as u32, t.height() as u32);
            if n.rotation.rem_euclid(180) == 90 {
                (w, h) = (h, w);
            }
        }
        Some(self.blanks.borrow_mut().entry((w, h)).or_insert_with(|| thumbs::blank_texture(w, h)).clone())
    }

    fn set_picture(&self, picture: &gtk::Picture, texture: Option<&gdk::Texture>, item: &PageItem) {
        picture.set_paintable(texture);
        for c in ["rot90", "rot180", "rot270"] {
            picture.remove_css_class(c);
        }
        match item.rotation.rem_euclid(360) {
            90 => picture.add_css_class("rot90"),
            180 => picture.add_css_class("rot180"),
            270 => picture.add_css_class("rot270"),
            _ => {}
        }
    }

    fn refresh_pictures(&self) {
        let items = self.items.borrow();
        for (pos, (cell, item)) in self.cells.borrow().iter().zip(items.iter()).enumerate() {
            self.set_picture(&cell.picture, self.texture_at(&items, pos).as_ref(), item);
        }
        self.update_count();
    }

    /// Cria a célula da posição `pos` (fixa durante toda a vida da célula).
    fn make_cell(self: &Rc<Self>, pos: usize) -> PageCell {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .width_request(THUMB)
            .height_request(THUMB)
            // Alocação quadrada: o `transform: rotate` do CSS gira a alocação
            // inteira, e um retângulo girado invadiria o rótulo abaixo.
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        let label = gtk::Label::new(None);
        label.add_css_class("caption");
        let rotation = gtk::Label::new(None);
        rotation.add_css_class("caption");
        rotation.add_css_class("thumb-rotated");
        let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
        b.add_css_class("thumb");
        b.append(&picture);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        footer.set_halign(gtk::Align::Center);
        footer.append(&label);
        footer.append(&rotation);
        b.append(&footer);
        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&b));

        // Arrastar para reordenar.
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        source.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&(pos as u32).to_value())));
        let pic = picture.clone();
        source.connect_drag_begin(move |_, drag| {
            let icon = gtk::DragIcon::for_drag(drag);
            let shown = gtk::Picture::for_paintable(&gtk::WidgetPaintable::new(Some(&pic)));
            shown.set_size_request(THUMB / 2, THUMB / 2);
            icon.set_child(Some(&shown));
        });
        child.add_controller(source);
        let target = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
        target.connect_drop(glib::clone!(
            #[weak(rename_to = org)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                let Ok(from) = value.get::<u32>() else { return false };
                org.drag_move(from as usize, pos);
                true
            }
        ));
        child.add_controller(target);
        PageCell { child, picture, label, rotation }
    }

    fn update_cell(&self, cell: &PageCell, items: &[PageItem], pos: usize) {
        let item = &items[pos];
        self.set_picture(&cell.picture, self.texture_at(items, pos).as_ref(), item);
        let mut text = format!("{}", pos + 1);
        if item.is_blank() {
            text.push_str(" · em branco");
        } else if item.page as usize != pos + 1 {
            text.push_str(&format!(" · pág. {}", item.page));
        }
        cell.label.set_text(&text);
        let angle = item.rotation.rem_euclid(360);
        cell.rotation.set_visible(angle != 0);
        cell.rotation.set_text(&format!("↻ {angle}°"));
        let mut desc = if item.is_blank() {
            format!("Posição {}, página em branco", pos + 1)
        } else {
            format!("Posição {}, página original {}", pos + 1, item.page)
        };
        if angle != 0 {
            desc.push_str(&format!(", girada {angle} graus"));
        }
        cell.child.update_property(&[gtk::accessible::Property::Label(&desc)]);
    }

    /// Ajusta as células à ordem atual e seleciona as posições `select`.
    fn rebuild(self: &Rc<Self>, select: &[usize]) {
        let items = self.items.borrow().clone();
        while self.cells.borrow().len() < items.len() {
            let cell = self.make_cell(self.cells.borrow().len());
            self.flow.append(&cell.child);
            self.cells.borrow_mut().push(cell);
        }
        let extra: Vec<PageCell> = {
            let mut cells = self.cells.borrow_mut();
            let keep = items.len().min(cells.len());
            cells.drain(keep..).collect()
        };
        for cell in extra.iter().rev() {
            self.flow.remove(&cell.child);
        }
        self.flow.unselect_all();
        for (pos, cell) in self.cells.borrow().iter().enumerate() {
            self.update_cell(cell, &items, pos);
            if select.contains(&pos) {
                self.flow.select_child(&cell.child);
            }
        }
        // Mantém o foco (e, com ele, a rolagem) na página em que o usuário está,
        // se ela continuar selecionada; senão, vai para a primeira selecionada.
        let target = match self.focused() {
            Some(i) if select.contains(&i) => Some(i),
            _ => select.first().copied(),
        };
        if let Some(pos) = target
            && let Some(cell) = self.cells.borrow().get(pos)
        {
            cell.child.grab_focus();
        }
        self.undo_button.set_sensitive(!self.history.is_empty());
        self.update_count();
        self.update_dirty();
    }

    /// Posição da página com o foco do teclado.
    fn focused(&self) -> Option<usize> {
        self.flow.focus_child().and_downcast::<gtk::FlowBoxChild>().map(|c| c.index() as usize)
    }

    fn dirty(&self) -> bool {
        *self.items.borrow() != *self.saved.borrow()
    }

    fn update_dirty(&self) {
        self.chrome.set_dirty(self.dirty());
    }

    /// Volta à tela anterior, confirmando se há alterações não salvas.
    fn leave(self: &Rc<Self>) {
        self.confirm_discard(|o| {
            let saved = o.saved.borrow().clone();
            o.items.replace(saved);
            o.update_dirty();
            if let Some(app) = o.app.upgrade() {
                app.nav.pop();
            }
        });
    }

    /// Executa `then` direto, ou depois de o usuário aceitar descartar as
    /// alterações não salvas.
    fn confirm_discard(self: &Rc<Self>, then: impl Fn(&Rc<Self>) + 'static) {
        let Some(app) = self.app.upgrade() else { return };
        if !self.dirty() || self.saving.get() {
            then(self);
            return;
        }
        let org = Rc::downgrade(self);
        editor::confirm_discard(
            &app,
            "Descartar as alterações?",
            "A nova ordem das páginas ainda não foi salva. O PDF original não foi alterado.",
            "Continuar organizando",
            move || {
                if let Some(org) = org.upgrade() {
                    then(&org);
                }
            },
        );
    }

    /// Insere uma página em branco depois da última página selecionada (ou
    /// no fim). O tamanho vem da página anterior quando o arquivo é salvo.
    fn insert_blank(self: &Rc<Self>) {
        if self.input.borrow().is_none() {
            return;
        }
        let at = self.selected().last().map_or(self.items.borrow().len(), |&i| i + 1);
        self.remember();
        self.items.borrow_mut().insert(at, PageItem::blank());
        self.rebuild(&[at]);
        self.toast("Página em branco inserida. Ctrl+Z desfaz.");
    }

    /// Amplia a página `pos` (ou a que tem o foco, ou a primeira selecionada).
    fn zoom(self: &Rc<Self>, pos: Option<usize>) {
        let Some(path) = self.input.borrow().clone() else { return };
        let Some(start) = pos.or_else(|| self.focused()).or_else(|| self.selected().first().copied()) else {
            self.toast("Escolha uma página para ampliar (clique duplo nela).");
            return;
        };
        let items = self.items.borrow().clone();
        let n = items.len();
        let zoom_items = items
            .iter()
            .enumerate()
            .map(|(i, it)| ZoomItem {
                page: it.page,
                rotation: it.rotation,
                label: if it.is_blank() {
                    format!("{} de {n} · em branco", i + 1)
                } else if it.page as usize == i + 1 {
                    format!("Página {} de {n}", i + 1)
                } else {
                    format!("{} de {n} · página original {}", i + 1, it.page)
                },
                texture: if it.is_blank() { self.texture_at(&items, i) } else { None },
            })
            .collect();
        if let Some(win) = self.window() {
            page_zoom::show(&win, path, zoom_items, start.min(n.saturating_sub(1)));
        }
    }

    fn selected(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.flow.selected_children().iter().map(|c| c.index() as usize).collect();
        v.sort_unstable();
        v
    }

    fn remember(&self) {
        self.history.push(self.items.borrow().clone());
    }

    fn update_count(&self) {
        let n = self.items.borrow().len();
        let sel = self.flow.selected_children().len();
        let loaded = self.textures.borrow().len() as u32;
        let mut text = format!("{n} página{}", if n == 1 { "" } else { "s" });
        if sel > 0 {
            text.push_str(&format!(" · {sel} selecionada{}", if sel == 1 { "" } else { "s" }));
        }
        if loaded < self.total.get() && self.loader.borrow().is_some() {
            text.push_str(&format!(" · miniaturas {loaded}/{}", self.total.get()));
        }
        self.count.set_text(&text);
    }

    fn need_selection(&self) -> Option<Vec<usize>> {
        let sel = self.selected();
        if sel.is_empty() {
            self.toast("Selecione uma ou mais páginas primeiro (clique; Ctrl+clique para várias).");
            None
        } else {
            Some(sel)
        }
    }

    fn rotate(self: &Rc<Self>, delta: i32) {
        let Some(sel) = self.need_selection() else { return };
        self.remember();
        for &i in &sel {
            let it = &mut self.items.borrow_mut()[i];
            it.rotation = (it.rotation + delta).rem_euclid(360);
        }
        self.rebuild(&sel);
    }

    fn duplicate(self: &Rc<Self>) {
        let Some(sel) = self.need_selection() else { return };
        self.remember();
        let mut items = self.items.borrow().clone();
        for &i in sel.iter().rev() {
            items.insert(i + 1, items[i]);
        }
        // Posições das cópias após a inserção.
        let copies: Vec<usize> = sel.iter().enumerate().map(|(k, &i)| i + k + 1).collect();
        self.items.replace(items);
        self.rebuild(&copies);
    }

    fn delete(self: &Rc<Self>) {
        let Some(sel) = self.need_selection() else { return };
        let originals_left = self.items.borrow().iter().enumerate().any(|(i, it)| !it.is_blank() && !sel.contains(&i));
        if !originals_left {
            self.toast("O documento precisa manter pelo menos uma página original.");
            return;
        }
        self.remember();
        let mut items = self.items.borrow().clone();
        for &i in sel.iter().rev() {
            items.remove(i);
        }
        let next = sel[0].min(items.len() - 1);
        self.items.replace(items);
        self.rebuild(&[next]);
        self.toast(&format!("{} página(s) excluída(s). Ctrl+Z desfaz.", sel.len()));
    }

    fn undo(self: &Rc<Self>) {
        let Some(previous) = self.history.pop() else { return };
        self.items.replace(previous);
        self.rebuild(&[]);
    }

    /// Move o bloco `positions` (em ordem) para antes (ou depois) de `to`.
    fn move_block(self: &Rc<Self>, positions: &[usize], to: usize) {
        let Some(&last) = positions.last() else { return };
        if positions.contains(&to) && positions.len() == 1 {
            return;
        }
        self.remember();
        let items = self.items.borrow().clone();
        let moving: Vec<PageItem> = positions.iter().map(|&i| items[i]).collect();
        let rest: Vec<(usize, PageItem)> =
            items.iter().copied().enumerate().filter(|(i, _)| !positions.contains(i)).collect();
        let after = to > last;
        let at = rest.iter().filter(|(i, _)| if after { *i <= to } else { *i < to }).count();
        let mut result: Vec<PageItem> = rest.iter().map(|(_, it)| *it).collect();
        for (k, it) in moving.iter().enumerate() {
            result.insert(at + k, *it);
        }
        self.items.replace(result);
        let moved: Vec<usize> = (at..at + positions.len()).collect();
        self.rebuild(&moved);
    }

    fn drag_move(self: &Rc<Self>, from: usize, to: usize) {
        let sel = self.selected();
        let block = if sel.contains(&from) { sel } else { vec![from] };
        if block.contains(&to) {
            return;
        }
        self.move_block(&block, to);
    }

    fn nudge(self: &Rc<Self>, delta: isize) {
        let Some(sel) = self.need_selection() else { return };
        let len = self.items.borrow().len();
        let to = if delta < 0 { sel[0].checked_sub(1) } else { Some(sel[sel.len() - 1] + 1).filter(|&t| t < len) };
        if let Some(to) = to {
            self.move_block(&sel, to);
        }
    }

    fn save_as(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let org = self.clone();
        glib::spawn_future_local(async move {
            let initial = org.dest_dir.borrow().clone();
            if let Some(dir) = widgets::pick_folder(&win, initial.as_deref()).await {
                org.save_to(Some(dir));
            }
        });
    }

    fn save_to(self: &Rc<Self>, dir: Option<PathBuf>) {
        if self.saving.get() {
            return;
        }
        let (Some(app), Some(input)) = (self.app.upgrade(), self.input.borrow().clone()) else { return };
        let Some(dir) = dir.or_else(|| self.dest_dir.borrow().clone()) else { return };
        // A ordem gravada (pode mudar durante a gravação).
        let pages = self.items.borrow().clone();
        let title = format!("Organizar — {}", human::file_name(&input));
        let request = Request::Organize { input, pages: pages.clone() };
        self.saving.set(true);
        self.chrome.set_saving(true);
        let org = self.clone();
        editor::save(&app, &title, request, dir, move |saved| {
            org.saving.set(false);
            org.chrome.set_saving(false);
            org.chrome.save.set_sensitive(true);
            if saved {
                org.saved.replace(pages);
                org.update_dirty();
            }
        });
    }
}
