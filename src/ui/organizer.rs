//! Organizar páginas: miniaturas, seleção múltipla, arrastar para reordenar,
//! girar, duplicar, excluir, inserir páginas em branco, ampliar e desfazer. O resultado é gravado pela operação
//! estrutural `Request::Organize` (sem rasterizar).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use pdftoolbox::catalog::{self, Accepts, ToolId};
use pdftoolbox::engines::{poppler, qpdf};
use pdftoolbox::error::display_name;
use pdftoolbox::jobs::{self, CancelToken, JobEvent};
use pdftoolbox::operations::{self, Destination, PageItem, Request};
use pdftoolbox::storage::JobDir;
use pdftoolbox::{OpError, OpResult};

use super::page_zoom::{self, ZoomItem};
use super::{AppState, primary_menu, thumbs, widgets};

const THUMB: i32 = 160;

/// Célula da grade. As células são reaproveitadas (o índice é estável);
/// recriar a grade com `remove_all()` deixa o cursor do GtkFlowBox inválido
/// e derruba o programa na próxima tecla de navegação.
struct PageCell {
    child: gtk::FlowBoxChild,
    picture: gtk::Picture,
    label: gtk::Label,
    rotation: gtk::Label,
}
const CHUNK: u32 = 12;

struct Organizer {
    app: Weak<AppState>,
    nav_page: glib::WeakRef<adw::NavigationPage>,
    input: RefCell<Option<PathBuf>>,
    total: Cell<u32>,
    items: RefCell<Vec<PageItem>>,
    /// Ordem gravada por último (ou a do arquivo aberto): diferente de
    /// `items` = alterações não salvas.
    saved: RefCell<Vec<PageItem>>,
    undo: RefCell<Vec<Vec<PageItem>>>,
    /// Folhas brancas já criadas, por tamanho em pixels.
    blanks: RefCell<HashMap<(u32, u32), gdk::Texture>>,
    textures: RefCell<HashMap<u32, gdk::Texture>>,
    cells: RefCell<Vec<PageCell>>,
    thumbs_dir: RefCell<Option<JobDir>>,
    loader: RefCell<Option<CancelToken>>,
    flow: gtk::FlowBox,
    stack: gtk::Stack,
    error_page: adw::StatusPage,
    count: gtk::Label,
    save: adw::SplitButton,
    action_bar: gtk::ActionBar,
    undo_button: gtk::Button,
    dest_dir: RefCell<Option<PathBuf>>,
    saving: Cell<bool>,
}

pub fn build(app: &Rc<AppState>, files: Vec<PathBuf>) -> adw::NavigationPage {
    let info = catalog::info(ToolId::Organize);
    let header = adw::HeaderBar::new();
    // Com alterações não salvas, a página não sai sozinha (can-pop falso):
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

    let choose = gtk::Button::builder().label("Escolher PDF…").halign(gtk::Align::Center).build();
    choose.add_css_class("pill");
    choose.add_css_class("suggested-action");
    let empty = adw::StatusPage::builder()
        .icon_name("view-grid-symbolic")
        .title("Escolha um PDF para organizar")
        .description("Ou arraste o arquivo para esta janela. Você poderá reordenar, girar, duplicar, excluir e inserir páginas em branco antes de salvar.")
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
    stack.add_named(&grid_scroll, Some("grade"));

    let action_bar = gtk::ActionBar::new();
    let rot_left = widgets::icon_button("object-rotate-left-symbolic", "Girar seleção à esquerda (Ctrl+Shift+R)");
    let rot_right = widgets::icon_button("object-rotate-right-symbolic", "Girar seleção à direita (Ctrl+R)");
    let duplicate = widgets::icon_button("edit-copy-symbolic", "Duplicar seleção (Ctrl+D)");
    let delete = widgets::icon_button("user-trash-symbolic", "Excluir seleção (Delete)");
    let blank = widgets::icon_button(
        "document-new-symbolic",
        "Inserir página em branco depois da seleção, do tamanho da página anterior (Ctrl+B)",
    );
    let zoom = widgets::icon_button("zoom-in-symbolic", "Ampliar a página (espaço ou clique duplo)");
    let move_left = widgets::icon_button("go-previous-symbolic", "Mover seleção para trás (Ctrl+←)");
    let move_right = widgets::icon_button("go-next-symbolic", "Mover seleção para frente (Ctrl+→)");
    let undo_button = widgets::icon_button("edit-undo-symbolic", "Desfazer (Ctrl+Z)");
    let select_all = widgets::icon_button("edit-select-all-symbolic", "Selecionar todas (Ctrl+A)");
    for b in [&rot_left, &rot_right, &duplicate, &delete, &blank] {
        action_bar.pack_start(b);
    }
    action_bar.pack_start(&gtk::Separator::new(gtk::Orientation::Vertical));
    action_bar.pack_start(&move_left);
    action_bar.pack_start(&move_right);
    action_bar.pack_start(&gtk::Separator::new(gtk::Orientation::Vertical));
    action_bar.pack_start(&zoom);
    let count = gtk::Label::new(None);
    count.add_css_class("dim-label");
    count.set_ellipsize(gtk::pango::EllipsizeMode::End);
    action_bar.set_center_widget(Some(&count));
    action_bar.pack_end(&select_all);
    action_bar.pack_end(&undo_button);
    action_bar.set_revealed(false);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    toolbar.add_bottom_bar(&action_bar);
    let nav_page = adw::NavigationPage::with_tag(&toolbar, info.title, info.key);
    nav_page.bind_property("can-pop", &back, "visible").invert_boolean().sync_create().build();

    let org = Rc::new(Organizer {
        app: Rc::downgrade(app),
        nav_page: nav_page.downgrade(),
        input: RefCell::new(None),
        total: Cell::new(0),
        items: RefCell::new(Vec::new()),
        saved: RefCell::new(Vec::new()),
        undo: RefCell::new(Vec::new()),
        blanks: RefCell::new(HashMap::new()),
        textures: RefCell::new(HashMap::new()),
        cells: RefCell::new(Vec::new()),
        thumbs_dir: RefCell::new(None),
        loader: RefCell::new(None),
        flow: flow.clone(),
        stack,
        error_page,
        count,
        save: save.clone(),
        action_bar,
        undo_button: undo_button.clone(),
        dest_dir: RefCell::new(None),
        saving: Cell::new(false),
    });

    // Ações da página e atalhos
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: fn(&Rc<Organizer>)| {
        let action = gio::SimpleAction::new(name, None);
        let o = Rc::downgrade(&org);
        action.connect_activate(move |_, _| {
            if let Some(o) = o.upgrade() {
                f(&o);
            }
        });
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
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Global);
    for (accel, action) in [
        ("<Control>s", "page.run"),
        ("<Control>r", "page.rotate-right"),
        ("<Control><Shift>r", "page.rotate-left"),
        ("<Control>d", "page.duplicate"),
        ("Delete", "page.delete"),
        ("<Control>z", "page.undo"),
        ("<Control>a", "page.select-all"),
        ("<Control>b", "page.insert-blank"),
        ("Insert", "page.insert-blank"),
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
    // Ctrl+setas precisam ser capturadas antes das teclas internas do FlowBox,
    // que as usam para mover o cursor sem alterar a seleção.
    let moves = gtk::ShortcutController::new();
    moves.set_propagation_phase(gtk::PropagationPhase::Capture);
    // Espaço amplia a página (o FlowBox o usaria para selecionar; Enter continua).
    for (accel, action) in
        [("<Control>Left", "page.move-left"), ("<Control>Right", "page.move-right"), ("space", "page.zoom")]
    {
        moves.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(accel),
            Some(gtk::NamedAction::new(action)),
        ));
    }
    flow.add_controller(moves);

    for (b, action) in [
        (&rot_left, "page.rotate-left"),
        (&rot_right, "page.rotate-right"),
        (&duplicate, "page.duplicate"),
        (&delete, "page.delete"),
        (&blank, "page.insert-blank"),
        (&zoom, "page.zoom"),
        (&move_left, "page.move-left"),
        (&move_right, "page.move-right"),
        (&undo_button, "page.undo"),
        (&select_all, "page.select-all"),
    ] {
        b.set_action_name(Some(action));
    }
    save.set_action_name(Some("page.run"));
    for b in [&open, &choose, &error_choose] {
        b.set_action_name(Some("page.add"));
    }
    // Clique duplo (ou Enter) numa página: ampliar.
    let o = Rc::downgrade(&org);
    flow.connect_child_activated(move |_, child| {
        if let Some(o) = o.upgrade() {
            o.zoom(Some(child.index() as usize));
        }
    });
    let o = Rc::downgrade(&org);
    flow.connect_selected_children_changed(move |_| {
        if let Some(o) = o.upgrade() {
            o.update_count();
        }
    });
    let o = Rc::downgrade(&org);
    widgets::add_file_drop(&nav_page, move |files| {
        if let Some(o) = o.upgrade()
            && let Some(f) = files.into_iter().find(|f| catalog::extension_ok(Accepts::Pdf, f))
        {
            o.confirm_discard(move |o| o.load(f.clone()));
        }
    });
    // Ao sair da página, interrompe as miniaturas e apaga os temporários.
    let o = Rc::downgrade(&org);
    app.nav.connect_popped(move |_, popped| {
        if let Some(o) = o.upgrade()
            && o.nav_page.upgrade().as_ref() == Some(popped)
        {
            o.stop_loader();
            o.thumbs_dir.borrow_mut().take();
        }
    });

    org.update_count();
    if let Some(f) = files.into_iter().next() {
        org.load(f);
    }
    let keep = org.clone();
    nav_page.connect_destroy(move |_| {
        let _ = &keep;
    });
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
        let o = self.clone();
        glib::spawn_future_local(async move {
            if let Some(f) = widgets::pick_files(&win, Some(Accepts::Pdf), false).await.into_iter().next() {
                o.load(f);
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
        self.stack.set_visible_child_name("carregando");
        self.action_bar.set_revealed(false);
        self.save.set_sensitive(false);
        let o = self.clone();
        glib::spawn_future_local(async move {
            let p = path.clone();
            let result = widgets::blocking(move || -> OpResult<(u32, JobDir)> {
                let n = qpdf::page_count(&p, &CancelToken::new())?;
                Ok((n, JobDir::new()?))
            })
            .await;
            match result {
                Ok((n, dir)) => o.start(path, n, dir),
                Err(e) => {
                    o.error_page.set_title(e.title());
                    o.error_page.set_description(Some(&glib::markup_escape_text(&e.to_string())));
                    o.stack.set_visible_child_name("erro");
                }
            }
        });
    }

    fn start(self: &Rc<Self>, path: PathBuf, total: u32, dir: JobDir) {
        if let Some(page) = self.nav_page.upgrade() {
            page.set_title(&format!("Organizar — {}", display_name(&path)));
        }
        *self.dest_dir.borrow_mut() = path.parent().map(Path::to_path_buf);
        self.total.set(total);
        *self.items.borrow_mut() = (1..=total).map(|page| PageItem { page, rotation: 0 }).collect();
        *self.saved.borrow_mut() = self.items.borrow().clone();
        self.undo.borrow_mut().clear();
        self.textures.borrow_mut().clear();
        let thumbs = dir.path().to_path_buf();
        *self.thumbs_dir.borrow_mut() = Some(dir);
        *self.input.borrow_mut() = Some(path.clone());
        self.rebuild(&[]);
        self.stack.set_visible_child_name("grade");
        self.action_bar.set_revealed(true);
        self.save.set_sensitive(true);

        // Miniaturas em blocos, numa thread própria, com cancelamento.
        let cancel = CancelToken::new();
        *self.loader.borrow_mut() = Some(cancel.clone());
        let (tx, rx) = async_channel::unbounded::<Vec<(u32, PathBuf)>>();
        std::thread::spawn(move || {
            let mut first = 1;
            while first <= total && !cancel.is_cancelled() {
                let last = (first + CHUNK - 1).min(total);
                match poppler::thumbnails(&path, first, last, THUMB as u32, &thumbs, &cancel) {
                    Ok(files) => {
                        let batch = (first..=last).zip(files).collect();
                        if tx.send_blocking(batch).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
                first = last + 1;
            }
        });
        let o = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(batch) = rx.recv().await {
                let Some(o) = o.upgrade() else { break };
                for (page, file) in batch {
                    if let Ok(tex) = gdk::Texture::from_filename(&file) {
                        o.textures.borrow_mut().insert(page, tex);
                    }
                    let _ = std::fs::remove_file(file);
                }
                o.refresh_pictures();
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
        let o = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(o), Ok(from)) = (o.upgrade(), value.get::<u32>()) else { return false };
            o.drag_move(from as usize, pos);
            true
        });
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

    fn rebuild(self: &Rc<Self>, select: &[usize]) {
        let items = self.items.borrow().clone();
        loop {
            let len = self.cells.borrow().len();
            if len >= items.len() {
                break;
            }
            let cell = self.make_cell(len);
            self.flow.append(&cell.child);
            self.cells.borrow_mut().push(cell);
        }
        while self.cells.borrow().len() > items.len() {
            let cell = self.cells.borrow_mut().pop().expect("célula");
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
        let focused = self.flow.focus_child().and_downcast::<gtk::FlowBoxChild>().map(|c| c.index() as usize);
        let target = match focused {
            Some(i) if select.contains(&i) => Some(i),
            _ => select.first().copied(),
        };
        if let Some(pos) = target
            && let Some(cell) = self.cells.borrow().get(pos)
        {
            cell.child.grab_focus();
        }
        self.undo_button.set_sensitive(!self.undo.borrow().is_empty());
        self.update_count();
        self.update_dirty();
    }

    fn dirty(&self) -> bool {
        *self.items.borrow() != *self.saved.borrow()
    }

    fn update_dirty(&self) {
        if let Some(page) = self.nav_page.upgrade() {
            page.set_can_pop(!self.dirty());
        }
    }

    /// Volta à tela anterior, confirmando se há alterações não salvas.
    fn leave(self: &Rc<Self>) {
        self.confirm_discard(|o| {
            let saved = o.saved.borrow().clone();
            *o.items.borrow_mut() = saved;
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
        let dialog = adw::AlertDialog::new(
            Some("Descartar as alterações?"),
            Some("A nova ordem das páginas ainda não foi salva. O PDF original não foi alterado."),
        );
        dialog.add_response("cancel", "Continuar organizando");
        dialog.add_response("discard", "Descartar");
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let o = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response == "discard"
                && let Some(o) = o.upgrade()
            {
                then(&o);
            }
        });
        dialog.present(Some(&app.window));
    }

    /// Insere uma página em branco depois da última página selecionada (ou
    /// no fim). O tamanho vem da página anterior quando o arquivo é salvo.
    fn insert_blank(self: &Rc<Self>) {
        if self.input.borrow().is_none() {
            return;
        }
        let at = self.selected().last().map(|&i| i + 1).unwrap_or(self.items.borrow().len());
        self.remember();
        self.items.borrow_mut().insert(at, PageItem::blank());
        self.rebuild(&[at]);
        self.toast("Página em branco inserida. Ctrl+Z desfaz.");
    }

    /// Amplia a página `pos` (ou a que tem o foco, ou a primeira selecionada).
    fn zoom(self: &Rc<Self>, pos: Option<usize>) {
        let Some(path) = self.input.borrow().clone() else { return };
        let focused = self.flow.focus_child().and_downcast::<gtk::FlowBoxChild>().map(|c| c.index() as usize);
        let Some(start) = pos.or(focused).or_else(|| self.selected().first().copied()) else {
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
        let snapshot = self.items.borrow().clone();
        let mut undo = self.undo.borrow_mut();
        undo.push(snapshot);
        if undo.len() > 100 {
            undo.remove(0);
        }
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
        let mut new_sel = Vec::new();
        for &i in sel.iter().rev() {
            items.insert(i + 1, items[i]);
        }
        // Posições das cópias após a inserção.
        for (k, &i) in sel.iter().enumerate() {
            new_sel.push(i + k + 1);
        }
        *self.items.borrow_mut() = items;
        self.rebuild(&new_sel);
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
        *self.items.borrow_mut() = items;
        self.rebuild(&[next]);
        self.toast(&format!("{} página(s) excluída(s). Ctrl+Z desfaz.", sel.len()));
    }

    fn undo(self: &Rc<Self>) {
        let Some(prev) = self.undo.borrow_mut().pop() else { return };
        *self.items.borrow_mut() = prev;
        self.rebuild(&[]);
    }

    /// Move o bloco `positions` para antes (ou depois) de `to`.
    fn move_block(self: &Rc<Self>, positions: &[usize], to: usize) {
        if positions.is_empty() || positions.contains(&to) && positions.len() == 1 {
            return;
        }
        self.remember();
        let items = self.items.borrow().clone();
        let moving: Vec<PageItem> = positions.iter().map(|&i| items[i]).collect();
        let rest: Vec<(usize, PageItem)> =
            items.iter().copied().enumerate().filter(|(i, _)| !positions.contains(i)).collect();
        let after = to > *positions.last().unwrap();
        let at = rest.iter().filter(|(i, _)| if after { *i <= to } else { *i < to }).count();
        let mut result: Vec<PageItem> = rest.iter().map(|(_, it)| *it).collect();
        for (k, it) in moving.iter().enumerate() {
            result.insert(at + k, *it);
        }
        *self.items.borrow_mut() = result;
        let new_sel: Vec<usize> = (at..at + positions.len()).collect();
        self.rebuild(&new_sel);
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
        let to = if delta < 0 {
            match sel[0].checked_sub(1) {
                Some(t) => t,
                None => return,
            }
        } else {
            let t = sel[sel.len() - 1] + 1;
            if t >= len {
                return;
            }
            t
        };
        self.move_block(&sel, to);
    }

    fn save_as(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let o = self.clone();
        glib::spawn_future_local(async move {
            let initial = o.dest_dir.borrow().clone();
            if let Some(dir) = widgets::pick_folder(&win, initial.as_deref()).await {
                o.save_to(Some(dir));
            }
        });
    }

    fn save_to(self: &Rc<Self>, dir: Option<PathBuf>) {
        if self.saving.get() {
            return;
        }
        let (Some(app), Some(input)) = (self.app.upgrade(), self.input.borrow().clone()) else { return };
        let Some(dir) = dir.or_else(|| self.dest_dir.borrow().clone()) else { return };
        let pages = self.items.borrow().clone();
        let dest = Destination { dir, name: None };
        let request = Request::Organize { input: input.clone(), pages: pages.clone() };
        let handle = jobs::spawn(move |ctx| operations::execute(request, &dest, ctx));
        let task = app.tasks.add(&format!("Organizar — {}", display_name(&input)), handle.cancel.clone());
        self.saving.set(true);
        self.save.set_sensitive(false);
        self.save.set_label("Salvando…");
        let o = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(ev) = handle.events.recv().await {
                match ev {
                    JobEvent::Queued => {}
                    JobEvent::Progress(f, t) => task.progress(f, &t),
                    JobEvent::Finished(result) => {
                        o.saving.set(false);
                        o.save.set_sensitive(true);
                        o.save.set_label("Salvar");
                        o.finished(result, &task, pages);
                        break;
                    }
                }
            }
        });
    }

    /// `pages`: a ordem que foi gravada (pode ter mudado durante a gravação).
    fn finished(&self, result: OpResult<operations::Outcome>, task: &super::tasks::TaskRow, pages: Vec<PageItem>) {
        let Some(app) = self.app.upgrade() else { return };
        match result {
            Ok(outcome) if !outcome.outputs.is_empty() => {
                *self.saved.borrow_mut() = pages;
                self.update_dirty();
                let path = outcome.outputs[0].path.clone();
                task.finish(&format!("Concluído: {}", display_name(&path)), Some(path.clone()));
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
                show_error_dialog(&app.window, &e);
            }
        }
    }
}

fn show_error_dialog(win: &adw::ApplicationWindow, e: &OpError) {
    let dialog = adw::AlertDialog::new(Some(e.title()), Some(&e.to_string()));
    if matches!(e, OpError::ToolFailed { .. } | OpError::Io { .. } | OpError::OutputInvalid(_)) {
        let details = gtk::Label::builder().label(e.technical()).wrap(true).selectable(true).xalign(0.0).build();
        details.add_css_class("monospace");
        details.add_css_class("caption");
        let exp = gtk::Expander::builder().label("Detalhes técnicos").child(&details).build();
        dialog.set_extra_child(Some(&exp));
    }
    dialog.add_response("ok", "Fechar");
    dialog.present(Some(win));
}
