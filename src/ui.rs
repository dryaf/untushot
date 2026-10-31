use crate::{
    autosave, capture,
    document::{self, Alignment, Annotation, Document, Tool},
    geometry::{Layout, Point, Rect},
    icons,
    monitor_map::MonitorMap,
    preferences::Preferences,
    recovery, selection,
    setup::{Mode, Setup},
    Args,
};
use anyhow::Result;
use gtk::prelude::*;
#[cfg(test)]
mod autosave_tests;
mod dialogs;
mod layer_drag;
mod layer_rename;
#[cfg(test)]
mod portal_tests;
#[cfg(test)]
mod showcase;
mod text_history;
#[cfg(test)]
mod ux_tests;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy)]
struct View {
    x: f64,
    y: f64,
    scale: f64,
}
impl View {
    fn layout(bounds: Rect, w: f64, h: f64, scale: f64, pan: Point) -> Self {
        Self {
            x: ((w - bounds.w * scale) / 2.).max(0.) - bounds.x * scale + pan.x,
            y: ((h - bounds.h * scale) / 2.).max(0.) - bounds.y * scale + pan.y,
            scale,
        }
    }
    fn point(self, x: f64, y: f64, bounds: Rect) -> Point {
        let p = self.unclamped_point(x, y);
        Point::new(
            p.x.clamp(bounds.x, bounds.right()),
            p.y.clamp(bounds.y, bounds.bottom()),
        )
    }
    fn unclamped_point(self, x: f64, y: f64) -> Point {
        Point::new((x - self.x) / self.scale, (y - self.y) / self.scale)
    }
    fn contains(self, x: f64, y: f64, bounds: Rect) -> bool {
        x >= self.x + bounds.x * self.scale
            && y >= self.y + bounds.y * self.scale
            && x < self.x + bounds.right() * self.scale
            && y < self.y + bounds.bottom() * self.scale
    }
}

struct State {
    document: Option<Document>,
    tool: Tool,
    color: (f64, f64, f64),
    width: f64,
    font_size: f64,
    pan: Point,
    mouse: Option<Point>,
    space: bool,
    pan_drag: Option<(Point, Point)>,
    project_state: Option<u64>,
    export_state: Option<u64>,
    draft: Option<Annotation>,
    view: View,
    zoom: Option<f64>,
    capturing: bool,
    fullscreen: bool,
    guides: bool,
    snap_bypass: bool,
    selected: Option<usize>,
    drag: Option<Drag>,
    selecting_region: bool,
    region_bounds: Option<Rect>,
    dirty: bool,
    generation: u64,
    selected_many: Vec<usize>,
    crop_drag: Option<selection::Drag>,
    pixel_selection: Option<Rect>,
    pointer: Option<Point>,
}

struct Drag {
    start: Point,
    originals: Vec<(usize, Annotation)>,
    previews: Vec<(usize, Annotation)>,
    handle: selection::Handle,
    guides: Vec<crate::snapping::Guide>,
    snap_targets: Vec<Rect>,
}

impl State {
    fn object_handle(&self, x: f64, y: f64) -> Option<(usize, selection::Handle)> {
        if self.tool != Tool::Move || self.indices().len() != 1 {
            return None;
        }
        let index = self.selected?;
        let annotation = self.document.as_ref()?.annotations().get(index)?;
        if !annotation.visible {
            return None;
        }
        selection::hit_handle(
            annotation.bounds(),
            self.view.unclamped_point(x, y),
            8. / self.view.scale,
        )
        .map(|handle| (index, handle))
    }
    fn indices(&self) -> Vec<usize> {
        if self.selected_many.is_empty() {
            self.selected.into_iter().collect()
        } else {
            self.selected_many.clone()
        }
    }
    fn canvas_bounds(&self) -> Option<Rect> {
        self.document.as_ref().map(|d| {
            if self.selecting_region {
                self.region_bounds.unwrap_or(d.bounds())
            } else if self.tool == Tool::Select {
                d.bounds()
            } else {
                d.selection().unwrap_or(d.bounds())
            }
        })
    }
}

struct TextEdit {
    history: text_history::History,
    index: Option<usize>,
    annotation: Annotation,
}

struct LayerRow {
    row: gtk::ListBoxRow,
    thumbnail: gtk::Image,
    label: gtk::Label,
    title: gtk::Stack,
    visibility: gtk::Button,
    revision: Rc<Cell<u64>>,
    content: Option<(Annotation, i32)>,
}

pub(crate) struct Editor {
    error_dialog: RefCell<Option<gtk::MessageDialog>>,
    shortcuts_dialog: RefCell<Option<gtk::Dialog>>,
    close_to_tray: gtk::CheckButton,
    close_notice: gtk::InfoBar,
    close_notice_text: gtk::Label,
    property_scroll: gtk::ScrolledWindow,
    auto_layers_hidden: Cell<bool>,
    layers_manually_shown: Cell<bool>,
    layer_drag: RefCell<Option<layer_drag::Session>>,
    layer_rename: Cell<Option<(usize, u64, u64)>>,
    layer_drag_serial: Cell<u64>,
    layer_gestures: RefCell<Vec<gtk::Gesture>>,
    layer_rows: RefCell<Vec<LayerRow>>,
    layer_selection: RefCell<Vec<usize>>,
    reveal_layer: Cell<bool>,
    snapping: gtk::CheckButton,
    object_menu: RefCell<Option<gtk::Menu>>,
    property_edit: Cell<Option<(usize, usize, u64)>>,
    property_refresh: Cell<Option<usize>>,
    shadow_grid: gtk::Grid,
    clipboard_epoch: Cell<u64>,
    clipboard_pending: Cell<bool>,
    selection_bar: gtk::Box,
    copy_selection_layer: gtk::Button,
    copy_selection: gtk::Button,
    paste_layer_button: gtk::Button,
    image_effects: gtk::Box,
    border_enabled: gtk::CheckButton,
    border_width: gtk::SpinButton,
    border_color: crate::color::Picker,
    shadow_enabled: gtk::CheckButton,
    shadow_blur: gtk::SpinButton,
    shadow_offset: gtk::SpinButton,
    shadow_opacity: gtk::SpinButton,
    weak_self: std::rc::Weak<Editor>,
    layer_previews: RefCell<crate::preview::LayerPreviews>,
    window: gtk::ApplicationWindow,
    header: gtk::Box,
    titlebar: gtk::HeaderBar,
    export_actions: gtk::Box,
    context: gtk::Box,
    file_menu: gtk::MenuButton,
    save_as: gtk::Button,
    export_button: gtk::Button,
    project_save_button: gtk::Button,
    export_path: RefCell<Option<PathBuf>>,
    project_path: RefCell<Option<PathBuf>>,
    autosave_target: RefCell<Option<std::sync::Arc<autosave::Target>>>,
    autosave_source: RefCell<Option<PathBuf>>,
    automatic_project: Cell<bool>,
    autosave_existing: Cell<bool>,
    recovery_content: Cell<Option<u64>>,
    project_as: gtk::Button,
    recent: Rc<crate::history::Strip>,
    new_capture: gtk::Button,
    history_controls: gtk::Box,
    layer_actions: gtk::Box,
    layers_heading: gtk::Label,
    empty_layers: gtk::Box,
    width_controls: gtk::Box,
    width_picker: gtk::MenuButton,
    storage_button: gtk::MenuButton,
    storage_caption: gtk::Label,
    region_instruction: gtk::Label,
    preview: RefCell<crate::preview::Preview>,
    cursor: Cell<Option<gdk::CursorType>>,
    area: gtk::DrawingArea,
    status: gtk::Label,
    dimensions: gtk::Label,
    monitors: gtk::ComboBoxText,
    undo: gtk::Button,
    redo: gtk::Button,
    save: gtk::Button,
    copy: gtk::Button,
    paste: gtk::Button,
    append_image: gtk::Button,
    base: gtk::Box,
    tools: Vec<gtk::RadioButton>,
    state: RefCell<State>,
    args: RefCell<Args>,
    stack: gtk::Stack,
    setup: Setup,
    sidebar: gtk::Box,
    inspector: gtk::Box,
    region_bar: gtk::Box,
    confirm_region: gtk::Button,
    project: gtk::Button,
    layers: gtk::ListBox,
    layer_scroll: gtk::ScrolledWindow,
    color: crate::color::Picker,
    color_controls: gtk::Box,
    monitor_guides: gtk::CheckButton,
    width: gtk::SpinButton,
    properties: [gtk::SpinButton; 4],
    visible: gtk::CheckButton,
    edit_text: gtk::Button,
    delete: gtk::Button,
    syncing: Cell<bool>,
    left_panel: gtk::ScrolledWindow,
    right_panel: gtk::Box,
    workspace_split: gtk::Paned,
    history_canvas: gtk::Box,
    history_setup: gtk::Box,
    layer_width_restore_serial: Cell<u64>,
    show_tools: gtk::ToggleButton,
    show_layers: gtk::ToggleButton,
    magnifier: gtk::CheckButton,
    monitor_map: MonitorMap,
    monitor_button: gtk::MenuButton,
    layer_name: gtk::Entry,
    alignment: gtk::ComboBoxText,
    align: gtk::Button,
    duplicate: gtk::Button,
    object_properties: gtk::Box,
    selection_label: gtk::Label,
    session: RefCell<Option<recovery::Session>>,
    recovery_revision: Cell<Option<u64>>,
    recovery_ticket: Cell<Option<u64>>,
    recovery_ack: Cell<Option<u64>>,
    recovery_error: RefCell<Option<String>>,
    storage: gtk::Label,
    font_size: gtk::SpinButton,
    font_picker: gtk::MenuButton,
    font_controls: gtk::Box,
    style_label: gtk::Label,
    inline_scale: Cell<f64>,
    inline_box: gtk::Box,
    inline_input: gtk::TextView,
    inline_css: gtk::CssProvider,
    inline_edit: RefCell<Option<TextEdit>>,
    preferences: RefCell<Preferences>,
    preferences_path: RefCell<Option<PathBuf>>,
    preferences_saved: RefCell<Option<Preferences>>,
    closed: Cell<bool>,
    resident: Cell<bool>,
    capture_cancel: RefCell<Option<capture::Cancellation>>,
}

fn button(label: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.set_tooltip_text(Some(tooltip));
    button.set_can_focus(true);
    button.set_focus_on_click(false);
    button
}

fn icon_button(icon: &'static str, label: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::new();
    if label.is_empty() {
        icons::icon_control(&b);
    } else {
        b.style_context().add_class("text-button");
    }
    b.set_tooltip_text(Some(tooltip));
    b.set_can_focus(true);
    b.set_focus_on_click(false);
    icons::accessible_name(&b, if label.is_empty() { tooltip } else { label });
    b.add(&icons::button_content(icon, label));
    b
}

fn caption(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.style_context().add_class("caption");
    label
}

pub(crate) fn launch(app: &gtk::Application, args: Args) -> Rc<Editor> {
    let restore = args.open.is_none() && !args.demo && args.capture.is_none();
    let demo = args.demo;
    let editor = build(app, args);
    if !demo {
        editor.load_preferences(&Preferences::path());
        match recovery::Session::open(&recovery::default_directory(), restore) {
            Ok((session, document)) => {
                let restored_autosave = session.restored_autosave.clone();
                *editor.session.borrow_mut() = Some(session);
                if let Some(doc) = document {
                    editor.set_document(
                        doc,
                        crate::i18n::tr(
                            "Letzte Sitzung wiederhergestellt · Original, Ebenen und Ausschnitt",
                        ),
                    );
                }
                if let Some(target) = restored_autosave
                    .as_ref()
                    .and_then(|l| autosave::Target::restore(l).ok())
                {
                    editor.project_path.replace(Some(target.path.clone()));
                    editor
                        .autosave_source
                        .replace(restored_autosave.unwrap().source);
                    editor.autosave_target.replace(Some(target));
                    editor.automatic_project.set(true);
                    editor.autosave_existing.set(true);
                }
                if let Err(error) = editor.save_recovery() {
                    editor
                        .status
                        .set_text(&crate::i18n::format!("Autosicherung: {error:#}"));
                }
            }
            Err(error) => editor.error(error),
        }
        let weak = Rc::downgrade(&editor);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(e) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if e.closed.get() {
                return glib::ControlFlow::Break;
            }
            e.poll_recovery();
            e.sync_monitor_layout(capture::desktop_layout());
            if let Err(error) = e.save_recovery() {
                *e.recovery_error.borrow_mut() = Some(crate::i18n::format!("{error:#}"));
            }
            if let Err(error) = e.save_preferences() {
                e.status.set_text(&crate::i18n::format!(
                    "Einstellungen konnten nicht gespeichert werden: {error:#}"
                ));
            }
            e.update_storage();
            glib::ControlFlow::Continue
        });
    }
    editor
}

fn build(app: &gtk::Application, args: Args) -> Rc<Editor> {
    let css = gtk::CssProvider::new();
    css.load_from_data(include_bytes!("style.css"))
        .expect("valid CSS");
    if let Some(screen) = gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let window = gtk::ApplicationWindow::new(app);
    window.set_title("UntuShot");
    window.set_default_size(1320, 860);
    window.set_widget_name("untushot");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let titlebar = gtk::HeaderBar::new();
    titlebar.set_show_close_button(true);
    titlebar.set_title(Some("UntuShot"));
    window.set_titlebar(Some(&titlebar));
    let file_menu = gtk::MenuButton::new();
    file_menu.add(&icons::button_content("menu", ""));
    icons::icon_control(&file_menu);
    file_menu.set_tooltip_text(Some(crate::i18n::tr(
        "Datei · Öffnen, Speichern und Exportieren",
    )));
    titlebar.pack_start(&file_menu);
    let file_popover = gtk::Popover::new(Some(&file_menu));
    let file_actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
    file_actions.set_border_width(8);
    let file_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    file_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    file_scroll.set_propagate_natural_width(true);
    file_scroll.set_propagate_natural_height(true);
    file_scroll.set_min_content_height(180);
    file_scroll.set_max_content_height(480);
    file_scroll.set_overlay_scrolling(false);
    file_scroll.add(&file_actions);
    file_popover.add(&file_scroll);
    file_menu.set_popover(Some(&file_popover));
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    titlebar.pack_start(&header);
    let export_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    titlebar.pack_end(&export_actions);
    let new = icon_button(
        "camera",
        crate::i18n::tr("Neue Aufnahme"),
        crate::i18n::tr("Strg+N · Aufnahme einrichten"),
    );
    let open = icon_button(
        "open",
        crate::i18n::tr("Bild oder Projekt öffnen …"),
        crate::i18n::tr("Bild oder bearbeitbares Projekt öffnen · Strg+O"),
    );
    header.pack_start(&new, false, false, 0);
    file_actions.pack_start(&open, false, false, 0);
    let open_project = icon_button(
        "open",
        crate::i18n::tr("Projekt öffnen …"),
        crate::i18n::tr("Gespeichertes Projekt mit allen bearbeitbaren Ebenen öffnen"),
    );
    let project = icon_button(
        "save",
        crate::i18n::tr("Bearbeitbares Projekt speichern"),
        crate::i18n::tr(
            "Zum Weiterbearbeiten: geöffnetes Projekt aktualisieren oder Ziel wählen · Strg+Alt+S",
        ),
    );
    let project_as = icon_button(
        "save",
        crate::i18n::tr("Bearbeitbares Projekt speichern unter …"),
        crate::i18n::tr("Bearbeitung in einer neuen Projektdatei speichern · Strg+Alt+Umschalt+S"),
    );
    let copy = icon_button(
        "copy",
        crate::i18n::tr("Kopieren"),
        crate::i18n::tr("Bild in die Zwischenablage kopieren · Strg+C"),
    );
    let export_button = icon_button(
        "export",
        crate::i18n::tr("Bild exportieren"),
        crate::i18n::tr("Fertiges Bild als PNG exportieren · Strg+S"),
    );
    export_button.style_context().add_class("suggested-action");
    let paste = icon_button(
        "paste",
        crate::i18n::tr("Bild als neues Dokument …"),
        crate::i18n::tr("Zwischenablage mit Vorschau als neues Dokument öffnen · Strg+Umschalt+V"),
    );
    let paste_layer_button = icon_button(
        "paste",
        crate::i18n::tr("Als neue Ebene einfügen"),
        crate::i18n::tr("Bild aus der Zwischenablage als Ebene einfügen · Strg+V"),
    );
    let append_image = icon_button(
        "image",
        crate::i18n::tr("Bild rechts anfügen …"),
        crate::i18n::tr("PNG oder JPEG als eigene Bildebene anfügen · Strg+I"),
    );
    let save = icon_button(
        "export",
        crate::i18n::tr("Bild exportieren"),
        crate::i18n::tr("Bestätigten PNG-Export aktualisieren, sonst Ziel wählen · Strg+S"),
    );
    let save_as = icon_button(
        "export",
        crate::i18n::tr("Bild exportieren unter …"),
        crate::i18n::tr("Neues PNG-Ziel wählen · Strg+Umschalt+S"),
    );
    for action in [
        &open,
        &open_project,
        &paste,
        &paste_layer_button,
        &append_image,
        &save,
        &save_as,
        &project,
        &project_as,
    ] {
        action.style_context().add_class("flat");
        action.style_context().add_class("file-action");
        action.child().unwrap().set_halign(gtk::Align::Fill);
        let popover = file_popover.clone();
        action.connect_clicked(move |_| popover.popdown());
    }
    file_actions.pack_start(&paste, false, false, 0);
    file_actions.pack_start(&paste_layer_button, false, false, 0);
    file_actions.pack_start(&append_image, false, false, 0);
    file_actions.pack_start(&save, false, false, 0);
    file_actions.pack_start(&save_as, false, false, 0);
    file_actions.pack_start(
        &gtk::Separator::new(gtk::Orientation::Horizontal),
        false,
        false,
        6,
    );
    file_actions.pack_start(&open_project, false, false, 0);
    file_actions.pack_start(&project, false, false, 0);
    file_actions.pack_start(&project_as, false, false, 0);
    file_actions.pack_start(
        &gtk::Separator::new(gtk::Orientation::Horizontal),
        false,
        false,
        6,
    );
    let close_window = icon_button(
        "close",
        crate::i18n::tr("Fenster schließen"),
        crate::i18n::tr(
            "Strg+W · Schließt die Anwendung, sofern Hintergrundbetrieb nicht aktiviert ist",
        ),
    );
    let quit_app = icon_button(
        "quit",
        crate::i18n::tr("UntuShot beenden"),
        crate::i18n::tr("Strg+Q · Sitzung sichern und vollständig beenden"),
    );
    for action in [&close_window, &quit_app] {
        action.style_context().add_class("flat");
        action.style_context().add_class("file-action");
        action.child().unwrap().set_halign(gtk::Align::Fill);
        file_actions.pack_start(action, false, false, 0);
        let popover = file_popover.clone();
        action.connect_clicked(move |_| popover.popdown());
    }
    let close_to_tray = gtk::CheckButton::with_label(crate::i18n::tr(
        "Beim Schließen im Hintergrund weiterlaufen",
    ));
    close_to_tray.set_tooltip_text(Some(crate::i18n::tr("Aufnahmekürzel bleiben aktiv. Mit „UntuShot beenden“ wird die Anwendung vollständig geschlossen.")));
    file_actions.pack_start(&close_to_tray, false, false, 6);
    let shortcuts = icon_button(
        "text",
        crate::i18n::tr("Tastenkürzel …"),
        crate::i18n::tr("Tastenkürzel anzeigen · F1"),
    );
    file_actions.pack_start(&shortcuts, false, false, 0);
    file_scroll.show_all();
    export_actions.pack_start(&copy, false, false, 0);
    export_actions.pack_start(&export_button, false, false, 0);
    let setup = Setup::new(&args);
    let capture_options = icon_button(
        "timer",
        crate::i18n::tr("Aufnahmeoptionen …"),
        crate::i18n::tr("Bildschirm und Verzögerung"),
    );
    capture_options.style_context().add_class("file-action");
    file_actions.pack_start(&capture_options, false, false, 0);
    file_actions.pack_start(
        &gtk::Separator::new(gtk::Orientation::Horizontal),
        false,
        false,
        6,
    );
    for action in [&setup.web, &setup.demo] {
        action.style_context().add_class("file-action");
        file_actions.pack_start(action, false, false, 0);
        let popover = file_popover.clone();
        action.connect_clicked(move |_| popover.popdown());
    }
    file_actions.show_all();
    icons::name_controls(file_actions.upcast_ref());
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::None);
    let setup_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    setup_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    setup_scroll.add(&setup.root);
    stack.add_named(&setup_scroll, "setup");
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let context = gtk::Box::new(gtk::Orientation::Vertical, 6);
    context.set_border_width(10);
    context.set_widget_name("contextbar");
    let source_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    context.pack_start(&source_row, false, false, 0);
    source_row.pack_start(
        &icons::content("crop", crate::i18n::tr("Ausschnitt")),
        false,
        false,
        0,
    );
    let monitors = crate::setup::source_selector(160, 36);
    monitors.set_tooltip_text(Some(crate::i18n::tr(
        "Ganzes Bild oder Monitor wählen. Frei zuschneiden mit Werkzeug 1.",
    )));
    source_row.pack_start(&monitors, true, true, 0);
    let monitor_map = MonitorMap::new(Vec::new());
    let map_button = gtk::MenuButton::new();
    map_button.add(&icons::button_content("monitor", ""));
    icons::icon_control(&map_button);
    map_button.set_tooltip_text(Some(crate::i18n::tr("Grafische Monitorübersicht")));
    let popover = gtk::Popover::new(Some(&map_button));
    popover.add(&monitor_map.area);
    monitor_map.area.show();
    map_button.set_popover(Some(&popover));
    map_button.style_context().add_class("flat");
    source_row.pack_start(&map_button, false, false, 0);
    let show_tools = gtk::ToggleButton::new();
    show_tools.add(&icons::button_content("pen", ""));
    icons::icon_control(&show_tools);
    show_tools.set_active(true);
    show_tools.set_tooltip_text(Some(crate::i18n::tr("Werkzeuge ein-/ausklappen · F8")));
    let show_layers = gtk::ToggleButton::new();
    show_layers.add(&icons::button_content("layers", ""));
    icons::icon_control(&show_layers);
    show_layers.set_active(true);
    show_layers.set_tooltip_text(Some(crate::i18n::tr(
        "Ebenen ein-/ausklappen · F9 · Strg+F6: beide Seitenleisten",
    )));
    show_tools.style_context().add_class("flat");
    show_layers.style_context().add_class("flat");
    let panels = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    panels.style_context().add_class("linked");
    panels.pack_start(&show_tools, false, false, 0);
    panels.pack_start(&show_layers, false, false, 0);
    source_row.pack_start(&panels, false, false, 0);
    source_row.reorder_child(&panels, 0);
    let dimensions = gtk::Label::new(None);
    dimensions.set_width_chars(22);
    dimensions.set_max_width_chars(22);
    dimensions.set_ellipsize(gtk::pango::EllipsizeMode::End);
    dimensions.style_context().add_class("muted");
    source_row.pack_start(&dimensions, false, false, 0);
    let minus = icon_button("minus", "", crate::i18n::tr("Verkleinern · −"));
    let plus = icon_button("plus", "", crate::i18n::tr("Vergrößern · +"));
    let fit = icon_button("fit", "", crate::i18n::tr("Gesamtes Bild einpassen · 0"));
    let zoom_controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    zoom_controls.style_context().add_class("linked");
    for b in [&minus, &plus, &fit] {
        zoom_controls.pack_start(b, false, false, 0);
    }
    source_row.pack_start(&zoom_controls, false, false, 0);
    page.pack_start(&context, false, false, 0);
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 4);
    sidebar.set_widget_name("sidebar");
    sidebar.set_border_width(12);
    sidebar.set_size_request(168, -1);
    sidebar.pack_start(&caption(crate::i18n::tr("Werkzeuge")), false, false, 8);
    let mut tools: Vec<gtk::RadioButton> = Vec::new();
    for (index, (tool, name, key)) in Tool::ALL.into_iter().enumerate() {
        let name = crate::i18n::tr(name);
        if index == 2 || index == 7 {
            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            separator.set_margin_top(6);
            separator.set_margin_bottom(6);
            sidebar.pack_start(&separator, false, false, 0);
        }
        let radio = gtk::RadioButton::new();
        if let Some(first) = tools.first() {
            radio.join_group(Some(first));
        }
        radio.set_mode(false);
        radio.style_context().add_class("flat");
        radio.style_context().add_class("tool-button");
        radio.set_can_focus(true);
        radio.set_focus_on_click(false);
        icons::accessible_name(&radio, name);
        radio.set_tooltip_text(Some(&crate::i18n::format!("{name} · {key}")));
        let row = icons::content(icons::tool(tool), name);
        let shortcut = gtk::Label::new(Some(key));
        shortcut.style_context().add_class("shortcut");
        shortcut.set_width_chars(2);
        shortcut.set_xalign(1.);
        shortcut.set_valign(gtk::Align::Center);
        row.pack_end(&shortcut, false, false, 0);
        radio.add(&row);
        sidebar.pack_start(&radio, false, false, 0);
        tools.push(radio);
    }
    tools[0].set_active(true);
    sidebar.reorder_child(&tools[10], 2);
    let style_label = caption(crate::i18n::tr("Zeichenstil"));
    style_label.set_margin_top(12);
    sidebar.pack_start(&style_label, false, false, 6);
    let color = crate::color::Picker::new(
        crate::i18n::tr("Objektfarbe"),
        gdk::RGBA::new(0.98, 0.32, 0.27, 1.),
        gtk::PositionType::Right,
    );
    let color_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let color_label = gtk::Label::new(Some(crate::i18n::tr("Farbe")));
    color_label.set_xalign(0.);
    color_row.pack_start(&color_label, true, true, 0);
    color.button.set_size_request(96, 34);
    color_row.pack_end(&color.button, false, false, 0);
    sidebar.pack_start(&color_row, false, false, 0);
    let width = gtk::SpinButton::with_range(1., 100., 1.);
    width.set_update_policy(gtk::SpinButtonUpdatePolicy::IfValid);
    width.set_value(4.);
    width.set_tooltip_text(Some(crate::i18n::tr(
        "Linienstärke in Bildpixeln; Text hat eine eigene Schriftgröße",
    )));
    let width_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let width_label = gtk::Label::new(Some(crate::i18n::tr("Stärke")));
    width_label.set_xalign(0.);
    width_controls.pack_start(&width_label, true, true, 0);
    width.set_width_chars(4);
    let width_picker = crate::stroke::chooser(&width);
    width_picker.set_size_request(96, 34);
    width_controls.pack_end(&width_picker, false, false, 0);
    sidebar.pack_start(&width_controls, false, false, 4);
    let font_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let font_label = gtk::Label::new(Some(crate::i18n::tr("Schrift")));
    font_label.set_xalign(0.);
    font_controls.pack_start(&font_label, true, true, 0);

    let font_size = gtk::SpinButton::with_range(6., 512., 1.);
    font_size.set_update_policy(gtk::SpinButtonUpdatePolicy::IfValid);
    font_size.set_value(28.);
    font_size.set_tooltip_text(Some(crate::i18n::tr(
        "Schriftgröße im exportierten Bild, unabhängig von der Linienstärke",
    )));
    font_size.set_width_chars(4);
    font_size.set_size_request(96, 34);
    let font_input = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    font_input.style_context().add_class("linked");
    font_input.pack_start(&font_size, false, false, 0);
    let font_picker = crate::font_size::chooser(&font_size);
    font_input.pack_start(&font_picker, false, false, 0);
    font_controls.pack_end(&font_input, false, false, 0);
    sidebar.pack_start(&font_controls, false, false, 4);
    let history = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    history.style_context().add_class("linked");
    let undo = icon_button("undo", "", crate::i18n::tr("Rückgängig · Strg+Z"));
    let redo = icon_button("redo", "", crate::i18n::tr("Wiederholen · Strg+Umschalt+Z"));
    history.pack_start(&undo, true, true, 0);
    history.pack_start(&redo, true, true, 0);
    header.pack_start(&history, false, false, 8);
    header.reorder_child(&history, 3);
    let guides = gtk::CheckButton::with_label(crate::i18n::tr("Monitorgrenzen"));
    guides.set_active(true);
    sidebar.pack_end(&guides, false, false, 4);
    let snapping = gtk::CheckButton::with_label(crate::i18n::tr("Einrasten · S"));
    snapping.set_active(true);
    snapping.set_tooltip_text(Some(crate::i18n::tr("Beim Verschieben an Kanten und Mitten ausrichten · S: umschalten · Alt: vorübergehend frei verschieben")));
    sidebar.pack_end(&snapping, false, false, 4);
    // Scroll side panels independently on small/portrait displays.
    let left_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    left_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    left_scroll.add(&sidebar);
    body.pack_start(&left_scroll, false, false, 0);
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_min_content_width(320);
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_can_focus(true);
    icons::accessible_name(
        &area,
        crate::i18n::tr("Zeichenfläche · F6 · Werkzeuge mit V, M oder 1–9 wählen"),
    );
    area.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::SMOOTH_SCROLL_MASK
            | gdk::EventMask::LEAVE_NOTIFY_MASK,
    );
    let overlay = gtk::Overlay::new();
    overlay.add(&area);
    let inline_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    inline_box.set_widget_name("inline-editor");
    inline_box.set_halign(gtk::Align::Start);
    inline_box.set_valign(gtk::Align::Start);
    let inline_input = gtk::TextView::new();
    inline_input.set_accepts_tab(false);
    inline_input.set_wrap_mode(gtk::WrapMode::None);
    inline_input.set_left_margin(0);
    inline_input.set_right_margin(0);
    inline_input.set_top_margin(0);
    let inline_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    inline_scroll.set_min_content_height(70);
    inline_scroll.set_size_request(240, 100);
    inline_scroll.add(&inline_input);
    inline_box.pack_start(&inline_scroll, true, true, 0);
    let inline_actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let accept_text = button(
        crate::i18n::tr("Übernehmen"),
        crate::i18n::tr("Enter · Shift+Enter: neue Zeile"),
    );
    let cancel_text = button(
        crate::i18n::tr("Abbrechen"),
        crate::i18n::tr("Esc · Textänderung verwerfen"),
    );
    inline_actions.pack_start(&accept_text, false, false, 0);
    inline_actions.pack_start(&cancel_text, false, false, 0);
    inline_box.pack_start(&inline_actions, false, false, 0);
    inline_box.set_tooltip_text(Some(crate::i18n::tr(
        "Enter übernimmt · Shift+Enter fügt eine Zeile ein · Esc verwirft",
    )));
    overlay.add_overlay(&inline_box);
    let inline_css = gtk::CssProvider::new();
    inline_input
        .style_context()
        .add_provider(&inline_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    scroll.add(&overlay);
    let workspace_split = gtk::Paned::new(gtk::Orientation::Horizontal);
    workspace_split.set_wide_handle(true);
    let canvas_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    canvas_column.pack_start(&scroll, true, true, 0);
    let history_canvas = gtk::Box::new(gtk::Orientation::Vertical, 0);
    canvas_column.pack_start(&history_canvas, false, false, 0);
    workspace_split.pack1(&canvas_column, true, false);
    body.pack_start(&workspace_split, true, true, 0);
    let inspector = gtk::Box::new(gtk::Orientation::Vertical, 8);
    inspector.set_widget_name("inspector");
    inspector.set_border_width(10);
    inspector.set_spacing(6);
    inspector.set_size_request(252, -1);
    let layers_heading = caption(crate::i18n::tr("Ebenen"));
    inspector.pack_start(&layers_heading, false, false, 8);
    let layers = gtk::ListBox::new();
    layers.set_selection_mode(gtk::SelectionMode::Multiple);
    layers.set_tooltip_text(Some(crate::i18n::tr(
        "Shift-/Strg-Klick: mehrere Objekte auswählen",
    )));
    let empty_layers = gtk::Box::new(gtk::Orientation::Vertical, 12);
    empty_layers.set_border_width(20);
    empty_layers.set_valign(gtk::Align::Center);
    let empty_icon = icons::widget("layers");
    empty_icon.set_halign(gtk::Align::Center);
    empty_icon.set_opacity(0.5);
    empty_layers.pack_start(&empty_icon, false, false, 0);
    let empty_title = gtk::Label::new(Some(crate::i18n::tr("Noch keine Ebenen")));
    empty_title.style_context().add_class("card-title");
    empty_layers.pack_start(&empty_title, false, false, 0);
    let empty_hint = gtk::Label::new(Some(crate::i18n::tr(
        "Pfeile, Formen und Text bleiben einzeln editierbar.",
    )));
    empty_hint.set_line_wrap(true);
    empty_hint.set_max_width_chars(22);
    empty_hint.set_justify(gtk::Justification::Center);
    empty_hint.style_context().add_class("muted");
    empty_layers.pack_start(&empty_hint, false, false, 0);
    layers.set_placeholder(Some(&empty_layers));
    empty_layers.show_all();
    let layer_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    layer_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    layer_scroll.set_min_content_height(120);
    layer_scroll.add(&layers);
    let inspector_split = gtk::Paned::new(gtk::Orientation::Vertical);
    inspector_split.set_wide_handle(true);
    inspector_split.pack1(&layer_scroll, true, false);
    inspector_split.set_position(280);
    inspector.pack_start(&inspector_split, true, true, 0);
    let layer_options = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let property_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    property_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    property_scroll.set_overlay_scrolling(false);
    property_scroll.set_min_content_height(120);
    property_scroll.add(&layer_options);
    inspector_split.pack2(&property_scroll, true, false);
    let layer_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    layer_actions.set_homogeneous(true);
    layer_actions.style_context().add_class("linked");
    let raise = icon_button("up", "", crate::i18n::tr("Ebene nach vorne"));
    let lower = icon_button("down", "", crate::i18n::tr("Ebene nach hinten"));
    let delete = icon_button(
        "trash",
        "",
        crate::i18n::tr("Ausgewählte Ebene löschen · Entf"),
    );
    for b in [&raise, &lower, &delete] {
        layer_actions.pack_start(b, true, true, 0);
    }
    layer_options.pack_start(&layer_actions, false, false, 0);
    let duplicate = icon_button(
        "copy",
        "",
        crate::i18n::tr("Ausgewählte Ebenen duplizieren · Strg+D"),
    );
    layer_actions.pack_start(&duplicate, true, true, 0);
    layer_actions.reorder_child(&duplicate, 2);
    let layer_name = gtk::Entry::new();
    layer_name.set_placeholder_text(Some(crate::i18n::tr("Ebenenname")));
    layer_name.set_tooltip_text(Some(crate::i18n::tr(
        "Ebenenname ändern · Enter bestätigen · F2",
    )));
    layer_name.set_width_chars(1);
    layer_name.set_max_width_chars(14);
    layer_name.set_no_show_all(true);
    let alignment = gtk::ComboBoxText::new();
    for (id, label) in [
        ("left", crate::i18n::tr("Links ausrichten")),
        ("center", crate::i18n::tr("Horizontal zentrieren")),
        ("right", crate::i18n::tr("Rechts ausrichten")),
        ("top", crate::i18n::tr("Oben ausrichten")),
        ("middle", crate::i18n::tr("Vertikal zentrieren")),
        ("bottom", crate::i18n::tr("Unten ausrichten")),
    ] {
        alignment.append(Some(id), label);
    }
    alignment.set_active(Some(0));
    layer_options.pack_start(&alignment, false, false, 0);
    let align = button(
        crate::i18n::tr("Auswahl ausrichten"),
        crate::i18n::tr(
            "Mindestens zwei Objekte mit Shift-Klick oder in der Ebenenliste auswählen",
        ),
    );
    layer_options.pack_start(&align, false, false, 0);
    let base = icons::content("monitor", crate::i18n::tr("Hintergrundbild"));
    // Match the inset of the editable layer rows above.
    base.set_border_width(3);
    base.set_tooltip_text(Some(
        "Dein Screenshot liegt unter den Markierungen. Pfeile, Formen und Text \
         kannst du darüber hinzufügen und einzeln bearbeiten. Das Hintergrundbild \
         lässt sich nicht wie eine Markierung verschieben oder löschen.",
    ));
    base.style_context().add_class("muted");
    layer_options.pack_start(&base, false, false, 2);
    let selection_label = caption(crate::i18n::tr("Eigenschaften"));
    let property_heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    property_heading.pack_start(&selection_label, true, true, 0);
    layer_options.pack_start(&property_heading, false, false, 2);
    let object_properties = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let visible = gtk::CheckButton::with_label(crate::i18n::tr("Ebene sichtbar"));
    property_heading.pack_end(&visible, false, false, 0);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(8);
    grid.set_row_spacing(6);
    let properties = std::array::from_fn(|i| {
        let spin = gtk::SpinButton::with_range(if i < 2 { -100000. } else { 1. }, 100000., 1.);
        spin.set_width_chars(6);
        grid.attach(
            &gtk::Label::new(Some(
                ["X", "Y", crate::i18n::tr("Breite"), crate::i18n::tr("Höhe")][i],
            )),
            0,
            i as i32,
            1,
            1,
        );
        grid.attach(&spin, 1, i as i32, 1, 1);
        spin
    });
    let geometry = gtk::Expander::new(Some(crate::i18n::tr("Position und Größe")));
    let geometry_controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
    geometry_controls.set_margin_top(8);
    geometry_controls.pack_start(&grid, false, false, 0);
    geometry.add(&geometry_controls);
    object_properties.pack_start(&geometry, false, false, 0);
    let edit_text = icon_button(
        "text",
        crate::i18n::tr("Text ändern"),
        crate::i18n::tr("Text der ausgewählten Ebene bearbeiten"),
    );
    object_properties.pack_start(&edit_text, false, false, 0);
    layer_options.pack_start(&object_properties, false, false, 0);
    let image_effects = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let border_enabled = gtk::CheckButton::with_label(crate::i18n::tr("Rand"));
    let border_width = gtk::SpinButton::with_range(1., 40., 1.);
    border_width.set_value(3.);
    border_width.set_width_chars(3);
    let border_color = crate::color::Picker::new(
        crate::i18n::tr("Randfarbe"),
        gdk::RGBA::new(1., 1., 1., 1.),
        gtk::PositionType::Left,
    );
    let border_controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    border_controls.pack_start(&border_enabled, true, true, 0);
    border_controls.pack_end(&border_color.button, false, false, 0);
    border_width.set_tooltip_text(Some(crate::i18n::tr(
        "Randstärke in Pixeln · direkt im Bild · Strg+Z: rückgängig",
    )));
    image_effects.pack_start(&border_controls, false, false, 0);
    let labels = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    let fields = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    let effect_row = |grid: &gtk::Grid, row, name: &str, spin: &gtk::SpinButton, unit: &str| {
        let label = gtk::Label::new(Some(name));
        label.set_xalign(0.);
        label.set_hexpand(true);
        labels.add_widget(&label);
        fields.add_widget(spin);
        spin.set_width_chars(3);
        spin.set_size_request(110, -1);
        let unit = gtk::Label::new(Some(unit));
        unit.set_width_chars(2);
        unit.set_xalign(0.);
        unit.style_context().add_class("muted");
        grid.attach(&label, 0, row, 1, 1);
        grid.attach(spin, 1, row, 1, 1);
        grid.attach(&unit, 2, row, 1, 1);
    };
    let border_grid = gtk::Grid::new();
    border_grid.set_column_spacing(8);
    effect_row(
        &border_grid,
        0,
        crate::i18n::tr("Stärke"),
        &border_width,
        "px",
    );
    image_effects.pack_start(&border_grid, false, false, 0);
    let shadow_enabled = gtk::CheckButton::with_label(crate::i18n::tr("Schatten"));
    shadow_enabled.set_margin_top(2);
    image_effects.pack_start(&shadow_enabled, false, false, 0);
    let shadow_grid = gtk::Grid::new();
    shadow_grid.set_row_spacing(6);
    shadow_grid.set_column_spacing(8);
    let shadow_blur = gtk::SpinButton::with_range(0., 80., 1.);
    let shadow_offset = gtk::SpinButton::with_range(-100., 100., 1.);
    let shadow_opacity = gtk::SpinButton::with_range(0., 100., 5.);
    shadow_opacity.set_tooltip_text(Some(crate::i18n::tr(
        "Deckkraft des Schattens in Prozent · direkt im Bild",
    )));
    for (row, (label, spin, value, unit)) in [
        (crate::i18n::tr("Deckkraft"), &shadow_opacity, 30., "%"),
        (crate::i18n::tr("Weichheit"), &shadow_blur, 12., "px"),
        (crate::i18n::tr("Versatz"), &shadow_offset, 8., "px"),
    ]
    .into_iter()
    .enumerate()
    {
        spin.set_value(value);
        effect_row(&shadow_grid, row as i32, label, spin, unit);
    }
    image_effects.pack_start(&shadow_grid, false, false, 0);
    layer_options.pack_start(&image_effects, false, false, 0);
    grid.set_tooltip_text(Some(crate::i18n::tr("V: Objekt anklicken und verschieben. An acht Griffen skalieren. Bilder und Text behalten ihre Proportionen. Esc bricht das Ziehen ab.")));
    let right_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    right_panel.pack_start(&inspector, true, true, 0);
    workspace_split.pack2(&right_panel, false, false);
    layers_heading.set_tooltip_text(Some(crate::i18n::tr(
        "Die Trennlinie links ziehen, um die Ebenenleiste breiter oder schmaler zu machen",
    )));
    page.pack_start(&body, true, true, 0);
    let region_bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    region_bar.set_border_width(14);
    region_bar.set_widget_name("regionbar");
    let instruction = gtk::Label::new(Some(crate::i18n::tr(
        "Bereich aufziehen → Ausschnitt bestätigen → annotieren",
    )));
    region_bar.pack_start(&instruction, true, true, 0);
    let confirm_region = button(
        crate::i18n::tr("Im Editor öffnen"),
        crate::i18n::tr("Bereich übernehmen · Enter"),
    );
    confirm_region.style_context().add_class("suggested-action");
    region_bar.pack_start(&confirm_region, false, false, 0);
    page.pack_start(&region_bar, false, false, 0);
    let selection_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    selection_bar.set_border_width(8);
    let selection_hint = gtk::Label::new(Some(crate::i18n::tr(
        "Auswahl aufziehen · Shift: neu · Esc: aufheben",
    )));
    selection_hint.set_line_wrap(true);
    selection_hint.set_max_width_chars(35);
    selection_bar.pack_start(&selection_hint, true, true, 0);
    let copy_selection = button(
        crate::i18n::tr("Auswahl kopieren"),
        crate::i18n::tr("Sichtbaren Inhalt der Auswahl kopieren · Strg+C"),
    );
    let copy_selection_layer = button(
        crate::i18n::tr("Als neue Ebene · Strg+J"),
        crate::i18n::tr("Sichtbaren Inhalt kopieren, Original beibehalten"),
    );
    copy_selection_layer
        .style_context()
        .add_class("suggested-action");
    selection_bar.pack_end(&copy_selection_layer, false, false, 0);
    selection_bar.pack_end(&copy_selection, false, false, 0);
    page.pack_start(&selection_bar, false, false, 0);
    let magnifier = gtk::CheckButton::with_label(crate::i18n::tr("Pixellupe"));
    magnifier.set_active(true);
    region_bar.pack_start(&magnifier, false, false, 0);
    instruction.set_line_wrap(true);
    instruction.set_max_width_chars(65);
    stack.add_named(&page, "editor");
    let close_notice = gtk::InfoBar::new();
    close_notice.set_message_type(gtk::MessageType::Info);
    close_notice.set_no_show_all(true);
    close_notice.add_button(crate::i18n::tr("Verstanden"), gtk::ResponseType::Close);
    let close_notice_text = gtk::Label::new(None);
    close_notice_text.set_line_wrap(true);
    close_notice_text.set_max_width_chars(85);
    close_notice_text.set_xalign(0.);
    close_notice.content_area().add(&close_notice_text);
    root.pack_start(&close_notice, false, false, 0);
    root.pack_start(&stack, true, true, 0);
    let recent = crate::history::Strip::new((!args.demo).then(crate::history::directory));
    let history_setup = setup.history.clone();
    recent.attach_to(&history_setup);
    let status = gtk::Label::new(Some(crate::i18n::tr("Bereit für deine nächste Aufnahme.")));
    status.set_widget_name("status");
    status.connect_label_notify(|label| label.set_tooltip_text(Some(&label.text())));
    status.set_xalign(0.);
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    status.set_hexpand(true);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.set_widget_name("footer");
    footer.set_border_width(8);
    footer.pack_start(&status, true, true, 8);
    let storage = gtk::Label::new(None);
    storage.set_xalign(0.);
    storage.set_line_wrap(true);
    storage.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
    storage.set_max_width_chars(44);
    let storage_button = gtk::MenuButton::new();
    storage_button.style_context().add_class("flat");
    let storage_row = icons::content("save", "");
    let storage_caption = gtk::Label::new(Some(crate::i18n::tr("Speicherstatus")));
    storage_caption.style_context().add_class("small");
    storage_row.pack_start(&storage_caption, false, false, 0);
    storage_button.add(&storage_row);
    let storage_popover = gtk::Popover::new(Some(&storage_button));
    let storage_details = gtk::Box::new(gtk::Orientation::Vertical, 12);
    storage_details.set_border_width(20);
    let storage_title = gtk::Label::new(Some(crate::i18n::tr("Dein Arbeitsstand")));
    storage_title.set_xalign(0.);
    storage_title.style_context().add_class("card-title");
    storage_details.pack_start(&storage_title, false, false, 0);
    storage_details.pack_start(&storage, false, false, 0);
    let storage_hint = gtk::Label::new(Some(crate::i18n::tr(
        "PNG für die Weitergabe exportieren. Projekte behalten das Original und alle Ebenen.",
    )));
    storage_hint.set_line_wrap(true);
    storage_hint.set_max_width_chars(42);
    storage_hint.set_xalign(0.);
    storage_hint.style_context().add_class("muted");
    storage_details.pack_start(&storage_hint, false, false, 0);
    storage_popover.add(&storage_details);
    storage_details.show_all();
    storage_button.set_popover(Some(&storage_popover));
    let project_save_button = icon_button(
        "save",
        crate::i18n::tr("Bearbeitbares Projekt speichern"),
        crate::i18n::tr("Alle Ebenen zum Weiterbearbeiten speichern · Strg+Alt+S"),
    );
    let project_status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    project_status.pack_start(&storage_button, false, false, 0);
    project_status.pack_start(&project_save_button, false, false, 0);
    footer.pack_end(&project_status, false, false, 0);
    root.pack_start(&footer, false, false, 0);
    window.add(&root);
    let editor = Rc::new_cyclic(|weak| Editor {
        error_dialog: RefCell::new(None),
        shortcuts_dialog: RefCell::new(None),
        close_to_tray,
        close_notice,
        close_notice_text,
        property_scroll,
        auto_layers_hidden: Cell::new(false),
        layers_manually_shown: Cell::new(false),
        layer_drag: RefCell::new(None),
        layer_rename: Cell::new(None),
        layer_drag_serial: Cell::new(0),
        layer_gestures: RefCell::new(Vec::new()),
        layer_rows: RefCell::new(Vec::new()),
        layer_selection: RefCell::new(Vec::new()),
        reveal_layer: Cell::new(false),
        snapping,
        object_menu: RefCell::new(None),
        property_edit: Cell::new(None),
        property_refresh: Cell::new(None),
        shadow_grid,
        clipboard_epoch: Cell::new(0),
        clipboard_pending: Cell::new(false),
        selection_bar,
        copy_selection_layer,
        copy_selection,
        paste_layer_button,
        image_effects,
        border_enabled,
        border_width,
        border_color,
        shadow_enabled,
        shadow_blur,
        shadow_offset,
        shadow_opacity,
        weak_self: weak.clone(),
        layer_previews: RefCell::new(crate::preview::LayerPreviews::default()),
        window,
        area,
        status,
        header,
        titlebar,
        export_actions,
        context,
        file_menu,
        save_as,
        export_button,
        project_save_button,
        export_path: RefCell::new(None),
        project_path: RefCell::new(None),
        autosave_target: RefCell::new(None),
        autosave_source: RefCell::new(None),
        automatic_project: Cell::new(false),
        autosave_existing: Cell::new(false),
        recovery_content: Cell::new(None),
        project_as,
        recent,
        new_capture: new.clone(),
        history_controls: history,
        layer_actions,
        layers_heading,
        empty_layers,
        width_controls,
        width_picker,
        storage_button,
        storage_caption,
        region_instruction: instruction,
        preview: RefCell::new(crate::preview::Preview::default()),
        cursor: Cell::new(None),
        dimensions,
        monitors,
        undo,
        redo,
        save,
        copy,
        paste,
        append_image,
        base,
        tools,
        args: RefCell::new(args),
        stack,
        setup,
        sidebar,
        inspector,
        region_bar,
        confirm_region,
        project,
        layers,
        layer_scroll,
        color,
        color_controls: color_row,
        monitor_guides: guides.clone(),
        width,
        properties,
        visible,
        edit_text,
        delete,
        syncing: Cell::new(false),
        left_panel: left_scroll,
        right_panel,
        workspace_split,
        history_canvas,
        history_setup,
        layer_width_restore_serial: Cell::new(0),
        show_tools,
        show_layers,
        magnifier,
        monitor_map,
        monitor_button: map_button,
        layer_name,
        alignment,
        align,
        duplicate,
        object_properties,
        selection_label,
        session: RefCell::new(None),
        recovery_revision: Cell::new(None),
        recovery_ticket: Cell::new(None),
        recovery_ack: Cell::new(None),
        recovery_error: RefCell::new(None),
        storage,
        font_size,
        font_picker,
        font_controls,
        style_label,
        inline_scale: Cell::new(0.),
        inline_box,
        inline_input,
        inline_css,
        inline_edit: RefCell::new(None),
        preferences: RefCell::new(Preferences::default()),
        preferences_path: RefCell::new(None),
        preferences_saved: RefCell::new(None),
        closed: Cell::new(false),
        resident: Cell::new(false),
        capture_cancel: RefCell::new(None),
        state: RefCell::new(State {
            document: None,
            tool: Tool::Move,
            color: (0.98, 0.32, 0.27),
            width: 4.,
            font_size: 28.,
            pan: Point::default(),
            mouse: None,
            space: false,
            pan_drag: None,
            project_state: None,
            export_state: None,
            draft: None,
            view: View {
                x: 0.,
                y: 0.,
                scale: 1.,
            },
            zoom: None,
            capturing: false,
            fullscreen: false,
            guides: true,
            snap_bypass: false,
            selected: None,
            drag: None,
            selecting_region: false,
            region_bounds: None,
            dirty: false,
            generation: 0,
            selected_many: Vec::new(),
            crop_drag: None,
            pixel_selection: None,
            pointer: None,
        }),
    });
    connect_canvas(&editor);
    layer_drag::connect(&editor);
    let e = editor.clone();
    editor
        .layers
        .connect_size_allocate(move |_, _| e.reveal_selected_layer());
    let adjustment = editor.layer_scroll.vadjustment();
    let previous_view = Rc::new(Cell::new((adjustment.value(), adjustment.page_size())));
    let view = previous_view.clone();
    adjustment.connect_value_changed(move |a| view.set((a.value(), view.get().1)));
    let e = editor.clone();
    adjustment.connect_changed(move |a| {
        let (previous_top, previous_height) = previous_view.replace((a.value(), a.page_size()));
        // Keep a visible selection in view when the viewport shrinks. A user
        // who has scrolled away from it keeps their chosen position instead.
        if !e.syncing.get() && previous_height > a.page_size() && a.page_size() > 1. {
            if let [row] = e.layers.selected_rows().as_slice() {
                let r = row.allocation();
                if r.height() > 1
                    && f64::from(r.y()) >= previous_top - 1.
                    && f64::from(r.y() + r.height()) <= previous_top + previous_height + 1.
                {
                    e.reveal_layer.set(true);
                }
            }
        }
        e.reveal_selected_layer();
    });
    let e = editor.clone();
    editor.snapping.connect_toggled(move |_| {
        if !e.syncing.get() {
            let mouse = e.state.borrow().mouse;
            if let Some(p) = mouse {
                e.motion(p.x, p.y);
            }
        }
    });
    for (field, spin) in editor.property_spins().into_iter().enumerate() {
        spin.set_update_policy(gtk::SpinButtonUpdatePolicy::IfValid);
        spin.set_tooltip_text(Some(match field {
            0 => crate::i18n::tr("X-Position in Pixeln · direkte Vorschau · Strg+Z: rückgängig"),
            1 => crate::i18n::tr("Y-Position in Pixeln · direkte Vorschau · Strg+Z: rückgängig"),
            2 => crate::i18n::tr(
                "Breite in Pixeln · direkte Vorschau · Bilder und Text bleiben proportional",
            ),
            3 => crate::i18n::tr(
                "Höhe in Pixeln · direkte Vorschau · Bilder und Text bleiben proportional",
            ),
            4 => crate::i18n::tr("Randstärke in Pixeln · direkte Vorschau"),
            5 => crate::i18n::tr("Weichheit des Schattens in Pixeln · direkte Vorschau"),
            6 => crate::i18n::tr("Versatz des Schattens in Pixeln · direkte Vorschau"),
            _ => crate::i18n::tr("Deckkraft des Schattens in Prozent · direkte Vorschau"),
        }));
        let e = editor.clone();
        spin.connect_value_changed(move |spin| e.live_property(field, spin.value()));
        let e = editor.clone();
        spin.connect_changed(move |spin| {
            if let Ok(value) = spin.text().replace(',', ".").parse::<f64>() {
                e.live_property(field, value);
            }
        });
        let e = editor.clone();
        spin.connect_focus_in_event(move |_, _| {
            e.property_edit.set(None);
            glib::Propagation::Proceed
        });
        let e = editor.clone();
        spin.connect_focus_out_event(move |spin, _| {
            if e.syncing.get() {
                e.property_edit.set(None);
                return glib::Propagation::Proceed;
            }
            spin.update();
            e.property_edit.set(None);
            e.refresh();
            glib::Propagation::Proceed
        });
    }
    let e = editor.clone();
    editor
        .copy_selection_layer
        .connect_clicked(move |_| e.selection_to_layer());
    let e = editor.clone();
    editor.copy_selection.connect_clicked(move |_| e.copy());
    let e = editor.clone();
    editor
        .paste_layer_button
        .connect_clicked(move |_| e.paste_layer());
    for toggle in [&editor.border_enabled, &editor.shadow_enabled] {
        let e = editor.clone();
        toggle.connect_toggled(move |_| {
            if !e.syncing.get() {
                e.apply_image_effects();
            }
        });
    }
    let e = editor.clone();
    editor
        .border_color
        .connect_color_set(move |_| e.apply_image_effects());
    for (i, radio) in editor.tools.iter().enumerate() {
        let e = editor.clone();
        radio.connect_toggled(move |r| {
            if r.is_active() {
                let keyboard_focus = r.is_focus();
                e.commit_pending_edits();
                let mut s = e.state.borrow_mut();
                let refit = (s.tool == Tool::Select) != (Tool::ALL[i].0 == Tool::Select);
                s.tool = Tool::ALL[i].0;
                s.draft = None;
                s.drag = None;
                s.crop_drag = None;
                if s.tool != Tool::Marquee {
                    s.pixel_selection = None;
                }
                if s.tool != Tool::Move {
                    s.selected = None;
                    s.selected_many.clear();
                }
                drop(s);
                if refit {
                    e.zoom(None);
                }
                e.refresh();
                if keyboard_focus {
                    r.grab_focus();
                } else {
                    e.area.grab_focus();
                }
            }
        });
    }
    let e = editor.clone();
    editor.color.connect_color_set(move |c| {
        e.state.borrow_mut().color = (c.red(), c.green(), c.blue());
        e.change_style();
    });
    let e = editor.clone();
    editor.width.connect_value_changed(move |w| {
        if !e.syncing.get() {
            e.state.borrow_mut().width = w.value();
            e.change_style();
        }
    });
    let e = editor.clone();
    editor.font_size.connect_value_changed(move |spin| {
        if e.syncing.get() {
            return;
        }
        e.prepare_text_change();
        let size = spin.value();
        e.state.borrow_mut().font_size = size;
        if let Some(edit) = e.inline_edit.borrow_mut().as_mut() {
            edit.annotation.font_size = Some(size);
        }
        if e.inline_edit.borrow().is_some() {
            e.finish_text_change();
            e.update_inline_style();
            e.area.queue_draw();
        } else {
            e.edit_selected(|a| {
                if a.tool == Tool::Text {
                    a.font_size = Some(size);
                }
            });
        }
    });
    let e = editor.clone();
    editor.font_size.connect_changed(move |_| {
        if !e.syncing.get() && e.inline_edit.borrow().is_some() {
            e.update_history_controls();
        }
    });
    let e = editor.clone();
    accept_text.connect_clicked(move |_| e.finish_text(true));
    let e = editor.clone();
    cancel_text.connect_clicked(move |_| e.finish_text(false));
    text_history::connect(&editor);
    let e = editor.clone();
    guides.connect_toggled(move |g| {
        e.state.borrow_mut().guides = g.is_active();
        e.area.queue_draw();
    });
    let e = editor.clone();
    new.connect_clicked(move |_| e.new_screenshot());
    let e = editor.clone();
    capture_options.connect_clicked(move |_| {
        e.file_menu.set_active(false);
        e.show_setup();
        e.setup.options.set_expanded(true);
        e.setup.delay.grab_focus();
    });
    let e = editor.clone();
    open.connect_clicked(move |_| e.open());
    let e = editor.clone();
    open_project.connect_clicked(move |_| e.open_dialog(true));
    let weak = Rc::downgrade(&editor);
    editor.recent.connect_open(move |path| {
        if let Some(e) = weak.upgrade() {
            e.open_path(path);
        }
    });
    let e = editor.clone();
    editor
        .append_image
        .connect_clicked(move |_| e.choose_image_layer());
    let weak = Rc::downgrade(&editor);
    editor.recent.connect_append(move |path| {
        if let Some(e) = weak.upgrade() {
            e.append_image_path(path);
        }
    });
    let e = editor.clone();
    editor.save_as.connect_clicked(move |_| {
        e.save_as();
    });
    let e = editor.clone();
    close_window.connect_clicked(move |_| e.window.close());
    let e = editor.clone();
    quit_app.connect_clicked(move |_| e.quit());
    let e = editor.clone();
    editor.setup.start.connect_clicked(move |_| e.capture());
    let e = editor.clone();
    editor.setup.demo.connect_clicked(move |_| {
        if e.can_discard() {
            match document::demo() {
                Ok(doc) => e.set_document(doc, crate::i18n::tr("Demo · keine Bildschirmaufnahme")),
                Err(err) => e.error(err),
            }
        }
    });
    let e = editor.clone();
    editor.setup.cancel.connect_clicked(move |_| {
        if let Some(cancel) = e.capture_cancel.borrow_mut().take() {
            cancel.cancel();
        }
        let mut s = e.state.borrow_mut();
        s.generation += 1;
        s.capturing = false;
        let has_document = s.document.is_some();
        drop(s);
        e.stack
            .set_visible_child_name(if has_document { "editor" } else { "setup" });
        e.setup.progress.set_text(crate::i18n::tr(
            "Countdown abgebrochen. Bereit für eine neue Aufnahme.",
        ));
        e.refresh();
    });
    let e = editor.clone();
    editor
        .confirm_region
        .connect_clicked(move |_| e.finish_region());
    let e = editor.clone();
    editor.save.connect_clicked(move |_| {
        e.save();
    });
    let e = editor.clone();
    editor.export_button.connect_clicked(move |_| {
        e.save();
    });
    let e = editor.clone();
    editor.project_save_button.connect_clicked(move |_| {
        e.save_project();
    });
    let e = editor.clone();
    editor.project.connect_clicked(move |_| {
        e.save_project();
    });
    let e = editor.clone();
    editor.project_as.connect_clicked(move |_| {
        e.save_project_as();
    });
    let e = editor.clone();
    editor.copy.connect_clicked(move |_| e.copy());
    let e = editor.clone();
    editor.paste.connect_clicked(move |_| e.import_image(false));
    let e = editor.clone();
    editor
        .setup
        .web
        .connect_clicked(move |_| e.import_image(true));
    let e = editor.clone();
    editor.undo.connect_clicked(move |_| e.history(false));
    let e = editor.clone();
    editor.redo.connect_clicked(move |_| e.history(true));
    let e = editor.clone();
    plus.connect_clicked(move |_| e.zoom(Some(1.25)));
    let e = editor.clone();
    minus.connect_clicked(move |_| e.zoom(Some(0.8)));
    let e = editor.clone();
    fit.connect_clicked(move |_| e.zoom(None));
    let e = editor.clone();
    editor.monitors.connect_changed(move |combo| {
        if e.syncing.get() {
            return;
        }
        let Some(id) = combo.active_id() else {
            return;
        };
        if id == "custom" {
            return;
        }
        e.finish_text(true);
        let mut s = e.state.borrow_mut();
        if let Some(doc) = &mut s.document {
            let rect = id
                .parse::<usize>()
                .ok()
                .and_then(|i| doc.monitors().get(i))
                .map(|m| m.rect);
            doc.select(rect);
            s.dirty = true;
        }
        drop(s);
        e.zoom(None);
        e.refresh();
    });
    let e = editor.clone();
    editor.layers.connect_selected_rows_changed(move |list| {
        if e.syncing.get() {
            return;
        }
        let indices: Vec<usize> = list
            .selected_rows()
            .iter()
            .filter_map(|r| r.widget_name().parse().ok())
            .collect();
        e.commit_pending_edits();
        let mut s = e.state.borrow_mut();
        s.selected = indices.first().copied();
        s.selected_many = indices;
        drop(s);
        e.tools[0].set_active(true);
        e.refresh();
    });
    let e = editor.clone();
    editor.visible.connect_toggled(move |b| {
        if !e.syncing.get() {
            e.edit_selected(|a| a.visible = b.is_active());
        }
    });
    let e = editor.clone();
    editor
        .edit_text
        .connect_clicked(move |_| e.text(Point::default()));
    let e = editor.clone();
    editor.delete.connect_clicked(move |_| e.delete_layer());
    let e = editor.clone();
    editor
        .duplicate
        .connect_clicked(move |_| e.duplicate_layers());
    let e = editor.clone();
    editor.layer_name.connect_activate(move |_| {
        e.finish_layer_rename(true);
        e.area.grab_focus();
    });
    let weak = editor.weak_self.clone();
    editor.window.connect_set_focus(move |_, focus| {
        if let Some(e) = weak.upgrade() {
            if !focus.is_some_and(|w| w == e.layer_name.upcast_ref::<gtk::Widget>()) {
                if let Some(session) = e.layer_rename.get() {
                    if focus.is_some_and(|w| !e.layer_name.is_ancestor(w)) {
                        // Commit before a newly focused property control can
                        // change the document and invalidate the name draft.
                        e.finish_layer_rename(true);
                        return;
                    }
                    let weak = e.weak_self.clone();
                    // A click inside an entry can briefly focus its list row.
                    // Only the final focus after event dispatch ends editing.
                    glib::idle_add_local_once(move || {
                        if let Some(e) = weak.upgrade() {
                            if e.layer_rename.get() == Some(session)
                                && e.window.focused_widget().as_ref()
                                    != Some(e.layer_name.upcast_ref())
                            {
                                e.finish_layer_rename(true);
                            }
                        }
                    });
                }
            }
        }
    });
    let e = editor.clone();
    editor.align.connect_clicked(move |_| {
        let a = match e.alignment.active_id().as_deref() {
            Some("center") => Alignment::Center,
            Some("right") => Alignment::Right,
            Some("top") => Alignment::Top,
            Some("middle") => Alignment::Middle,
            Some("bottom") => Alignment::Bottom,
            _ => Alignment::Left,
        };
        e.align_layers(a);
    });
    let weak = editor.weak_self.clone();
    shortcuts.connect_clicked(move |_| {
        if let Some(e) = weak.upgrade() {
            e.file_menu.set_active(false);
            e.show_shortcuts();
        }
    });
    let weak = editor.weak_self.clone();
    editor.close_to_tray.connect_toggled(move |toggle| {
        if let Some(e) = weak.upgrade() {
            e.preferences.borrow_mut().close_to_tray = toggle.is_active();
            e.update_close_notice();
        }
    });
    let weak = editor.weak_self.clone();
    editor.close_notice.connect_response(move |bar, _| {
        bar.hide();
        if let Some(e) = weak.upgrade() {
            e.preferences.borrow_mut().close_behavior_explained = true;
        }
    });
    let e = editor.clone();
    editor.show_tools.connect_toggled(move |_| {
        if !e.syncing.get() {
            e.refresh();
        }
    });
    let e = editor.clone();
    editor.show_layers.connect_toggled(move |button| {
        if !e.syncing.get() {
            e.auto_layers_hidden.set(false);
            e.layers_manually_shown.set(button.is_active());
            e.refresh();
        }
    });
    let weak = editor.weak_self.clone();
    editor
        .right_panel
        .connect_size_allocate(move |panel, allocation| {
            if panel.is_mapped() {
                if let Some(e) = weak.upgrade() {
                    e.preferences.borrow_mut().layer_panel_width = allocation.width();
                }
            }
        });
    let weak = editor.weak_self.clone();
    editor.right_panel.connect_map(move |_| {
        if let Some(e) = weak.upgrade() {
            let width = e.preferences.borrow().layer_panel_width;
            e.restore_layer_width(width);
        }
    });
    let e = editor.clone();
    editor
        .magnifier
        .connect_toggled(move |_| e.area.queue_draw());
    let e = editor.clone();
    editor.monitor_map.connect_selected(move |i| {
        e.monitors.set_active_id(Some(&i.to_string()));
        popover.popdown();
    });
    let e = editor.clone();
    raise.connect_clicked(move |_| e.reorder(true));
    let e = editor.clone();
    lower.connect_clicked(move |_| e.reorder(false));
    let e = editor.clone();
    editor
        .window
        .connect_key_press_event(move |_, event| e.key(event));
    let e = editor.clone();
    editor.window.connect_delete_event(move |_, _| {
        if e.resident.get() && e.close_to_tray.is_active() {
            e.hide_to_tray();
            glib::Propagation::Stop
        } else if e.close() {
            if e.resident.get() {
                if let Some(app) = e.window.application() {
                    app.quit();
                }
            }
            glib::Propagation::Proceed
        } else {
            glib::Propagation::Stop
        }
    });
    let e = editor.clone();
    editor.window.connect_key_release_event(move |_, event| {
        if matches!(
            event.keyval(),
            gdk::keys::constants::Alt_L | gdk::keys::constants::Alt_R
        ) {
            e.state.borrow_mut().snap_bypass = false;
            let mouse = e.state.borrow().mouse;
            if let Some(p) = mouse {
                e.motion(p.x, p.y);
            }
        }
        if event.keyval() == gdk::keys::constants::space {
            let mut s = e.state.borrow_mut();
            s.space = false;
            s.pan_drag = None;
        }
        glib::Propagation::Proceed
    });
    let e = editor.clone();
    editor.window.connect_focus_out_event(move |_, _| {
        e.property_edit.set(None);
        let mut s = e.state.borrow_mut();
        s.space = false;
        s.snap_bypass = false;
        s.pan_drag = None;
        glib::Propagation::Proceed
    });
    icons::name_controls(editor.window.upcast_ref());
    editor.window.show_all();
    editor.show_setup();
    if editor.args.borrow().demo {
        match document::demo() {
            Ok(doc) => editor.set_document(
                doc,
                crate::i18n::tr("Demo · 2 Displays · keine Bildschirmaufnahme"),
            ),
            Err(err) => editor.error(err),
        }
    } else if let Some(path) = &editor.args.borrow().open {
        editor.open_path(path);
    }
    editor
}

impl Editor {
    pub(crate) fn set_resident(&self, resident: bool) {
        self.resident.set(resident);
        self.close_to_tray.set_sensitive(resident);
        self.update_close_notice();
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.get()
    }
    pub(crate) fn is_capturing(&self) -> bool {
        self.state.borrow().capturing
    }
    pub(crate) fn capture_status(&self) -> String {
        if self.is_capturing() {
            self.setup.progress.text().to_string()
        } else {
            crate::i18n::tr("Bereit · Strg+Shift+X").into()
        }
    }
    pub(crate) fn present(&self) {
        self.window.present();
        self.refresh();
    }
    pub(crate) fn cancel_capture(&self) {
        if self.is_capturing() {
            self.setup.cancel.emit_clicked();
            self.present();
        }
    }
    pub(crate) fn shutdown(&self) -> bool {
        self.close()
    }
    fn quit(&self) {
        if self.close() {
            if let Some(app) = self.window.application() {
                app.quit();
            }
        }
    }
    pub(crate) fn hide_to_tray(&self) -> bool {
        self.commit_pending_edits();
        if self.is_capturing() {
            self.setup.cancel.emit_clicked();
        }
        let result = self
            .save_preferences()
            .and_then(|_| self.save_recovery())
            .and_then(|_| self.session.borrow().as_ref().map_or(Ok(()), |s| s.flush()));
        if let Err(error) = result {
            self.present();
            self.error(error.context(crate::i18n::tr(
                "Vor dem Ausblenden konnte nicht gesichert werden",
            )));
            return false;
        }
        if self.session.borrow().is_none() && self.state.borrow().dirty {
            self.present();
            self.error(crate::i18n::anyhow!(
                "Autosicherung nicht verfügbar. Bitte zuerst ein bearbeitbares Projekt speichern."
            ));
            return false;
        }
        self.poll_recovery();
        self.dismiss_error();
        self.dismiss_shortcuts();
        self.window.hide();
        true
    }
    pub(crate) fn request(self: &Rc<Self>, args: Args) -> bool {
        if self.is_capturing() {
            return false;
        }
        *self.args.borrow_mut() = args.clone();
        self.setup.set_backend(args.backend);
        if let Some(path) = &args.open {
            self.present();
            if !self.open_path(path) {
                return false;
            }
        } else if let Some(mode) = args.capture {
            self.show_setup();
            self.setup.delay.set_value(args.delay as f64);
            if let Some(index) = args.monitor.filter(|_| mode != Mode::System) {
                if !self
                    .setup
                    .source
                    .set_active_id(Some(&(index - 1).to_string()))
                {
                    let note = crate::i18n::format!("Bildschirm {index} ist nicht direkt auswählbar. Bitte Quelle prüfen oder Fenster / Bildschirm … verwenden.");
                    self.setup.progress.set_text(&note);
                    self.status.set_text(&note);
                    self.present();
                    return false;
                }
            }
            match mode {
                Mode::Region => self.setup.region.set_active(true),
                Mode::Full => self.setup.full.set_active(true),
                Mode::System => self.setup.system.set_active(true),
            }
            if self.state.borrow().dirty {
                self.present();
            }
            self.capture_with_visibility(true);
            return self.is_capturing();
        }
        self.present();
        true
    }
    fn load_preferences(&self, path: &std::path::Path) {
        let prefs = match Preferences::load(path) {
            Ok(p) => p,
            Err(error) => {
                self.status.set_text(&crate::i18n::format!(
                    "Einstellungen nicht geladen: {error:#}"
                ));
                return;
            }
        };
        *self.preferences_path.borrow_mut() = Some(path.to_path_buf());
        *self.preferences_saved.borrow_mut() = Some(prefs.clone());
        *self.preferences.borrow_mut() = prefs.clone();
        self.apply_preferences(&prefs);
    }
    fn apply_preferences(&self, prefs: &Preferences) {
        self.close_to_tray.set_active(prefs.close_to_tray);
        self.update_close_notice();
        self.restore_layer_width(prefs.layer_panel_width);
        self.recent.toggle.set_active(prefs.history_visible);
        {
            let mut s = self.state.borrow_mut();
            s.color = prefs.color;
            s.width = prefs.width;
            s.font_size = prefs.font_size;
        }
        self.syncing.set(true);
        self.color.set_rgba(&gdk::RGBA::new(
            prefs.color.0,
            prefs.color.1,
            prefs.color.2,
            1.,
        ));
        self.width.set_value(prefs.width);
        self.font_size.set_value(prefs.font_size);
        self.snapping.set_active(prefs.snapping);
        self.syncing.set(false);
        self.show_tools.set_active(prefs.tools_visible);
        self.syncing.set(true);
        self.auto_layers_hidden.set(false);
        self.layers_manually_shown.set(false);
        self.show_layers.set_active(prefs.layers_visible);
        self.syncing.set(false);
        if self.args.borrow().monitor.is_none() {
            if let Some(index) = prefs.monitor_index(&self.setup.map.monitors()) {
                self.setup.source.set_active_id(Some(&index.to_string()));
            }
        }
        if let Some(index) = Tool::ALL
            .iter()
            .position(|(tool, _, _)| *tool == prefs.tool)
        {
            self.tools[index].set_active(true);
        }
        self.refresh();
    }
    fn restore_layer_width(&self, width: i32) {
        let serial = self.layer_width_restore_serial.get().wrapping_add(1);
        self.layer_width_restore_serial.set(serial);
        let weak = self.weak_self.clone();
        let allocated = std::cell::Cell::new(false);
        self.workspace_split.add_tick_callback(move |split, _| {
            if !allocated.replace(true) {
                return glib::ControlFlow::Continue;
            }
            if let Some(e) = weak.upgrade() {
                // A saved preference loaded after map supersedes the initial
                // default, regardless of GTK's frame-callback order.
                if e.layer_width_restore_serial.get() != serial {
                    return glib::ControlFlow::Break;
                }
                if e.right_panel.is_mapped() {
                    let left = split.child1().unwrap().allocated_width();
                    let handle =
                        (split.allocated_width() - left - e.right_panel.allocated_width()).max(0);
                    split.set_position(
                        (split.allocated_width() - width.clamp(252, 16384) - handle).max(0),
                    );
                }
            }
            glib::ControlFlow::Break
        });
    }

    fn save_preferences(&self) -> Result<()> {
        let path = self.preferences_path.borrow();
        let Some(path) = path.as_ref() else {
            return Ok(());
        };
        let s = self.state.borrow();
        let mut prefs = self.preferences.borrow().clone();
        prefs.snapping = self.snapping.is_active();
        if !matches!(s.tool, Tool::Select | Tool::Marquee) {
            prefs.tool = s.tool;
        }
        prefs.color = s.color;
        prefs.width = s.width;
        prefs.font_size = s.font_size;
        prefs.history_visible = self.recent.toggle.is_active();
        prefs.tools_visible = self.show_tools.is_active();
        prefs.layers_visible = self.show_layers.is_active() || self.auto_layers_hidden.get();
        prefs.close_to_tray = self.close_to_tray.is_active();
        prefs.monitor = self
            .setup
            .source
            .active_id()
            .and_then(|id| id.parse::<usize>().ok())
            .and_then(|i| self.setup.map.monitors().get(i).cloned());
        if self.preferences_saved.borrow().as_ref() != Some(&prefs) {
            prefs.save(path)?;
            *self.preferences_saved.borrow_mut() = Some(prefs.clone());
        }
        *self.preferences.borrow_mut() = prefs;
        Ok(())
    }
    fn commit_pending_edits(&self) {
        // Toolbar buttons retain focus. Read the spin entry before committing
        // inline text, so a newly typed font size reaches the saved annotation.
        if let Some(spin) = self
            .window
            .focused_widget()
            .and_then(|w| w.downcast::<gtk::SpinButton>().ok())
        {
            spin.update();
        }
        self.finish_layer_rename(true);
        self.finish_text(true);
    }

    fn mark_project_saved(&self) {
        self.property_edit.set(None);
        let mut s = self.state.borrow_mut();
        s.project_state = s.document.as_ref().map(Document::content_id);
        s.dirty = false;
    }
    fn mark_exported(&self, path: &std::path::Path) {
        self.property_edit.set(None);
        *self.export_path.borrow_mut() = Some(path.to_path_buf());
        if self.session.borrow().is_some()
            && (self.automatic_project.get() || self.project_path.borrow().is_none())
        {
            if let Some(session) = self.session.borrow().as_ref() {
                let _ = session.flush();
            }
            self.poll_recovery();
            let source = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let sidecar = autosave::sidecar_path(&source);
            let owned = self.project_path.borrow().as_ref() == Some(&sidecar);
            self.autosave_existing
                .set(owned || autosave::matches_image(&source));
            self.autosave_target.borrow_mut().take();
            self.project_path.replace(
                if owned || !sidecar.exists() || autosave::matches_image(&source) {
                    Some(sidecar)
                } else {
                    None
                },
            );
            self.autosave_source.replace(Some(source));
            self.automatic_project.set(true);
            self.recovery_revision.set(None);
            self.state.borrow_mut().project_state = None;
        }
        self.recent.refresh();
        let mut s = self.state.borrow_mut();
        s.export_state = s.document.as_ref().map(Document::content_id);
        drop(s);
        self.preferences.borrow_mut().export_directory = path
            .parent()
            .filter(|p| p.is_absolute())
            .map(|p| p.to_path_buf());
    }
    fn cancel_action(&self) {
        if self.inline_edit.borrow().is_some() {
            self.finish_text(false);
            return;
        }
        let mut s = self.state.borrow_mut();
        if s.draft.is_some() || s.drag.is_some() || s.crop_drag.is_some() || s.pan_drag.is_some() {
            s.draft = None;
            s.drag = None;
            s.crop_drag = None;
            if let Some((_, pan)) = s.pan_drag.take() {
                s.pan = pan;
            }
            drop(s);
            self.refresh();
            return;
        }
        if s.selecting_region {
            drop(s);
            self.show_setup();
            return;
        }
        if s.tool == Tool::Marquee {
            if s.pixel_selection.take().is_some() {
                drop(s);
                self.refresh();
            } else {
                drop(s);
                self.tools[0].set_active(true);
            }
            return;
        }
        if s.tool == Tool::Select {
            drop(s);
            self.tools[0].set_active(true);
            return;
        }
        if !s.indices().is_empty() {
            s.selected = None;
            s.selected_many.clear();
            drop(s);
            self.refresh();
            return;
        }
        drop(s);
        self.status.set_text(crate::i18n::tr(
            "Bereit · Neue Aufnahme mit Strg+N · Schließen über das Fenster",
        ));
    }
    fn confirm_action(&self) {
        if self.inline_edit.borrow().is_some() {
            self.finish_text(true);
            return;
        }
        let gesture = {
            let s = self.state.borrow();
            (s.draft.is_some() || s.drag.is_some() || s.crop_drag.is_some())
                .then_some(s.mouse)
                .flatten()
        };
        if let Some(p) = gesture {
            self.release(p.x, p.y);
        }
        if self.state.borrow().tool == Tool::Marquee {
            self.selection_to_layer();
        } else if self.state.borrow().tool == Tool::Select {
            self.finish_region();
        } else if self.state.borrow().indices().len() == 1
            && self
                .state
                .borrow()
                .selected
                .and_then(|i| {
                    self.state
                        .borrow()
                        .document
                        .as_ref()
                        .map(|d| d.annotations()[i].tool == Tool::Text)
                })
                .unwrap_or(false)
        {
            self.text(Point::default());
        }
    }
    fn prepare_autosave(&self) -> Result<()> {
        if self.autosave_target.borrow().is_some() {
            return Ok(());
        }
        let previous = self.project_path.borrow().clone();
        if previous.is_none() && !self.state.borrow().dirty {
            return Ok(());
        }
        let source = self.autosave_source.borrow().clone();
        let path = if let Some(path) = previous.as_ref() {
            path.clone()
        } else if let Some(path) = source
            .as_ref()
            .filter(|p| !autosave::sidecar_path(p).exists() || autosave::matches_image(p))
        {
            autosave::sidecar_path(path)
        } else {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            self.export_directory()?.join(format!(
                "{}-{stamp}.untushot",
                suggested_filename("untushot").trim_end_matches(".untushot")
            ))
        };
        let target = autosave::Target::open(
            path.clone(),
            source,
            (previous.is_some() && self.autosave_existing.get())
                || self
                    .autosave_source
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| autosave::matches_image(p)),
        )?;
        self.autosave_target.replace(Some(target));
        self.project_path.replace(Some(path));
        if previous.is_none() {
            self.automatic_project.set(true);
        }
        Ok(())
    }

    fn flush_autosave(&self) -> Result<()> {
        self.poll_recovery();
        self.save_recovery()?;
        let result = self.session.borrow().as_ref().map_or(Ok(()), |s| s.flush());
        self.poll_recovery();
        result
    }

    fn save_recovery(&self) -> Result<()> {
        let session = self.session.borrow();
        let Some(session) = session.as_ref() else {
            return Ok(());
        };
        let target = self
            .prepare_autosave()
            .map(|_| self.autosave_target.borrow().clone())
            .map_err(|error| format!("{error:#}"));
        let s = self.state.borrow();
        let Some(doc) = &s.document else {
            return Ok(());
        };
        if self.recovery_revision.get() == Some(doc.revision()) {
            return Ok(());
        }
        self.recovery_content.set(
            target
                .as_ref()
                .ok()
                .and_then(|t| t.as_ref())
                .map(|_| doc.content_id()),
        );
        let ticket =
            session.save_with_sidecar(doc, self.recovery_revision.get().is_none(), target)?;
        self.recovery_revision.set(Some(doc.revision()));
        self.recovery_ticket.set(Some(ticket));
        drop(s);
        self.update_storage();
        Ok(())
    }
    fn poll_recovery(&self) {
        if let Some(session) = self.session.borrow().as_ref() {
            for completion in session.completions() {
                if Some(completion.ticket) != self.recovery_ticket.get() {
                    continue;
                }
                match completion.result {
                    Ok(()) => {
                        self.recovery_ack.set(Some(completion.ticket));
                        if let Some(content) = self.recovery_content.get() {
                            let mut s = self.state.borrow_mut();
                            s.project_state = Some(content);
                            s.dirty = s
                                .document
                                .as_ref()
                                .is_some_and(|d| d.content_id() != content);
                        }
                        *self.recovery_error.borrow_mut() = None;
                    }
                    Err(e) => {
                        *self.recovery_error.borrow_mut() = Some(e);
                        self.recovery_revision.set(None);
                    }
                }
            }
        }
        self.update_storage();
    }
    fn update_storage(&self) {
        let s = self.state.borrow();
        let Some(doc) = &s.document else {
            self.storage.set_text("");
            self.storage_button.hide();
            self.project_save_button.hide();
            return;
        };
        let revision = doc.revision();
        let session = if self.recovery_error.borrow().is_some() {
            crate::i18n::tr("Sicherung fehlgeschlagen")
        } else if self.session.borrow().is_none() {
            crate::i18n::tr("Autosicherung aus")
        } else if self.recovery_revision.get() == Some(revision)
            && self.recovery_ticket.get().is_some()
            && self.recovery_ack.get() == self.recovery_ticket.get()
        {
            crate::i18n::tr("Automatisch gesichert")
        } else {
            crate::i18n::tr("Sicherung ausstehend")
        };
        let content_id = doc.content_id();
        let project = match s.project_state {
            Some(r) if r == content_id => crate::i18n::tr("Projekt gespeichert"),
            Some(_) => crate::i18n::tr("Projekt geändert"),
            None => crate::i18n::tr("Projekt noch nicht gespeichert"),
        };
        let export = match s.export_state {
            Some(r) if r == content_id => crate::i18n::tr("PNG aktuell"),
            Some(_) => crate::i18n::tr("PNG enthält älteren Stand"),
            None => crate::i18n::tr("Noch kein PNG-Export"),
        };
        let pending = if self.inline_edit.borrow().is_some() {
            crate::i18n::tr(" · Text in Bearbeitung")
        } else if self.layer_rename.get().is_some() {
            crate::i18n::tr(" · Ebenenname in Bearbeitung")
        } else {
            ""
        };
        let image_path = self.export_path.borrow();
        let project_path = self.project_path.borrow();
        let filename = |path: &Option<PathBuf>| {
            path.as_ref()
                .and_then(|p| p.file_name())
                .map(|p| crate::i18n::format!("\n{}", p.to_string_lossy()))
                .unwrap_or_default()
        };
        let sections = [
            (
                crate::i18n::tr("Bilddatei zum Teilen"),
                export,
                filename(&image_path),
            ),
            (
                crate::i18n::tr("Projekt zum Weiterbearbeiten"),
                project,
                filename(&project_path),
            ),
            (
                crate::i18n::tr("Beim nächsten Start"),
                session,
                pending.to_string(),
            ),
        ];
        let message = sections
            .iter()
            .map(|(title, state, file)| crate::i18n::format!("{title}\n{state}{file}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut details = sections
            .iter()
            .map(|(title, state, file)| {
                crate::i18n::format!(
                    "<b>{}</b>\n{}{}",
                    glib::markup_escape_text(title),
                    glib::markup_escape_text(state),
                    glib::markup_escape_text(file),
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut tooltip = message.clone();
        for path in [&*image_path, &*project_path].into_iter().flatten() {
            tooltip.push_str(&crate::i18n::format!("\n\n{}", path.display()));
        }
        let show_project =
            self.stack.visible_child_name().as_deref() == Some("editor") && !s.selecting_region;
        self.storage_button.set_visible(show_project);
        self.project_save_button
            .set_visible(show_project && self.session.borrow().is_none());
        let project_status = if self.inline_edit.borrow().is_some() {
            crate::i18n::tr("Projekt: Text in Bearbeitung")
        } else if self.layer_rename.get().is_some() {
            crate::i18n::tr("Projekt: Name in Bearbeitung")
        } else if self.session.borrow().is_some() {
            session
        } else {
            match s.project_state {
                Some(r) if r == content_id => crate::i18n::tr("Projekt: gespeichert"),
                Some(_) => crate::i18n::tr("Projekt: geändert"),
                None => crate::i18n::tr("Projekt: ungespeichert"),
            }
        };
        self.storage_caption
            .set_text(&if self.recovery_error.borrow().is_some() {
                crate::i18n::format!("{project_status} · Sicherungsfehler")
            } else {
                project_status.into()
            });
        if self.recovery_error.borrow().is_some() {
            self.storage_button.style_context().add_class("error");
        } else {
            self.storage_button.style_context().remove_class("error");
        }
        self.storage_button.set_tooltip_text(Some(
            &self
                .recovery_error
                .borrow()
                .clone()
                .unwrap_or_else(|| tooltip.clone()),
        ));
        if let Some(error) = self.recovery_error.borrow().as_ref() {
            details.push_str(&crate::i18n::format!(
                "\n\n{}",
                glib::markup_escape_text(error)
            ));
        }
        self.storage.set_markup(&details);
        self.storage.set_tooltip_text(Some(
            &self.recovery_error.borrow().clone().unwrap_or(tooltip),
        ));
    }
    fn close(&self) -> bool {
        if self.closed.get() {
            return true;
        }
        self.commit_pending_edits();
        if self.session.borrow().is_none() && !self.can_discard() {
            return false;
        }
        if let Err(error) = self.save_preferences() {
            self.error(error.context(crate::i18n::tr(
                "Einstellungen konnten nicht gespeichert werden",
            )));
            return false;
        }
        let result = self
            .save_recovery()
            .and_then(|_| self.session.borrow().as_ref().map_or(Ok(()), |s| s.flush()));
        if let Err(error) = result {
            self.error(error.context(crate::i18n::tr(
                "Sitzung konnte vor dem Schließen nicht gesichert werden",
            )));
            return false;
        }
        self.poll_recovery();
        self.session.borrow_mut().take();
        self.autosave_target.borrow_mut().take();
        {
            let mut s = self.state.borrow_mut();
            s.generation += 1;
            s.capturing = false;
        }
        if let Some(cancel) = self.capture_cancel.borrow_mut().take() {
            cancel.cancel();
        }
        self.recent.stop();
        self.closed.set(true);
        self.dismiss_error();
        self.dismiss_shortcuts();
        true
    }
    fn duplicate_layers(&self) {
        self.commit_pending_edits();
        let mut s = self.state.borrow_mut();
        let indices = s.indices();
        if indices.is_empty() {
            return;
        }
        if let Some(doc) = &mut s.document {
            let copies = doc.duplicate_many(&indices);
            s.selected = copies.first().copied();
            s.selected_many = copies;
            s.dirty = true;
        }
        drop(s);
        self.tools[0].set_active(true);
        self.refresh();
    }
    fn align_layers(&self, alignment: Alignment) {
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        let indices = s.indices();
        if indices.len() < 2 {
            drop(s);
            self.status.set_text(crate::i18n::tr(
                "Zum Ausrichten mindestens zwei Objekte mit Shift-Klick auswählen.",
            ));
            return;
        }
        if let Some(doc) = &mut s.document {
            doc.align(&indices, alignment);
            s.dirty = true;
        }
        drop(s);
        self.refresh();
    }
    fn nudge(&self, dx: f64, dy: f64, resize: bool) {
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        if s.tool == Tool::Marquee {
            if let (Some(r), Some(bounds)) = (s.pixel_selection, s.canvas_bounds()) {
                s.pixel_selection = Some(selection::nudge(r, dx, dy, resize, bounds));
            }
            drop(s);
            self.refresh();
            return;
        }
        let crop = s.tool == Tool::Select || s.selecting_region;
        let indices = s.indices();
        if let Some(doc) = &mut s.document {
            let revision = doc.revision();
            if crop {
                if let Some(r) = doc.selection() {
                    doc.select(Some(selection::nudge(r, dx, dy, resize, doc.bounds())));
                }
            } else {
                let edits = indices
                    .iter()
                    .filter_map(|i| {
                        doc.annotations()
                            .get(*i)
                            .map(|a| (*i, a.translated(dx, dy)))
                    })
                    .collect();
                doc.replace_many(edits);
            }
            s.dirty |= doc.revision() != revision;
        }
        drop(s);
        self.refresh();
    }

    fn refresh(&self) {
        self.update_close_notice();
        self.recent.attach_to(
            if self.stack.visible_child_name().as_deref() == Some("editor") {
                &self.history_canvas
            } else {
                &self.history_setup
            },
        );
        if self.layer_rename.get().is_some() && !self.layer_rename_valid() {
            self.finish_layer_rename(false);
        }
        self.syncing.set(true);
        let mut s = self.state.borrow_mut();
        let selected = s
            .selected
            .and_then(|i| s.document.as_ref().and_then(|d| d.annotations().get(i)))
            .cloned();
        if let Some(a) = selected
            .as_ref()
            .filter(|a| !matches!(a.tool, Tool::Image(_)))
        {
            s.color = a.color;
            if a.tool == Tool::Text {
                s.font_size = a.text_size();
            } else {
                s.width = a.width;
            }
        }
        if let Some(edit) = self.inline_edit.borrow().as_ref() {
            s.font_size = edit.annotation.text_size();
            s.color = edit.annotation.color;
        }
        let editing = self
            .stack
            .visible_child_name()
            .is_some_and(|n| n == "editor")
            && !s.selecting_region;
        let ready = s.document.is_some() && !s.capturing && editing;
        self.save.set_sensitive(ready);
        self.export_button.set_sensitive(ready);
        self.project_save_button.set_sensitive(ready);
        self.save_as.set_sensitive(ready);
        self.file_menu.set_visible(!s.selecting_region);
        self.file_menu.set_sensitive(!s.capturing);
        self.recent.set_available(!s.selecting_region);
        self.recent.root.set_sensitive(!s.capturing);
        self.recent.sync_visibility();
        self.copy
            .set_sensitive(ready && (s.tool != Tool::Marquee || s.pixel_selection.is_some()));
        self.paste_layer_button.set_sensitive(ready);
        let marquee = editing && s.tool == Tool::Marquee;
        self.selection_bar.set_visible(marquee);
        self.copy_selection_layer
            .set_sensitive(ready && s.pixel_selection.is_some());
        self.copy_selection
            .set_sensitive(ready && s.pixel_selection.is_some());
        let can_append = ready;
        self.append_image.set_sensitive(can_append);
        self.recent.set_can_append(can_append);
        self.base
            .set_visible(s.document.as_ref().is_some_and(|d| !d.is_composition()));
        self.paste
            .set_sensitive(!s.capturing && !s.selecting_region);
        self.project.set_sensitive(ready);
        self.project_as.set_sensitive(ready);
        self.header.set_visible(!s.selecting_region);
        self.export_actions.set_visible(editing);
        self.new_capture.set_visible(editing);
        self.history_controls.set_visible(editing);
        self.titlebar.set_title(Some(if s.selecting_region {
            crate::i18n::tr("Bereich auswählen")
        } else {
            "UntuShot"
        }));
        self.context.set_visible(!s.selecting_region);
        self.monitor_guides
            .set_visible(s.document.as_ref().is_some_and(|d| d.monitors().len() > 1));
        self.monitor_button.set_visible(
            s.document
                .as_ref()
                .is_some_and(|d| !d.monitors().is_empty()),
        );
        self.region_instruction.set_text(if s.selecting_region {
            if s.document.as_ref().is_some_and(|d| d.selection().is_some()) {
                crate::i18n::tr("Ausschnitt prüfen · Griffe ziehen zum Anpassen · Enter: im Editor öffnen · Esc: zurück")
            } else {
                crate::i18n::tr("Bereich aufziehen · Maustaste gedrückt halten · Esc: zurück")
            }
        } else {crate::i18n::tr("Griffe ziehen · Innen verschieben · Shift+Ziehen: neu · Pfeile: 1 px")});
        self.sidebar.set_visible(!s.selecting_region);
        self.left_panel
            .set_visible(!s.selecting_region && self.show_tools.is_active());
        let has_layers = s
            .document
            .as_ref()
            .is_some_and(|d| !d.annotations().is_empty());
        if !has_layers && !self.layers_manually_shown.get() && self.show_layers.is_active() {
            self.auto_layers_hidden.set(true);
            self.show_layers.set_active(false);
        } else if has_layers && self.auto_layers_hidden.replace(false) {
            self.show_layers.set_active(true);
        }
        self.right_panel
            .set_visible(!s.selecting_region && self.show_layers.is_active());
        self.inspector.set_visible(!s.selecting_region);
        self.region_bar
            .set_visible(s.selecting_region || s.tool == Tool::Select);
        self.confirm_region
            .set_sensitive(s.document.as_ref().is_some_and(|d| d.selection().is_some()));
        self.setup.start.set_sensitive(!s.capturing);
        self.setup.demo.set_sensitive(!s.capturing);
        self.setup.web.set_sensitive(!s.capturing);
        self.setup
            .cancel
            .set_visible(s.document.is_some() || s.capturing);
        self.setup.cancel.set_label(if s.capturing {
            crate::i18n::tr("Countdown abbrechen")
        } else {
            crate::i18n::tr("Zurück zum Editor")
        });
        let layer_count = s.document.as_ref().map_or(0, |d| d.annotations().len());
        self.layer_previews.borrow_mut().truncate(layer_count);
        self.layers_heading.set_text(&if layer_count == 0 {
            crate::i18n::tr("Ebenen").into()
        } else {
            crate::i18n::format!("Ebenen · {layer_count}")
        });
        if let Some(doc) = &s.document {
            let (_, _, w, h) = doc
                .selection()
                .unwrap_or(doc.bounds())
                .pixel_bounds(doc.width(), doc.height());
            self.dimensions.set_text(&crate::i18n::format!(
                "{w} × {h} px · {}%",
                (s.view.scale * 100.).round()
            ));
            let preset = if let Some(rect) = doc.selection() {
                doc.monitors()
                    .iter()
                    .position(|m| m.rect == rect)
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "custom".into())
            } else {
                "all".into()
            };
            self.monitors.set_active_id(Some(&preset));
            self.monitor_map.select(preset.parse().ok());
            self.sync_layer_rows(doc, &s.indices(), ready);
        }
        self.layers.show_all();
        let count = s.indices().len();
        self.empty_layers.set_visible(layer_count == 0);
        self.layer_actions.set_visible(count > 0);
        self.property_scroll.set_visible(count > 0);
        self.selection_label.set_visible(count > 0);
        self.visible.set_visible(count > 0);
        self.duplicate.set_visible(count > 0);
        self.duplicate.set_sensitive(count > 0);
        self.object_properties
            .set_visible(count == 1 && self.inline_edit.borrow().is_none());
        self.alignment.set_visible(count > 1);
        self.align.set_visible(count > 1);
        self.selection_label.set_text(&if count > 1 {
            crate::i18n::format!("{count} Objekte ausgewählt")
        } else {
            crate::i18n::tr("Eigenschaften").into()
        });
        self.visible.set_label(crate::i18n::tr("Sichtbar"));
        let mixed = selected.as_ref().is_some_and(|a| {
            s.indices()
                .iter()
                .any(|i| s.document.as_ref().unwrap().annotations()[*i].visible != a.visible)
        });
        self.visible.set_inconsistent(mixed);
        self.visible.set_sensitive(selected.is_some());
        self.layer_name
            .set_sensitive(selected.is_some() && s.selected_many.len() == 1);
        if self.layer_rename.get().is_none() {
            self.layer_name.set_text(
                &selected
                    .as_ref()
                    .filter(|_| count == 1)
                    .map(|a| a.name())
                    .unwrap_or_default(),
            );
        }
        self.delete.set_sensitive(selected.is_some());
        self.edit_text
            .set_visible(count == 1 && selected.as_ref().is_some_and(|a| a.tool == Tool::Text));
        let image_selected = count == 1
            && selected
                .as_ref()
                .is_some_and(|a| matches!(a.tool, Tool::Image(_)));
        self.image_effects.set_visible(image_selected);
        self.image_effects.set_sensitive(ready);
        if image_selected {
            let e = &selected.as_ref().unwrap().effects;
            self.border_enabled.set_active(e.border);
            self.sync_property(4, e.border_width);
            self.border_color.set_rgba(&gdk::RGBA::new(
                e.border_color.0,
                e.border_color.1,
                e.border_color.2,
                1.,
            ));
            self.shadow_enabled.set_active(e.shadow);
            self.sync_property(5, e.shadow_blur);
            self.sync_property(6, e.shadow_offset);
            self.sync_property(7, e.shadow_opacity * 100.);
            self.shadow_grid.set_visible(e.shadow);
            self.border_width.set_sensitive(e.border);
            self.border_color.set_sensitive(e.border);
            for spin in [&self.shadow_blur, &self.shadow_offset, &self.shadow_opacity] {
                spin.set_sensitive(e.shadow);
            }
        }
        for spin in &self.properties {
            spin.set_sensitive(selected.is_some() && s.selected_many.len() == 1);
        }
        if let Some(a) = selected {
            let r = a.bounds();
            for (field, value) in [r.x, r.y, r.w, r.h].into_iter().enumerate() {
                self.sync_property(field, value);
            }
            self.visible.set_active(a.visible);
            self.color
                .set_rgba(&gdk::RGBA::new(a.color.0, a.color.1, a.color.2, 1.));
        }
        self.font_controls.set_visible(
            s.tool == Tool::Text
                || self.inline_edit.borrow().is_some()
                || s.indices().iter().any(|i| {
                    s.document
                        .as_ref()
                        .is_some_and(|d| d.annotations()[*i].tool == Tool::Text)
                }),
        );
        self.width.set_value(s.width);
        self.color
            .set_rgba(&gdk::RGBA::new(s.color.0, s.color.1, s.color.2, 1.));
        let text_only = self.inline_edit.borrow().is_some()
            || s.tool == Tool::Text
            || (!s.indices().is_empty()
                && s.indices().iter().all(|i| {
                    s.document
                        .as_ref()
                        .is_some_and(|d| d.annotations()[*i].tool == Tool::Text)
                }));
        let images_only = s.tool == Tool::Move
            && !s.indices().is_empty()
            && s.indices().iter().all(|i| {
                s.document
                    .as_ref()
                    .is_some_and(|d| matches!(d.annotations()[*i].tool, Tool::Image(_)))
            });
        let drawing_style = !images_only && !matches!(s.tool, Tool::Select | Tool::Marquee);
        self.color.set_sensitive(drawing_style);
        self.color_controls.set_visible(drawing_style);
        self.style_label.set_visible(drawing_style);
        self.properties[2].set_tooltip_text(Some(if images_only {
            crate::i18n::tr("Breite ändern · Höhe wird proportional angepasst")
        } else {
            crate::i18n::tr("Breite in Bildpixeln")
        }));
        self.properties[3].set_tooltip_text(Some(if images_only {
            crate::i18n::tr("Höhe ändern · Breite wird proportional angepasst")
        } else {
            crate::i18n::tr("Höhe in Bildpixeln")
        }));
        self.width.set_visible(!text_only && drawing_style);
        self.width_controls.set_visible(!text_only && drawing_style);
        self.width_picker.set_visible(!text_only && drawing_style);
        self.style_label.set_text(if text_only {
            crate::i18n::tr("Textstil")
        } else {
            crate::i18n::tr("Zeichenstil")
        });
        self.font_size.set_value(s.font_size);
        self.inline_box
            .set_visible(self.inline_edit.borrow().is_some());
        s.dirty = s.document.as_ref().is_some_and(|d| {
            (d.content_id() > 0 || !d.annotations().is_empty() || d.selection().is_some())
                && s.project_state != Some(d.content_id())
        });
        self.window.set_title(if s.selecting_region {
            crate::i18n::tr("UntuShot · Bereich auswählen")
        } else if s.dirty && s.export_state == s.document.as_ref().map(Document::content_id) {
            crate::i18n::tr("UntuShot · Bild gespeichert")
        } else if s.dirty {
            crate::i18n::tr("UntuShot · Bearbeitet")
        } else {
            "UntuShot"
        });
        let indices = s.indices();
        let changed_selection = *self.layer_selection.borrow() != indices;
        let reveal_single = changed_selection && indices.len() == 1;
        *self.layer_selection.borrow_mut() = indices;
        drop(s);
        self.syncing.set(false);
        if reveal_single {
            let weak = self.weak_self.clone();
            let allocated = Cell::new(false);
            // Selection can reveal the effects controls and shrink the list.
            // Wait for that layout before using its final page size to scroll.
            self.area.add_tick_callback(move |_, _| {
                if !allocated.replace(true) {
                    return glib::ControlFlow::Continue;
                }
                if let Some(e) = weak.upgrade() {
                    e.reveal_layer.set(true);
                    e.reveal_selected_layer();
                }
                glib::ControlFlow::Break
            });
        }
        self.update_history_controls();
        self.update_storage();
        self.area.queue_draw();
    }

    fn sync_layer_rows(&self, doc: &Document, selected: &[usize], ready: bool) {
        let mut rows = self.layer_rows.borrow_mut();
        if rows.len() != doc.annotations().len()
            || (rows.is_empty() && !self.layers.children().is_empty())
        {
            for child in self.layers.children() {
                self.layers.remove(&child);
            }
            rows.clear();
            for i in 0..doc.annotations().len() {
                let row = gtk::ListBoxRow::new();
                row.set_widget_name(&i.to_string());
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                content.set_border_width(6);
                let grip = gtk::Label::new(Some("⠿"));
                grip.set_opacity(0.45);
                grip.set_tooltip_text(Some(crate::i18n::tr(
                    "Ebene ziehen, um die Reihenfolge zu ändern",
                )));
                content.pack_start(&grip, false, false, 0);
                let thumbnail = gtk::Image::new();
                let label = gtk::Label::new(None);
                label.set_xalign(0.);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_width_chars(1);
                label.set_max_width_chars(14);
                let visibility = icon_button("eye", "", crate::i18n::tr("Ebene ausblenden"));
                visibility.set_widget_name("layer-visibility");
                visibility.style_context().add_class("flat");
                content.pack_start(&thumbnail, false, false, 0);
                let title = gtk::Stack::new();
                title.set_homogeneous(false);
                title.add_named(&label, "name");
                content.pack_start(&title, true, true, 0);
                content.pack_end(&visibility, false, false, 0);
                row.add(&content);
                let revision = Rc::new(Cell::new(doc.revision()));
                let token = revision.clone();
                let weak = self.weak_self.clone();
                let weak_row = row.downgrade();
                visibility.connect_clicked(move |_| {
                    if let (Some(e), Some(row)) = (weak.upgrade(), weak_row.upgrade()) {
                        // Detached rows must not act on a replacement document/layer.
                        if row.parent().as_ref() == Some(e.layers.upcast_ref()) {
                            e.toggle_layer_visibility(i, token.get());
                        }
                    }
                });
                self.layers.insert(&row, 0);
                rows.push(LayerRow {
                    row,
                    thumbnail,
                    label,
                    title,
                    visibility,
                    revision,
                    content: None,
                });
            }
        }
        let scale = self.window.scale_factor().clamp(1, 8);
        for (i, (row, a)) in rows.iter_mut().zip(doc.annotations()).enumerate() {
            row.revision.set(doc.revision());
            row.visibility.set_sensitive(ready);
            let key = (crate::preview::relative_layer(a), scale);
            if row.content.as_ref() != Some(&key) {
                match self.layer_previews.borrow_mut().get(doc, i, scale) {
                    Ok(surface) => {
                        row.thumbnail.set_from_surface(Some(&surface));
                        row.thumbnail.set_tooltip_text(None);
                    }
                    Err(error) => {
                        row.thumbnail.set_from_icon_name(
                            Some("image-missing-symbolic"),
                            gtk::IconSize::SmallToolbar,
                        );
                        row.thumbnail.set_tooltip_text(Some(&crate::i18n::format!(
                            "Vorschau nicht verfügbar: {error}"
                        )));
                    }
                }
                row.label.set_text(&a.name());
                row.label.set_tooltip_text(Some(&a.name()));
                row.thumbnail.set_opacity(if a.visible { 1. } else { 0.45 });
                row.label.set_opacity(if a.visible { 1. } else { 0.55 });
                if row.content.as_ref().map(|(a, _)| a.visible) != Some(a.visible) {
                    if let Some(child) = row.visibility.child() {
                        row.visibility.remove(&child);
                    }
                    row.visibility.add(&icons::button_content(
                        if a.visible { "eye" } else { "eye-off" },
                        "",
                    ));
                }
                row.visibility.set_tooltip_text(Some(&crate::i18n::format!(
                    "{}: {}",
                    a.name(),
                    if a.visible {
                        crate::i18n::tr("Ebene ausblenden")
                    } else {
                        crate::i18n::tr("Ebene einblenden")
                    }
                )));
                row.content = Some(key);
            }
            if selected.contains(&i) != row.row.is_selected() {
                if selected.contains(&i) {
                    self.layers.select_row(Some(&row.row));
                } else {
                    self.layers.unselect_row(&row.row);
                }
            }
        }
    }

    fn reveal_selected_layer(&self) {
        if !self.reveal_layer.get() || self.syncing.get() || self.layer_drag.borrow().is_some() {
            return;
        }
        let selection = self.layer_selection.borrow();
        let [index] = selection.as_slice() else {
            self.reveal_layer.set(false);
            return;
        };
        let rows = self.layer_rows.borrow();
        let Some(row) = rows.get(*index) else {
            return;
        };
        let allocation = row.row.allocation();
        let adjustment = self.layer_scroll.vadjustment();
        let top = f64::from(allocation.y());
        let bottom = top + f64::from(allocation.height());
        if allocation.height() <= 1 || adjustment.page_size() <= 1. || adjustment.upper() < bottom {
            return;
        }
        self.reveal_layer.set(false);
        adjustment.clamp_page(top, bottom);
    }

    fn rebuild_sources(&self, doc: &Document) {
        let syncing = self.syncing.replace(true);
        self.monitor_map.set_monitors(doc.monitors().to_vec());
        self.monitors.remove_all();
        self.monitors.append(
            Some("all"),
            if doc.monitors().len() > 1 {
                crate::i18n::tr("Ganzes Bild — alle aufgenommenen Bildschirme")
            } else {
                crate::i18n::tr("Ganzes Bild")
            },
        );
        for (i, m) in doc.monitors().iter().enumerate() {
            self.monitors.append(
                Some(&i.to_string()),
                &crate::i18n::format!(
                    "Bildschirm {} · {} · {} × {}",
                    i + 1,
                    m.name,
                    m.rect.w,
                    m.rect.h
                ),
            );
        }
        self.monitors
            .append(Some("custom"), crate::i18n::tr("Eigener Ausschnitt"));
        self.syncing.set(syncing);
    }

    fn set_document(&self, doc: Document, note: &str) {
        self.dismiss_error();
        self.finish_layer_rename(false);
        layer_drag::cancel(self);
        if let Some(menu) = self.object_menu.borrow_mut().take() {
            menu.popdown();
        }
        self.clipboard_pending.set(false);
        self.clipboard_epoch
            .set(self.clipboard_epoch.get().wrapping_add(1));
        self.export_path.borrow_mut().take();
        self.project_path.borrow_mut().take();
        self.autosave_target.borrow_mut().take();
        self.autosave_source.borrow_mut().take();
        self.automatic_project.set(false);
        self.autosave_existing.set(false);
        self.recovery_content.set(None);
        self.preview.borrow_mut().clear();
        self.finish_text(false);
        self.layer_previews.borrow_mut().clear();
        self.layer_rows.borrow_mut().clear();
        self.layer_selection.borrow_mut().clear();
        self.rebuild_sources(&doc);
        let mut s = self.state.borrow_mut();
        self.recovery_revision.set(None);
        self.recovery_ticket.set(None);
        self.recovery_ack.set(None);
        *self.recovery_error.borrow_mut() = None;
        s.project_state = None;
        s.export_state = None;
        s.pan = Point::default();
        s.pan_drag = None;
        s.mouse = None;
        s.space = false;
        s.snap_bypass = false;
        s.document = Some(doc);
        s.draft = None;
        s.drag = None;
        s.selected = None;
        s.selected_many.clear();
        s.crop_drag = None;
        s.pixel_selection = None;
        s.pointer = None;
        s.zoom = None;
        s.selecting_region = false;
        s.region_bounds = None;
        s.dirty = false;
        drop(s);
        self.area.set_size_request(-1, -1);
        self.stack.set_visible_child_name("editor");
        self.tools[0].set_active(true);
        self.status.set_text(note);
        self.status.set_tooltip_text(Some(note));
        self.area.grab_focus();
        self.refresh();
    }

    fn show_setup(&self) {
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        s.draft = None;
        s.drag = None;
        s.crop_drag = None;
        s.pointer = None;
        s.selecting_region = false;
        drop(s);
        self.stack.set_visible_child_name("setup");
        self.setup.refresh_layout();
        self.setup.progress.set_text("");
        self.refresh();
    }

    fn sync_monitor_layout(&self, layout: Option<Layout>) {
        let capturing = self.is_capturing();
        if !capturing && self.stack.visible_child_name().as_deref() != Some("setup") {
            return;
        }
        if Layout::new(self.setup.map.monitors()) == layout {
            return;
        }
        if capturing {
            self.setup.cancel.emit_clicked();
            self.stack.set_visible_child_name("setup");
            self.window.present();
        }
        self.setup
            .update_monitors(layout.map(|l| l.monitors).unwrap_or_default());
        let note = if capturing {
            crate::i18n::tr("Aufnahme gestoppt: Monitor-Anordnung geändert. Bitte Quelle prüfen und erneut starten.")
        } else {
            crate::i18n::tr("Monitor-Anordnung aktualisiert. Bitte die gewünschte Quelle prüfen.")
        };
        self.setup.progress.set_text(note);
        self.status.set_text(note);
        self.refresh();
    }

    fn finish_region(&self) {
        if !self
            .state
            .borrow()
            .document
            .as_ref()
            .is_some_and(|d| d.selection().is_some())
        {
            return;
        }
        self.state.borrow_mut().selecting_region = false;
        self.tools[0].set_active(true);
        self.status.set_text(crate::i18n::tr(
            "Ausschnitt übernommen · Zeichnen oder V zum Bearbeiten einzelner Objekte",
        ));
        self.refresh();
    }

    fn can_discard(&self) -> bool {
        self.commit_pending_edits();
        if self.session.borrow().is_some() {
            return match self.flush_autosave() {
                Ok(()) => true,
                Err(error) => {
                    self.error(error.context(crate::i18n::tr(
                        "Änderungen konnten nicht automatisch gespeichert werden",
                    )));
                    false
                }
            };
        }
        if !self.state.borrow().dirty {
            return true;
        }
        let image_saved = {
            let s = self.state.borrow();
            s.export_state.is_some()
                && s.export_state == s.document.as_ref().map(Document::content_id)
        };
        let dialog = gtk::MessageDialog::new(
            Some(&self.window),
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Question,
            gtk::ButtonsType::None,
            if image_saved {
                crate::i18n::tr("Bild gespeichert – Bearbeitung behalten?")
            } else {
                crate::i18n::tr("Änderungen speichern?")
            },
        );
        dialog.set_title(crate::i18n::tr("Änderungen speichern"));
        dialog.set_secondary_text(Some(if image_saved {
            crate::i18n::tr("Die PNG-Datei ist aktuell. Ein Projekt erhält zusätzlich das ursprüngliche Bild, den Ausschnitt und die einzeln bearbeitbaren Markierungen.")
        } else {
            crate::i18n::tr("Als PNG speicherst du das fertige Bild zum Teilen. Ein Projekt erhält zusätzlich das ursprüngliche Bild und die einzeln bearbeitbaren Markierungen.")
        }));
        let retention = gtk::Label::new(Some(crate::i18n::tr("Bearbeitbare Projekte enthalten das Originalbild – auch verdeckte oder geschwärzte Inhalte. Zum Teilen ein Bild exportieren.")));
        retention.set_line_wrap(true);
        retention.set_max_width_chars(65);
        retention.set_xalign(0.);
        retention.show();
        dialog.content_area().add(&retention);
        dialog.add_buttons(&[
            (crate::i18n::tr("Zurück"), gtk::ResponseType::Cancel),
            (
                if image_saved {
                    crate::i18n::tr("Ohne Projekt weiter")
                } else {
                    crate::i18n::tr("Nicht speichern")
                },
                gtk::ResponseType::Accept,
            ),
            (
                crate::i18n::tr("Bearbeitbares Projekt speichern"),
                gtk::ResponseType::Other(2),
            ),
        ]);
        if !image_saved {
            dialog.add_button(
                crate::i18n::tr("Bild speichern"),
                gtk::ResponseType::Other(1),
            );
        }
        dialog.set_default_response(gtk::ResponseType::Cancel);
        dialog.connect_key_press_event(|dialog, event| {
            use gdk::keys::constants as key;
            if event.state().contains(gdk::ModifierType::CONTROL_MASK) {
                match event.keyval() {
                    key::s | key::S => {
                        dialog.response(if event.state().contains(gdk::ModifierType::MOD1_MASK) {
                            if event.state().contains(gdk::ModifierType::SHIFT_MASK) {
                                gtk::ResponseType::Other(4)
                            } else {
                                gtk::ResponseType::Other(2)
                            }
                        } else if event.state().contains(gdk::ModifierType::SHIFT_MASK) {
                            gtk::ResponseType::Other(3)
                        } else {
                            gtk::ResponseType::Other(1)
                        });
                        return glib::Propagation::Stop;
                    }
                    key::w | key::W => {
                        dialog.response(gtk::ResponseType::Cancel);
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
            glib::Propagation::Proceed
        });
        let result = dialog.run();
        dialog.hide();
        dialog.close();
        match result {
            gtk::ResponseType::Accept => true,
            gtk::ResponseType::Other(1) => self.save(),
            gtk::ResponseType::Other(3) => self.save_as(),
            gtk::ResponseType::Other(2) => self.save_project(),
            gtk::ResponseType::Other(4) => self.save_project_as(),
            _ => false,
        }
    }

    fn toggle_layer_visibility(&self, index: usize, revision: u64) {
        {
            let s = self.state.borrow();
            if s.capturing
                || s.selecting_region
                || self.stack.visible_child_name().as_deref() != Some("editor")
                || !s
                    .document
                    .as_ref()
                    .is_some_and(|d| d.revision() == revision && index < d.annotations().len())
            {
                return;
            }
        }
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        let Some(doc) = &mut s.document else {
            return;
        };
        if let Some(mut layer) = doc.annotations().get(index).cloned() {
            layer.visible = !layer.visible;
            doc.replace(index, layer);
        }
        drop(s);
        self.refresh();
    }

    fn edit_selected(&self, mut edit: impl FnMut(&mut Annotation)) {
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        let indices = s.indices();
        if indices.is_empty() {
            return;
        }
        if let Some(doc) = &mut s.document {
            let edits = indices
                .iter()
                .filter_map(|i| doc.annotations().get(*i).map(|a| (*i, a.clone())))
                .map(|(i, mut a)| {
                    edit(&mut a);
                    (i, a)
                })
                .collect();
            doc.replace_many(edits);
            s.dirty = true;
        }
        drop(s);
        self.refresh();
    }
    fn change_style(&self) {
        let s = self.state.borrow();
        let color = s.color;
        let width = s.width;
        drop(s);
        self.prepare_text_change();
        if let Some(edit) = self.inline_edit.borrow_mut().as_mut() {
            edit.annotation.color = color;
        }
        if self.inline_edit.borrow().is_some() {
            self.finish_text_change();
            self.update_inline_style();
            self.area.queue_draw();
            return;
        }
        self.edit_selected(|a| {
            if matches!(a.tool, Tool::Image(_)) {
                return;
            }
            a.color = color;
            if a.tool != Tool::Text {
                a.width = width;
            }
        });
    }
    fn property_spins(&self) -> [&gtk::SpinButton; 8] {
        [
            &self.properties[0],
            &self.properties[1],
            &self.properties[2],
            &self.properties[3],
            &self.border_width,
            &self.shadow_blur,
            &self.shadow_offset,
            &self.shadow_opacity,
        ]
    }
    fn sync_property(&self, field: usize, value: f64) {
        let typing = self
            .property_edit
            .get()
            .is_some_and(|(active, _, _)| active == field)
            && self
                .window
                .focused_widget()
                .is_some_and(|w| w == *self.property_spins()[field]);
        if self.property_refresh.get() != Some(field) && !typing {
            self.property_spins()[field].set_value(value);
        }
    }
    fn live_property(&self, field: usize, value: f64) {
        if self.syncing.get() || !value.is_finite() {
            return;
        }
        let adjustment = self.property_spins()[field].adjustment();
        if value < adjustment.lower() || value > adjustment.upper() {
            return;
        }
        let mut s = self.state.borrow_mut();
        if s.capturing || s.selecting_region || s.indices().len() != 1 {
            return;
        }
        let index = s.indices()[0];
        let Some(doc) = s.document.as_mut() else {
            return;
        };
        let Some(mut a) = doc.annotations().get(index).cloned() else {
            return;
        };
        if field >= 4 && !matches!(a.tool, Tool::Image(_)) {
            return;
        }
        let original = a.clone();
        let r = a.bounds();
        let proportional = matches!(a.tool, Tool::Image(_) | Tool::Text);
        match field {
            0 => a = a.translated(value - r.x, 0.),
            1 => a = a.translated(0., value - r.y),
            2 => {
                a = a.resized(
                    value,
                    if proportional {
                        r.h * value / r.w.max(1.)
                    } else {
                        r.h
                    },
                )
            }
            3 => {
                a = a.resized(
                    if proportional {
                        r.w * value / r.h.max(1.)
                    } else {
                        r.w
                    },
                    value,
                )
            }
            4 => a.effects.border_width = value,
            5 => a.effects.shadow_blur = value,
            6 => a.effects.shadow_offset = value,
            7 => a.effects.shadow_opacity = value / 100.,
            _ => unreachable!(),
        }
        if original == a {
            return;
        }
        let previous = self
            .property_edit
            .get()
            .filter(|(f, i, _)| *f == field && *i == index)
            .map(|(_, _, revision)| revision);
        let continuation = doc.replace_property(index, a, previous);
        self.property_edit
            .set(continuation.map(|revision| (field, index, revision)));
        s.dirty = true;
        drop(s);
        // Keep partial typed input intact; synchronize the paired dimension and
        // the rest of the inspector without stealing keyboard focus.
        self.property_refresh.set(Some(field));
        self.refresh();
        self.property_refresh.set(None);
    }
    fn delete_layer(&self) {
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        let indices = s.indices();
        if indices.is_empty() {
            return;
        }
        if let Some(doc) = &mut s.document {
            doc.remove_many(&indices);
            s.dirty = true;
        }
        s.selected = None;
        s.selected_many.clear();
        drop(s);
        self.refresh();
    }
    fn reorder(&self, up: bool) {
        self.layer_rows.borrow_mut().clear();
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        if s.indices().len() > 1 {
            drop(s);
            self.status.set_text(crate::i18n::tr(
                "Zum Ändern der Reihenfolge eine einzelne Ebene auswählen.",
            ));
            return;
        }
        if let Some(i) = s.selected {
            if let Some(doc) = &mut s.document {
                let new = doc.reorder(i, up);
                s.selected = Some(new);
                s.selected_many = vec![new];
                s.dirty = true;
            }
        }
        drop(s);
        self.refresh();
    }

    fn menu_at_object(&self, x: f64, y: f64) -> Option<gtk::Menu> {
        self.finish_text(true);
        let (indices, revision, count, text) = {
            let s = self.state.borrow();
            if s.capturing
                || s.selecting_region
                || s.drag.is_some()
                || s.draft.is_some()
                || s.crop_drag.is_some()
            {
                return None;
            }
            let bounds = s.canvas_bounds()?;
            if !s.view.contains(x, y, bounds) {
                return None;
            }
            let doc = s.document.as_ref()?;
            let hit = doc.hit_test(s.view.unclamped_point(x, y), 6. / s.view.scale)?;
            let indices = if s.indices().contains(&hit) {
                s.indices()
            } else {
                vec![hit]
            };
            let text = indices.len() == 1 && doc.annotations()[indices[0]].tool == Tool::Text;
            (indices, doc.revision(), doc.annotations().len(), text)
        };
        self.tools[0].set_active(true);
        {
            let mut s = self.state.borrow_mut();
            s.selected = indices.first().copied();
            s.selected_many = indices.clone();
        }
        self.refresh();
        let menu = gtk::Menu::new();
        let epoch = self.clipboard_epoch.get();
        let single = indices.len() == 1;
        for (action, label, enabled) in [
            (0, crate::i18n::tr("Duplizieren"), true),
            (
                1,
                crate::i18n::tr("Eine Ebene nach vorne"),
                single && indices[0] + 1 < count,
            ),
            (
                2,
                crate::i18n::tr("Eine Ebene nach hinten"),
                single && indices[0] > 0,
            ),
            (3, crate::i18n::tr("Löschen"), true),
            (4, crate::i18n::tr("Text bearbeiten"), text),
        ] {
            if action == 4 && !text {
                continue;
            }
            let item = gtk::MenuItem::with_label(label);
            item.set_sensitive(enabled);
            item.set_tooltip_text(match action {
                0 => Some(crate::i18n::tr("Strg+D")),
                3 => Some(crate::i18n::tr("Entf · mit Strg+Z rückgängig")),
                _ => None,
            });
            let weak = self.weak_self.clone();
            let expected = indices.clone();
            item.connect_activate(move |_| {
                if !enabled {
                    return;
                }
                let Some(e) = weak.upgrade() else {
                    return;
                };
                {
                    let s = e.state.borrow();
                    if e.closed.get()
                        || e.clipboard_epoch.get() != epoch
                        || s.capturing
                        || s.selecting_region
                        || s.indices() != expected
                        || !s
                            .document
                            .as_ref()
                            .is_some_and(|d| d.revision() == revision)
                    {
                        return;
                    }
                }
                match action {
                    0 => e.duplicate_layers(),
                    1 => e.reorder(true),
                    2 => e.reorder(false),
                    3 => e.delete_layer(),
                    4 => e.text(Point::default()),
                    _ => unreachable!(),
                }
            });
            menu.append(&item);
        }
        menu.show_all();
        Some(menu)
    }

    fn open_path(&self, path: &std::path::Path) -> bool {
        if self.is_capturing() {
            return false;
        }
        if self.session.borrow().is_some() && !self.can_discard() {
            return false;
        }
        let image = match std::fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) => {
                self.error(error.into());
                return false;
            }
        };
        let actual = if autosave::matches_image(&image) {
            autosave::sidecar_path(&image)
        } else {
            image.clone()
        };
        let (doc, project) = match open_document(&actual) {
            Ok(opened) => opened,
            Err(error) => {
                self.error(error);
                return false;
            }
        };
        if self.session.borrow().is_none() && !self.can_discard() {
            return false;
        }
        self.set_document(
            doc,
            if project {
                crate::i18n::tr("Projekt geöffnet · Änderungen werden automatisch gespeichert")
            } else {
                crate::i18n::tr("Bild geöffnet · Das Original bleibt unverändert")
            },
        );
        if project {
            self.project_path.replace(Some(actual.clone()));
            self.autosave_existing.set(true);
            self.mark_project_saved();
        }
        if !project || actual != image {
            self.autosave_source.replace(Some(image.clone()));
        } else if project && autosave::matches_image(&actual.with_extension("")) {
            self.autosave_source
                .replace(Some(actual.with_extension("")));
        }
        self.automatic_project.set(actual != image);
        self.refresh();
        true
    }
    fn export_directory(&self) -> Result<PathBuf> {
        let directory = self
            .preferences
            .borrow()
            .export_directory
            .clone()
            .unwrap_or_else(crate::history::directory);
        std::fs::create_dir_all(&directory)?;
        Ok(directory)
    }

    fn open(&self) {
        self.open_dialog(false);
    }

    fn open_dialog(&self, projects_only: bool) {
        self.commit_pending_edits();
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(if projects_only {
                crate::i18n::tr("Projekt öffnen")
            } else {
                crate::i18n::tr("Bild oder Projekt öffnen")
            }),
            Some(&self.window),
            gtk::FileChooserAction::Open,
            &[
                (crate::i18n::tr("Abbrechen"), gtk::ResponseType::Cancel),
                (crate::i18n::tr("Öffnen"), gtk::ResponseType::Accept),
            ],
        );
        let directory = self
            .preferences
            .borrow()
            .export_directory
            .clone()
            .unwrap_or_else(crate::history::directory);
        if directory.is_dir() {
            dialog.set_current_folder(&directory);
        }
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(if projects_only {
            crate::i18n::tr("UntuShot-Projekte (*.untushot, *.rustshot)")
        } else {
            crate::i18n::tr("Bilder und UntuShot-Projekte")
        }));
        for p in ["*.untushot", "*.rustshot"] {
            filter.add_pattern(p);
        }
        if !projects_only {
            for p in ["*.png", "*.jpg", "*.jpeg"] {
                filter.add_pattern(p);
            }
        }
        dialog.add_filter(filter);
        let all = gtk::FileFilter::new();
        all.set_name(Some(crate::i18n::tr("Alle Dateien")));
        all.add_pattern("*");
        dialog.add_filter(all);
        let response = dialog.run();
        let path = dialog.filename();
        dialog.hide();
        dialog.close();
        if response == gtk::ResponseType::Accept {
            if let Some(path) = path {
                self.open_path(&path);
            }
        }
    }

    fn save_project(&self) -> bool {
        self.commit_pending_edits();
        let path = self.project_path.borrow().clone();
        match path {
            Some(path) => self.save_project_to(&path),
            None => self.save_project_as(),
        }
    }

    fn save_project_to(&self, path: &std::path::Path) -> bool {
        self.commit_pending_edits();
        // Drain background writes before an explicit save; Save As can also
        // rescue a document whose automatic destination is unavailable.
        if let Some(session) = self.session.borrow().as_ref() {
            let _ = session.flush();
        }
        self.poll_recovery();
        let source = self
            .autosave_source
            .borrow()
            .clone()
            .filter(|_| self.project_path.borrow().as_deref() == Some(path));
        let result = (|| -> Result<()> {
            let s = self.state.borrow();
            let doc = s
                .document
                .as_ref()
                .ok_or_else(|| crate::i18n::anyhow!("Kein Bild geöffnet"))?;
            let stamp = source
                .as_deref()
                .map(autosave::SourceStamp::read)
                .transpose()?;
            atomic_write(path, |file| {
                doc.write_project_with_source(file, stamp.as_ref())
            })
        })();
        match result {
            Ok(()) => {
                self.autosave_target.borrow_mut().take();
                self.autosave_source.replace(source);
                self.automatic_project.set(false);
                self.autosave_existing.set(true);
                self.recovery_revision.set(None);
                self.project_path.replace(Some(path.to_path_buf()));
                self.mark_project_saved();
                self.status.set_text(&crate::i18n::format!(
                    "Projekt gespeichert: {}",
                    path.display()
                ));
                self.refresh();
                true
            }
            Err(error) => {
                self.error(error);
                false
            }
        }
    }

    fn save_project_as(&self) -> bool {
        self.commit_pending_edits();
        if self.state.borrow().document.is_none() {
            return false;
        }
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(crate::i18n::tr("Bearbeitbares Projekt speichern")),
            Some(&self.window),
            gtk::FileChooserAction::Save,
            &[
                (crate::i18n::tr("Abbrechen"), gtk::ResponseType::Cancel),
                (
                    crate::i18n::tr("Bearbeitbares Projekt speichern"),
                    gtk::ResponseType::Accept,
                ),
            ],
        );
        let warning = gtk::Label::new(Some(crate::i18n::tr("Bearbeitbare Projekte enthalten das Originalbild – auch verdeckte oder geschwärzte Inhalte. Zum Teilen ein Bild exportieren.")));
        warning.set_line_wrap(true);
        warning.set_max_width_chars(65);
        warning.set_xalign(0.);
        warning.show();
        dialog.set_extra_widget(&warning);
        let previous = self.project_path.borrow().clone();
        let directory = previous
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(|| self.export_directory());
        match directory {
            Ok(directory) => {
                dialog.set_current_folder(directory);
            }
            Err(error) => {
                dialog.hide();
                dialog.close();
                self.error(error);
                return false;
            }
        }
        let name = previous
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| suggested_filename("untushot"));
        dialog.set_current_name(&name);
        dialog.set_do_overwrite_confirmation(true);
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(crate::i18n::tr("UntuShot-Projekt (*.untushot)")));
        filter.add_pattern("*.untushot");
        dialog.add_filter(filter);
        let response = dialog.run();
        let path = dialog.filename();
        dialog.hide();
        dialog.close();
        response == gtk::ResponseType::Accept && path.is_some_and(|p| self.save_project_to(&p))
    }

    fn new_screenshot(self: &Rc<Self>) {
        if self.is_capturing() {
            return;
        }
        if self.setup.uses_desktop_chooser() {
            self.setup.refresh_layout();
            self.setup.system.set_active(true);
            self.setup.source.set_active_id(Some("all"));
            self.setup.delay.set_value(0.);
            self.capture_with_visibility(true);
        } else {
            self.show_setup();
        }
    }

    fn capture(self: &Rc<Self>) {
        self.capture_with_visibility(false);
    }
    fn capture_with_visibility(self: &Rc<Self>, quick: bool) {
        if self.state.borrow().capturing {
            return;
        }
        self.dismiss_error();
        let expected_layout = Layout::new(self.setup.map.monitors());
        let current_layout = capture::desktop_layout();
        if expected_layout != current_layout {
            self.sync_monitor_layout(current_layout);
            self.present();
            return;
        }
        if !self.can_discard() {
            return;
        }
        self.dismiss_shortcuts();
        let mode = self.setup.mode();
        let source = self
            .setup
            .source
            .active_id()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|_| mode != Mode::System);
        let delay = self.setup.delay.value_as_int() as u64;
        let mut s = self.state.borrow_mut();
        s.capturing = true;
        s.draft = None;
        s.drag = None;
        s.generation += 1;
        let generation = s.generation;
        drop(s);
        self.refresh();
        if quick {
            self.window.hide();
        }
        let cancel = capture::Cancellation::default();
        *self.capture_cancel.borrow_mut() = Some(cancel.clone());
        let e = self.clone();
        glib::MainContext::default().spawn_local(async move {
            // Cancellation can arrive before the task's first poll. Do not
            // overwrite its message or hide the restored window afterwards.
            if e.state.borrow().generation != generation || e.closed.get() {
                return;
            }
            for remaining in (1..=delay).rev() {
                e.setup
                    .progress
                    .set_text(&crate::i18n::format!("Aufnahme in {remaining} Sekunden …"));
                glib::timeout_future(Duration::from_secs(1)).await;
                if e.state.borrow().generation != generation {
                    return;
                }
            }
            // Help can be opened during the countdown. It must not appear in
            // the screenshot when the countdown finishes.
            e.dismiss_shortcuts();
            e.window.hide();
            glib::timeout_future(Duration::from_millis(350)).await;
            if e.state.borrow().generation != generation || e.closed.get() {
                return;
            }
            e.setup
                .progress
                .set_text(crate::i18n::tr("Aufnahme läuft …"));
            let backend = e.args.borrow().backend;
            let result =
                capture::take(backend, mode == Mode::System, cancel, expected_layout).await;
            if e.state.borrow().generation != generation || e.closed.get() {
                return;
            }
            e.finish_capture(result, mode, source);
        });
    }

    fn finish_capture(&self, result: Result<capture::Capture>, mode: Mode, source: Option<usize>) {
        self.capture_cancel.borrow_mut().take();
        self.state.borrow_mut().capturing = false;
        let result = result.and_then(|capture| {
            let doc = Document::new(capture.image, capture.monitors)?;
            if capture.desktop_confirmed {
                // The desktop already completed selection. In particular, an
                // Auto -> portal fallback must not apply the old monitor index
                // or force a second region selection to unlock the editor.
                self.set_document(doc, &capture.note);
            } else {
                self.accept_capture(doc, &capture.note, mode, source);
            }
            Ok(())
        });
        let portal_failure = result
            .as_ref()
            .err()
            .and_then(|error| error.downcast_ref::<capture::PortalFailure>())
            .copied();
        let quiet_end = matches!(
            portal_failure,
            Some(capture::PortalFailure::Cancelled | capture::PortalFailure::NoImage)
        );
        if result.is_err() {
            if quiet_end && self.state.borrow().document.is_some() {
                self.stack.set_visible_child_name("editor");
            } else {
                self.show_setup();
            }
        }
        // Restore the editor before presenting an error, so it cannot cover
        // the popup. No dialog may keep the capture task alive.
        self.refresh();
        self.window.show();
        self.window.present();
        if let Err(error) = result {
            if quiet_end {
                let note = if portal_failure == Some(capture::PortalFailure::NoImage) {
                    crate::i18n::eprintln!("UntuShot: {error:#}");
                    crate::i18n::tr("Keine Aufnahme übernommen.")
                } else if self.state.borrow().document.is_some() {
                    crate::i18n::tr("Aufnahme abgebrochen")
                } else {
                    crate::i18n::tr("Aufnahme abgebrochen. Du kannst erneut aufnehmen oder zum Editor zurückkehren.")
                };
                self.setup.progress.set_text(note);
                self.status.set_text(note);
                self.status.set_tooltip_text(None);
            } else {
                self.capture_error(error);
            }
        }
    }

    fn accept_capture(&self, mut doc: Document, note: &str, mode: Mode, source: Option<usize>) {
        let source = source.filter(|_| mode != Mode::System);
        let mut needs_region = mode == Mode::Region;
        let mut region_bounds = None;
        if let Some(index) = source {
            if let Some(monitor) = doc.monitors().get(index) {
                if needs_region {
                    region_bounds = Some(monitor.rect);
                } else {
                    doc.select(Some(monitor.rect));
                }
            } else {
                needs_region = true;
            }
        }
        self.set_document(doc, note);
        if needs_region {
            {
                let mut s = self.state.borrow_mut();
                s.selecting_region = true;
                s.region_bounds = region_bounds;
            }
            self.tools[1].set_active(true);
            self.status.set_text(if source.is_some() && region_bounds.is_none() {
                crate::i18n::tr("Monitor nicht eindeutig zugeordnet · Bitte den gewünschten Bereich im Bild aufziehen")
            } else {
                crate::i18n::tr("Bereich auswählen · Erst nach Bestätigung öffnet sich der Editor")
            });
            self.set_cursor(gdk::CursorType::Crosshair);
        }
        self.refresh();
    }

    fn set_cursor(&self, cursor: gdk::CursorType) {
        if self.cursor.get() != Some(cursor) {
            if let Some(window) = self.area.window() {
                window.set_cursor(gdk::Cursor::for_display(&self.area.display(), cursor).as_ref());
                self.cursor.set(Some(cursor));
            }
        }
    }

    fn update_history_controls(&self) {
        let s = self.state.borrow();
        let ready = s.document.is_some()
            && !s.capturing
            && !s.selecting_region
            && self.stack.visible_child_name().as_deref() == Some("editor");
        let renaming = self.layer_rename.get().is_some();
        let editing = self.inline_edit.borrow();
        let (undo, redo, hint) = if let Some(edit) = editing.as_ref() {
            let pending = self.pending_text_size();
            (
                edit.history.can_undo() || pending,
                edit.history.can_redo() && !pending,
                crate::i18n::tr("Texteingabe rückgängig · Strg+Z"),
            )
        } else if renaming {
            (
                true,
                false,
                crate::i18n::tr(
                    "Umbenennen rückgängig · Strg+Z · mit Wiederholen wiederherstellbar",
                ),
            )
        } else {
            (
                s.document.as_ref().is_some_and(Document::can_undo),
                s.document.as_ref().is_some_and(Document::can_redo),
                crate::i18n::tr("Rückgängig · Strg+Z"),
            )
        };
        self.undo.set_sensitive(ready && undo);
        self.redo.set_sensitive(ready && redo);
        self.undo.set_tooltip_text(Some(hint));
    }

    fn history(&self, redo: bool) {
        if self.inline_edit.borrow().is_some() {
            self.text_history(redo);
            return;
        }
        let draft = self.layer_rename.get().is_some();
        if redo && draft {
            return;
        }
        if draft {
            // Finish the draft before undoing it, so Redo can recover it.
            // Keep layer rows intact until the name editor has detached.
            let before = self
                .state
                .borrow()
                .document
                .as_ref()
                .map(Document::content_id);
            self.finish_layer_rename(true);
            if self
                .state
                .borrow()
                .document
                .as_ref()
                .map(Document::content_id)
                == before
            {
                // Empty/unchanged or stale input must not undo an older action.
                return;
            }
        }
        if !draft {
            self.commit_pending_edits();
        }
        layer_drag::cancel(self);
        self.layer_rows.borrow_mut().clear();
        self.property_edit.set(None);
        let mut s = self.state.borrow_mut();
        s.draft = None;
        s.drag = None;
        s.selected = None;
        s.selected_many.clear();
        s.crop_drag = None;
        s.dirty = true;
        if let Some(doc) = &mut s.document {
            let size = (doc.width(), doc.height());
            if redo {
                doc.redo();
            } else {
                doc.undo();
            }
            self.rebuild_sources(doc);
            if size != (doc.width(), doc.height()) {
                s.zoom = None;
                s.pan = Point::default();
            }
        }
        drop(s);
        self.refresh();
    }

    fn zoom(&self, factor: Option<f64>) {
        let anchor = self.state.borrow().mouse.unwrap_or(Point::new(
            self.area.allocated_width() as f64 / 2.,
            self.area.allocated_height() as f64 / 2.,
        ));
        self.zoom_at(factor, anchor);
    }
    fn zoom_at(&self, factor: Option<f64>, anchor: Point) {
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        let Some(bounds) = s.canvas_bounds() else {
            return;
        };
        if let Some(factor) = factor {
            let scale = (s.zoom.unwrap_or(s.view.scale) * factor).clamp(0.05, 8.);
            let before = s.view;
            let base = View::layout(
                bounds,
                self.area.allocated_width() as f64,
                self.area.allocated_height() as f64,
                scale,
                Point::default(),
            );
            s.pan = Point::new(
                anchor.x - (anchor.x - before.x) / before.scale * scale - base.x,
                anchor.y - (anchor.y - before.y) / before.scale * scale - base.y,
            );
            s.zoom = Some(scale);
            s.view = View::layout(
                bounds,
                self.area.allocated_width() as f64,
                self.area.allocated_height() as f64,
                scale,
                s.pan,
            );
        } else {
            s.zoom = None;
            s.pan = Point::default();
        }
        s.pan_drag = None;
        drop(s);
        self.area.queue_draw();
    }
    fn start_pan(&self, point: Point) {
        self.finish_text(true);
        let mut s = self.state.borrow_mut();
        s.draft = None;
        s.drag = None;
        s.crop_drag = None;
        s.pan_drag = Some((point, s.pan));
    }
    fn save(&self) -> bool {
        self.commit_pending_edits();
        let path = self.export_path.borrow().clone();
        if let Some(path) = path {
            let result = (|| -> Result<()> {
                let image = self
                    .state
                    .borrow()
                    .document
                    .as_ref()
                    .ok_or_else(|| crate::i18n::anyhow!("Kein Bild geöffnet"))?
                    .export()?;
                save_png(&image, &path)
            })();
            match result {
                Ok(()) => {
                    self.mark_exported(&path);
                    self.status
                        .set_text(&crate::i18n::format!("PNG gespeichert: {}", path.display()));
                    self.refresh();
                    true
                }
                Err(error) => {
                    self.error(error);
                    false
                }
            }
        } else {
            self.save_as()
        }
    }
    fn save_as(&self) -> bool {
        self.commit_pending_edits();
        if self.state.borrow().document.is_none() {
            return false;
        }
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(crate::i18n::tr("Bild als PNG exportieren")),
            Some(&self.window),
            gtk::FileChooserAction::Save,
            &[
                (crate::i18n::tr("Abbrechen"), gtk::ResponseType::Cancel),
                (crate::i18n::tr("Speichern"), gtk::ResponseType::Accept),
            ],
        );
        dialog.set_do_overwrite_confirmation(true);
        let name = suggested_filename("png");
        dialog.set_current_name(&name);
        if let Some(path) = &self.args.borrow().output {
            let absolute = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path)
            };
            if let Some(parent) = absolute.parent() {
                dialog.set_current_folder(parent);
            }
            if let Some(name) = absolute.file_name() {
                dialog.set_current_name(&name.to_string_lossy());
            }
        } else {
            match self.export_directory() {
                Ok(directory) => {
                    dialog.set_current_folder(directory);
                }
                Err(error) => {
                    dialog.hide();
                    dialog.close();
                    self.error(error);
                    return false;
                }
            }
        }
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(crate::i18n::tr("PNG-Bild (*.png)")));
        filter.add_pattern("*.png");
        dialog.add_filter(filter);
        let result = dialog.run();
        let path = dialog.filename();
        dialog.hide();
        dialog.close();
        if result == gtk::ResponseType::Accept {
            if let Some(path) = path {
                // Keep the confirmed path verbatim; appending a suffix here could
                // overwrite another file which the chooser did not confirm.
                self.commit_pending_edits();
                let result = self
                    .state
                    .borrow()
                    .document
                    .as_ref()
                    .ok_or_else(|| crate::i18n::anyhow!("Kein Bild geöffnet"))
                    .and_then(Document::export)
                    .and_then(|image| save_png(&image, &path));
                match result {
                    Ok(()) => {
                        self.mark_exported(&path);
                        self.status.set_text(&crate::i18n::format!(
                            "PNG exportiert: {} · Bearbeitung wird automatisch gespeichert",
                            path.display()
                        ));
                        self.refresh();
                        return true;
                    }
                    Err(err) => self.error(err),
                }
            }
        }
        false
    }

    fn copy(&self) {
        self.commit_pending_edits();
        let result = {
            let s = self.state.borrow();
            s.document.as_ref().map(|doc| {
                if s.tool == Tool::Marquee {
                    s.pixel_selection
                        .ok_or_else(|| crate::i18n::anyhow!("Bitte zuerst eine Auswahl aufziehen"))
                        .and_then(|r| {
                            doc.export_region(r.clamp(doc.selection().unwrap_or(doc.bounds())))
                        })
                } else {
                    doc.export()
                }
            })
        };
        match result {
            Some(Ok(image)) => {
                let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
                clipboard.set_image(&image);
                clipboard.store();
                self.status.set_text(crate::i18n::tr("Bild kopiert · UntuShot bis zum Einfügen geöffnet lassen, sofern kein Clipboard-Manager läuft."));
            }
            Some(Err(err)) => self.error(err),
            None => {}
        }
    }

    fn selection_to_layer(&self) {
        self.finish_text(true);
        let result = {
            let mut s = self.state.borrow_mut();
            if s.capturing || s.selecting_region || s.tool != Tool::Marquee {
                return;
            }
            let Some(r) = s.pixel_selection else {
                self.status
                    .set_text(crate::i18n::tr("Bitte zuerst eine Auswahl aufziehen · M"));
                return;
            };
            s.document.as_mut().map(|doc| doc.copy_region_to_layer(r))
        };
        match result {
            Some(Ok(index)) => self.select_new_image(index, crate::i18n::tr("Auswahl als neue Ebene kopiert · An den Griffen vergrößern · Rand und Schatten rechts einstellen")),
            Some(Err(err)) => self.error(err),
            None => {}
        }
    }

    fn select_new_image(&self, index: usize, note: &str) {
        self.commit_pending_edits();
        self.tools[0].set_active(true);
        let mut s = self.state.borrow_mut();
        s.pixel_selection = None;
        s.crop_drag = None;
        s.draft = None;
        s.drag = None;
        s.selected = Some(index);
        s.selected_many = vec![index];
        s.dirty = true;
        drop(s);
        self.show_layers.set_active(true);
        self.refresh();
        self.status.set_text(note);
        self.area.grab_focus();
    }

    fn paste_layer(&self) {
        self.finish_text(true);
        if !self.paste_layer_button.is_sensitive() || self.clipboard_pending.replace(true) {
            return;
        }
        let weak = self.weak_self.clone();
        let epoch = self.clipboard_epoch.get();
        self.status.set_text(crate::i18n::tr(
            "Bild aus der Zwischenablage wird geladen …",
        ));
        crate::web_import::request_layer(move |result| {
            let Some(editor) = weak.upgrade() else {
                return;
            };
            if editor.closed.get() || editor.clipboard_epoch.get() != epoch {
                return;
            }
            editor.clipboard_pending.set(false);
            if !editor.paste_layer_button.is_sensitive() {
                return;
            }
            match result {
                Ok(image) => editor.insert_pasted_image(image),
                Err(error) => editor.status.set_text(&crate::i18n::format!("{error:#}")),
            }
        });
    }

    fn insert_pasted_image(&self, image: gdk_pixbuf::Pixbuf) {
        let result = {
            let mut s = self.state.borrow_mut();
            let Some(bounds) = s.canvas_bounds() else {
                return;
            };
            let w = f64::from(image.width());
            let h = f64::from(image.height());
            let scale = (bounds.w / w).min(bounds.h / h).min(1.);
            let (w, h) = (w * scale, h * scale);
            let position = s.pixel_selection.map_or(
                Point::new(
                    bounds.x + (bounds.w - w) / 2.,
                    bounds.y + (bounds.h - h) / 2.,
                ),
                |r| Point::new(r.x, r.y),
            );
            let rect = Rect::new(
                position.x.clamp(bounds.x, bounds.right() - w),
                position.y.clamp(bounds.y, bounds.bottom() - h),
                w,
                h,
            );
            s.document.as_mut().map(|doc| {
                doc.insert_image(image, rect, crate::i18n::tr("Eingefügtes Bild").into())
            })
        };
        match result {
            Some(Ok(index)) => self.select_new_image(index, crate::i18n::tr("Bild als neue Ebene eingefügt · Verschieben, vergrößern und mit Rand oder Schatten hervorheben")),
            Some(Err(err)) => self.error(err),
            None => {}
        }
    }

    fn apply_image_effects(&self) {
        if self.syncing.get() {
            return;
        }
        let color = self.border_color.rgba();
        let effects = crate::effects::ImageEffects {
            border: self.border_enabled.is_active(),
            border_width: self.border_width.value(),
            border_color: (color.red(), color.green(), color.blue()),
            shadow: self.shadow_enabled.is_active(),
            shadow_blur: self.shadow_blur.value(),
            shadow_offset: self.shadow_offset.value(),
            shadow_opacity: self.shadow_opacity.value() / 100.,
        };
        self.edit_selected(|a| {
            if matches!(a.tool, Tool::Image(_)) {
                a.effects = effects.clone();
            }
        });
    }

    fn choose_image_layer(&self) {
        if !self.append_image.is_sensitive() {
            return;
        }
        let dialog = gtk::FileChooserDialog::with_buttons(
            Some(crate::i18n::tr("Bild rechts anfügen")),
            Some(&self.window),
            gtk::FileChooserAction::Open,
            &[
                (crate::i18n::tr("Abbrechen"), gtk::ResponseType::Cancel),
                (crate::i18n::tr("Anfügen"), gtk::ResponseType::Accept),
            ],
        );
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(crate::i18n::tr("Bilder (PNG, JPEG)")));
        filter.add_mime_type("image/png");
        filter.add_mime_type("image/jpeg");
        dialog.add_filter(filter);
        let response = dialog.run();
        let path = dialog.filename();
        dialog.hide();
        dialog.close();
        if response == gtk::ResponseType::Accept {
            if let Some(path) = path {
                self.append_image_path(&path);
            }
        }
    }
    fn append_image_path(&self, path: &std::path::Path) {
        if !self.append_image.is_sensitive() {
            return;
        }
        self.finish_text(true);
        let result = (|| -> Result<usize> {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(128 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            let image = crate::pixels::from_rgba(crate::pixels::decode_layer(&bytes)?)?;
            let label = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .replace('\0', "");
            let mut s = self.state.borrow_mut();
            let doc = s
                .document
                .as_mut()
                .ok_or_else(|| crate::i18n::anyhow!("Kein Bild geöffnet"))?;
            let index = doc.append_image_right(image, label)?;
            self.rebuild_sources(doc);
            s.selected = Some(index);
            s.selected_many = vec![index];
            s.draft = None;
            s.drag = None;
            s.crop_drag = None;
            s.zoom = None;
            s.pan = Point::default();
            s.dirty = true;
            Ok(index)
        })();
        match result {
            Ok(_) => {
                self.tools[0].set_active(true);
                self.refresh();
                self.area.grab_focus();
                let note = crate::i18n::tr("Bildebene rechts angefügt · Ziehen: verschieben · Acht Griffe: proportional skalieren · Strg+Z: zurück");
                self.status.set_text(note);
                self.status.set_tooltip_text(Some(note));
            }
            Err(error) => self.error(error),
        }
    }

    fn import_image(&self, browser: bool) {
        if self.is_capturing() || self.state.borrow().selecting_region {
            return;
        }
        self.finish_text(true);
        match crate::web_import::run(&self.window, browser) {
            crate::web_import::Outcome::Image(image) => {
                self.accept_import(image);
            }
            crate::web_import::Outcome::Quit => self.quit(),
            crate::web_import::Outcome::Cancel => {}
        }
    }

    fn accept_import(&self, image: gdk_pixbuf::Pixbuf) -> bool {
        match Document::new(image, Vec::new()) {
            Ok(document) => {
                if !self.can_discard() {
                    return false;
                }
                self.set_document(
                    document,
                    crate::i18n::tr(
                        "Bild übernommen · Markierungen hinzufügen oder als Bild exportieren",
                    ),
                );
                true
            }
            Err(error) => {
                self.error(error);
                false
            }
        }
    }

    fn text(&self, point: Point) {
        self.finish_text(true);
        let s = self.state.borrow();
        let existing = s.selected.and_then(|i| {
            s.document
                .as_ref()
                .and_then(|d| d.annotations().get(i))
                .filter(|a| a.tool == Tool::Text)
                .map(|a| (i, a.clone()))
        });
        let (index, annotation) = existing.map(|(i, a)| (Some(i), a)).unwrap_or_else(|| {
            (
                None,
                Annotation {
                    effects: Default::default(),
                    clip: None,
                    label: String::new(),
                    font_size: Some(s.font_size),
                    tool: Tool::Text,
                    points: vec![point],
                    color: s.color,
                    width: s.width,
                    text: String::new(),
                    visible: true,
                },
            )
        });
        let anchor = annotation.points[0];
        let x = s.view.x + anchor.x * s.view.scale;
        let y = s.view.y + anchor.y * s.view.scale;
        drop(s);
        self.inline_input
            .buffer()
            .unwrap()
            .set_text(&annotation.text);
        *self.inline_edit.borrow_mut() = Some(TextEdit {
            index,
            annotation,
            history: Default::default(),
        });
        self.inline_box.set_margin_start(
            x.round()
                .max(0.)
                .min((self.area.allocated_width() - 260).max(0) as f64) as i32,
        );
        self.inline_box.set_margin_top(
            y.round()
                .max(0.)
                .min((self.area.allocated_height() - 150).max(0) as f64) as i32,
        );
        self.update_inline_style();
        self.inline_box.show_all();
        self.refresh();
        self.inline_input.grab_focus();
        self.status.set_text(crate::i18n::tr(
            "Text direkt bearbeiten · Enter übernimmt · Shift+Enter: neue Zeile · Esc verwirft",
        ));
    }
    fn position_inline(&self, view: View, w: f64, h: f64) {
        if let Some(edit) = self.inline_edit.borrow().as_ref() {
            let p = edit.annotation.points[0];
            let x = (view.x + p.x * view.scale)
                .round()
                .max(0.)
                .min((w - 260.).max(0.)) as i32;
            let y = (view.y + p.y * view.scale)
                .round()
                .max(0.)
                .min((h - 150.).max(0.)) as i32;
            if self.inline_box.margin_start() != x {
                self.inline_box.set_margin_start(x);
            }
            if self.inline_box.margin_top() != y {
                self.inline_box.set_margin_top(y);
            }
        }
        if self.inline_scale.get() != view.scale {
            self.update_inline_style();
        }
    }
    fn update_inline_style(&self) {
        if let Some(edit) = self.inline_edit.borrow().as_ref() {
            let scale = self.state.borrow().view.scale;
            self.inline_scale.set(scale);
            let size = (edit.annotation.text_size() * scale).max(1.);
            let (r, g, b) = edit.annotation.color;
            let css=crate::i18n::format!("textview, textview text {{font-family: sans-serif; font-weight: bold; font-size: {size}px; color: rgb({},{},{});}}",(r*255.) as u8,(g*255.) as u8,(b*255.) as u8);
            let _ = self.inline_css.load_from_data(css.as_bytes());
        }
    }
    fn finish_text(&self, commit: bool) {
        if commit && self.inline_edit.borrow().is_some() {
            // Apply buttons retain focus, so the spin entry can still contain
            // an unconfirmed size. Read it while it still belongs to the draft.
            self.font_size.update();
        }
        self.property_edit.set(None);
        let Some(edit) = self.inline_edit.borrow_mut().take() else {
            return;
        };
        self.inline_box.hide();
        if commit && !edit.annotation.text.trim().is_empty() {
            let mut s = self.state.borrow_mut();
            if let Some(doc) = s.document.as_mut() {
                let index = if let Some(index) = edit.index {
                    doc.replace(index, edit.annotation);
                    index
                } else {
                    doc.add(edit.annotation);
                    doc.annotations().len() - 1
                };
                s.selected = Some(index);
                s.selected_many = vec![index];
            }
        }
        // Focus-out can update the spin. It must not turn a cancelled draft's
        // pending size into a new edit of the original annotation.
        let syncing = self.syncing.replace(true);
        self.area.grab_focus();
        self.syncing.set(syncing);
        self.refresh();
    }

    fn key(self: &Rc<Self>, event: &gdk::EventKey) -> glib::Propagation {
        self.key_input(event.keyval(), event.state())
    }
    fn key_input(
        self: &Rc<Self>,
        k: gdk::keys::Key,
        modifiers: gdk::ModifierType,
    ) -> glib::Propagation {
        use gdk::keys::constants as key;
        if matches!(k, key::Tab | key::ISO_Left_Tab) {
            return glib::Propagation::Proceed;
        }
        if k == key::F1 {
            self.show_shortcuts();
            return glib::Propagation::Stop;
        }
        if k == key::F6 {
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                let show = !(self.show_tools.is_active() || self.show_layers.is_active());
                self.auto_layers_hidden.set(false);
                self.layers_manually_shown.set(show);
                self.show_tools.set_active(show);
                self.show_layers.set_active(show);
            } else {
                self.area.grab_focus();
            }
            return glib::Propagation::Stop;
        }
        if k == key::Escape && self.layer_rename.get().is_some() {
            self.finish_layer_rename(false);
            self.area.grab_focus();
            return glib::Propagation::Stop;
        }
        if let Some(popover) = self.font_picker.popover().filter(|p| p.is_visible()) {
            if k == key::Escape {
                popover.popdown();
                return glib::Propagation::Stop;
            }
            // Let the size list handle arrows/Enter without editing the canvas
            // or confirming the text being written behind the popover.
            return glib::Propagation::Proceed;
        }
        if k == key::Escape && self.layer_drag.borrow().is_some() {
            layer_drag::cancel(self);
            return glib::Propagation::Stop;
        }
        if matches!(k, key::Alt_L | key::Alt_R) && self.state.borrow().drag.is_some() {
            self.state.borrow_mut().snap_bypass = true;
            let mouse = self.state.borrow().mouse;
            if let Some(p) = mouse {
                self.motion(p.x, p.y);
            }
            return glib::Propagation::Stop;
        }
        if k == key::F11 {
            let mut s = self.state.borrow_mut();
            s.fullscreen = !s.fullscreen;
            if s.fullscreen {
                self.window.fullscreen();
            } else {
                self.window.unfullscreen();
            }
            return glib::Propagation::Stop;
        }
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        // Window/file commands must work even when an entry, spin button or
        // inline text editor owns the focus. Text editing shortcuts stay local.
        if ctrl {
            match k {
                key::n | key::N => {
                    if !self.state.borrow().selecting_region {
                        self.new_screenshot();
                    }
                    return glib::Propagation::Stop;
                }
                key::v | key::V if shift => {
                    self.import_image(false);
                    return glib::Propagation::Stop;
                }
                key::o | key::O => {
                    if !self.is_capturing() && !self.state.borrow().selecting_region {
                        self.open();
                    }
                    return glib::Propagation::Stop;
                }
                key::w | key::W => {
                    self.window.close();
                    return glib::Propagation::Stop;
                }
                key::i | key::I => {
                    self.choose_image_layer();
                    return glib::Propagation::Stop;
                }
                key::q | key::Q => {
                    self.quit();
                    return glib::Propagation::Stop;
                }
                key::s | key::S => {
                    self.finish_layer_rename(true);
                    if !self.is_capturing() && !self.state.borrow().selecting_region {
                        if let Some(spin) = self
                            .window
                            .focused_widget()
                            .and_then(|w| w.downcast::<gtk::SpinButton>().ok())
                        {
                            spin.update();
                        }
                        if modifiers.contains(gdk::ModifierType::MOD1_MASK) {
                            if shift {
                                self.save_project_as();
                            } else {
                                self.save_project();
                            }
                        } else if shift {
                            self.save_as();
                        } else {
                            self.save();
                        }
                    }
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        if ctrl
            && (self.inline_edit.borrow().is_some() || self.layer_rename.get().is_some())
            && matches!(k, key::z | key::Z | key::y | key::Y)
        {
            self.history(shift || matches!(k, key::y | key::Y));
            return glib::Propagation::Stop;
        }
        let focused_spin = self
            .window
            .focused_widget()
            .and_then(|w| w.downcast::<gtk::SpinButton>().ok());
        if ctrl
            && focused_spin.as_ref().is_some_and(|spin| {
                self.property_spins().contains(&spin)
                    || spin == &self.font_size
                    || spin == &self.width
            })
            && matches!(k, key::z | key::Z | key::y | key::Y)
        {
            focused_spin.as_ref().unwrap().update();
            if focused_spin.as_ref() == Some(&self.width) {
                if let Some(popover) = self.width_picker.popover() {
                    popover.popdown();
                }
            }
            self.area.grab_focus();
            self.history(shift || matches!(k, key::y | key::Y));
            return glib::Propagation::Stop;
        }
        if matches!(k, key::Return | key::KP_Enter) {
            if let Some(spin) = &focused_spin {
                spin.update();
            }
        }
        if self.inline_edit.borrow().is_some() {
            if k == key::Escape {
                self.finish_text(false);
                return glib::Propagation::Stop;
            }
            if matches!(k, key::Return | key::KP_Enter) && !shift {
                self.finish_text(true);
                return glib::Propagation::Stop;
            }
            if self
                .window
                .focused_widget()
                .is_some_and(|w| w.is::<gtk::TextView>())
            {
                return glib::Propagation::Proceed;
            }
        }
        if ctrl
            && matches!(k, key::v | key::V)
            && !self.window.focused_widget().is_some_and(|w| {
                w.is::<gtk::Entry>() || w.is::<gtk::SpinButton>() || w.is::<gtk::TextView>()
            })
        {
            if self.paste_layer_button.is_sensitive() {
                self.paste_layer();
            } else if !self.state.borrow().selecting_region {
                self.import_image(false);
            }
            return glib::Propagation::Stop;
        }
        let native_control_focused = self.window.focused_widget().is_some_and(|w| {
            !w.is::<gtk::DrawingArea>()
                && !w.is::<gtk::TextView>()
                && !w.is::<gtk::Entry>()
                && !w.is::<gtk::SpinButton>()
        });
        if native_control_focused
            && !ctrl
            && !matches!(k, key::Escape | key::F2 | key::Delete | key::F8 | key::F9)
        {
            return glib::Propagation::Proceed;
        }
        if self
            .stack
            .visible_child_name()
            .is_some_and(|n| n == "setup")
        {
            if k == key::Escape {
                if self.state.borrow().capturing || self.state.borrow().document.is_some() {
                    self.setup.cancel.emit_clicked();
                }
                return glib::Propagation::Stop;
            }
            if matches!(k, key::Return | key::KP_Enter) && !self.state.borrow().capturing {
                self.capture();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if self
            .window
            .focused_widget()
            .is_some_and(|w| w.is::<gtk::Entry>() || w.is::<gtk::SpinButton>())
        {
            if matches!(k, key::Return | key::KP_Enter) && focused_spin.is_some() {
                self.property_edit.set(None);
                self.area.grab_focus();
                return glib::Propagation::Stop;
            }
            if k == key::Escape {
                self.property_edit.set(None);
                self.area.grab_focus();
                self.refresh();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if k == key::Escape {
            self.cancel_action();
            return glib::Propagation::Stop;
        }
        if matches!(k, key::Return | key::KP_Enter) {
            self.confirm_action();
            return glib::Propagation::Stop;
        }
        if k == key::space && !ctrl {
            self.state.borrow_mut().space = true;
            return glib::Propagation::Stop;
        }
        match k {
            key::F8 => {
                self.show_tools.set_active(!self.show_tools.is_active());
                return glib::Propagation::Stop;
            }
            key::F9 => {
                self.show_layers.set_active(!self.show_layers.is_active());
                return glib::Propagation::Stop;
            }
            key::Left | key::Right | key::Up | key::Down => {
                let step = if shift { 10. } else { 1. };
                let (dx, dy) = match k {
                    key::Left => (-step, 0.),
                    key::Right => (step, 0.),
                    key::Up => (0., -step),
                    _ => (0., step),
                };
                self.nudge(dx, dy, ctrl);
                return glib::Propagation::Stop;
            }
            _ => {}
        }
        if self.state.borrow().selecting_region {
            return glib::Propagation::Proceed;
        }
        if ctrl {
            match k {
                key::d | key::D => self.duplicate_layers(),
                key::c | key::C => self.copy(),
                key::j | key::J => self.selection_to_layer(),
                key::z | key::Z => self.history(shift),
                key::y | key::Y => self.history(true),
                key::a | key::A => {
                    if self.state.borrow().tool == Tool::Marquee {
                        let mut s = self.state.borrow_mut();
                        s.pixel_selection = s.canvas_bounds();
                        s.crop_drag = None;
                        drop(s);
                        self.refresh();
                    } else {
                        self.monitors.set_active_id(Some("all"));
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
        } else {
            match k {
                key::F2 => {
                    self.begin_layer_rename();
                }
                key::v | key::V => self.tools[0].set_active(true),
                key::m | key::M => self.tools[10].set_active(true),
                key::s | key::S => self.snapping.set_active(!self.snapping.is_active()),
                key::Delete | key::BackSpace => self.delete_layer(),
                key::_1
                | key::_2
                | key::_3
                | key::_4
                | key::_5
                | key::_6
                | key::_7
                | key::_8
                | key::_9 => {
                    if let Some(c) = k.to_unicode() {
                        self.tools[c as usize - '1' as usize + 1].set_active(true);
                    }
                }
                key::_0 => self.zoom(None),
                key::plus | key::equal => self.zoom(Some(1.25)),
                key::minus => self.zoom(Some(0.8)),
                _ => return glib::Propagation::Proceed,
            }
        }
        glib::Propagation::Stop
    }
}

impl Editor {
    #[cfg(test)]
    fn press(&self, x: f64, y: f64, double: bool) {
        self.press_modified(x, y, double, false);
    }
    fn press_modified(&self, x: f64, y: f64, double: bool, extend: bool) {
        self.commit_pending_edits();
        let mut s = self.state.borrow_mut();
        s.mouse = Some(Point::new(x, y));
        let Some(bounds) = s.canvas_bounds() else {
            return;
        };
        let Some(doc) = &s.document else {
            return;
        };
        // Grips take precedence over overlapping objects and remain usable
        // on both sides of the canvas edge.
        let resize = (!extend).then(|| s.object_handle(x, y)).flatten();
        if !s.view.contains(x, y, bounds) && resize.is_none() {
            return;
        }
        let point = if s.tool == Tool::Move {
            s.view.unclamped_point(x, y)
        } else {
            s.view.point(x, y, bounds)
        };
        if matches!(s.tool, Tool::Select | Tool::Marquee) {
            let point = Point::new(point.x.round(), point.y.round());
            let selected = if s.tool == Tool::Marquee {
                s.pixel_selection
            } else {
                doc.selection()
            };
            let handle = if extend {
                None
            } else {
                selected.and_then(|r| selection::hit(r, point, 7. / s.view.scale))
            };
            let original = selected.unwrap_or(Rect::new(point.x, point.y, 0., 0.));
            s.crop_drag = Some(selection::Drag {
                original,
                start: point,
                handle: handle.unwrap_or(selection::Handle::New),
                preview: if handle.is_some() {
                    original
                } else {
                    Rect::new(point.x, point.y, 0., 0.)
                },
            });
            s.selected = None;
            s.selected_many.clear();
            drop(s);
            self.area.queue_draw();
            return;
        }
        if s.tool == Tool::Move {
            let hit = resize
                .map(|(i, _)| i)
                .or_else(|| doc.hit_test(point, 6. / s.view.scale));
            let mut indices = s.indices();
            if extend {
                if let Some(i) = hit {
                    if indices.contains(&i) {
                        indices.retain(|v| *v != i);
                    } else {
                        indices.push(i);
                    }
                }
            } else if !hit.is_some_and(|i| indices.contains(&i)) {
                indices = hit.into_iter().collect();
            }
            let originals: Vec<_> = indices
                .iter()
                .filter_map(|i| doc.annotations().get(*i).map(|a| (*i, a.clone())))
                .collect();
            let text = originals.len() == 1 && originals[0].1.tool == Tool::Text;
            let mut snap_targets = vec![bounds];
            snap_targets.extend(
                doc.annotations()
                    .iter()
                    .enumerate()
                    .filter(|(i, a)| a.visible && !indices.contains(i))
                    .filter_map(|(_, a)| {
                        let original = a.bounds();
                        let r = a.clip.map_or(original, |clip| original.clamp(clip));
                        if r.right() < bounds.x
                            || r.x > bounds.right()
                            || r.bottom() < bounds.y
                            || r.y > bounds.bottom()
                        {
                            return None;
                        }
                        let r = r.clamp(bounds);
                        ((original.w == 0. || r.w > 0.)
                            && (original.h == 0. || r.h > 0.)
                            && (r.w > 0. || r.h > 0.))
                            .then_some(r)
                    }),
            );
            s.selected = indices.first().copied();
            s.selected_many = indices;
            s.drag = if !extend && !originals.is_empty() {
                Some(Drag {
                    guides: Vec::new(),
                    snap_targets,
                    start: point,
                    previews: originals.clone(),
                    originals,
                    handle: resize.map_or(selection::Handle::Move, |(_, handle)| handle),
                })
            } else {
                None
            };
            if double && text && resize.is_none() {
                s.drag = None;
                drop(s);
                self.text(point);
            } else {
                drop(s);
                self.refresh();
            }
        } else if s.tool == Tool::Text {
            s.selected = None;
            s.selected_many.clear();
            drop(s);
            self.text(point);
        } else {
            s.selected = None;
            s.selected_many.clear();
            s.draft = Some(Annotation {
                effects: Default::default(),
                clip: None,
                label: String::new(),
                font_size: None,
                tool: s.tool,
                points: vec![point],
                color: s.color,
                width: s.width,
                text: String::new(),
                visible: true,
            });
            drop(s);
            self.area.queue_draw();
        }
    }
    fn motion(&self, x: f64, y: f64) {
        let mut s = self.state.borrow_mut();
        s.mouse = Some(Point::new(x, y));
        if let Some((start, pan)) = s.pan_drag {
            s.pan = Point::new(pan.x + x - start.x, pan.y + y - start.y);
            drop(s);
            self.set_cursor(gdk::CursorType::Fleur);
            self.area.queue_draw();
            return;
        }
        let Some(bounds) = s.canvas_bounds() else {
            return;
        };
        let point = if s.tool == Tool::Move {
            s.view.unclamped_point(x, y)
        } else {
            s.view.point(x, y, bounds)
        };
        s.pointer = s.view.contains(x, y, bounds).then_some(point);
        let cursor = if matches!(s.tool, Tool::Select | Tool::Marquee) {
            handle_cursor(
                s.crop_drag.as_ref().map(|d| d.handle).or_else(|| {
                    (if s.tool == Tool::Marquee {
                        s.pixel_selection
                    } else {
                        s.document.as_ref().and_then(|d| d.selection())
                    })
                    .and_then(|r| selection::hit(r, point, 7. / s.view.scale))
                }),
                gdk::CursorType::Crosshair,
            )
        } else if s.tool == Tool::Move {
            let handle = s
                .drag
                .as_ref()
                .map(|d| d.handle)
                .or_else(|| s.object_handle(x, y).map(|(_, h)| h))
                .or_else(|| {
                    (s.view.contains(x, y, bounds)
                        && s.document
                            .as_ref()
                            .is_some_and(|d| d.hit_test(point, 6. / s.view.scale).is_some()))
                    .then_some(selection::Handle::Move)
                });
            handle_cursor(handle, gdk::CursorType::LeftPtr)
        } else {
            gdk::CursorType::Crosshair
        };
        self.set_cursor(cursor);
        if let Some(crop) = &mut s.crop_drag {
            crop.update(point, bounds);
        }
        let snap = self.snapping.is_active() && !s.snap_bypass;
        let scale = s.view.scale;
        if let Some(drag) = &mut s.drag {
            let dx = point.x - drag.start.x;
            let dy = point.y - drag.start.y;
            drag.guides.clear();
            drag.previews = drag
                .originals
                .iter()
                .map(|(i, a)| {
                    (
                        *i,
                        if drag.handle != selection::Handle::Move {
                            a.resized_from(drag.handle, dx, dy)
                        } else {
                            a.translated(dx, dy)
                        },
                    )
                })
                .collect();
            if snap && drag.handle == selection::Handle::Move && dx.hypot(dy) >= 2. / scale {
                if let Some(r) = drag
                    .previews
                    .iter()
                    .filter(|(_, a)| a.visible)
                    .map(|(_, a)| a.bounds())
                    .reduce(Rect::union)
                {
                    let (offset, guides) =
                        crate::snapping::align(r, &drag.snap_targets, 6. / scale);
                    for (_, a) in &mut drag.previews {
                        *a = a.translated(offset.x, offset.y);
                    }
                    drag.guides = guides;
                }
            }
        }
        if let Some(draft) = &mut s.draft {
            update_draft(draft, point);
        }
        let redraw =
            matches!(s.tool, Tool::Select | Tool::Marquee) || s.drag.is_some() || s.draft.is_some();
        drop(s);
        if redraw {
            self.area.queue_draw();
        }
    }
    fn release(&self, x: f64, y: f64) {
        self.motion(x, y);
        let mut s = self.state.borrow_mut();
        if s.pan_drag.take().is_some() {
            return;
        }
        if let Some(drag) = s.drag.take() {
            if drag.originals != drag.previews {
                if let Some(doc) = &mut s.document {
                    doc.replace_many(drag.previews);
                    s.dirty = true;
                }
            }
        }
        if let Some(crop) = s.crop_drag.take() {
            if crop.preview.w >= 1. && crop.preview.h >= 1. {
                if s.tool == Tool::Marquee {
                    s.pixel_selection = Some(crop.preview);
                } else if let Some(doc) = &mut s.document {
                    doc.select(Some(crop.preview));
                    s.dirty = true;
                }
            }
        }
        if let Some(draft) = s.draft.take() {
            if let Some(doc) = &mut s.document {
                doc.add(draft);
                let index = doc.annotations().len() - 1;
                s.selected = Some(index);
                s.selected_many = vec![index];
                s.dirty = true;
            }
        }
        drop(s);
        self.refresh();
    }
}

fn handle_cursor(handle: Option<selection::Handle>, fallback: gdk::CursorType) -> gdk::CursorType {
    use selection::Handle::*;
    match handle {
        Some(Nw) => gdk::CursorType::TopLeftCorner,
        Some(Se) => gdk::CursorType::BottomRightCorner,
        Some(Ne) => gdk::CursorType::TopRightCorner,
        Some(Sw) => gdk::CursorType::BottomLeftCorner,
        Some(N | S) => gdk::CursorType::SbVDoubleArrow,
        Some(E | W) => gdk::CursorType::SbHDoubleArrow,
        Some(Move) => gdk::CursorType::Fleur,
        _ => fallback,
    }
}

fn connect_canvas(editor: &Rc<Editor>) {
    let e = editor.clone();
    editor.area.connect_draw(move |area, cr| {
        if let Err(error) = draw(&e, area, cr) {
            e.status
                .set_text(&crate::i18n::format!("Zeichenfehler: {error}"));
        }
        glib::Propagation::Stop
    });
    let e = editor.clone();
    editor.area.connect_button_press_event(move |area, event| {
        if event.button() == 3 {
            let (x, y) = event.position();
            area.grab_focus();
            if let Some(menu) = e.menu_at_object(x, y) {
                menu.popup_at_pointer(Some(event));
                *e.object_menu.borrow_mut() = Some(menu);
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if event.button() == 2 || (event.button() == 1 && e.state.borrow().space) {
            let (x, y) = event.position();
            area.grab_focus();
            e.start_pan(Point::new(x, y));
            return glib::Propagation::Stop;
        }
        if event.button() != 1 {
            return glib::Propagation::Proceed;
        }
        area.grab_focus();
        let (x, y) = event.position();
        e.state.borrow_mut().snap_bypass = event.state().contains(gdk::ModifierType::MOD1_MASK);
        e.press_modified(
            x,
            y,
            event.event_type() == gdk::EventType::DoubleButtonPress,
            event
                .state()
                .intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK),
        );
        glib::Propagation::Stop
    });
    let e = editor.clone();
    editor.area.connect_motion_notify_event(move |_, event| {
        let (x, y) = event.position();
        e.state.borrow_mut().snap_bypass = event.state().contains(gdk::ModifierType::MOD1_MASK);
        e.motion(x, y);
        glib::Propagation::Stop
    });
    let e = editor.clone();
    editor.area.connect_button_release_event(move |_, event| {
        if event.button() != 1 && event.button() != 2 {
            return glib::Propagation::Proceed;
        }
        let (x, y) = event.position();
        e.state.borrow_mut().snap_bypass = event.state().contains(gdk::ModifierType::MOD1_MASK);
        e.release(x, y);
        glib::Propagation::Stop
    });
    let e = editor.clone();
    editor.area.connect_scroll_event(move |_, event| {
        let (dx, dy) = event
            .scroll_deltas()
            .unwrap_or_else(|| match event.direction() {
                gdk::ScrollDirection::Up => (0., -1.),
                gdk::ScrollDirection::Down => (0., 1.),
                gdk::ScrollDirection::Left => (-1., 0.),
                gdk::ScrollDirection::Right => (1., 0.),
                _ => (0., 0.),
            });
        if event.state().contains(gdk::ModifierType::CONTROL_MASK) {
            let (x, y) = event.position();
            e.zoom_at(Some(1.15_f64.powf(-dy)), Point::new(x, y));
        } else {
            e.finish_text(true);
            let mut s = e.state.borrow_mut();
            if event.state().contains(gdk::ModifierType::SHIFT_MASK) {
                s.pan.x -= dy * 45.;
            } else {
                s.pan.x -= dx * 45.;
                s.pan.y -= dy * 45.;
            }
            drop(s);
            e.area.queue_draw();
        }
        glib::Propagation::Stop
    });
    let e = editor.clone();
    editor.area.connect_leave_notify_event(move |_, _| {
        let mut s = e.state.borrow_mut();
        s.pointer = None;
        if s.pan_drag.is_none() {
            s.mouse = None;
        }
        drop(s);
        e.area.queue_draw();
        glib::Propagation::Proceed
    });
}

fn update_draft(draft: &mut Annotation, point: Point) {
    if draft.tool == Tool::Pen || draft.points.len() == 1 {
        draft.points.push(point);
    } else {
        draft.points[1] = point;
    }
}

fn draw(editor: &Editor, area: &gtk::DrawingArea, cr: &cairo::Context) -> Result<()> {
    let palette = crate::theme::Palette::for_widget(area);
    crate::theme::source(cr, &palette.base, 1.);
    cr.paint()?;
    let mut s = editor.state.borrow_mut();
    let Some(doc) = &s.document else {
        crate::theme::source(cr, &palette.foreground, 0.75);
        cr.set_font_size(20.0);
        cr.move_to(60.0, 100.0);
        cr.show_text(crate::i18n::tr(
            "UntuShot · Neue Aufnahme starten oder --open BILD.png verwenden",
        ))?;
        return Ok(());
    };
    let bounds = doc.bounds();
    let viewport = s.canvas_bounds().unwrap();
    let w = area.allocated_width() as f64;
    let h = area.allocated_height() as f64;
    let scale = s.zoom.unwrap_or_else(|| {
        ((w - 32.0) / viewport.w)
            .min((h - 32.0) / viewport.h)
            .max(0.001)
    });
    let view = View::layout(viewport, w, h, scale, s.pan);
    s.view = view;
    if !s.selecting_region {
        let x = view.x + viewport.x * scale;
        let y = view.y + viewport.y * scale;
        let width = viewport.w * scale;
        let height = viewport.h * scale;
        cr.set_source_rgba(0., 0., 0., 0.12);
        cr.rectangle(x - 2., y - 1., width + 4., height + 5.);
        cr.fill()?;
        crate::theme::source(cr, &palette.border, 0.8);
        cr.set_line_width(1.);
        cr.rectangle(x - 0.5, y - 0.5, width + 1., height + 1.);
        cr.stroke()?;
    }
    let doc = s.document.as_ref().unwrap();
    let mut previews = s
        .drag
        .as_ref()
        .map(|d| d.previews.clone())
        .unwrap_or_default();
    if let Some(edit) = editor.inline_edit.borrow().as_ref() {
        if let Some(index) = edit.index {
            previews.push((index, edit.annotation.clone()));
        }
    }
    let dynamic = !previews.is_empty();
    let key = crate::preview::Key {
        revision: doc.revision(),
        width: area.allocated_width(),
        height: area.allocated_height(),
        device_scale: area.scale_factor().max(1),
        x: view.x,
        y: view.y,
        zoom: scale,
        viewport,
    };
    if dynamic {
        editor
            .preview
            .borrow_mut()
            .paint_dynamic(cr, doc, &previews, key)?;
    } else {
        editor.preview.borrow_mut().paint(cr, doc, key)?;
    }
    cr.save()?;
    cr.translate(view.x, view.y);
    cr.scale(scale, scale);
    cr.rectangle(viewport.x, viewport.y, viewport.w, viewport.h);
    cr.clip();
    if let Some(edit) = editor.inline_edit.borrow().as_ref() {
        if edit.index.is_none() {
            document::paint_annotation(cr, &edit.annotation)?;
        }
    }
    cr.save()?;
    cr.rectangle(0.0, 0.0, bounds.w, bounds.h);
    cr.clip();
    if let Some(draft) = &s.draft {
        if draft.tool != Tool::Select {
            document::paint_annotation(cr, draft)?;
        }
    }
    cr.restore()?;
    if s.guides && doc.monitors().len() > 1 {
        cr.set_line_width(1.0 / scale);
        crate::theme::source(cr, &palette.accent, 0.65);
        cr.set_dash(&[6.0 / scale, 6.0 / scale], 0.0);
        for m in doc.monitors() {
            cr.rectangle(m.rect.x, m.rect.y, m.rect.w, m.rect.h);
            cr.stroke()?;
        }
        cr.set_dash(&[], 0.0);
    }
    let marquee = s.tool == Tool::Marquee;
    let selection = if marquee {
        doc.selection()
    } else {
        s.crop_drag.as_ref().map(|d| d.preview).or(doc.selection())
    };
    let outline = if marquee {
        s.crop_drag
            .as_ref()
            .map(|d| d.preview)
            .or(s.pixel_selection)
    } else {
        selection.filter(|_| s.tool == Tool::Select || s.selecting_region)
    };
    if s.selecting_region && selection.is_none() {
        cr.set_source_rgba(0., 0., 0., 0.45);
        cr.rectangle(bounds.x, bounds.y, bounds.w, bounds.h);
        cr.fill()?;
    }
    if let Some(r) = outline {
        cr.save()?;
        cr.set_source_rgba(0.0, 0.0, 0.0, if marquee { 0.22 } else { 0.55 });
        cr.rectangle(0.0, 0.0, bounds.w, bounds.h);
        cr.rectangle(r.x, r.y, r.w, r.h);
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        cr.fill()?;
        cr.restore()?;
        crate::theme::source(cr, &palette.accent, 1.);
        cr.set_line_width(1.5 / scale);
        cr.rectangle(r.x, r.y, r.w, r.h);
        cr.set_source_rgba(0., 0., 0., 0.75);
        cr.set_line_width(3.5 / scale);
        cr.stroke_preserve()?;
        crate::theme::source(cr, &palette.accent, 1.);
        cr.set_line_width(1.5 / scale);
        if marquee {
            cr.set_dash(&[5. / scale, 3. / scale], 0.);
        }
        cr.stroke()?;
        cr.set_dash(&[], 0.);
        for (point, _) in selection::handles(r) {
            cr.rectangle(
                point.x - 3. / scale,
                point.y - 3. / scale,
                6. / scale,
                6. / scale,
            );
            cr.fill()?;
        }
    }
    if let Some(drag) = &s.drag {
        for guide in &drag.guides {
            cr.save()?;
            if guide.vertical {
                cr.move_to(guide.coordinate, guide.start);
                cr.line_to(guide.coordinate, guide.end);
            } else {
                cr.move_to(guide.start, guide.coordinate);
                cr.line_to(guide.end, guide.coordinate);
            }
            cr.set_source_rgba(1., 1., 1., 0.8);
            cr.set_line_width(3. / scale);
            cr.stroke_preserve()?;
            crate::theme::source(cr, &palette.accent, 1.);
            cr.set_line_width(1. / scale);
            cr.set_dash(&[5. / scale, 3. / scale], 0.);
            cr.stroke()?;
            cr.restore()?;
        }
    }
    // Selection controls are overlays: keep the full grips visible even when
    // their centers lie on the image boundary.
    cr.restore()?;
    cr.save()?;
    cr.translate(view.x, view.y);
    cr.scale(scale, scale);
    let indices = s.indices();
    for index in &indices {
        if editor
            .inline_edit
            .borrow()
            .as_ref()
            .is_some_and(|e| e.index == Some(*index))
        {
            continue;
        }
        if let Some(original) = doc.annotations().get(*index).filter(|a| a.visible) {
            let a = s
                .drag
                .as_ref()
                .and_then(|d| d.previews.iter().find(|(i, _)| i == index).map(|(_, a)| a))
                .unwrap_or(original);
            let r = a.bounds();
            cr.set_source_rgba(1., 1., 1., 0.95);
            cr.set_line_width(3.5 / scale);
            cr.rectangle(r.x, r.y, r.w.max(1.), r.h.max(1.));
            cr.stroke_preserve()?;
            crate::theme::source(cr, &palette.accent, 1.);
            cr.set_line_width(1.5 / scale);
            if indices.len() > 1 {
                cr.set_dash(&[4. / scale, 3. / scale], 0.);
            }
            cr.stroke()?;
            cr.set_dash(&[], 0.);
            if indices.len() == 1 && s.tool == Tool::Move {
                for (point, handle) in selection::handles(r) {
                    cr.rectangle(
                        point.x - 4. / scale,
                        point.y - 4. / scale,
                        8. / scale,
                        8. / scale,
                    );
                    if s.drag.as_ref().is_some_and(|d| d.handle == handle) {
                        crate::theme::source(cr, &palette.accent, 1.);
                    } else {
                        cr.set_source_rgb(1., 1., 1.);
                    }
                    cr.fill_preserve()?;
                    cr.set_source_rgba(0., 0., 0., 0.8);
                    cr.set_line_width(1. / scale);
                    cr.stroke()?;
                }
            }
        }
    }
    cr.restore()?;
    if s.tool == Tool::Select && editor.magnifier.is_active() {
        if let Some(point) = s.pointer {
            doc.paint_loupe(cr, point, (w - 146.).max(0.), 16., &palette.accent)?;
        }
    }
    let (_, _, ew, eh) = selection
        .unwrap_or(bounds)
        .pixel_bounds(doc.width(), doc.height());
    let dimensions = crate::i18n::format!("{ew} × {eh} px  ·  {}%", (scale * 100.0).round());
    if editor.dimensions.text().as_str() != dimensions {
        editor.dimensions.set_text(&dimensions);
    }
    if s.selecting_region {
        cr.set_font_size(14.);
        let text = if selection.is_some() {
            dimensions.as_str()
        } else {
            crate::i18n::tr("Bereich mit der Maus aufziehen")
        };
        let extents = cr.text_extents(text)?;
        let x = (w - extents.width()) / 2.;
        cr.set_source_rgba(0.035, 0.045, 0.06, 0.92);
        cr.rectangle(x - 16., 18., extents.width() + 32., 36.);
        cr.fill()?;
        cr.set_source_rgb(0.94, 0.96, 0.98);
        cr.move_to(x, 42.);
        cr.show_text(text)?;
    }
    drop(s);
    editor.position_inline(view, w, h);
    Ok(())
}

fn suggested_filename(extension: &str) -> String {
    let stamp = glib::DateTime::now_local()
        .and_then(|d| d.format("%Y-%m-%d_%H-%M-%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string()
        });
    crate::i18n::format!("UntuShot-{stamp}.{extension}")
}

fn open_document(path: &std::path::Path) -> Result<(Document, bool)> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let is_project = reader.fill_buf()?.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{');
    if path
        .extension()
        .is_some_and(|e| e == "untushot" || e == "rustshot")
        || is_project
    {
        Document::read_project(reader).map(|d| (d, true))
    } else {
        Document::new(crate::pixels::open(path)?, Vec::new()).map(|d| (d, false))
    }
}
fn atomic_write(
    path: &std::path::Path,
    write: impl FnOnce(&std::fs::File) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let temp = tempfile::Builder::new()
        .prefix(".untushot-")
        .tempfile_in(parent)?;
    write(temp.as_file())?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn save_png(image: &gdk_pixbuf::Pixbuf, path: &std::path::Path) -> Result<()> {
    atomic_write(path, |file| crate::pixels::write_png(image, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    #[ignore = "requires a private GTK display"]
    fn gtk_capture_error_workflow() {
        gtk::init().expect("GTK needs a display");
        let app = gtk::Application::new(
            Some("io.github.untushot.CaptureErrorTest"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gio::Cancellable::NONE).unwrap();
        let editor = build(&app, Args::parse_from(["untushot", "--demo"]));
        let content = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .content_id();
        editor.state.borrow_mut().capturing = true;
        editor
            .capture_cancel
            .replace(Some(capture::Cancellation::default()));
        editor.window.hide();
        let completed = Rc::new(Cell::new(false));
        let timed_out = Rc::new(Cell::new(false));
        let timeout = timed_out.clone();
        // Rescue the old blocking implementation so a regression fails instead
        // of leaving the test runner stuck in Dialog::run().
        let watchdog = glib::timeout_add_local_once(Duration::from_secs(2), move || {
            timeout.set(true);
            for window in gtk::Window::list_toplevels() {
                if let Ok(dialog) = window.downcast::<gtk::MessageDialog>() {
                    dialog.response(gtk::ResponseType::Close);
                }
            }
        });
        let e = editor.clone();
        let done = completed.clone();
        glib::MainContext::default().spawn_local(async move {
            glib::timeout_future(Duration::from_millis(10)).await;
            e.finish_capture(
                Err(crate::i18n::anyhow!(
                    "Desktop-Portal hat die Aufnahme abgelehnt"
                )),
                Mode::Full,
                None,
            );
            done.set(true);
        });
        while !completed.get() && !timed_out.get() {
            glib::MainContext::default().iteration(true);
        }
        assert!(!timed_out.get(), "capture task blocked on the error popup");
        watchdog.remove();
        assert!(!editor.is_capturing());
        assert!(editor.capture_cancel.borrow().is_none());
        assert!(editor.window.is_visible());
        assert!(editor.setup.start.is_sensitive());
        assert_eq!(editor.stack.visible_child_name().as_deref(), Some("setup"));
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            content
        );

        let popup = || {
            gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::MessageDialog>().ok())
                .find(|d| {
                    d.is_visible() && d.transient_for().as_ref() == Some(editor.window.upcast_ref())
                })
                .expect("visible error popup")
        };
        let dialog = popup();
        assert!(!dialog.is_modal());
        dialog
            .widget_for_response(gtk::ResponseType::Close)
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        assert!(!dialog.is_visible());
        for escape in [true, false] {
            editor.error(crate::i18n::anyhow!(
                "Desktop-Portal hat die Aufnahme abgelehnt"
            ));
            let dialog = popup();
            if escape {
                // GtkDialog's Escape binding emits the close action.
                dialog.emit_by_name::<()>("close", &[]);
            } else {
                dialog.close();
            }
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            assert!(!dialog.is_visible());
        }
        editor.error(crate::i18n::anyhow!(
            "Desktop-Portal hat die Aufnahme abgelehnt"
        ));
        let dialog = popup();
        crate::runtime::assert_quit_with_error(&app, &editor);
        editor.window.close();
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        assert!(!dialog.is_visible());
    }

    #[test]
    #[ignore = "requires a private GTK display"]
    fn gtk_many_layers() {
        gtk::init().expect("GTK needs a display");
        let app = gtk::Application::new(
            Some("io.github.untushot.ManyLayersTest"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gio::Cancellable::NONE).unwrap();
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(150);
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(3));
            }
        };
        let editor = build(&app, Args::parse_from(["untushot", "--demo"]));
        let background =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 3840, 2160).unwrap();
        background.fill(0xe9edf2ff);
        let mut doc = Document::new(background, vec![]).unwrap();
        for i in 0..30 {
            let x = 100. + (i % 6) as f64 * 560.;
            let y = 100. + (i / 6) as f64 * 350.;
            let label = format!(
                "Ebene {i:02} – Ausführliche Beschreibung eines Bilddetails mit langem Namen"
            );
            if i % 3 == 0 {
                let image = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 360, 220)
                    .unwrap();
                image.fill(0x3086bcff + (i as u32 * 0x00020000));
                let index = doc
                    .insert_image(image, Rect::new(x, y, 360., 220.), label)
                    .unwrap();
                let mut a = doc.annotations()[index].clone();
                a.effects.border = true;
                a.effects.shadow = true;
                doc.replace(index, a);
            } else {
                doc.add(Annotation {
                    effects: Default::default(),
                    clip: None,
                    label,
                    font_size: Some(32.),
                    tool: if i % 3 == 1 {
                        Tool::Text
                    } else {
                        Tool::Rectangle
                    },
                    points: vec![Point::new(x, y), Point::new(x + 420., y + 220.)],
                    color: (0.1, 0.2, 0.4),
                    width: 5.,
                    text: format!("Hinweis {i:02}"),
                    visible: i != 14,
                });
            }
        }
        editor.accept_capture(doc, "4K · 30 Ebenen · Praxistest", Mode::Full, None);
        editor.window.resize(1100, 850);
        settle();
        editor.layer_scroll.vadjustment().set_value(0.);
        editor.select_new_image(0, "Unterste Ebene auswählen");
        settle();
        let row = editor.layers.row_at_index(29).unwrap();
        let adjustment = editor.layer_scroll.vadjustment();
        let r = row.allocation();
        let revealed = f64::from(r.y()) >= adjustment.value() - 1.
            && f64::from(r.y() + r.height()) <= adjustment.value() + adjustment.page_size() + 1.;
        assert!(
            revealed,
            "canvas selection must reveal its layer after the effects controls resize the list"
        );
        adjustment.set_value(0.);
        editor.window.resize(1100, 700);
        settle();
        assert_eq!(
            adjustment.value(),
            0.,
            "resizing must preserve a manual scroll away from the selection"
        );
        editor.window.resize(1100, 850);
        settle();
        adjustment.set_value((adjustment.upper() - adjustment.page_size()) / 2.);
        let before_scroll = adjustment.value();
        let previous_row = editor.layers.row_at_index(29).unwrap();
        editor.nudge(1., 0., false);
        settle();
        let after_scroll = adjustment.value();
        let reused = previous_row == editor.layers.row_at_index(29).unwrap();
        assert!(
            reused,
            "small edits must preserve layer row widgets and keyboard focus"
        );
        assert_eq!(
            before_scroll, after_scroll,
            "editing the same layer must preserve manual scroll position"
        );
        editor.begin_layer_rename();
        editor.layer_name.set_text("Noch nicht bestätigter Name");
        editor.refresh();
        assert_eq!(editor.layer_name.text(), "Noch nicht bestätigter Name");
        editor.key_input(gdk::keys::constants::Escape, gdk::ModifierType::empty());
        editor.refresh();
        assert_ne!(editor.layer_name.text(), "Noch nicht bestätigter Name");
        let annotations = || {
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .to_vec()
        };
        let mut expected_history = vec![annotations()];
        let mut edit_times = vec![];
        for i in 0..130 {
            let start = std::time::Instant::now();
            editor.nudge(1., 0., false);
            edit_times.push(start.elapsed().as_secs_f64() * 1000.);
            expected_history.push(annotations());
            if (i + 1) % 13 == 0 {
                editor.select_new_image(1, "Text in längerer Sitzung bearbeiten");
                let before = annotations();
                editor.text(Point::default());
                let buffer = editor.inline_input.buffer().unwrap();
                let suffix = format!(" · Schritt {}", i + 1);
                buffer.place_cursor(&buffer.end_iter());
                buffer.begin_user_action();
                buffer.insert_at_cursor(&suffix);
                buffer.end_user_action();
                editor.undo.emit_clicked();
                assert_eq!(
                    editor
                        .inline_edit
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .annotation
                        .text,
                    before[1].text
                );
                editor.redo.emit_clicked();
                assert_eq!(
                    editor
                        .inline_edit
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .annotation
                        .text,
                    format!("{}{suffix}", before[1].text)
                );
                assert_eq!(
                    annotations(),
                    before,
                    "local text undo leaves the document untouched"
                );
                editor.finish_text(true);
                expected_history.push(annotations());
                editor.select_new_image(0, "Bildebene weiter bearbeiten");
            }
        }
        // Traverse the entire retained history after it has rolled over. Each
        // text session counts once despite local Undo/Redo within that session.
        for expected in expected_history.iter().rev().skip(1).take(100) {
            editor.undo.emit_clicked();
            assert_eq!(&annotations(), expected);
        }
        assert!(
            !editor.undo.is_sensitive(),
            "only the latest 100 document steps remain"
        );
        for expected in expected_history.iter().skip(expected_history.len() - 100) {
            editor.redo.emit_clicked();
            assert_eq!(&annotations(), expected);
        }
        assert!(!editor.redo.is_sensitive());
        editor.select_new_image(0, "Längere Sitzung geprüft");
        editor.snapping.set_active(false);
        settle();
        let view = editor.state.borrow().view;
        let r = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .bounds();
        let (x, y) = (
            view.x + (r.x + r.w / 2.) * view.scale,
            view.y + (r.y + r.h / 2.) * view.scale,
        );
        editor.press(x, y, false);
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.area.allocated_width(),
            editor.area.allocated_height(),
        )
        .unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        let mut frame_times = vec![];
        for i in 0..30 {
            let start = std::time::Instant::now();
            editor.motion(x + f64::from(i), y + f64::from(i));
            draw(&editor, &editor.area, &cr).unwrap();
            frame_times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        editor.cancel_action();
        // A full file/clipboard cycle preserves the rendered image and every editable layer.
        layer_drag::verify(&editor);
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("30 Ebenen.untushot");
        let png = directory.path().join("4K Export.png");
        editor.begin_layer_rename();
        editor
            .layer_name
            .set_text("Detail mit besonders langem Namen – dauerhaft gespeichert");
        editor.layer_name.emit_activate();
        editor.shadow_blur.set_value(24.);
        assert!(editor.save_project_to(&project));
        editor.mark_exported(&png);
        assert!(editor.save());
        assert!(editor.status.text().starts_with("PNG gespeichert:"));
        let expected = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .to_vec();
        let expected_pixels = crate::pixels::open(&png).unwrap().read_pixel_bytes();
        let recovery_dir = directory.path().join("recovery");
        let (recovery, _) = crate::recovery::Session::open(&recovery_dir, false).unwrap();
        recovery
            .save(editor.state.borrow().document.as_ref().unwrap(), true)
            .unwrap();
        recovery.flush().unwrap();
        drop(recovery);
        let (_recovery, restored) = crate::recovery::Session::open(&recovery_dir, true).unwrap();
        let restored = restored.unwrap();
        assert_eq!(restored.annotations(), expected);
        assert_eq!(
            restored.export().unwrap().read_pixel_bytes(),
            expected_pixels
        );
        editor.copy();
        assert!(editor.status.text().starts_with("Bild kopiert"));
        let copied = Rc::new(RefCell::new(None));
        let result = copied.clone();
        crate::web_import::request_layer(move |image| *result.borrow_mut() = Some(image.unwrap()));
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while copied.borrow().is_none() && std::time::Instant::now() < deadline {
            settle();
        }
        let copied = copied
            .borrow_mut()
            .take()
            .expect("clipboard transfer completed");
        assert_eq!((copied.width(), copied.height()), (3840, 2160));
        assert_eq!(copied.read_pixel_bytes(), expected_pixels);
        editor.duplicate_layers();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            31
        );
        editor.history(false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations(),
            expected
        );
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        assert!(editor.open_path(&png));
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .is_empty());
        assert!(editor.open_path(&project));
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations(),
            expected
        );
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .export()
                .unwrap()
                .read_pixel_bytes(),
            expected_pixels
        );
        editor.select_new_image(0, "30 Ebenen · Projekt wieder geöffnet");
        settle();
        let image = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&image).unwrap());
        image
            .write_to_png(&mut std::fs::File::create("target/gtk-many-layers.png").unwrap())
            .unwrap();
        edit_times.sort_by(f64::total_cmp);
        frame_times.sort_by(f64::total_cmp);
        let report = serde_json::json!({"fixture": "3840x2160, 30 layers: 10 images, 10 texts, 10 rectangles", "build": if cfg!(debug_assertions) { "debug" } else { "release" },
            "selected_row_visible": revealed, "row_reused_after_edit": reused,
            "scroll_before": before_scroll, "scroll_after": after_scroll,
            "edit_median_ms": edit_times[edit_times.len() / 2], "edit_p95_ms": edit_times[(edit_times.len() - 1) * 95 / 100],
            "session_document_steps": expected_history.len() - 1,
            "session_text_edits": 10, "undo_redo_steps_verified": 100,
            "drag_frame_median_ms": frame_times[15], "drag_frame_p95_ms": frame_times[28],
            "project_export_clipboard_reopen": "passed", "recovery_round_trip": "passed"});
        std::fs::write(
            "target/ux-audit.json",
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        editor.mark_project_saved();
        editor.window.close();
    }

    #[test]
    #[ignore = "requires a private GTK display"]
    fn gtk_editor_workflow() {
        gtk::init().expect("GTK needs a display");
        let app = gtk::Application::new(
            Some("io.github.untushot.EditorWorkflowTest"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gio::Cancellable::NONE).unwrap();
        input_transitions(&app);
        layer_name_history(&app);
        text_edit_history(&app);
        let history_editor = build(&app, Args::parse_from(["untushot", "--demo"]));
        let mut history_doc = crate::document::demo().unwrap();
        let tile = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 160, 100).unwrap();
        tile.fill(0x3086bcff);
        for index in 0..30 {
            let layer = history_doc
                .insert_image(
                    tile.clone(),
                    Rect::new(
                        100. + (index % 6) as f64 * 340.,
                        100. + (index / 6) as f64 * 180.,
                        160.,
                        100.,
                    ),
                    format!("Detail {index:02} mit langem Ebenennamen"),
                )
                .unwrap();
            let mut annotation = history_doc.annotations()[layer].clone();
            annotation.effects.border = true;
            annotation.effects.shadow = true;
            history_doc.replace(layer, annotation);
        }
        history_editor.accept_capture(
            history_doc,
            "30 Ebenen und 60 Verlaufsbilder",
            Mode::Full,
            None,
        );
        history_editor.select_new_image(0, "Eigenschaften der Bildebene");
        let parent = history_editor.history_canvas.clone();
        let panel_height = Cell::new(None);
        crate::history::test_many_images(&parent, |_| {
            let height = history_editor.layer_scroll.allocated_height();
            assert!(height >= 48, "layers keep their scrollable minimum");
            if let Some(previous) = panel_height.replace(Some(height)) {
                assert_eq!(
                    height, previous,
                    "history must not take height from the layers"
                );
            }
            let row = history_editor.layers.selected_rows()[0].allocation();
            let scroll = history_editor.layer_scroll.vadjustment();
            assert!(
                f64::from(row.y()) >= scroll.value() - 1.
                    && f64::from(row.y() + row.height())
                        <= scroll.value() + scroll.page_size() + 1.,
                "a visible selected layer must remain visible when resizing the window"
            );
            assert!(history_editor.shadow_offset.is_mapped());
            let viewport = history_editor
                .property_scroll
                .child()
                .unwrap()
                .downcast::<gtk::Viewport>()
                .unwrap();
            let content = viewport.child().unwrap();
            let bottom = history_editor
                .shadow_offset
                .translate_coordinates(&content, 0, 0)
                .unwrap()
                .1
                + history_editor.shadow_offset.allocated_height();
            assert!(
                f64::from(bottom) <= history_editor.property_scroll.vadjustment().upper(),
                "all effect controls remain reachable by scrolling"
            );
        });
        history_editor.show_setup();
        assert_eq!(
            history_editor.recent.root.parent(),
            Some(history_editor.history_setup.clone().upcast())
        );
        history_editor.setup.cancel.emit_clicked();
        assert_eq!(
            history_editor.recent.root.parent(),
            Some(history_editor.history_canvas.clone().upcast())
        );
        history_editor.mark_project_saved();
        history_editor.window.close();
        shortcuts_and_image_switch(&app);
        object_resize_handles(&app);
        layer_list_controls(&app);
        selection_and_layer_effects(&app);
        live_properties(&app);
        snapping_and_context_menu(&app);
        image_layers(&app);
        let editor = build(&app, Args::parse_from(["untushot", "--demo"]));
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(300);
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        settle();
        assert!(editor.export_button.is_mapped() && editor.copy.is_mapped());
        assert!(editor.project_save_button.is_mapped());
        assert_eq!(editor.storage_caption.text(), "Projekt: ungespeichert");
        assert!(!editor.recent.root.is_visible());
        assert!(editor.context.allocated_height() < 70);
        assert!(!editor.object_properties.is_mapped());
        let snapshot = |name| {
            let surface = cairo::ImageSurface::create(
                cairo::Format::ARgb32,
                editor.window.allocated_width(),
                editor.window.allocated_height(),
            )
            .unwrap();
            editor.window.draw(&cairo::Context::new(&surface).unwrap());
            surface
                .write_to_png(&mut std::fs::File::create(format!("target/{name}.png")).unwrap())
                .unwrap();
        };
        snapshot("gtk-editor-simplified");
        editor.window.resize(1000, 700);
        settle();
        assert!(editor.window.allocated_width() <= 1080);
        assert!(editor.area.allocated_width() > 360);
        assert!(editor.export_button.is_mapped() && editor.project_save_button.is_mapped());
        snapshot("gtk-editor-simplified-compact");
        editor.show_layers.set_active(true);
        settle();
        let initial_panel_width = editor.right_panel.allocated_width();
        let initial_position = editor.workspace_split.position();
        editor.workspace_split.set_position(initial_position - 100);
        settle();
        let wide_panel = editor.right_panel.allocated_width();
        assert!((wide_panel - initial_panel_width - 100).abs() <= 2);
        editor.show_layers.set_active(false);
        settle();
        editor.show_layers.set_active(true);
        settle();
        assert!(
            (editor.right_panel.allocated_width() - wide_panel).abs() <= 2,
            "hiding and showing layers must preserve the chosen width"
        );
        editor.window.resize(1100, 700);
        settle();
        assert!(
            (editor.right_panel.allocated_width() - wide_panel).abs() <= 2,
            "window resizing must leave the inspector width stable"
        );
        editor.window.resize(1000, 700);
        settle();
        let panel_prefs = tempfile::tempdir().unwrap();
        let panel_path = panel_prefs.path().join("prefs.json");
        *editor.preferences_path.borrow_mut() = Some(panel_path.clone());
        editor.save_preferences().unwrap();
        assert_eq!(
            Preferences::load(&panel_path).unwrap().layer_panel_width,
            wide_panel
        );
        editor.restore_layer_width(initial_panel_width);
        settle();
        editor.load_preferences(&panel_path);
        editor.show_layers.set_active(true);
        settle();
        assert!(
            (editor.right_panel.allocated_width() - wide_panel).abs() <= 2,
            "saved inspector width must be restored"
        );
        snapshot("gtk-resizable-layers");
        let reopened = build(&app, Args::parse_from(["untushot", "--demo"]));
        reopened.load_preferences(&panel_path);
        reopened.show_layers.set_active(true);
        settle();
        assert!(
            (reopened.right_panel.allocated_width() - wide_panel).abs() <= 2,
            "a new editor must restore the saved panel width: {} vs {}",
            reopened.right_panel.allocated_width(),
            wide_panel
        );
        reopened.mark_project_saved();
        reopened.window.close();
        editor.window.present();
        editor.restore_layer_width(initial_panel_width);
        settle();
        editor.text(Point::new(200., 200.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Editierbare Textebene");
        let font_picker = editor.font_picker.clone();
        let font_popover = font_picker.popover().unwrap();
        let font_root = font_popover
            .child()
            .unwrap()
            .downcast::<gtk::Box>()
            .unwrap();
        let font_scroll = font_root
            .children()
            .into_iter()
            .find_map(|w| w.downcast::<gtk::ScrolledWindow>().ok())
            .unwrap();
        let font_list = font_scroll
            .child()
            .unwrap()
            .downcast::<gtk::Viewport>()
            .unwrap()
            .child()
            .unwrap()
            .downcast::<gtk::ListBox>()
            .unwrap();
        editor.font_size.set_value(72.);
        let before_size = editor.font_size.value();
        let before_text = editor
            .inline_input
            .buffer()
            .unwrap()
            .text(
                &editor.inline_input.buffer().unwrap().start_iter(),
                &editor.inline_input.buffer().unwrap().end_iter(),
                true,
            )
            .unwrap();
        font_picker.set_active(true);
        settle();
        assert!(font_popover.is_mapped());
        let current = font_list.selected_row().unwrap();
        let top = f64::from(current.allocation().y());
        let adjustment = font_scroll.vadjustment();
        assert!(
            top >= adjustment.value() - 1.
                && top + f64::from(current.allocated_height())
                    <= adjustment.value() + adjustment.page_size() + 1.,
            "current preset must be fully visible when opening"
        );
        assert_eq!(
            font_popover.relative_to().as_ref(),
            Some(font_picker.upcast_ref())
        );
        assert_eq!(
            editor.font_size.value(),
            before_size,
            "opening previews must not edit text"
        );
        assert!(font_scroll.vadjustment().upper() > font_scroll.vadjustment().page_size());
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-font-size-picker.png").unwrap())
            .unwrap();
        assert!(
            font_popover.allocated_width() <= 370,
            "font previews must remain compact: {} px",
            font_popover.allocated_width()
        );
        assert_eq!(
            editor.key_input(gdk::keys::constants::Return, gdk::ModifierType::empty()),
            glib::Propagation::Proceed
        );
        editor.key_input(gdk::keys::constants::Escape, gdk::ModifierType::empty());
        settle();
        assert_eq!(
            editor.font_size.value(),
            before_size,
            "dismissing previews must preserve size"
        );
        font_picker.set_active(true);
        settle();
        let row = font_list
            .children()
            .into_iter()
            .find(|w| w.widget_name() == "font-size-36")
            .unwrap()
            .downcast::<gtk::ListBoxRow>()
            .unwrap();
        font_list.emit_by_name::<()>("row-activated", &[&row]);
        settle();
        assert!(!font_popover.is_mapped());
        assert_eq!(
            editor
                .inline_edit
                .borrow()
                .as_ref()
                .unwrap()
                .annotation
                .text_size(),
            36.
        );
        assert_eq!(
            editor
                .inline_input
                .buffer()
                .unwrap()
                .text(
                    &editor.inline_input.buffer().unwrap().start_iter(),
                    &editor.inline_input.buffer().unwrap().end_iter(),
                    true
                )
                .unwrap(),
            before_text
        );
        editor.font_size.set_value(40.);
        assert!(
            font_list.selected_row().is_none(),
            "custom sizes must not mark an unrelated preset"
        );
        editor.font_size.set_value(before_size);
        editor.finish_text(true);
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        settle();
        assert!(editor.edit_text.is_mapped());
        // The anchored color editor stages changes until confirmation.
        let original_color = editor.color.rgba();
        let popover = editor.color.button.popover().unwrap();
        let root = popover.child().unwrap().downcast::<gtk::Box>().unwrap();
        let chooser = root
            .children()
            .into_iter()
            .find_map(|w| w.downcast::<gtk::ColorChooserWidget>().ok())
            .unwrap();
        editor.color.button.set_active(true);
        settle();
        assert!(popover.is_mapped());
        assert_eq!(
            popover.relative_to().as_ref(),
            Some(editor.color.button.upcast_ref())
        );
        snapshot("gtk-color-anchored");
        chooser.set_rgba(&gdk::RGBA::new(0.1, 0.7, 0.3, 1.));
        popover.popdown();
        settle();
        assert_eq!(editor.color.rgba(), original_color);
        editor.color.button.set_active(true);
        settle();
        assert_eq!(chooser.rgba(), original_color);
        chooser.set_show_editor(true);
        chooser.set_rgba(&gdk::RGBA::new(0.1, 0.7, 0.3, 1.));
        settle();
        snapshot("gtk-color-custom-anchored");
        let actions = root
            .children()
            .into_iter()
            .rev()
            .find_map(|w| w.downcast::<gtk::Box>().ok())
            .unwrap();
        let apply = actions
            .children()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| b.label().as_deref() == Some("Übernehmen"))
            .unwrap();
        apply.emit_clicked();
        let chosen = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .color;
        assert!((chosen.1 - 0.7).abs() < 0.01);
        editor.history(false);
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        assert_eq!(editor.color.rgba(), original_color);
        let geometry = editor.object_properties.children()[0]
            .clone()
            .downcast::<gtk::Expander>()
            .unwrap();
        assert!(!geometry.is_expanded());
        let collapsed_height = geometry.allocated_height();
        geometry.set_expanded(true);
        settle();
        assert!(geometry.allocated_height() > collapsed_height);
        assert!(editor.properties[0].is_mapped());
        let directory = tempfile::tempdir().unwrap();
        assert!(editor.save_project_to(&directory.path().join("Project.untushot")));
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        editor.nudge(1., 0., false);
        assert_eq!(editor.storage_caption.text(), "Projekt: geändert");
        editor.history(false);
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        editor.show_setup();
        assert!(!editor.project_save_button.is_visible());
        assert!(!editor.export_actions.is_visible());
        editor.setup.cancel.emit_clicked();
        let parent = editor.history_canvas.clone();
        crate::history::test_widget(&parent, |_| {});
        editor.window.close();
    }

    #[test]
    #[ignore = "requires a running graphical Wayland session"]
    fn gtk_smoke() {
        gtk::init().expect("GTK needs a graphical session");
        let app = gtk::Application::new(
            Some("io.github.untushot.SmokeTest"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gio::Cancellable::NONE).unwrap();
        shortcuts_and_image_switch(&app);
        monitor_changes(&app);
        browser_import(&app);
        object_resize_handles(&app);
        layer_list_controls(&app);
        image_layers(&app);
        let editor = build(&app, Args::parse_from(["untushot", "--demo", "--windowed"]));
        let context = glib::MainContext::default();
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(250);
            while std::time::Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        settle();
        assert!(editor.window.is_visible());
        assert!(editor.save.is_sensitive());
        assert!(editor.empty_layers.is_mapped());
        assert!(!editor.selection_label.is_visible());
        assert!(editor.storage_button.is_visible());
        assert!(editor.export_button.is_mapped());
        assert!(editor.copy.is_mapped());
        assert!(editor.project_save_button.is_mapped());
        assert_eq!(editor.storage_caption.text(), "Projekt: ungespeichert");
        assert!(!editor.recent.root.is_visible());
        let initial_inspector_width = editor.right_panel.allocated_width();
        assert!(editor.monitors.set_active_id(Some("1")));
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().selection(),
            Some(Rect::new(1440.0, 0.0, 960.0, 1100.0))
        );
        editor.undo.emit_clicked();
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .selection()
            .is_none());
        editor.redo.emit_clicked();
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .selection()
            .is_some());
        editor.tools[4].set_active(true);
        assert_eq!(editor.state.borrow().tool, Tool::Arrow);
        editor.monitors.set_active_id(Some("all"));
        editor.zoom(Some(1.25));
        settle();
        editor.zoom(None);
        settle();
        // Exercise the same canvas handlers used by pointer events, including
        // transformed coordinates and a single undo step for an entire drag.
        let v = editor.state.borrow().view;
        let xy = |x: f64, y: f64| (v.x + x * v.scale, v.y + y * v.scale);
        editor.tools[5].set_active(true);
        let (x, y) = xy(300., 280.);
        editor.press(x, y, false);
        let (x, y) = xy(900., 550.);
        editor.release(x, y);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
        settle();
        assert!(!editor.empty_layers.is_visible());
        assert!(editor.selection_label.is_visible());
        assert_eq!(
            editor.right_panel.allocated_width(),
            initial_inspector_width,
            "selecting a layer must not resize the canvas"
        );
        editor.tools[0].set_active(true);
        let (x, y) = xy(500., 400.);
        editor.press(x, y, false);
        let (x, y) = xy(600., 450.);
        editor.release(x, y);
        let bounds = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .bounds();
        assert!(
            (bounds.x - 400.).abs() < 1e-8
                && (bounds.y - 330.).abs() < 1e-8
                && (bounds.w - 600.).abs() < 1e-8
        );
        editor.undo.emit_clicked();
        let bounds = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .bounds();
        assert!((bounds.x - 300.).abs() < 1e-8 && (bounds.y - 280.).abs() < 1e-8);
        editor.redo.emit_clicked();
        let (x, y) = xy(600., 450.);
        editor.press(x, y, false);
        editor.release(x, y);
        editor.width.set_value(8.);
        let preset = crate::stroke::preset(&editor.width_picker, 12);
        let popup = editor.width_picker.popover().unwrap();
        popup.popup();
        settle();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            popup.allocated_width(),
            popup.allocated_height(),
        )
        .unwrap();
        popup.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-stroke-picker.png").unwrap())
            .unwrap();
        preset.emit_clicked();
        assert_eq!(editor.width.value(), 12.);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .width,
            12.
        );
        editor.history(false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .width,
            8.
        );
        // Undo deliberately clears object selection. Selecting the restored
        // layer must also restore its value and preset highlight in the picker.
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        assert_eq!(editor.width.value(), 8.);
        assert!(crate::stroke::preset(&editor.width_picker, 8).is_active());
        editor.width.set_value(13.);
        assert!(!crate::stroke::preset(&editor.width_picker, 8).is_active());
        editor.history(false);
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .width,
            8.
        );
        editor.visible.set_active(false);
        assert!(
            !editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .visible
        );
        editor.visible.set_active(true);
        // Group actions use actual canvas/list signals and remain one undo step.
        editor.duplicate_layers();
        editor.begin_layer_rename();
        editor.layer_name.set_text("Hinweis");
        editor.layer_name.emit_activate();
        editor.nudge(180., 230., false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[1]
                .name(),
            "Hinweis"
        );
        let (x, y) = xy(450., 350.);
        editor.press_modified(x, y, false, true);
        editor.release(x, y);
        assert_eq!(editor.state.borrow().indices().len(), 2);
        editor.nudge(10., 0., false);
        // Native themes change the viewport scale; transformed coordinates
        // may differ by floating-point roundoff, far below a pixel.
        assert!(
            (editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .bounds()
                .x
                - 410.)
                .abs()
                < 1e-8
        );
        editor.align_layers(Alignment::Left);
        {
            let s = editor.state.borrow();
            let a = s.document.as_ref().unwrap().annotations();
            assert_eq!(a[0].bounds().x, a[1].bounds().x);
        }
        editor.duplicate_layers();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            4
        );
        editor.undo.emit_clicked();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            2
        );
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        editor
            .layers
            .select_row(editor.layers.row_at_index(1).as_ref());
        assert_eq!(editor.state.borrow().indices().len(), 2);
        settle();
        let before = editor.area.allocated_width();
        editor.show_tools.set_active(false);
        editor.show_layers.set_active(false);
        settle();
        assert!(!editor.left_panel.is_visible() && !editor.right_panel.is_visible());
        assert!(editor.area.allocated_width() > before);
        editor.show_tools.set_active(true);
        editor.show_layers.set_active(true);
        settle();
        assert!(editor.monitors.allocated_width() >= 280);
        settle();
        // Render only our own widgets, never capture the user's desktop.
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        editor.window.draw(&cr);
        let mut file = std::fs::File::create("target/gtk-smoke.png").unwrap();
        surface.write_to_png(&mut file).unwrap();
        editor.show_setup();
        settle();
        assert_eq!(editor.stack.visible_child_name().as_deref(), Some("setup"));
        assert!(!editor.save.is_sensitive());
        assert!(!editor.setup.options.is_expanded());
        editor.setup.options.set_expanded(true);
        settle();
        assert!(editor.setup.source.allocated_width() >= 440);
        let setup_surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor
            .window
            .draw(&cairo::Context::new(&setup_surface).unwrap());
        setup_surface
            .write_to_png(&mut std::fs::File::create("target/gtk-capture.png").unwrap())
            .unwrap();
        editor.setup.cancel.emit_clicked();
        let mut captured = editor.state.borrow_mut().document.take().unwrap();
        captured.select(None);
        editor.accept_capture(captured, "Testaufnahme", Mode::Region, None);
        assert!(editor.state.borrow().selecting_region);
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .selection()
            .is_none());
        assert!(!editor.confirm_region.is_sensitive());
        assert!(!editor.header.is_visible());
        assert!(!editor.context.is_visible());
        settle();
        assert!(!editor.sidebar.is_visible());
        assert!(!editor.inspector.is_visible());
        assert!(!editor.save.is_sensitive());
        let v = editor.state.borrow().view;
        editor.press(v.x + 250. * v.scale, v.y + 200. * v.scale, false);
        let builds = editor.preview.borrow().builds;
        let frame = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.area.allocated_width(),
            editor.area.allocated_height(),
        )
        .unwrap();
        let frame_context = cairo::Context::new(&frame).unwrap();
        for step in 0..100 {
            editor.motion(
                v.x + (300. + step as f64 * 10.) * v.scale,
                v.y + 600. * v.scale,
            );
            draw(&editor, &editor.area, &frame_context).unwrap();
        }
        assert_eq!(
            editor.preview.borrow().builds,
            builds,
            "crop drag must reuse the raster"
        );
        editor.release(v.x + 1900. * v.scale, v.y + 700. * v.scale);
        settle();
        assert!(editor.confirm_region.is_sensitive());
        let r = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .selection()
            .unwrap();
        editor.press(v.x + r.right() * v.scale, v.y + r.bottom() * v.scale, false);
        editor.release(
            v.x + (r.right() + 20.) * v.scale,
            v.y + (r.bottom() + 30.) * v.scale,
        );
        let resized = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .selection()
            .unwrap();
        assert_eq!(resized.w, r.w + 20.);
        assert_eq!(resized.h, r.h + 30.);
        editor.press(
            v.x + (r.x + 100.) * v.scale,
            v.y + (r.y + 100.) * v.scale,
            false,
        );
        editor.release(v.x + (r.x + 110.) * v.scale, v.y + (r.y + 105.) * v.scale);
        editor.nudge(1., 0., false);
        let moved = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .selection()
            .unwrap();
        assert_eq!(moved.x, r.x + 11.);
        assert_eq!(moved.y, r.y + 5.);
        assert_eq!(moved.w, resized.w);
        assert_eq!(moved.h, resized.h);
        settle();
        let region_surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor
            .window
            .draw(&cairo::Context::new(&region_surface).unwrap());
        region_surface
            .write_to_png(&mut std::fs::File::create("target/gtk-region.png").unwrap())
            .unwrap();
        editor.confirm_region.emit_clicked();
        assert!(!editor.state.borrow().selecting_region);
        assert!(editor.header.is_visible());
        assert!(editor.context.is_visible());
        assert!(editor.save.is_sensitive());
        settle();
        assert_eq!(
            editor.state.borrow().canvas_bounds(),
            editor.state.borrow().document.as_ref().unwrap().selection()
        );
        let cropped_surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor
            .window
            .draw(&cairo::Context::new(&cropped_surface).unwrap());
        cropped_surface
            .write_to_png(&mut std::fs::File::create("target/gtk-editor-cropped.png").unwrap())
            .unwrap();
        editor.show_setup();
        editor.state.borrow_mut().dirty = false;
        editor.setup.delay.set_value(2.);
        editor.setup.options.set_expanded(true);
        editor.setup.delay.grab_focus();
        editor.key_input(gdk::keys::constants::Return, gdk::ModifierType::empty());
        settle();
        assert!(editor.state.borrow().capturing);
        editor.key_input(gdk::keys::constants::Escape, gdk::ModifierType::empty());
        for _ in 0..5 {
            settle();
        }
        assert!(!editor.state.borrow().capturing);
        assert!(editor.window.is_visible());
        // A long monitor label must not widen the editor, including with both
        // side panels open. The controls stay available in their scroll areas.
        let long_monitor = "Bildschirm 3 · Sehr langer Monitorname ".repeat(8);
        editor
            .monitors
            .append(Some("long-layout-test"), &long_monitor);
        editor
            .setup
            .source
            .append(Some("long-layout-test"), &long_monitor);
        editor.show_tools.set_active(true);
        editor.show_layers.set_active(true);
        editor.window.resize(1000, 700);
        settle();
        assert!(
            editor.window.allocated_width() <= 1080,
            "Long monitor labels widened the editor: {}",
            editor.window.allocated_width()
        );
        assert!(editor.area.allocated_width() > 360);
        let panels = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&panels).unwrap());
        panels
            .write_to_png(&mut std::fs::File::create("target/gtk-compact-panels.png").unwrap())
            .unwrap();
        editor.show_tools.set_active(false);
        editor.show_layers.set_active(false);
        editor.window.resize(1000, 700);
        settle();
        assert!(
            editor.window.allocated_width() <= 1080,
            "Compact width: {}",
            editor.window.allocated_width()
        );
        assert!(editor.area.allocated_width() > 750);
        let compact = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&compact).unwrap());
        compact
            .write_to_png(&mut std::fs::File::create("target/gtk-compact.png").unwrap())
            .unwrap();
        // Closing flushes the last edit even before the periodic timer ticks.
        let recovery_dir = tempfile::tempdir().unwrap();
        let (session, _) = recovery::Session::open(recovery_dir.path(), false).unwrap();
        editor.session.replace(Some(session));
        editor.recovery_revision.set(None);
        editor.state.borrow_mut().dirty = true;
        editor.window.close();
        settle();
        assert!(editor.session.borrow().is_none());
        let (_session, restored) = recovery::Session::open(recovery_dir.path(), true).unwrap();
        let restored = restored.unwrap();
        assert_eq!(restored.annotations().len(), 2);
        assert_eq!(restored.annotations()[1].name(), "Hinweis");
        assert_eq!(restored.selection(), Some(moved));
        let modes = build(&app, Args::parse_from(["untushot", "--demo"]));
        modes.accept_capture(document::demo().unwrap(), "Testaufnahme", Mode::Full, None);
        assert!(!modes.state.borrow().selecting_region);
        assert!(modes.save.is_sensitive());
        modes.accept_capture(
            document::demo().unwrap(),
            "Testaufnahme",
            Mode::Region,
            Some(1),
        );
        assert_eq!(
            modes.state.borrow().canvas_bounds(),
            Some(Rect::new(1440., 0., 960., 1100.))
        );
        assert!(!modes.confirm_region.is_sensitive());
        modes.window.close();
        settle();

        // Test the five everyday workflows on a fresh document, using the same
        // keyboard dispatcher and pointer handlers as the live window.
        let editor = build(
            &app,
            Args::parse_from(["untushot", "--demo", "--backend", "grim"]),
        );
        settle();
        use gdk::keys::constants as key;
        let none = gdk::ModifierType::empty();
        editor.key_input(key::Escape, none);
        assert!(editor.window.is_visible());
        editor.tools[4].set_active(true);
        let v = editor.state.borrow().view;
        editor.press(v.x + 200. * v.scale, v.y + 200. * v.scale, false);
        editor.motion(v.x + 500. * v.scale, v.y + 400. * v.scale);
        editor.key_input(key::Escape, none);
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .is_empty());
        editor.press(v.x + 200. * v.scale, v.y + 200. * v.scale, false);
        editor.motion(v.x + 500. * v.scale, v.y + 400. * v.scale);
        editor.key_input(key::Return, none);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
        editor.set_document(document::demo().unwrap(), "Text-Test");
        settle();
        editor.tools[9].set_active(true);
        let v = editor.state.borrow().view;
        let (x, y) = (v.x + 400. * v.scale, v.y + 300. * v.scale);
        editor.press(x, y, false);
        assert!(editor.inline_box.is_visible());
        editor.inline_input.buffer().unwrap().set_text("Verwerfen");
        editor.key_input(key::Escape, none);
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .is_empty());
        editor.press(x, y, false);
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Direkt im Bild\nZweite Zeile");
        editor.font_size.set_value(40.);
        editor.width.set_value(11.);
        assert_eq!(
            editor
                .inline_edit
                .borrow()
                .as_ref()
                .unwrap()
                .annotation
                .text_size(),
            40.
        );
        settle();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-inline-text.png").unwrap())
            .unwrap();
        editor.inline_input.grab_focus();
        editor.key_input(key::Return, none);
        assert!(editor.inline_edit.borrow().is_none());
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .text_size(),
            40.
        );
        editor.tools[0].set_active(true);
        editor.key_input(key::Return, none);
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Abgebrochene Korrektur");
        editor.key_input(key::Escape, none);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .text,
            "Direkt im Bild\nZweite Zeile"
        );
        editor.key_input(key::Return, none);
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Überarbeitet");
        editor.key_input(key::Return, none);
        editor.history(false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .text,
            "Direkt im Bild\nZweite Zeile"
        );
        editor.history(true);
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        editor.area.grab_focus();
        editor.key_input(key::Return, none);
        editor.font_size.grab_focus();
        editor.font_size.set_text("56");
        editor.key_input(key::Return, none);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .text_size(),
            56.
        );
        editor.key_input(key::Return, none);
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Kopie bearbeiten");
        editor.duplicate_layers();
        assert!(editor.inline_edit.borrow().is_none());
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            2
        );
        editor.history(false);
        editor.history(false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .text,
            "Überarbeitet"
        );
        settle();
        let annotations = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .to_vec();
        let v = editor.state.borrow().view;
        let anchor = Point::new(v.x + 850. * v.scale, v.y + 450. * v.scale);
        editor.zoom_at(Some(2.), anchor);
        settle();
        let v = editor.state.borrow().view;
        assert!(((anchor.x - v.x) / v.scale - 850.).abs() < 1e-8);
        assert!(((anchor.y - v.y) / v.scale - 450.).abs() < 1e-8);
        let pan = editor.state.borrow().pan;
        editor.start_pan(anchor);
        editor.motion(anchor.x + 50., anchor.y - 30.);
        editor.key_input(key::Escape, none);
        assert_eq!(editor.state.borrow().pan, pan);
        editor.start_pan(anchor);
        editor.release(anchor.x + 50., anchor.y - 30.);
        assert_eq!(
            editor.state.borrow().pan,
            Point::new(pan.x + 50., pan.y - 30.)
        );
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations(),
            annotations
        );
        editor.zoom(None);
        settle();
        let config = tempfile::tempdir().unwrap();
        let prefs_path = config.path().join("prefs.json");
        editor.load_preferences(&prefs_path);
        editor.tools[4].set_active(true);
        editor.width.set_value(7.);
        editor.font_size.set_value(42.);
        editor.state.borrow_mut().color = (0.2, 0.6, 0.8);
        editor.show_tools.set_active(false);
        editor.show_layers.set_active(false);
        if !editor.setup.map.monitors().is_empty() {
            editor.setup.source.set_active_id(Some("0"));
        }
        let output = config.path().join("result.png");
        save_png(
            &editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .export()
                .unwrap(),
            &output,
        )
        .unwrap();
        editor.mark_exported(&output);
        let exported_before = std::fs::read(&output).unwrap();
        editor.save();
        assert_eq!(std::fs::read(&output).unwrap(), exported_before);
        assert_eq!(editor.export_path.borrow().as_ref(), Some(&output));
        editor.refresh();
        assert!(
            editor.state.borrow().dirty,
            "PNG must not mark the editable project saved"
        );
        assert!(editor.storage.text().contains("PNG aktuell"));
        editor.save_preferences().unwrap();
        let loaded = Preferences::load(&prefs_path).unwrap();
        assert_eq!(loaded.tool, Tool::Arrow);
        assert_eq!(loaded.width, 7.);
        assert_eq!(loaded.font_size, 42.);
        assert_eq!(loaded.color, (0.2, 0.6, 0.8));
        assert!(!loaded.tools_visible && !loaded.layers_visible);
        assert_eq!(loaded.export_directory.as_deref(), Some(config.path()));
        let reopened = build(
            &app,
            Args::parse_from(["untushot", "--demo", "--backend", "grim"]),
        );
        reopened.load_preferences(&prefs_path);
        settle();
        assert_eq!(reopened.state.borrow().tool, Tool::Arrow);
        assert_eq!(reopened.state.borrow().font_size, 42.);
        assert!(!reopened.left_panel.is_visible() && !reopened.right_panel.is_visible());
        if loaded.monitor.is_some() {
            assert_eq!(reopened.setup.source.active_id().as_deref(), Some("0"));
        }
        reopened.window.close();
        settle();
        let project = config.path().join("test.untushot");
        atomic_write(&project, |file| {
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .write_project(file)
        })
        .unwrap();
        editor.mark_project_saved();
        editor.refresh();
        assert!(!editor.state.borrow().dirty);
        assert!(editor.storage.text().contains("Projekt gespeichert"));
        assert!(open_document(&project).unwrap().1);
        let session_dir = tempfile::tempdir().unwrap();
        editor.session.replace(Some(
            recovery::Session::open(session_dir.path(), false)
                .unwrap()
                .0,
        ));
        editor.save_recovery().unwrap();
        assert!(editor.storage.text().contains("Sicherung ausstehend"));
        editor.session.borrow().as_ref().unwrap().flush().unwrap();
        editor.poll_recovery();
        assert!(editor.storage.text().contains("Automatisch gesichert"));
        assert_eq!(editor.storage_caption.text(), "Automatisch gesichert");
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        editor.nudge(1., 0., false);
        assert!(editor.storage.text().contains("Projekt geändert"));
        assert_eq!(editor.storage_caption.text(), "Sicherung ausstehend");
        assert!(editor.storage.text().contains("PNG enthält älteren Stand"));
        editor.history(false);
        assert!(editor.storage.text().contains("Projekt gespeichert"));
        assert_eq!(editor.storage_caption.text(), "Sicherung ausstehend");
        assert!(editor.storage.text().contains("PNG aktuell"));
        assert!(!editor.state.borrow().dirty);
        editor.history(true);
        assert!(editor.storage.text().contains("Projekt geändert"));
        assert!(editor.storage.text().contains("Sicherung ausstehend"));
        editor.save_recovery().unwrap();
        assert!(editor.storage.text().contains("Sicherung ausstehend"));
        editor.window.close();
        settle();
        assert!(editor.closed.get());
        let (_session, recovered) = recovery::Session::open(session_dir.path(), true).unwrap();
        assert_eq!(recovered.unwrap().annotations()[0].text, "Überarbeitet");
        // Opting into background operation saves and hides; delayed actions are
        // cancellable without allowing a real desktop capture in this test.
        let resident = build(&app, Args::parse_from(["untushot", "--demo"]));
        settle();
        let resident_dir = tempfile::tempdir().unwrap();
        resident.session.replace(Some(
            recovery::Session::open(resident_dir.path(), false)
                .unwrap()
                .0,
        ));
        resident.set_resident(true);
        resident.close_to_tray.set_active(true);
        resident.window.close();
        settle();
        assert!(!resident.window.is_visible());
        assert!(!resident.is_closed());
        assert!(resident.session.borrow().is_some());
        resident.present();
        settle();
        assert!(resident.window.is_visible());
        let request = Args::parse_from(["untushot", "--capture", "region", "--delay", "60"]);
        assert!(resident.request(request));
        assert!(resident.is_capturing());
        assert!(!resident.window.is_visible());
        resident.cancel_capture();
        settle();
        assert!(!resident.is_capturing());
        assert!(resident.window.is_visible());
        assert_eq!(
            resident.state.borrow().document.as_ref().unwrap().width(),
            2400
        );
        assert!(resident.shutdown());
        resident.set_resident(false);
        resident.window.close();
        settle();
        // Theme changes are local to this test process, never written to GNOME.
        let themed = build(&app, Args::parse_from(["untushot", "--demo"]));
        let settings = gtk::Settings::default().unwrap();
        let original_theme = settings.property::<String>("gtk-theme-name");
        let original_dark = settings.property::<bool>("gtk-application-prefer-dark-theme");
        settings.set_property("gtk-application-prefer-dark-theme", false);
        let yaru = std::path::Path::new("/usr/share/themes/Yaru-sage-dark").exists();
        let dark = if yaru {
            "Yaru-sage-dark"
        } else {
            "Adwaita-dark"
        };
        let light = if yaru { "Yaru-blue" } else { "Adwaita" };
        settings.set_property("gtk-theme-name", dark);
        settle();
        let dark_palette = crate::theme::Palette::for_widget(&themed.area);
        assert!(dark_palette.background.red() < 0.5);
        assert_eq!(
            Some(dark_palette.accent),
            themed
                .save
                .style_context()
                .lookup_color("theme_selected_bg_color")
        );
        assert!(themed.window.titlebar().unwrap().is::<gtk::HeaderBar>());
        let snapshot = |name: &str| {
            let surface = cairo::ImageSurface::create(
                cairo::Format::ARgb32,
                themed.window.allocated_width(),
                themed.window.allocated_height(),
            )
            .unwrap();
            themed.window.draw(&cairo::Context::new(&surface).unwrap());
            surface
                .write_to_png(&mut std::fs::File::create(format!("target/{name}.png")).unwrap())
                .unwrap();
        };
        snapshot("gtk-theme-dark");
        themed.file_menu.set_active(true);
        settle();
        let menu = themed.file_menu.popover().unwrap();
        let menu_surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            menu.allocated_width(),
            menu.allocated_height(),
        )
        .unwrap();
        menu.draw(&cairo::Context::new(&menu_surface).unwrap());
        menu_surface
            .write_to_png(&mut std::fs::File::create("target/gtk-file-menu.png").unwrap())
            .unwrap();
        themed.file_menu.set_active(false);
        settle();
        themed.storage_button.set_active(true);
        settle();
        assert!(themed.storage.is_mapped());
        assert!(themed
            .storage
            .text()
            .contains("Projekt noch nicht gespeichert"));
        let storage = themed.storage_button.popover().unwrap();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            storage.allocated_width(),
            storage.allocated_height(),
        )
        .unwrap();
        storage.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-storage.png").unwrap())
            .unwrap();
        themed.storage_button.set_active(false);
        settle();
        let annotation_color = themed.state.borrow().color;
        settings.set_property("gtk-theme-name", light);
        settle();
        let light_palette = crate::theme::Palette::for_widget(&themed.area);
        assert!(light_palette.background.red() > 0.5);
        if yaru {
            assert_ne!(light_palette.accent, dark_palette.accent);
        }
        assert_eq!(
            Some(light_palette.accent),
            themed
                .save
                .style_context()
                .lookup_color("theme_selected_bg_color")
        );
        assert_eq!(themed.state.borrow().color, annotation_color);
        snapshot("gtk-theme-light");
        themed.show_setup();
        settle();
        snapshot("gtk-theme-setup-light");
        settings.set_property("gtk-theme-name", dark);
        settle();
        snapshot("gtk-theme-setup-dark");
        assert!(!themed.export_actions.is_visible());
        assert!(!themed.storage_button.is_visible());
        assert!(!themed.project_save_button.is_visible());
        let start_caption = || {
            themed
                .setup
                .start
                .child()
                .unwrap()
                .downcast::<gtk::Box>()
                .unwrap()
                .children()
                .last()
                .unwrap()
                .clone()
                .downcast::<gtk::Label>()
                .unwrap()
                .text()
                .to_string()
        };
        themed.setup.delay.set_value(3.);
        assert_eq!(start_caption(), "Bereich auswählen · 3 s");
        themed.setup.full.set_active(true);
        assert_eq!(start_caption(), "Bildschirm aufnehmen · 3 s");
        themed.setup.system.set_active(true);
        themed.setup.delay.set_value(0.);
        assert_eq!(start_caption(), "Fenster oder Bildschirm wählen");
        settings.set_property("gtk-theme-name", original_theme);
        settings.set_property("gtk-application-prefer-dark-theme", original_dark);
        themed.setup.cancel.emit_clicked();
        settle();
        let parent = themed.history_canvas.clone();
        parent.remove(&themed.recent.root);
        let weak = Rc::downgrade(&themed);
        crate::history::test_widget(&parent, move |path| {
            let editor = weak.upgrade().unwrap();
            assert!(editor.open_path(path));
            assert!(
                editor.export_path.borrow().is_none(),
                "opening must not silently target the original for overwriting"
            );
            assert_eq!(
                editor.state.borrow().document.as_ref().unwrap().width(),
                2400
            );
        });
        themed.window.close();
        settle();
    }
    fn image_layers(app: &gtk::Application) {
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        let directory = tempfile::tempdir().unwrap();
        let card = |title: &str, color: (f64, f64, f64)| {
            let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 620, 420).unwrap();
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgb(0.96, 0.96, 0.95);
            cr.paint().unwrap();
            cr.set_source_rgb(color.0, color.1, color.2);
            cr.rectangle(0., 0., 620., 12.);
            cr.fill().unwrap();
            cr.select_font_face(
                "sans-serif",
                cairo::FontSlant::Normal,
                cairo::FontWeight::Bold,
            );
            cr.set_font_size(40.);
            cr.move_to(36., 78.);
            cr.show_text(title).unwrap();
            cr.set_source_rgb(0.88, 0.89, 0.87);
            cr.rectangle(36., 112., 548., 238.);
            cr.fill().unwrap();
            let mut png = Vec::new();
            surface.write_to_png(&mut png).unwrap();
            crate::pixels::from_bytes(&png).unwrap()
        };
        let second = directory.path().join("02 – Bearbeiten.png");
        crate::pixels::write_png(
            &card("02 · Bearbeiten", (0.26, 0.40, 0.31)),
            std::fs::File::create(&second).unwrap(),
        )
        .unwrap();
        editor.set_document(
            Document::new(card("01 · Aufnahme", (0.25, 0.35, 0.50)), Vec::new()).unwrap(),
            "Flow zusammenstellen",
        );
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(250);
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        settle();
        editor.append_image_path(&second);
        settle();
        assert_eq!(editor.state.borrow().selected, Some(1));
        assert_eq!(editor.layers.children().len(), 2);
        assert!(!editor.base.is_visible());
        assert!(!editor.width_controls.is_visible());
        assert!(!editor.color_controls.is_visible());
        assert!(!editor.monitor_guides.is_visible());
        assert!(editor.export_path.borrow().is_none());
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().width(),
            1264
        );
        // Pointer resize keeps the image ratio, even with unequal dx / dy.
        let view = editor.state.borrow().view;
        editor.press(
            view.x + 1264. * view.scale,
            view.y + 420. * view.scale,
            false,
        );
        editor.motion(view.x + 1140. * view.scale, view.y + 300. * view.scale);
        editor.release(view.x + 1140. * view.scale, view.y + 300. * view.scale);
        let bounds = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[1]
            .bounds();
        assert!((bounds.w / bounds.h - 620. / 420.).abs() < 0.0001);
        assert!(bounds.w < 620.);
        editor.history(false);
        editor.history(false);
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().width(),
            620
        );
        assert!(!editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .is_composition());
        assert!(!editor.right_panel.is_visible());
        editor.history(true);
        settle();
        assert_eq!(editor.layers.children().len(), 2);
        let project = directory.path().join("Flow.untushot");
        assert!(editor.save_project_to(&project));
        let saved = Document::read_project(std::fs::File::open(&project).unwrap()).unwrap();
        assert_eq!(saved.annotations().len(), 2);
        assert_eq!(saved.export().unwrap().width(), 1264);
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        settle();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-image-layers.png").unwrap())
            .unwrap();
        // A failed import is atomic and leaves saved targets and layers intact.
        let revision = editor.state.borrow().document.as_ref().unwrap().revision();
        glib::idle_add_local_once(|| {
            for dialog in gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::MessageDialog>().ok())
            {
                if dialog.is_visible() {
                    dialog.response(gtk::ResponseType::Close);
                }
            }
        });
        editor.append_image_path(&directory.path().join("missing.png"));
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().revision(),
            revision
        );
        assert_eq!(editor.project_path.borrow().as_ref(), Some(&project));
        editor.window.close();
    }

    fn browser_import(app: &gtk::Application) {
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        editor.text(Point::new(100., 100.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Keep this annotation");
        assert!(editor
            .window
            .focused_widget()
            .unwrap()
            .is::<gtk::TextView>());
        assert_eq!(
            editor.key_input(gdk::keys::constants::v, gdk::ModifierType::CONTROL_MASK),
            glib::Propagation::Proceed
        );
        editor.finish_text(true);
        let before = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .content_id();
        let image = crate::web_import::gtk_smoke(&editor.window);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            before
        );
        glib::idle_add_local_once(|| {
            let dialog = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Dialog>().ok())
                .find(|d| d.is_visible() && d.title().as_deref() == Some("Änderungen speichern"))
                .unwrap();
            dialog.response(gtk::ResponseType::Cancel);
        });
        assert!(!editor.accept_import(image.clone()));
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            before
        );
        editor.mark_project_saved();
        assert!(editor.accept_import(image));
        assert!(editor.export_path.borrow().is_none());
        assert!(editor.project_path.borrow().is_none());
        assert!(!editor.monitor_button.is_visible());
        let exported = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .export()
            .unwrap();
        assert_eq!((exported.width(), exported.height()), (900, 2400));
        let offset = 2399 * exported.rowstride() as usize + 899 * exported.n_channels() as usize;
        assert_eq!(
            &exported.read_pixel_bytes().as_ref()[offset..offset + 3],
            &[30, 100, 200]
        );
        glib::idle_add_local_once(|| {
            let dialog = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Dialog>().ok())
                .find(|d| d.is_visible() && d.title().as_deref() == Some("Webseite übernehmen"))
                .unwrap();
            dialog.response(gtk::ResponseType::Cancel);
        });
        editor.setup.web.emit_clicked();
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().height(),
            2400
        );
        editor.window.close();
    }

    fn monitor_changes(app: &gtk::Application) {
        let editor = build(
            app,
            Args::parse_from(["untushot", "--demo", "--backend", "grim"]),
        );
        let a = crate::geometry::Monitor {
            name: "Same 4K model".into(),
            rect: Rect::new(2560., 0., 1440., 2560.),
            scale: 2.,
        };
        let b = crate::geometry::Monitor {
            name: "Same 4K model".into(),
            rect: Rect::new(0., 752., 2560., 1440.),
            scale: 2.,
        };
        editor.show_setup();
        editor.setup.update_monitors(vec![a.clone(), b.clone()]);
        assert!(editor.setup.source.set_active_id(Some("1")));
        // Keep the chosen display, never its old index, after enumeration changes.
        editor.setup.update_monitors(vec![b.clone(), a.clone()]);
        assert_eq!(editor.setup.source.active_id().as_deref(), Some("0"));
        editor.setup.update_monitors(vec![b.clone(), b.clone()]);
        assert_eq!(editor.setup.source.active_id().as_deref(), Some("all"));
        editor.setup.set_backend(crate::Backend::Portal);
        assert!(!editor.setup.source.set_active_id(Some("0")));
        editor.setup.system.set_active(true);
        editor.setup.set_backend(crate::Backend::Grim);
        assert!(!editor.setup.map.area.is_sensitive());
        editor.setup.full.set_active(true);
        assert!(editor.setup.map.area.is_sensitive());
        editor.setup.update_monitors(vec![a, b]);
        assert!(editor.setup.source.set_active_id(Some("1")));
        let settle = |duration: Duration| {
            let until = std::time::Instant::now() + duration;
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        settle(Duration::from_millis(250));
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-multimonitor-setup.png").unwrap())
            .unwrap();
        editor.accept_capture(document::demo().unwrap(), "Test", Mode::Full, Some(1));
        let image = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .export()
            .unwrap();
        assert_eq!((image.width(), image.height()), (960, 1100));
        assert!(!editor.state.borrow().selecting_region);
        editor.accept_capture(document::demo().unwrap(), "Test", Mode::System, Some(1));
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .selection()
            .is_none());
        assert!(!editor.state.borrow().selecting_region);
        assert!(editor.monitor_button.is_visible());
        let mut unknown = document::demo().unwrap();
        unknown.monitors.clear();
        editor.accept_capture(unknown, "Portal test", Mode::Full, Some(1));
        assert!(editor.state.borrow().selecting_region);
        assert!(!editor.monitor_button.is_visible());
        assert!(editor.status.text().contains("nicht eindeutig"));
        assert!(!editor.confirm_region.is_sensitive());
        // The test never invokes a capture backend: the changed topology must
        // cancel the timer and invalidate the continuation before its deadline.
        editor.show_setup();
        let current = capture::desktop_layout().expect("GTK session has a monitor");
        let mut changed = current.clone();
        changed.monitors[0].rect.x += 100.;
        let changed = Layout::new(changed.monitors).unwrap();
        editor.setup.delay.set_value(1.);
        editor.capture_with_visibility(true);
        assert!(editor.is_capturing());
        assert!(!editor.window.is_visible());
        editor.sync_monitor_layout(Some(changed));
        assert!(!editor.is_capturing());
        assert!(editor.capture_cancel.borrow().is_none());
        assert!(editor.window.is_visible());
        assert_eq!(editor.stack.visible_child_name().as_deref(), Some("setup"));
        assert!(editor.setup.progress.text().contains("Aufnahme gestoppt"));
        settle(Duration::from_millis(1600));
        assert!(!editor.is_capturing());
        assert!(editor.window.is_visible());
        assert!(editor.setup.progress.text().contains("Aufnahme gestoppt"));
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().width(),
            2400
        );
        // A stale setup is also rejected before starting any timer or backend.
        editor.capture_with_visibility(true);
        assert!(!editor.is_capturing());
        assert_eq!(editor.setup.map.monitors(), current.monitors);
        assert!(editor.setup.progress.text().contains("aktualisiert"));
        editor.setup.delay.set_value(0.);
        editor.capture_with_visibility(true);
        let mut changed = current.clone();
        changed.monitors[0].rect.y += 100.;
        editor.sync_monitor_layout(Layout::new(changed.monitors));
        settle(Duration::from_millis(500));
        assert!(!editor.is_capturing());
        assert!(editor.window.is_visible());
        assert!(editor.setup.progress.text().contains("Aufnahme gestoppt"));
        assert!(!editor.request(Args::parse_from([
            "untushot",
            "--capture",
            "full",
            "--backend",
            "portal",
            "--monitor",
            "1",
        ])));
        assert!(!editor.is_capturing());
        assert!(editor
            .setup
            .progress
            .text()
            .contains("nicht direkt auswählbar"));
        editor.window.close();
    }

    fn layer_list_controls(app: &gtk::Application) {
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        let solid = |rgba| {
            let image =
                gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 100, 60).unwrap();
            image.fill(rgba);
            image
        };
        let mut doc = Document::new(solid(0x406080ff), Vec::new()).unwrap();
        doc.append_image_right(solid(0x48a080ff), "Zweiter Screenshot".into())
            .unwrap();
        for (tool, label) in [(Tool::Text, "Hinweis"), (Tool::Rectangle, "Markierung")] {
            doc.add(Annotation {
                effects: Default::default(),
                tool,
                points: vec![Point::new(10., 10.), Point::new(80., 50.)],
                color: (1., 0.2, 0.1),
                width: 3.,
                visible: true,
                label: label.into(),
                text: "Text".into(),
                font_size: Some(18.),
                clip: None,
            });
        }
        editor.set_document(doc, "Ebenen: Vorschau und Sichtbarkeit");
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("Layers.untushot");
        assert!(editor.save_project_to(&project));
        {
            let mut s = editor.state.borrow_mut();
            s.selected = Some(0);
            s.selected_many = vec![0, 2];
        }
        editor.refresh();
        let visibility = |index: usize| {
            let row = editor
                .layers
                .children()
                .into_iter()
                .find(|row| row.widget_name() == index.to_string())
                .unwrap()
                .downcast::<gtk::ListBoxRow>()
                .unwrap();
            row.child()
                .unwrap()
                .downcast::<gtk::Box>()
                .unwrap()
                .children()
                .into_iter()
                .find(|w| w.widget_name() == "layer-visibility")
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap()
        };
        let builds = editor.layer_previews.borrow().builds;
        let stale = visibility(1);
        stale.emit_clicked();
        assert_eq!(
            editor.state.borrow().indices(),
            vec![0, 2],
            "eye button must preserve a different multi-selection"
        );
        assert_eq!(
            editor.layer_previews.borrow().builds,
            builds,
            "visibility does not rerender thumbnails"
        );
        assert!(
            !editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[1]
                .visible
        );
        assert!(visibility(1).tooltip_text().unwrap().contains("einblenden"));
        assert_eq!(editor.storage_caption.text(), "Projekt: geändert");
        editor.history(false);
        assert!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[1]
                .visible
        );
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        let id = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .content_id();
        stale.emit_clicked();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            id
        );
        editor.history(true);
        assert!(
            !editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[1]
                .visible
        );
        assert!(editor.state.borrow().indices().is_empty());
        visibility(1).emit_clicked();
        assert!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[1]
                .visible
        );
        assert!(
            editor.state.borrow().indices().is_empty(),
            "eye also works with no selection"
        );
        visibility(1).emit_clicked();
        assert!(editor.save_project_to(&project));
        let loaded = Document::read_project(std::fs::File::open(&project).unwrap()).unwrap();
        assert!(!loaded.annotations()[1].visible);
        assert!(loaded.annotations()[0].visible && loaded.annotations()[2].visible);
        let until = std::time::Instant::now() + Duration::from_millis(300);
        while std::time::Instant::now() < until {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-layer-previews.png").unwrap())
            .unwrap();
        editor.window.close();
    }

    fn snapping_and_context_menu(app: &gtk::Application) {
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(250);
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        let mut doc = document::demo().unwrap();
        doc.select(Some(Rect::new(0., 0., 900., 600.)));
        let original = Rect::new(100., 120., 120., 80.);
        doc.copy_region_to_layer(original).unwrap();
        doc.copy_region_to_layer(Rect::new(400., 300., 160., 100.))
            .unwrap();
        let mut hidden = doc.annotations()[1].translated(-2., -2.);
        hidden.visible = false;
        doc.add(hidden);
        let outside = doc.annotations()[1].translated(1000., -2.);
        doc.add(outside);
        let layer_count = doc.annotations().len();
        editor.set_document(doc, "Einrasten und Objektaktionen");
        editor.select_new_image(0, "Einrasten und Objektaktionen");
        editor.window.resize(1000, 700);
        settle();
        let view = editor.state.borrow().view;
        let at = |x, y| (view.x + x * view.scale, view.y + y * view.scale);
        let (x, y) = at(160., 160.);
        let (ex, ey) = at(457., 337.);
        let revision = editor.state.borrow().document.as_ref().unwrap().revision();
        // Clicking without moving does not snap or create an undo step.
        editor.press(x, y, false);
        editor.release(x, y);
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().revision(),
            revision
        );
        editor.press(x, y, false);
        editor.motion(ex, ey);
        {
            let s = editor.state.borrow();
            let drag = s.drag.as_ref().unwrap();
            assert_eq!(
                drag.previews[0].1.bounds(),
                Rect::new(400., 300., 120., 80.)
            );
            assert_eq!(drag.guides.len(), 2);
            // Hidden layer at 398/298 must not attract the moving image.
            assert_eq!(
                s.document.as_ref().unwrap().annotations()[0].bounds(),
                original
            );
        }
        editor.state.borrow_mut().snap_bypass = true;
        editor.motion(ex, ey);
        assert!(editor
            .state
            .borrow()
            .drag
            .as_ref()
            .unwrap()
            .guides
            .is_empty());
        assert!(
            (editor.state.borrow().drag.as_ref().unwrap().previews[0]
                .1
                .bounds()
                .x
                - 397.)
                .abs()
                < 0.001
        );
        editor.state.borrow_mut().snap_bypass = false;
        editor.snapping.set_active(false);
        editor.motion(ex, ey);
        assert!(editor
            .state
            .borrow()
            .drag
            .as_ref()
            .unwrap()
            .guides
            .is_empty());
        editor.snapping.set_active(true);
        settle();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-snapping-guides.png").unwrap())
            .unwrap();
        editor.cancel_action();
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().revision(),
            revision
        );
        editor.press(x, y, false);
        editor.release(ex, ey);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .bounds(),
            Rect::new(400., 300., 120., 80.)
        );
        editor.history(false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .bounds(),
            original
        );
        {
            let mut s = editor.state.borrow_mut();
            s.selected = Some(0);
            s.selected_many = vec![0, 1];
        }
        editor.press(x, y, false);
        let (gx, gy) = at(63., 43.);
        editor.motion(gx, gy);
        {
            let s = editor.state.borrow();
            let drag = s.drag.as_ref().unwrap();
            assert_eq!(drag.previews[0].1.bounds(), Rect::new(0., 0., 120., 80.));
            assert_eq!(
                drag.previews[1].1.bounds(),
                Rect::new(300., 180., 160., 100.)
            );
        }
        editor.cancel_action();
        editor.history(true);
        let (mx, my) = at(405., 310.);
        let item = |menu: &gtk::Menu, index: usize| {
            menu.children()[index]
                .clone()
                .downcast::<gtk::MenuItem>()
                .unwrap()
        };
        let menu = editor.menu_at_object(mx, my).unwrap();
        assert_eq!(editor.state.borrow().selected, Some(1));
        item(&menu, 0).activate();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            layer_count + 1
        );
        // Already opened actions cannot mutate a newer document state.
        item(&menu, 0).activate();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            layer_count + 1
        );
        editor.history(false);
        let menu = editor.menu_at_object(mx, my).unwrap();
        item(&menu, 2).activate();
        assert_eq!(editor.state.borrow().selected, Some(0));
        editor.history(false);
        let menu = editor.menu_at_object(mx, my).unwrap();
        item(&menu, 3).activate();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            layer_count - 1
        );
        editor.history(false);
        {
            let mut s = editor.state.borrow_mut();
            s.selected = Some(0);
            s.selected_many = vec![0, 1];
        }
        let menu = editor.menu_at_object(mx, my).unwrap();
        assert_eq!(editor.state.borrow().indices(), vec![0, 1]);
        assert!(!item(&menu, 1).is_sensitive() && !item(&menu, 2).is_sensitive());
        item(&menu, 0).activate();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            layer_count + 2
        );
        editor.history(false);
        assert!(editor.menu_at_object(view.x - 10., view.y - 10.).is_none());
        let menu = editor.menu_at_object(mx, my).unwrap();
        editor.set_document(document::demo().unwrap(), "Neues Dokument");
        item(&menu, 3).activate();
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .is_empty());
        editor.mark_project_saved();
        editor.window.close();
    }

    fn live_properties(app: &gtk::Application) {
        use gdk::keys::constants as key;
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(250);
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        let mut doc = document::demo().unwrap();
        doc.select(Some(Rect::new(0., 0., 1000., 700.)));
        doc.copy_region_to_layer(Rect::new(70., 75., 450., 190.))
            .unwrap();
        editor.set_document(doc, "Eigenschaften direkt bearbeiten");
        editor.select_new_image(
            0,
            "Direkte Vorschau · Strg+Z macht eine Feldänderung rückgängig",
        );
        editor.border_enabled.set_active(true);
        editor.shadow_enabled.set_active(true);
        editor.window.resize(1000, 700);
        settle();
        assert!(editor.window.allocated_width() <= 1080);
        // Keyboard navigation reveals the last effect in the independent
        // properties scroller at the compact window size.
        editor.shadow_offset.grab_focus();
        settle();
        let (_, y) = editor
            .shadow_offset
            .translate_coordinates(&editor.property_scroll, 0, 0)
            .unwrap();
        assert!(
            y >= 0
                && y + editor.shadow_offset.allocated_height()
                    <= editor.property_scroll.allocated_height()
        );
        let original = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .clone();
        editor.mark_project_saved();
        editor.shadow_blur.grab_focus();
        let saved_id = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .content_id();
        editor.shadow_blur.set_text("200");
        editor.area.grab_focus();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            saved_id
        );
        editor.shadow_blur.grab_focus();
        editor.shadow_blur.set_text("2");
        editor.shadow_blur.set_text("24");
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .effects
                .shadow_blur,
            24.
        );
        assert_eq!(editor.shadow_blur.text(), "24");
        editor.shadow_blur.set_text("-");
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .effects
                .shadow_blur,
            24.
        );
        editor.shadow_blur.set_text("25");
        editor.key_input(key::z, gdk::ModifierType::CONTROL_MASK);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0],
            original
        );
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        editor.history(true);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .effects
                .shadow_blur,
            25.
        );
        editor.select_new_image(0, "Direkte Vorschau");
        editor.shadow_offset.grab_focus();
        editor.shadow_offset.set_text("10");
        editor.shadow_offset.set_text("16");
        editor.key_input(key::Return, gdk::ModifierType::empty());
        editor.history(false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .effects
                .shadow_blur,
            25.
        );
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .effects
                .shadow_offset,
            original.effects.shadow_offset
        );
        editor.select_new_image(0, "Direkte Vorschau");
        let geometry = editor.object_properties.children()[0]
            .clone()
            .downcast::<gtk::Expander>()
            .unwrap();
        geometry.set_expanded(true);
        settle();
        editor.properties[2].grab_focus();
        editor.properties[2].set_text("500");
        editor.properties[2].set_text("600");
        let bounds = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .bounds();
        assert!((bounds.w - 600.).abs() < 0.01);
        assert!((bounds.w / bounds.h - 450. / 190.).abs() < 0.001);
        editor.key_input(key::Return, gdk::ModifierType::empty());
        // Saving splits the group so Undo can return exactly to the saved state.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.untushot");
        assert!(editor.save_project_to(&path));
        editor.properties[2].set_value(650.);
        editor.properties[2].set_value(700.);
        editor.history(false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .bounds(),
            bounds
        );
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        let loaded = Document::read_project(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(loaded.annotations()[0].bounds(), bounds);
        assert_eq!(loaded.annotations()[0].effects.shadow_blur, 25.);
        editor.select_new_image(0, "Größe, Rand und Schatten direkt im Bild einstellen");
        geometry.set_expanded(false);
        editor.nudge(200., 180., false);
        editor.layer_scroll.vadjustment().set_value(0.);
        settle();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-live-properties.png").unwrap())
            .unwrap();
        // Both panes gain space in a taller window and scroll independently.
        editor.window.resize(1000, 700);
        settle();
        let compact_height = editor.layer_scroll.allocated_height();
        let compact_properties = editor.property_scroll.allocated_height();
        editor.window.resize(1000, 850);
        settle();
        assert!(editor.layer_scroll.allocated_height() > compact_height);
        assert!(editor.property_scroll.allocated_height() > compact_properties);
        editor.window.resize(1000, 700);
        {
            let mut s = editor.state.borrow_mut();
            let doc = s.document.as_mut().unwrap();
            for _ in 0..10 {
                doc.duplicate_many(&[0]);
            }
        }
        editor.refresh();
        settle();
        let options_position = editor
            .shadow_offset
            .translate_coordinates(&editor.right_panel, 0, 0)
            .unwrap();
        let adjustment = editor.layer_scroll.vadjustment();
        assert!(adjustment.upper() > adjustment.page_size());
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
        settle();
        assert!(adjustment.value() > 0.);
        assert_eq!(
            editor
                .shadow_offset
                .translate_coordinates(&editor.right_panel, 0, 0)
                .unwrap(),
            options_position
        );
        let collapsed_height = editor.layer_scroll.allocated_height();
        geometry.set_expanded(true);
        settle();
        assert_eq!(editor.layer_scroll.allocated_height(), collapsed_height);
        editor.area.grab_focus();
        editor.shadow_offset.grab_focus();
        settle();
        let (_, bottom) = editor
            .shadow_offset
            .translate_coordinates(&editor.property_scroll, 0, 0)
            .unwrap();
        assert!(
            bottom >= 0
                && bottom + editor.shadow_offset.allocated_height()
                    <= editor.property_scroll.allocated_height()
        );
        geometry.set_expanded(false);
        settle();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-flexible-layers.png").unwrap())
            .unwrap();
        editor.mark_project_saved();
        editor.window.close();
    }

    fn selection_and_layer_effects(app: &gtk::Application) {
        use gdk::keys::constants as key;
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let none = gdk::ModifierType::empty();
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(250);
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        editor.window.resize(1100, 800);
        let crop = Rect::new(0., 0., 1000., 700.);
        let mut doc = document::demo().unwrap();
        doc.select(Some(crop));
        editor.set_document(doc, "Bilddetails hervorheben");
        editor.mark_project_saved();
        editor.area.grab_focus();
        editor.key_input(key::m, none);
        settle();
        assert_eq!(editor.state.borrow().tool, Tool::Marquee);
        assert!(editor.selection_bar.is_mapped());
        assert!(!editor.copy_selection_layer.is_sensitive());
        let view = editor.state.borrow().view;
        let region = Rect::new(110., 90., 260., 170.);
        let point = |x, y| (view.x + x * view.scale, view.y + y * view.scale);
        let (x, y) = point(region.x, region.y);
        let (ex, ey) = point(region.right(), region.bottom());
        let id = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .content_id();
        editor.press(x, y, false);
        editor.release(ex, ey);
        assert_eq!(editor.state.borrow().pixel_selection, Some(region));
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            id
        );
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().selection(),
            Some(crop)
        );
        assert!(editor.copy_selection_layer.is_sensitive());
        editor.motion(x, y);
        assert_eq!(editor.cursor.get(), Some(gdk::CursorType::TopLeftCorner));
        editor.press_modified(ex + 10., ey + 10., false, true);
        editor.motion(ex + 30., ey + 30.);
        editor.key_input(key::Escape, none);
        assert_eq!(editor.state.borrow().pixel_selection, Some(region));
        // Select all is transient in this tool, and never clears the export crop.
        editor.key_input(key::a, ctrl);
        assert_eq!(editor.state.borrow().pixel_selection, Some(crop));
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            id
        );
        editor.press_modified(x, y, false, true);
        editor.release(ex, ey);
        editor.window.resize(1000, 700);
        settle();
        assert!(editor.window.allocated_width() <= 1080);
        assert!(editor.selection_bar.is_mapped());
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-pixel-selection.png").unwrap())
            .unwrap();
        editor.key_input(key::j, ctrl);
        settle();
        assert_eq!(editor.state.borrow().tool, Tool::Move);
        assert_eq!(editor.state.borrow().selected, Some(0));
        assert!(editor.image_effects.is_mapped());
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .bounds(),
            region
        );
        // Resize the copied detail using a real canvas grip, then decorate it.
        let view = editor.state.borrow().view;
        let x = view.x + region.right() * view.scale;
        let y = view.y + region.bottom() * view.scale;
        editor.press(x, y, false);
        editor.release(x + 90. * view.scale, y + 60. * view.scale);
        let enlarged = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .bounds();
        assert!(enlarged.w > region.w && enlarged.h > region.h);
        editor.nudge(280., 160., false);
        editor.border_enabled.set_active(true);
        editor.shadow_enabled.set_active(true);
        editor.border_width.set_value(4.);
        editor.shadow_blur.set_value(18.);
        editor.shadow_offset.set_value(12.);
        editor.shadow_opacity.set_value(35.);
        editor.apply_image_effects();
        let styled = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .clone();
        assert!(styled.effects.border && styled.effects.shadow);
        assert_eq!(styled.effects.border_width, 4.);
        assert_eq!(styled.effects.shadow_opacity, 0.35);
        settle();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-selection-effects.png").unwrap())
            .unwrap();
        // Real GTK image clipboard transport; the harness uses a private display.
        editor.area.grab_focus();
        editor.key_input(key::m, none);
        editor.state.borrow_mut().pixel_selection = Some(region);
        editor.refresh();
        editor.key_input(key::c, ctrl);
        editor.key_input(key::v, ctrl);
        settle();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            2
        );
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[1]
                .bounds(),
            region
        );
        assert_eq!(
            editor.state.borrow().document.as_ref().unwrap().selection(),
            Some(crop)
        );
        editor.history(false);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            1
        );
        editor.history(true);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("details.untushot");
        assert!(editor.save_project_to(&path));
        let loaded = Document::read_project(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(loaded.annotations()[0], styled);
        assert_eq!(loaded.selection(), Some(crop));
        assert_eq!(loaded.annotations().len(), 2);
        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        clipboard.set_text("Kein Bild");
        editor.area.grab_focus();
        editor.key_input(key::v, ctrl);
        settle();
        assert!(!editor.clipboard_pending.get());
        assert!(editor.status.text().contains("Kein PNG- oder JPEG-Bild"));
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .len(),
            2
        );
        // A late decoder result must never paste into a subsequently opened document.
        clipboard.set_image(&loaded.export().unwrap());
        editor.key_input(key::v, ctrl);
        editor.set_document(document::demo().unwrap(), "Neues Dokument");
        settle();
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .is_empty());
        editor.mark_project_saved();
        editor.window.close();
    }

    fn object_resize_handles(app: &gtk::Application) {
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        let mut doc = document::demo().unwrap();
        let original = Annotation {
            effects: Default::default(),
            tool: Tool::Rectangle,
            points: vec![Point::new(0., 0.), Point::new(240., 160.)],
            color: (1., 0., 0.),
            width: 4.,
            text: String::new(),
            label: "Acht Griffe".into(),
            visible: true,
            clip: None,
            font_size: None,
        };
        doc.add(original.clone());
        editor.set_document(doc, "Objektbearbeitung prüfen");
        editor.tools[0].set_active(true);
        let settle = || {
            let until = std::time::Instant::now() + Duration::from_millis(200);
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        for zoom in [0.5, 1.25] {
            {
                let mut s = editor.state.borrow_mut();
                s.zoom = Some(zoom);
                s.pan = Point::new(24., 18.);
            }
            editor.area.queue_draw();
            settle();
            for (point, handle) in selection::handles(original.bounds()) {
                editor
                    .layers
                    .select_row(editor.layers.row_at_index(0).as_ref());
                let view = editor.state.borrow().view;
                // Grab slightly outside the corner, including outside the image.
                let x = view.x + point.x * view.scale - 2.;
                let y = view.y + point.y * view.scale - 2.;
                let id = editor
                    .state
                    .borrow()
                    .document
                    .as_ref()
                    .unwrap()
                    .content_id();
                editor.motion(x, y);
                assert_eq!(
                    editor.cursor.get(),
                    Some(handle_cursor(Some(handle), gdk::CursorType::LeftPtr))
                );
                editor.press(x, y, false);
                assert_eq!(editor.state.borrow().drag.as_ref().unwrap().handle, handle);
                editor.release(x, y);
                assert_eq!(
                    editor
                        .state
                        .borrow()
                        .document
                        .as_ref()
                        .unwrap()
                        .content_id(),
                    id,
                    "clicking a grip must not change the object"
                );
                editor.press(x, y, false);
                editor.motion(x + 12., y + 8.);
                assert_eq!(
                    editor
                        .state
                        .borrow()
                        .document
                        .as_ref()
                        .unwrap()
                        .annotations()[0],
                    original,
                    "preview must not modify the document"
                );
                editor.cancel_action();
                editor.release(x + 12., y + 8.);
                assert_eq!(
                    editor
                        .state
                        .borrow()
                        .document
                        .as_ref()
                        .unwrap()
                        .content_id(),
                    id
                );
                editor.press(x, y, false);
                editor.motion(x + 6., y + 4.);
                editor.release(x + 12., y + 8.);
                let changed = editor
                    .state
                    .borrow()
                    .document
                    .as_ref()
                    .unwrap()
                    .annotations()[0]
                    .clone();
                assert_ne!(changed, original);
                editor.history(false);
                assert_eq!(
                    editor
                        .state
                        .borrow()
                        .document
                        .as_ref()
                        .unwrap()
                        .annotations()[0],
                    original,
                    "one undo restores the complete drag"
                );
                editor.history(true);
                assert_eq!(
                    editor
                        .state
                        .borrow()
                        .document
                        .as_ref()
                        .unwrap()
                        .annotations()[0],
                    changed
                );
                editor.history(false);
            }
        }
        editor
            .layers
            .select_row(editor.layers.row_at_index(0).as_ref());
        editor.motion(100., 100.);
        assert_eq!(editor.cursor.get(), Some(gdk::CursorType::Fleur));
        settle();
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            editor.window.allocated_width(),
            editor.window.allocated_height(),
        )
        .unwrap();
        editor.window.draw(&cairo::Context::new(&surface).unwrap());
        surface
            .write_to_png(&mut std::fs::File::create("target/gtk-object-handles.png").unwrap())
            .unwrap();
        editor.mark_project_saved();
        editor.window.close();
    }

    fn shortcut(window: &gtk::Window, key: gdk::keys::Key, modifiers: gdk::ModifierType) {
        use glib::translate::IntoGlib;
        let mut event = gdk::Event::new(gdk::EventType::KeyPress)
            .downcast::<gdk::EventKey>()
            .unwrap();
        event.as_mut().keyval = key.into_glib();
        event.as_mut().state = modifiers.bits();
        assert!(window.emit_by_name::<bool>("key-press-event", &[&*event]));
    }

    fn input_transitions(app: &gtk::Application) {
        use gdk::keys::constants as key;
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let settle = || {
            let deadline = std::time::Instant::now() + Duration::from_millis(150);
            while std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        editor.text(Point::new(150., 150.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Erste Ebene");
        editor.finish_text(true);
        let mut second = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .clone();
        second.text = "Zweite Ebene".into();
        second.points = vec![Point::new(900., 500.)];
        second.font_size = Some(46.);
        editor
            .state
            .borrow_mut()
            .document
            .as_mut()
            .unwrap()
            .add(second);
        editor.refresh();
        let sizes = || {
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .iter()
                .map(Annotation::text_size)
                .collect::<Vec<_>>()
        };
        editor.select_new_image(0, "Erste Ebene");
        editor.font_size.grab_focus();
        editor.font_size.set_text("57");
        editor.select_new_image(1, "Zweite Ebene");
        settle();
        assert_eq!(
            sizes(),
            vec![57., 46.],
            "pending size belongs to the previous selection"
        );

        editor.font_size.grab_focus();
        editor.font_size.set_text("63");
        let row = editor.layers.row_at_index(1).unwrap();
        editor.syncing.set(true);
        editor.layers.unselect_all();
        editor.syncing.set(false);
        editor.layers.select_row(Some(&row));
        settle();
        assert_eq!(sizes(), vec![57., 63.]);
        assert_eq!(editor.state.borrow().indices(), vec![0]);
        editor.font_size.grab_focus();
        editor.font_size.set_text("67");
        editor.tools[4].set_active(true);
        settle();
        assert_eq!(
            sizes(),
            vec![67., 63.],
            "tool switches commit input before clearing the selection"
        );

        editor.select_new_image(0, "Erste Ebene");
        editor.font_size.grab_focus();
        editor.font_size.set_text("71");
        editor.duplicate.emit_clicked();
        settle();
        assert_eq!(
            sizes(),
            vec![71., 63., 71.],
            "duplication includes the pending size in both original and copy"
        );
        editor.undo.emit_clicked();
        assert_eq!(sizes(), vec![71., 63.]);
        editor.undo.emit_clicked();
        assert_eq!(sizes(), vec![67., 63.]);

        editor.select_new_image(0, "Erste Ebene");
        editor.font_size.grab_focus();
        editor.font_size.set_text("73");
        editor.undo.emit_clicked();
        settle();
        assert_eq!(
            sizes(),
            vec![67., 63.],
            "toolbar Undo must undo pending size input, not remove a layer"
        );
        editor.redo.emit_clicked();
        assert_eq!(sizes(), vec![73., 63.]);
        editor.select_new_image(0, "Erste Ebene");
        editor.font_size.grab_focus();
        editor.font_size.set_text("75");
        shortcut(editor.window.upcast_ref(), key::z, ctrl);
        assert_eq!(sizes(), vec![73., 63.]);
        shortcut(editor.window.upcast_ref(), key::y, ctrl);
        assert_eq!(sizes(), vec![75., 63.]);

        editor.select_new_image(0, "Erste Ebene");
        editor.font_size.grab_focus();
        editor.font_size.set_text("79");
        let view = editor.state.borrow().view;
        editor.press(
            view.x + 920. * view.scale,
            view.y + 510. * view.scale,
            false,
        );
        editor.cancel_action();
        settle();
        assert_eq!(
            sizes(),
            vec![79., 63.],
            "a canvas click commits size before selecting another object"
        );
        let mut rectangle = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()[0]
            .clone();
        rectangle.tool = Tool::Rectangle;
        rectangle.points = vec![Point::new(400., 200.), Point::new(600., 400.)];
        rectangle.text.clear();
        rectangle.width = 4.;
        editor
            .state
            .borrow_mut()
            .document
            .as_mut()
            .unwrap()
            .add(rectangle);
        editor.select_new_image(2, "Rechteck auswählen");
        editor.width_picker.set_active(true);
        settle();
        editor.width.grab_focus();
        editor.width.set_text("19");
        shortcut(editor.window.upcast_ref(), key::z, ctrl);
        settle();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[2]
                .width,
            4.
        );
        assert!(!editor.width_picker.popover().unwrap().is_visible());
        shortcut(editor.window.upcast_ref(), key::y, ctrl);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[2]
                .width,
            19.
        );
        editor.mark_project_saved();
        editor.window.close();
    }

    fn layer_name_history(app: &gtk::Application) {
        use gdk::keys::constants as key;
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        let settle = || {
            let deadline = std::time::Instant::now() + Duration::from_millis(150);
            while std::time::Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let none = gdk::ModifierType::empty();
        let layers = || {
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .to_vec()
        };
        let revision = || {
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id()
        };
        editor.text(Point::new(100., 100.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Bestehender Text");
        editor.finish_text(true);
        let original = layers();
        editor.begin_layer_rename();
        editor.layer_name.set_text("Neuer Ebenenname");
        editor.undo.emit_clicked();
        settle();
        assert_eq!(
            layers(),
            original,
            "undoing a pending name must not remove the existing layer"
        );
        assert!(
            editor.redo.is_sensitive(),
            "the pending name must remain recoverable"
        );
        editor.redo.emit_clicked();
        assert_eq!(layers()[0].name(), "Neuer Ebenenname");

        editor.select_new_image(0, "Ebene auswählen");
        editor.font_size.grab_focus();
        editor.font_size.set_text("51");
        editor.begin_layer_rename();
        settle();
        assert!(
            editor.layer_rename_valid(),
            "pending size input must be committed before the name session starts"
        );
        assert_eq!(layers()[0].text_size(), 51.);
        editor.layer_name.set_text("Name nach Größenänderung");
        editor.undo.emit_clicked();
        assert_eq!(layers()[0].name(), "Neuer Ebenenname");
        assert_eq!(layers()[0].text_size(), 51.);
        editor.undo.emit_clicked();
        assert_eq!(layers()[0].text_size(), 28.);
        editor.redo.emit_clicked();
        assert_eq!(layers()[0].text_size(), 51.);

        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("Umbenennen.untushot");
        assert!(editor.save_project_to(&project));
        assert!(editor.open_path(&project));
        assert!(!editor.undo.is_sensitive());
        editor.select_new_image(0, "Ebene auswählen");
        editor.begin_layer_rename();
        assert!(
            editor.undo.is_sensitive(),
            "a name draft can be undone even after opening a saved project"
        );
        editor.layer_name.set_text("Per Tastatur umbenannt");
        shortcut(editor.window.upcast_ref(), key::z, ctrl);
        settle();
        assert!(editor.layer_rename.get().is_none());
        assert_eq!(layers()[0].name(), "Neuer Ebenenname");
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        shortcut(
            editor.window.upcast_ref(),
            key::z,
            ctrl | gdk::ModifierType::SHIFT_MASK,
        );
        assert_eq!(layers()[0].name(), "Per Tastatur umbenannt");
        assert_eq!(editor.storage_caption.text(), "Projekt: geändert");

        editor.select_new_image(0, "Ebene auswählen");
        editor.begin_layer_rename();
        let unchanged = layers();
        let unchanged_revision = revision();
        shortcut(editor.window.upcast_ref(), key::z, ctrl);
        assert_eq!(
            layers(),
            unchanged,
            "an unchanged name must not undo earlier edits"
        );
        assert_eq!(revision(), unchanged_revision);

        editor.history(false);
        editor.select_new_image(0, "Ebene auswählen");
        editor.begin_layer_rename();
        editor.layer_name.set_text("Noch nicht übernehmen");
        assert!(!editor.redo.is_sensitive());
        shortcut(editor.window.upcast_ref(), key::y, ctrl);
        assert!(editor.layer_rename_valid());
        assert_eq!(editor.layer_name.text(), "Noch nicht übernehmen");
        shortcut(editor.window.upcast_ref(), key::Escape, none);
        settle();
        assert_eq!(layers()[0].name(), "Neuer Ebenenname");
        assert!(
            editor.redo.is_sensitive(),
            "Escape preserves the existing redo branch"
        );
        editor.redo.emit_clicked();
        assert_eq!(layers()[0].name(), "Per Tastatur umbenannt");

        editor.select_new_image(0, "Ebene auswählen");
        editor.text(Point::default());
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Text vor dem Umbenennen übernehmen");
        editor.begin_layer_rename();
        assert!(
            editor.inline_edit.borrow().is_none(),
            "name and text must not share an unfinished editing session"
        );
        assert!(editor.layer_rename_valid());
        editor.layer_name.set_text("Name nach Textbearbeitung");
        editor.undo.emit_clicked();
        assert_eq!(layers()[0].name(), "Per Tastatur umbenannt");
        assert_eq!(layers()[0].text, "Text vor dem Umbenennen übernehmen");
        editor.undo.emit_clicked();
        assert_eq!(layers()[0].text, "Bestehender Text");
        editor.redo.emit_clicked();
        editor.redo.emit_clicked();
        assert_eq!(layers()[0].name(), "Name nach Textbearbeitung");
        assert_eq!(layers()[0].text, "Text vor dem Umbenennen übernehmen");
        editor.select_new_image(0, "Ebene auswählen");
        editor.begin_layer_rename();
        editor.layer_name.set_text("Veralteter Entwurf");
        let mut edited = layers()[0].clone();
        edited.color = (0.2, 0.3, 0.4);
        editor
            .state
            .borrow_mut()
            .document
            .as_mut()
            .unwrap()
            .replace(0, edited);
        let latest = layers();
        let latest_revision = revision();
        editor.undo.emit_clicked();
        settle();
        assert_eq!(
            layers(),
            latest,
            "discarding a stale name must not undo a newer document action"
        );
        assert_eq!(revision(), latest_revision);
        assert!(editor.layer_rename.get().is_none());
        editor.mark_project_saved();
        editor.window.close();
    }

    fn text_edit_history(app: &gtk::Application) {
        use gdk::keys::constants as key;
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let none = gdk::ModifierType::empty();
        let text_layer = || {
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .clone()
        };
        let content_id = || {
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id()
        };
        let buffer = editor.inline_input.buffer().unwrap();
        let draft = || {
            editor
                .inline_edit
                .borrow()
                .as_ref()
                .unwrap()
                .annotation
                .clone()
        };
        let marks = || {
            (
                buffer.iter_at_mark(&buffer.get_insert().unwrap()).offset(),
                buffer
                    .iter_at_mark(&buffer.selection_bound().unwrap())
                    .offset(),
            )
        };
        let insert = |text: &str| {
            buffer.begin_user_action();
            buffer.delete_selection(true, true);
            buffer.insert_at_cursor(text);
            buffer.end_user_action();
        };
        let replace = |text: &str| {
            buffer.select_range(&buffer.end_iter(), &buffer.start_iter());
            insert(text);
        };
        editor.text(Point::new(100., 100.));
        let empty_id = content_id();
        assert!(!editor.undo.is_sensitive());
        insert("Grüße");
        insert(" 🙂");
        assert_eq!(marks(), (7, 7));
        editor.undo.emit_clicked();
        assert_eq!(draft().text, "Grüße");
        assert_eq!(marks(), (5, 5));
        assert!(editor.redo.is_sensitive());
        editor.redo.emit_clicked();
        assert_eq!(draft().text, "Grüße 🙂");
        assert_eq!(marks(), (7, 7));

        // Replacement is one native GTK user action, even though it emits
        // both delete-range and insert-text. Preserve backwards selections.
        buffer.select_range(&buffer.iter_at_offset(2), &buffer.iter_at_offset(5));
        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        clipboard.set_text("X\n世界");
        buffer.paste_clipboard(&clipboard, None, true);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while draft().text != "GrX\n世界 🙂" {
            assert!(
                std::time::Instant::now() < deadline,
                "native paste did not finish"
            );
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(draft().text, "GrX\n世界 🙂");
        assert_eq!(marks(), (6, 6));
        shortcut(editor.window.upcast_ref(), key::z, ctrl);
        assert_eq!(draft().text, "Grüße 🙂");
        assert_eq!(marks(), (2, 5));
        shortcut(
            editor.window.upcast_ref(),
            key::z,
            ctrl | gdk::ModifierType::SHIFT_MASK,
        );
        assert_eq!(draft().text, "GrX\n世界 🙂");
        assert_eq!(marks(), (6, 6));
        buffer.place_cursor(&buffer.end_iter());
        assert!(buffer.backspace(&mut buffer.end_iter(), true, true));
        assert_eq!(draft().text, "GrX\n世界 ");
        editor.history(false);
        assert_eq!(draft().text, "GrX\n世界 🙂");
        assert_eq!(marks(), (8, 8));
        insert("!");
        assert!(
            !editor.redo.is_sensitive(),
            "new input must discard local redo"
        );
        shortcut(editor.window.upcast_ref(), key::y, ctrl);
        assert_eq!(draft().text, "GrX\n世界 🙂!");
        assert_eq!(
            content_id(),
            empty_id,
            "local undo must not edit the document"
        );
        for _ in 0..4 {
            editor.history(false);
        }
        assert_eq!(draft().text, "");
        assert!(!editor.undo.is_sensitive());
        editor.history(false);
        assert_eq!(content_id(), empty_id);
        assert!(
            editor.inline_edit.borrow().is_some(),
            "undo keeps the editor open"
        );
        editor.finish_text(false);

        editor.text(Point::new(100., 100.));
        insert("Ursprünglicher Text");
        editor.font_size.grab_focus();
        editor.font_size.set_text("57");
        let actions = editor.inline_box.children()[1]
            .clone()
            .downcast::<gtk::Box>()
            .unwrap();
        actions.children()[0]
            .clone()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        assert_eq!(
            text_layer().text_size(),
            57.,
            "Apply includes a still-pending size"
        );
        let original = text_layer();
        editor.history(false);
        assert!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .is_empty(),
            "the completed session is one document undo step"
        );
        editor.history(true);
        assert_eq!(text_layer(), original);

        editor.select_new_image(0, "Text auswählen");
        editor.text(Point::default());
        assert!(
            !editor.undo.is_sensitive(),
            "a fresh session has no local undo"
        );
        editor.history(false);
        editor.history(true);
        assert_eq!(
            text_layer(),
            original,
            "local history cannot traverse older document edits"
        );
        editor.font_size.grab_focus();
        editor.font_size.set_text("61");
        assert!(
            editor.undo.is_sensitive(),
            "a pending size alone enables local undo"
        );
        editor.undo.emit_clicked();
        assert_eq!(draft().text_size(), 57.);
        assert!(!editor.undo.is_sensitive());
        assert!(editor.redo.is_sensitive());
        editor.redo.emit_clicked();
        assert_eq!(draft().text_size(), 61.);
        editor.undo.emit_clicked();
        replace("Überarbeiteter Text");
        editor.font_size.grab_focus();
        editor.font_size.set_text("63");
        editor.undo.emit_clicked();
        assert_eq!(draft().text, "Überarbeiteter Text");
        assert_eq!(draft().text_size(), 57.);
        assert_eq!(text_layer(), original);
        editor.redo.emit_clicked();
        assert_eq!(draft().text_size(), 63.);
        editor.undo.emit_clicked();
        editor.undo.emit_clicked();
        assert_eq!(draft(), original);
        editor.redo.emit_clicked();
        editor.redo.emit_clicked();
        assert_eq!(draft().text, "Überarbeiteter Text");
        assert_eq!(draft().text_size(), 63.);
        let old_color = draft().color;
        editor.state.borrow_mut().color = (0.2, 0.5, 0.8);
        editor.change_style();
        editor.undo.emit_clicked();
        assert_eq!(draft().color, old_color);
        editor.redo.emit_clicked();
        assert_eq!(draft().color, (0.2, 0.5, 0.8));
        editor.finish_text(true);
        let revised = text_layer();
        editor.history(false);
        assert_eq!(text_layer(), original, "text and styles commit together");
        editor.history(true);
        assert_eq!(text_layer(), revised);

        editor.select_new_image(0, "Text auswählen");
        editor.text(Point::default());
        replace("Nicht übernehmen");
        editor.font_size.grab_focus();
        editor.font_size.set_text("77");
        let before_cancel = content_id();
        shortcut(editor.window.upcast_ref(), key::Escape, none);
        let deadline = std::time::Instant::now() + Duration::from_millis(150);
        while std::time::Instant::now() < deadline {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            text_layer(),
            revised,
            "Escape must not apply pending size on focus-out"
        );
        assert_eq!(content_id(), before_cancel);

        editor.history(false);
        editor.select_new_image(0, "Text auswählen");
        editor.text(Point::default());
        let before_redo = content_id();
        assert!(
            !editor.redo.is_sensitive(),
            "document redo is hidden inside a text session"
        );
        replace("Aktuelle Eingabe behalten");
        editor.key_input(key::y, ctrl);
        assert_eq!(draft().text, "Aktuelle Eingabe behalten");
        assert_eq!(content_id(), before_redo);
        editor.key_input(key::Escape, none);
        assert!(
            editor.redo.is_sensitive(),
            "cancel preserves the document redo branch"
        );
        editor.redo.emit_clicked();
        editor.select_new_image(0, "Text auswählen");
        let valid = text_layer();
        let valid_id = content_id();
        editor.text(Point::default());
        replace("");
        editor.history(false);
        assert_eq!(draft(), valid);
        editor.history(true);
        assert_eq!(draft().text, "");
        editor.finish_text(true);
        assert_eq!(
            text_layer(),
            valid,
            "empty drafts preserve the original layer"
        );
        assert_eq!(content_id(), valid_id);
        for invalid in ["kein Wert", "9999", "-5"] {
            editor.text(Point::default());
            editor.font_size.grab_focus();
            editor.font_size.set_text(invalid);
            editor.finish_text(true);
            assert_eq!(
                text_layer(),
                valid,
                "invalid font size must preserve the last valid value"
            );
            assert_eq!(content_id(), valid_id);
        }
        let mut shape = valid;
        shape.tool = Tool::Rectangle;
        shape.text.clear();
        shape.font_size = None;
        shape.width = 4.;
        shape.points = vec![Point::new(400., 200.), Point::new(600., 400.)];
        editor
            .state
            .borrow_mut()
            .document
            .as_mut()
            .unwrap()
            .add(shape);
        editor.select_new_image(1, "Rechteck auswählen");
        let stroke = || {
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[1]
                .width
        };
        for invalid in ["kein Wert", "999", "-5"] {
            editor.width.set_text(invalid);
            editor.width.update();
            assert_eq!(
                stroke(),
                4.,
                "invalid width must not become the minimum or maximum"
            );
        }
        crate::stroke::preset(&editor.width_picker, 12).emit_clicked();
        assert_eq!(stroke(), 12.);
        editor.undo.emit_clicked();
        assert_eq!(stroke(), 4.);
        editor.redo.emit_clicked();
        assert_eq!(stroke(), 12.);
        editor.mark_project_saved();
        editor.window.close();
    }

    fn shortcuts_and_image_switch(app: &gtk::Application) {
        use gdk::keys::constants as key;
        fn dismiss_error(editor: &Editor) {
            let dialog = editor
                .error_dialog
                .borrow()
                .clone()
                .expect("non-blocking error popup");
            assert!(dialog.is_visible());
            dialog.response(gtk::ResponseType::Close);
            assert!(editor.error_dialog.borrow().is_none());
        }
        fn answer(title: &'static str, callback: impl Fn(&gtk::Dialog) + 'static) {
            let existing: Vec<_> = gtk::Window::list_toplevels()
                .into_iter()
                .map(|w| w.as_ptr() as usize)
                .collect();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            glib::timeout_add_local(Duration::from_millis(20), move || {
                let dialog = gtk::Window::list_toplevels()
                    .into_iter()
                    .filter_map(|w| w.downcast::<gtk::Dialog>().ok())
                    .find(|d| {
                        !existing.contains(&(d.as_ptr() as usize))
                            && d.is_visible()
                            && d.title().as_deref() == Some(title)
                    });
                if let Some(dialog) = dialog {
                    callback(&dialog);
                    return glib::ControlFlow::Break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "dialog did not open: {title}"
                );
                glib::ControlFlow::Continue
            });
        }
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let temp = tempfile::tempdir().unwrap();
        let editor = build(app, Args::parse_from(["untushot", "--demo"]));
        editor.preferences.borrow_mut().export_directory = Some(temp.path().into());
        let output = temp.path().join("Saved with shortcut.png");
        editor.mark_exported(&output);
        editor.text(Point::new(200., 200.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Save while typing");
        assert!(editor
            .window
            .focused_widget()
            .unwrap()
            .is::<gtk::TextView>());
        shortcut(editor.window.upcast_ref(), key::s, ctrl);
        assert!(output.is_file());
        assert!(editor.inline_edit.borrow().is_none());
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()[0]
                .text,
            "Save while typing"
        );
        editor.begin_layer_rename();
        for focus in [
            editor.layer_name.clone().upcast::<gtk::Widget>(),
            editor.font_size.clone().upcast(),
        ] {
            focus.grab_focus();
            editor.state.borrow_mut().export_state = None;
            shortcut(editor.window.upcast_ref(), key::s, ctrl);
            assert_eq!(
                editor.state.borrow().export_state,
                editor
                    .state
                    .borrow()
                    .document
                    .as_ref()
                    .map(Document::content_id)
            );
        }
        // Open remains a document command when an input owns focus. Cancelling
        // its chooser preserves the current document and its export target.
        for field in 0..3 {
            match field {
                0 => {
                    editor.font_size.grab_focus();
                    editor.font_size.set_text("49");
                }
                1 => {
                    editor.text(Point::default());
                    editor
                        .inline_input
                        .buffer()
                        .unwrap()
                        .set_text("Text vor dem Öffnen");
                }
                _ => {
                    editor.begin_layer_rename();
                    editor.layer_name.set_text("Name vor dem Öffnen");
                }
            }
            answer("Bild oder Projekt öffnen", |dialog| {
                dialog.response(gtk::ResponseType::Cancel)
            });
            shortcut(editor.window.upcast_ref(), key::o, ctrl);
            assert!(editor.inline_edit.borrow().is_none());
            assert!(editor.layer_rename.get().is_none());
            assert_eq!(editor.export_path.borrow().as_ref(), Some(&output));
            let s = editor.state.borrow();
            let layers = s.document.as_ref().unwrap().annotations();
            assert_eq!(layers.len(), 1);
            assert_eq!(layers[0].text_size(), 49.);
            if field >= 1 {
                assert_eq!(layers[0].text, "Text vor dem Öffnen");
            }
            if field == 2 {
                assert_eq!(layers[0].name(), "Name vor dem Öffnen");
            }
        }
        // The visible export action commits in-progress text and leaves the
        // editable project unsaved, just like its keyboard/menu counterparts.
        editor.text(Point::new(200., 200.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Visible export action");
        editor.font_size.grab_focus();
        editor.font_size.set_text("57");
        editor.export_button.emit_clicked();
        assert!(editor.inline_edit.borrow().is_none());
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .last()
                .unwrap()
                .text_size(),
            57.,
            "clicking export must include a font size still being typed"
        );
        assert_eq!(
            crate::pixels::open(&output).unwrap().read_pixel_bytes(),
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .export()
                .unwrap()
                .read_pixel_bytes(),
            "PNG and the document marked as exported must contain the same pixels"
        );
        assert_eq!(
            editor.state.borrow().export_state,
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .map(Document::content_id)
        );
        assert_eq!(editor.storage_caption.text(), "Projekt: ungespeichert");
        assert!(editor.window.title().unwrap().contains("Bild gespeichert"));
        answer("Änderungen speichern", |d| {
            let message = d.clone().downcast::<gtk::MessageDialog>().unwrap();
            assert!(message
                .property::<String>("text")
                .contains("Bild gespeichert"));
            assert!(d.widget_for_response(gtk::ResponseType::Other(1)).is_none());
            d.response(gtk::ResponseType::Cancel);
        });
        assert!(!editor.can_discard());
        // Save As must open a chooser even with an existing PNG target; cancel
        // must preserve both the target and the current editable document.
        answer("Bild als PNG exportieren", |d| {
            d.response(gtk::ResponseType::Cancel)
        });
        shortcut(
            editor.window.upcast_ref(),
            key::s,
            ctrl | gdk::ModifierType::SHIFT_MASK,
        );
        assert_eq!(editor.export_path.borrow().as_ref(), Some(&output));
        let fresh = temp.path().join("Fresh.png");
        let destination = fresh.clone();
        answer("Bild als PNG exportieren", move |dialog| {
            let chooser = dialog.clone().downcast::<gtk::FileChooserDialog>().unwrap();
            chooser.set_current_folder(destination.parent().unwrap());
            chooser.set_current_name(destination.file_name().unwrap().to_str().unwrap());
            let destination = destination.clone();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            glib::timeout_add_local(Duration::from_millis(20), move || {
                if chooser.filename().as_ref() == Some(&destination) {
                    chooser.response(gtk::ResponseType::Accept);
                    return glib::ControlFlow::Break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "save chooser did not resolve the target"
                );
                glib::ControlFlow::Continue
            });
        });
        editor.font_size.grab_focus();
        editor.font_size.set_text("61");
        assert!(editor.save_as());
        assert_eq!(editor.export_path.borrow().as_ref(), Some(&fresh));
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .last()
                .unwrap()
                .text_size(),
            61.
        );
        assert_eq!(
            crate::pixels::open(&fresh).unwrap().read_pixel_bytes(),
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .export()
                .unwrap()
                .read_pixel_bytes()
        );
        editor.export_path.replace(Some(output.clone()));
        editor.font_size.grab_focus();
        editor.font_size.set_text("65");
        editor.copy();
        let copied = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD)
            .wait_for_image()
            .unwrap();
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .last()
                .unwrap()
                .text_size(),
            65.
        );
        assert_eq!(
            copied.read_pixel_bytes(),
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .export()
                .unwrap()
                .read_pixel_bytes()
        );
        assert_ne!(
            editor.state.borrow().export_state,
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .map(Document::content_id),
            "copying must not mark an older PNG as current"
        );
        assert!(editor.save());
        let next = temp.path().join("Next screenshot.png");
        save_png(&document::demo().unwrap().export().unwrap(), &next).unwrap();
        let current_id = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .content_id();
        answer("Änderungen speichern", |d| {
            answer("Bild als PNG exportieren", |d| {
                d.response(gtk::ResponseType::Cancel)
            });
            shortcut(d.upcast_ref(), key::s, gdk::ModifierType::CONTROL_MASK);
        });
        editor.export_path.replace(None);
        assert!(!editor.open_path(&next));
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            current_id
        );
        // Failed writes also leave the original document open.
        editor
            .export_path
            .replace(Some(temp.path().join("missing/failed.png")));
        answer("Änderungen speichern", |d| {
            d.response(gtk::ResponseType::Other(1));
        });
        assert!(!editor.open_path(&next));
        dismiss_error(&editor);
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .content_id(),
            current_id
        );
        // Ctrl+S in the modal switch dialog saves, then opens the chosen image.
        editor.export_path.replace(Some(output.clone()));
        answer("Änderungen speichern", |d| {
            shortcut(d.upcast_ref(), key::s, gdk::ModifierType::CONTROL_MASK);
        });
        assert!(editor.open_path(&next));
        assert!(editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .is_empty());
        assert!(editor.export_path.borrow().is_none());
        assert!(crate::pixels::open(&output).is_ok());
        // Project updates reuse only an explicitly saved/opened project target;
        // Save As cancellation and failed writes preserve the current document.
        assert!(editor.project_path.borrow().is_none());
        editor.text(Point::new(100., 100.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("First project annotation");
        editor.font_size.grab_focus();
        editor.font_size.set_text("63");
        let project_path = temp.path().join("Editable project.untushot");
        assert!(editor.save_project_to(&project_path));
        assert_eq!(
            open_document(&project_path).unwrap().0.annotations()[0].text_size(),
            63.
        );
        assert!(editor.inline_edit.borrow().is_none());
        editor.begin_layer_rename();
        assert_eq!(
            editor.storage_caption.text(),
            "Projekt: Name in Bearbeitung"
        );
        editor.key_input(key::Escape, gdk::ModifierType::empty());
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        editor.begin_layer_rename();
        editor.layer_name.set_text("Gespeicherter Ebenenname");
        assert!(editor.save_project());
        assert_eq!(
            open_document(&project_path).unwrap().0.annotations()[0].name(),
            "Gespeicherter Ebenenname"
        );
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        editor.text(Point::new(300., 300.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Second project annotation");
        editor.finish_text(true);
        shortcut(
            editor.window.upcast_ref(),
            key::s,
            ctrl | gdk::ModifierType::MOD1_MASK,
        );
        assert_eq!(
            open_document(&project_path).unwrap().0.annotations()[0].text,
            "Second project annotation"
        );
        assert!(!editor.state.borrow().dirty);
        assert!(editor.state.borrow().export_state.is_none());
        let saved = std::fs::read(&project_path).unwrap();
        answer("Bearbeitbares Projekt speichern", |d| {
            let chooser = d.clone().downcast::<gtk::FileChooserDialog>().unwrap();
            let warning = chooser
                .extra_widget()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap();
            assert!(warning.is_visible() && warning.text().contains("Originalbild"));
            d.response(gtk::ResponseType::Cancel)
        });
        shortcut(
            editor.window.upcast_ref(),
            key::s,
            ctrl | gdk::ModifierType::MOD1_MASK | gdk::ModifierType::SHIFT_MASK,
        );
        assert_eq!(editor.project_path.borrow().as_ref(), Some(&project_path));
        assert_eq!(std::fs::read(&project_path).unwrap(), saved);
        assert!(editor.open_path(&next));
        assert!(editor.project_path.borrow().is_none());
        assert!(editor.open_path(&project_path));
        assert_eq!(editor.project_path.borrow().as_ref(), Some(&project_path));
        editor.text(Point::new(400., 400.));
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Third project annotation");
        editor.finish_text(true);
        let damaged = temp.path().join("Damaged.untushot");
        std::fs::write(&damaged, b"Not an UntuShot project").unwrap();
        let before_open = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .to_vec();
        let before_id = editor
            .state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .content_id();
        for path in [temp.path().join("Missing.png"), damaged] {
            // An unreadable target must show only the error, not a save/discard
            // prompt for the current document before it can even be loaded.
            assert!(!editor.open_path(&path));
            dismiss_error(&editor);
            assert_eq!(
                editor
                    .state
                    .borrow()
                    .document
                    .as_ref()
                    .unwrap()
                    .content_id(),
                before_id
            );
            assert_eq!(
                editor
                    .state
                    .borrow()
                    .document
                    .as_ref()
                    .unwrap()
                    .annotations(),
                before_open
            );
            assert_eq!(editor.project_path.borrow().as_ref(), Some(&project_path));
            assert!(editor.state.borrow().dirty);
            assert_eq!(std::fs::read(&project_path).unwrap(), saved);
        }
        answer("Änderungen speichern", |d| {
            answer("Bearbeitbares Projekt speichern", |d| {
                d.response(gtk::ResponseType::Cancel)
            });
            shortcut(
                d.upcast_ref(),
                key::s,
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::MOD1_MASK
                    | gdk::ModifierType::SHIFT_MASK,
            );
        });
        assert!(!editor.can_discard());
        assert_eq!(std::fs::read(&project_path).unwrap(), saved);
        editor
            .project_path
            .replace(Some(temp.path().join("missing/project.untushot")));
        assert!(!editor.save_project());
        dismiss_error(&editor);
        assert!(editor.state.borrow().dirty);
        assert_eq!(std::fs::read(&project_path).unwrap(), saved);
        editor.project_path.replace(Some(project_path.clone()));
        editor.project_save_button.emit_clicked();
        assert_eq!(editor.storage_caption.text(), "Projekt: gespeichert");
        assert_eq!(
            open_document(&project_path).unwrap().0.annotations().len(),
            2
        );
        // With background operation enabled, W keeps the session; Q closes it
        // even from setup with a spin button focused.
        editor.session.replace(Some(
            recovery::Session::open(&temp.path().join("recovery"), false)
                .unwrap()
                .0,
        ));
        editor.set_resident(true);
        editor.close_to_tray.set_active(true);
        editor.text(Point::default());
        editor
            .inline_input
            .buffer()
            .unwrap()
            .set_text("Beim Ausblenden übernehmen");
        editor.font_size.grab_focus();
        editor.font_size.set_text("69");
        shortcut(editor.window.upcast_ref(), key::w, ctrl);
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        assert!(!editor.window.is_visible());
        assert!(!editor.is_closed());
        assert!(editor.inline_edit.borrow().is_none());
        assert_eq!(
            editor
                .state
                .borrow()
                .document
                .as_ref()
                .unwrap()
                .annotations()
                .last()
                .unwrap()
                .text_size(),
            69.
        );
        assert_eq!(
            open_document(&project_path)
                .unwrap()
                .0
                .annotations()
                .last()
                .unwrap()
                .text_size(),
            69.,
            "hiding must autosave the latest editable project"
        );
        editor.present();
        editor.show_setup();
        editor.setup.options.set_expanded(true);
        editor.setup.delay.grab_focus();
        shortcut(editor.window.upcast_ref(), key::q, ctrl);
        assert!(editor.is_closed());
        assert!(editor.session.borrow().is_none());
        // The test application has no run loop for quit(); remove the window.
        editor.set_resident(false);
        editor.window.close();
    }
    #[test]
    fn viewport_round_trip_with_zoom_and_letterbox() {
        let v = View {
            x: 20.0,
            y: 40.0,
            scale: 0.5,
        };
        let bounds = Rect::new(0.0, 0.0, 3840.0, 1080.0);
        assert_eq!(v.point(980.0, 290.0, bounds), Point::new(1920.0, 500.0));
        assert_eq!(v.point(-100.0, 2000.0, bounds), Point::new(0.0, 1080.0));
        assert!(!v.contains(0.0, 0.0, bounds));
        let crop = Rect::new(100., 200., 300., 400.);
        let v = View {
            x: -40.,
            y: -80.,
            scale: 0.5,
        };
        assert_eq!(v.point(10., 20., crop), Point::new(100., 200.));
        assert!(!v.contains(0., 0., crop));
        assert!(v.contains(11., 21., crop));
    }
    #[test]
    fn legacy_projects_and_recovery_remain_readable_after_rename() {
        let directory = tempfile::tempdir().unwrap();
        let old = directory.path().join("session.rustshot");
        let mut document = document::demo().unwrap();
        document.select(Some(Rect::new(100., 80., 300., 200.)));
        document
            .write_project(std::fs::File::create(&old).unwrap())
            .unwrap();
        let (loaded, project) = open_document(&old).unwrap();
        assert!(project);
        assert_eq!(loaded.selection(), document.selection());
        let (session, restored) = crate::recovery::Session::open(directory.path(), true).unwrap();
        assert_eq!(restored.unwrap().selection(), document.selection());
        drop(session);
        let new = directory.path().join("project.untushot");
        loaded
            .write_project(std::fs::File::create(&new).unwrap())
            .unwrap();
        assert!(open_document(&new).unwrap().1);
        assert!(suggested_filename("untushot").starts_with("UntuShot-"));
    }

    #[test]
    fn png_export_round_trips_and_replaces_confirmed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("screenshot.png");
        std::fs::write(&path, b"old").unwrap();
        let image = document::demo().unwrap().export().unwrap();
        save_png(&image, &path).unwrap();
        let decoded = crate::pixels::open(&path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2400, 1100));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
