//! Screenshot folder strip with one cancellable background decoder and bounded,
//! private thumbnail cache. GTK widgets/pixbufs are created only on the UI thread.
use anyhow::Result;
use futures_util::StreamExt;
use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::{hash_map::DefaultHasher, HashMap},
    fs,
    hash::{Hash, Hasher},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, UNIX_EPOCH},
};
const ITEMS: usize = 60;
const CACHE_ITEMS: usize = 256;

pub fn directory() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Pictures)
        .filter(|p| p.is_absolute() && p != &glib::home_dir())
        .unwrap_or_else(|| glib::home_dir().join("Pictures"))
        .join("Screenshots")
}
fn cache_directory() -> PathBuf {
    glib::user_cache_dir().join("untushot/thumbnails-v1")
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Stamp {
    path: PathBuf,
    bytes: u64,
    modified: u128,
}
impl Stamp {
    fn read(path: PathBuf) -> Result<Self> {
        let m = fs::metadata(&path)?;
        Ok(Self {
            path,
            bytes: m.len(),
            modified: m
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        })
    }
}
fn scan(directory: &Path) -> Result<Vec<Stamp>> {
    let entries = match fs::read_dir(directory) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            e.path().extension().is_some_and(|s| {
                matches!(
                    s.to_string_lossy().to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg"
                )
            })
        })
        .filter_map(|e| Stamp::read(e.path()).ok())
        .collect();
    files.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    files.truncate(ITEMS);
    Ok(files)
}
struct Thumbnail {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}
fn cache_paths(cache: &Path, stamp: &Stamp) -> (PathBuf, PathBuf) {
    let mut hash = DefaultHasher::new();
    stamp.hash(&mut hash);
    let stem = crate::i18n::format!("{:016x}", hash.finish());
    (
        cache.join(crate::i18n::format!("{stem}.png")),
        cache.join(crate::i18n::format!("{stem}.json")),
    )
}
fn thumbnail(stamp: &Stamp, cache: &Path) -> Result<Thumbnail> {
    let (png, meta) = cache_paths(cache, stamp);
    let cached = fs::read(&meta)
        .ok()
        .and_then(|b| serde_json::from_slice::<Stamp>(&b).ok())
        .filter(|s| s == stamp)
        .and_then(|_| {
            let reader = image::io::Reader::open(&png).ok()?;
            let (w, h) = reader.into_dimensions().ok()?;
            if w > 160 || h > 96 {
                return None;
            }
            image::open(&png).ok()
        });
    let image = if let Some(image) = cached {
        image
    } else {
        let mut reader = image::io::Reader::open(&stamp.path)?.with_guessed_format()?;
        let mut limits = image::io::Limits::default();
        limits.max_alloc = Some(256 * 1024 * 1024);
        limits.max_image_width = Some(32768);
        limits.max_image_height = Some(32768);
        reader.limits(limits);
        let image = reader.decode()?.thumbnail(160, 96);
        crate::i18n::ensure!(
            Stamp::read(stamp.path.clone())? == *stamp,
            "Datei wurde während des Lesens geändert"
        );
        // Cache failure must not prevent opening/previewing a readable screenshot.
        let _ = (|| -> Result<()> {
            fs::create_dir_all(cache)?;
            fs::set_permissions(cache, fs::Permissions::from_mode(0o700))?;
            let mut temp = tempfile::NamedTempFile::new_in(cache)?;
            image.write_to(&mut temp, image::ImageOutputFormat::Png)?;
            temp.persist(&png)?;
            let temp = tempfile::NamedTempFile::new_in(cache)?;
            serde_json::to_writer(temp.as_file(), stamp)?;
            temp.persist(&meta)?;
            Ok(())
        })();
        image
    };
    let image = image.into_rgba8();
    Ok(Thumbnail {
        width: image.width(),
        height: image.height(),
        pixels: image.into_raw(),
    })
}
fn prune(cache: &Path) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .map(|e| {
            (
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH),
                e.path(),
            )
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(CACHE_ITEMS) {
        let _ = fs::remove_file(path.with_extension("json"));
        let _ = fs::remove_file(path);
    }
}
enum Event {
    List(Vec<Stamp>),
    Image(usize, Result<Thumbnail, String>),
    Done,
    Error(String),
}
type OpenCallback = Box<dyn Fn(&Path)>;
pub struct Strip {
    pub root: gtk::Box,
    pub toggle: gtk::ToggleButton,
    row: gtk::Box,
    scroller: gtk::ScrolledWindow,
    caption: gtk::Label,
    directory: Option<PathBuf>,
    cache: PathBuf,
    manifest: RefCell<Vec<Stamp>>,
    images: RefCell<Vec<gtk::Image>>,
    busy: Cell<bool>,
    pending: Cell<bool>,
    stopped: Arc<AtomicBool>,
    open: RefCell<Option<OpenCallback>>,
    append: RefCell<Option<OpenCallback>>,
    can_append: Cell<bool>,
    available: Cell<bool>,
    scan_error: Cell<bool>,
    scroll_anchor: RefCell<Option<(PathBuf, f64)>>,
}
impl Strip {
    pub fn new(directory: Option<PathBuf>) -> Rc<Self> {
        Self::with_cache(directory, cache_directory())
    }
    fn with_cache(directory: Option<PathBuf>, cache: PathBuf) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_widget_name("history");
        root.set_border_width(8);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let toggle = gtk::ToggleButton::with_label(crate::i18n::tr("Screenshots"));
        toggle.set_active(true);
        toggle.style_context().add_class("flat");
        toggle.set_tooltip_text(Some(crate::i18n::tr("Screenshot-Verlauf ein-/ausklappen")));
        bar.pack_start(&toggle, false, false, 0);
        let caption = gtk::Label::new(Some(crate::i18n::tr("Noch keine Screenshots")));
        caption.set_xalign(0.);
        caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
        caption.style_context().add_class("muted");
        bar.pack_start(&caption, true, true, 0);
        let folder = gtk::Button::with_label(crate::i18n::tr("Ordner öffnen"));
        folder.style_context().add_class("flat");
        folder.set_sensitive(directory.is_some());
        bar.pack_end(&folder, false, false, 0);
        if let Some(path) = &directory {
            folder.set_tooltip_text(Some(&path.display().to_string()));
        }
        root.pack_start(&bar, false, false, 0);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let scroller = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        scroller.set_min_content_height(126);
        scroller.add(&row);
        root.pack_start(&scroller, false, false, 0);
        let strip = Rc::new(Self {
            root,
            toggle,
            row,
            scroller,
            caption,
            directory,
            cache,
            manifest: RefCell::new(Vec::new()),
            images: RefCell::new(Vec::new()),
            busy: Cell::new(false),
            pending: Cell::new(false),
            stopped: Arc::new(AtomicBool::new(false)),
            open: RefCell::new(None),
            append: RefCell::new(None),
            can_append: Cell::new(false),
            available: Cell::new(true),
            scan_error: Cell::new(false),
            scroll_anchor: RefCell::new(None),
        });
        let weak = Rc::downgrade(&strip);
        strip.row.connect_size_allocate(move |_, _| {
            let Some(s) = weak.upgrade() else {
                return;
            };
            if s.scroll_anchor.borrow().is_none() {
                return;
            }
            let weak = Rc::downgrade(&s);
            // The viewport updates its adjustment bounds around child allocation.
            // Restore only once those bounds and all tile positions are current.
            glib::idle_add_local_once(move || {
                if let Some(s) = weak.upgrade() {
                    s.restore_scroll_anchor();
                }
            });
        });
        let weak = Rc::downgrade(&strip);
        strip.toggle.connect_toggled(move |_| {
            if let Some(s) = weak.upgrade() {
                s.sync_visibility();
                if s.toggle.is_active() {
                    s.refresh();
                }
            }
        });
        let weak = Rc::downgrade(&strip);
        folder.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                if let Some(path) = &s.directory {
                    let result = (|| -> Result<()> {
                        fs::create_dir_all(path)?;
                        gio::AppInfo::launch_default_for_uri(
                            &gio::File::for_path(path).uri(),
                            gio::AppLaunchContext::NONE,
                        )?;
                        Ok(())
                    })();
                    if let Err(e) = result {
                        s.caption.set_text(&crate::i18n::format!("Ordner: {e}"));
                    }
                }
            }
        });
        let weak = Rc::downgrade(&strip);
        glib::timeout_add_local(Duration::from_secs(5), move || {
            let Some(s) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if s.stopped.load(Ordering::Relaxed) {
                return glib::ControlFlow::Break;
            }
            // An empty strip is hidden, but must discover the first new image.
            if s.available.get()
                && (s.toggle.is_active() || s.manifest.borrow().is_empty())
                && s.root.toplevel().is_some_and(|w| w.is_mapped())
            {
                s.refresh();
            }
            glib::ControlFlow::Continue
        });
        strip.root.show_all();
        strip.root.set_no_show_all(true);
        strip.sync_visibility();
        strip.refresh();
        strip
    }
    pub fn attach_to(&self, host: &gtk::Box) {
        if self.root.parent().as_ref() == Some(host.upcast_ref()) {
            return;
        }
        self.remember_scroll_anchor(&self.manifest.borrow());
        if let Some(parent) = self
            .root
            .parent()
            .and_then(|w| w.downcast::<gtk::Container>().ok())
        {
            parent.remove(&self.root);
        }
        host.pack_start(&self.root, false, false, 0);
        self.sync_visibility();
    }
    pub fn connect_append(&self, callback: impl Fn(&Path) + 'static) {
        *self.append.borrow_mut() = Some(Box::new(callback));
    }
    pub fn set_can_append(&self, available: bool) {
        self.can_append.set(available);
    }
    fn context_menu(self: &Rc<Self>, path: &Path) -> gtk::Menu {
        let menu = gtk::Menu::new();
        let open = gtk::MenuItem::with_label(crate::i18n::tr("Im Editor öffnen"));
        let append = gtk::MenuItem::with_label(crate::i18n::tr("Rechts an aktuelles Bild anfügen"));
        append.set_sensitive(self.can_append.get() && self.append.borrow().is_some());
        for (item, insert) in [(&open, false), (&append, true)] {
            let weak = Rc::downgrade(self);
            let path = path.to_path_buf();
            item.connect_activate(move |_| {
                if let Some(s) = weak.upgrade() {
                    let callback = if insert { &s.append } else { &s.open };
                    if insert && !s.can_append.get() {
                        return;
                    }
                    if let Some(callback) = callback.borrow().as_ref() {
                        callback(&path);
                    }
                }
            });
            menu.append(item);
        }
        menu.show_all();
        menu
    }
    pub fn connect_open(&self, callback: impl Fn(&Path) + 'static) {
        *self.open.borrow_mut() = Some(Box::new(callback));
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
    pub fn sync_visibility(&self) {
        self.root.set_visible(
            self.available.get() && (!self.manifest.borrow().is_empty() || self.scan_error.get()),
        );
        self.scroller
            .set_visible(self.toggle.is_active() && !self.manifest.borrow().is_empty());
    }
    pub fn set_available(&self, available: bool) {
        self.available.set(available);
        self.sync_visibility();
    }
    pub fn refresh(self: &Rc<Self>) {
        let Some(directory) = self.directory.clone() else {
            return;
        };
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        if self.busy.replace(true) {
            self.pending.set(true);
            return;
        }
        let previous = self.manifest.borrow().clone();
        let cache = self.cache.clone();
        let stop = self.stopped.clone();
        let (tx, mut rx) = futures_channel::mpsc::unbounded();
        std::thread::spawn(move || {
            match scan(&directory) {
                Ok(files) => {
                    if tx.unbounded_send(Event::List(files.clone())).is_err() {
                        return;
                    }
                    if files != previous {
                        for (i, stamp) in files.iter().enumerate() {
                            if stop.load(Ordering::Relaxed) {
                                return;
                            }
                            if previous.contains(stamp) {
                                continue;
                            }
                            let result =
                                thumbnail(stamp, &cache).map_err(|e| crate::i18n::format!("{e:#}"));
                            if tx.unbounded_send(Event::Image(i, result)).is_err() {
                                return;
                            }
                        }
                        prune(&cache);
                    }
                }
                Err(e) => {
                    let _ = tx.unbounded_send(Event::Error(crate::i18n::format!("{e:#}")));
                }
            }
            let _ = tx.unbounded_send(Event::Done);
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            while let Some(event) = rx.next().await {
                let Some(s) = weak.upgrade() else {
                    break;
                };
                if s.stopped.load(Ordering::Relaxed) {
                    break;
                }
                match event {
                    Event::List(files) => s.rebuild(files),
                    Event::Image(i, Ok(t)) => {
                        if let Some(image) = s.images.borrow().get(i) {
                            let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(
                                &glib::Bytes::from_owned(t.pixels),
                                gdk_pixbuf::Colorspace::Rgb,
                                true,
                                8,
                                t.width as i32,
                                t.height as i32,
                                t.width as i32 * 4,
                            );
                            image.set_from_pixbuf(Some(&pixbuf));
                        }
                    }
                    Event::Image(i, Err(e)) => {
                        if let Some(image) = s.images.borrow().get(i) {
                            image.set_from_icon_name(
                                Some("image-missing-symbolic"),
                                gtk::IconSize::Dialog,
                            );
                            image.set_tooltip_text(Some(&e));
                        }
                    }
                    Event::Error(e) => {
                        s.scan_error.set(true);
                        s.caption.set_text(crate::i18n::tr("Ordner nicht lesbar"));
                        s.caption.set_tooltip_text(Some(&e));
                        s.sync_visibility();
                    }
                    Event::Done => {
                        s.busy.set(false);
                        if s.pending.replace(false) {
                            s.refresh();
                        }
                    }
                }
            }
        });
    }
    fn rebuild(self: &Rc<Self>, files: Vec<Stamp>) {
        self.scan_error.set(false);
        self.caption.set_text(&if files.is_empty() {
            crate::i18n::tr("Noch keine Screenshots").into()
        } else {
            crate::i18n::format!("{} zuletzt geänderte Bilder", files.len())
        });
        self.caption.set_tooltip_text(
            self.directory
                .as_ref()
                .map(|p| p.to_string_lossy())
                .as_deref(),
        );
        if files == *self.manifest.borrow() {
            self.sync_visibility();
            return;
        }
        let children = self.row.children();
        self.remember_scroll_anchor(&files);
        let mut existing: HashMap<_, _> = self
            .manifest
            .borrow()
            .iter()
            .zip(children)
            .zip(self.images.borrow().iter())
            .map(|((stamp, widget), image)| {
                (
                    stamp.path.clone(),
                    (
                        stamp.clone(),
                        widget.downcast::<gtk::Button>().unwrap(),
                        image.clone(),
                    ),
                )
            })
            .collect();
        let mut images = Vec::with_capacity(files.len());
        for (index, stamp) in files.iter().enumerate() {
            let (button, image) = if let Some((old, button, image)) = existing.remove(&stamp.path) {
                if old != *stamp {
                    image.set_from_icon_name(
                        Some("image-x-generic-symbolic"),
                        gtk::IconSize::Dialog,
                    );
                    image.set_tooltip_text(None);
                }
                (button, image)
            } else {
                let (button, image) = self.create_tile(stamp);
                self.row.pack_start(&button, false, false, 0);
                (button, image)
            };
            self.row.reorder_child(&button, index as i32);
            images.push(image);
        }
        for (_, button, _) in existing.into_values() {
            self.row.remove(&button);
        }
        *self.images.borrow_mut() = images;
        *self.manifest.borrow_mut() = files;
        self.row.show_all();
        self.sync_visibility();
    }
    fn remember_scroll_anchor(&self, files: &[Stamp]) {
        let children = self.row.children();
        let scroll = self.scroller.hadjustment().value();
        if scroll > 0. && self.scroll_anchor.borrow().is_none() {
            let manifest = self.manifest.borrow();
            let anchor = manifest.iter().zip(&children).find_map(|(stamp, child)| {
                let x = child.translate_coordinates(&self.row, 0, 0)?.0;
                (f64::from(x + child.allocated_width()) > scroll
                    && files.iter().any(|f| f.path == stamp.path))
                .then(|| (stamp.path.clone(), f64::from(x) - scroll))
            });
            *self.scroll_anchor.borrow_mut() = anchor;
        }
    }
    fn restore_scroll_anchor(&self) {
        let Some((path, offset)) = self.scroll_anchor.borrow_mut().take() else {
            return;
        };
        let Some(index) = self.manifest.borrow().iter().position(|s| s.path == path) else {
            return;
        };
        let Some(x) = self
            .row
            .children()
            .get(index)
            .and_then(|child| child.translate_coordinates(&self.row, 0, 0))
            .map(|p| p.0)
        else {
            return;
        };
        let adjustment = self.scroller.hadjustment();
        adjustment.set_value((f64::from(x) - offset).clamp(
            adjustment.lower(),
            (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
        ));
    }
    fn create_tile(self: &Rc<Self>, stamp: &Stamp) -> (gtk::Button, gtk::Image) {
        let button = gtk::Button::new();
        button.style_context().add_class("flat");
        button.style_context().add_class("thumbnail");
        button.set_tooltip_text(Some(&stamp.path.display().to_string()));
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let image =
            gtk::Image::from_icon_name(Some("image-x-generic-symbolic"), gtk::IconSize::Dialog);
        image.set_size_request(160, 96);
        tile.pack_start(&image, false, false, 0);
        let label = gtk::Label::new(Some(
            &stamp.path.file_name().unwrap_or_default().to_string_lossy(),
        ));
        label.set_width_chars(19);
        label.set_max_width_chars(19);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        label.style_context().add_class("small");
        tile.pack_start(&label, false, false, 0);
        button.add(&tile);
        let path = stamp.path.clone();
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                if let Some(open) = s.open.borrow().as_ref() {
                    open(&path);
                }
            }
        });
        let weak = Rc::downgrade(self);
        let menu = self.context_menu(&stamp.path);
        menu.set_attach_widget(Some(&button));
        button.connect_button_press_event(move |_, event| {
            if event.button() == 3 {
                if let Some(s) = weak.upgrade() {
                    menu.children()[1]
                        .set_sensitive(s.can_append.get() && s.append.borrow().is_some());
                    menu.popup_at_pointer(Some(event));
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        (button, image)
    }
}
impl Drop for Strip {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
pub(crate) fn test_widget(parent: &gtk::Box, open: impl Fn(&Path) + 'static) {
    let directory = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    for name in ["Screenshot mit Leerzeichen.png", "Zweite Aufnahme.png"] {
        let image = crate::document::demo().unwrap().export().unwrap();
        crate::pixels::write_png(
            &image,
            fs::File::create(directory.path().join(name)).unwrap(),
        )
        .unwrap();
    }
    let strip = Strip::with_cache(Some(directory.path().into()), cache.path().into());
    parent.pack_start(&strip.root, false, false, 0);
    parent.reorder_child(&strip.root, 1);
    let opened = Rc::new(RefCell::new(None));
    let output = opened.clone();
    strip.connect_open(move |path| {
        *output.borrow_mut() = Some(path.to_path_buf());
        open(path);
    });
    let settle = || {
        let until = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            if !strip.busy.get() {
                break;
            }
            assert!(
                std::time::Instant::now() < until,
                "thumbnail worker timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    settle();
    assert_eq!(strip.manifest.borrow().len(), 2);
    assert!(strip.root.is_visible());
    assert!(strip.images.borrow().iter().all(|i| i.pixbuf().is_some()));
    strip.toggle.set_active(false);
    assert!(!strip.scroller.is_visible());
    strip.toggle.set_active(true);
    settle();
    let first = strip.manifest.borrow()[0].path.clone();
    let appended = Rc::new(RefCell::new(None));
    let output = appended.clone();
    strip.connect_append(move |path| *output.borrow_mut() = Some(path.to_path_buf()));
    assert!(!strip.context_menu(&first).children()[1].is_sensitive());
    strip.set_can_append(true);
    let menu = strip.context_menu(&first);
    menu.children()[1]
        .clone()
        .downcast::<gtk::MenuItem>()
        .unwrap()
        .activate();
    assert_eq!(appended.borrow().as_ref(), Some(&first));
    assert!(
        opened.borrow().is_none(),
        "anfügen darf das aktuelle Bild nicht ersetzen"
    );
    strip.set_can_append(false);
    *appended.borrow_mut() = None;
    menu.children()[1]
        .clone()
        .downcast::<gtk::MenuItem>()
        .unwrap()
        .activate();
    assert!(
        appended.borrow().is_none(),
        "ein älteres Menü darf den Aufnahmeschutz nicht umgehen"
    );

    strip.row.children()[0]
        .clone()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    assert_eq!(opened.borrow().as_ref(), Some(&first));
    let until = std::time::Instant::now() + Duration::from_millis(300);
    while std::time::Instant::now() < until {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let window = parent
        .toplevel()
        .unwrap()
        .downcast::<gtk::Window>()
        .unwrap();
    let surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        window.allocated_width(),
        window.allocated_height(),
    )
    .unwrap();
    window.draw(&cairo::Context::new(&surface).unwrap());
    surface
        .write_to_png(&mut fs::File::create("target/gtk-history.png").unwrap())
        .unwrap();
    fs::remove_file(first).unwrap();
    strip.refresh();
    settle();
    assert_eq!(strip.manifest.borrow().len(), 1);
    strip.set_available(false);
    strip.refresh();
    settle();
    assert!(
        !strip.root.is_visible(),
        "refresh must respect region selection"
    );
    strip.set_available(true);
    assert!(strip.root.is_visible());
    strip.toggle.set_active(false);
    let remaining = strip.manifest.borrow()[0].path.clone();
    fs::remove_file(&remaining).unwrap();
    strip.refresh();
    settle();
    assert!(!strip.root.is_visible());
    parent.show_all();
    assert!(
        !strip.root.is_visible(),
        "show_all must not reveal an empty strip"
    );
    // Hidden empty history still discovers new files through its timer.
    crate::pixels::write_png(
        &crate::document::demo().unwrap().export().unwrap(),
        fs::File::create(&remaining).unwrap(),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while !strip.root.is_visible() {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "empty history stopped polling"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    settle();
    assert_eq!(strip.manifest.borrow().len(), 1);
    assert!(
        !strip.scroller.is_visible(),
        "keep the collapsed preference"
    );
    strip.stop();
    parent.remove(&strip.root);
}

#[cfg(test)]
pub(crate) fn test_many_images(parent: &gtk::Box, inspect: impl Fn(&Strip)) {
    let directory = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let write = |path: &Path, time, color| {
        image::RgbaImage::from_pixel(80, 40, image::Rgba(color))
            .save(path)
            .unwrap();
        fs::File::open(path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(time))
            .unwrap();
    };
    for index in 0..ITEMS {
        write(
            &directory.path().join(format!(
                "Aufnahme-{index:02}-mit-einem-sehr-langen-Dateinamen.png"
            )),
            1000 + index as u64,
            [200, 40, 20, 255],
        );
    }
    let strip = Strip::with_cache(Some(directory.path().into()), cache.path().into());
    parent.pack_start(&strip.root, false, false, 0);
    parent.reorder_child(&strip.root, 1);
    let settle = || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut ready = None;
        loop {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            if !strip.busy.get() {
                if ready.get_or_insert_with(std::time::Instant::now).elapsed()
                    >= Duration::from_millis(150)
                {
                    break;
                }
            } else {
                ready = None;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "history did not settle"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let window = parent
        .toplevel()
        .unwrap()
        .downcast::<gtk::Window>()
        .unwrap();
    settle();
    window.resize(1000, 700);
    settle();
    assert_eq!(strip.manifest.borrow().len(), ITEMS);
    assert!(strip
        .images
        .borrow()
        .iter()
        .all(|image| image.pixbuf().is_some()));
    assert!(strip.scroller.allocated_width() < strip.row.allocated_width());
    assert!(strip.root.allocated_height() < 190);
    assert!(
        window.size().1 <= 700,
        "history must fit the compact editor, got {} with minimum {}",
        window.size().1,
        window.preferred_height().0
    );
    inspect(&strip);
    let anchor = strip.row.children()[20].clone();
    let anchor_path = strip.manifest.borrow()[20].path.clone();
    let anchor_pixels = strip.images.borrow()[20].pixbuf().unwrap();
    let x = anchor.translate_coordinates(&strip.row, 0, 0).unwrap().0;
    strip.scroller.hadjustment().set_value(f64::from(x + 13));
    settle();
    let position = anchor
        .translate_coordinates(&strip.scroller, 0, 0)
        .unwrap()
        .0;
    let check_anchor = || {
        let index = strip
            .manifest
            .borrow()
            .iter()
            .position(|s| s.path == anchor_path)
            .unwrap();
        assert_eq!(
            strip.row.children()[index],
            anchor,
            "refresh must preserve existing tiles"
        );
        assert_eq!(
            strip.images.borrow()[index].pixbuf().unwrap(),
            anchor_pixels,
            "unchanged thumbnails must not be decoded/replaced again"
        );
        let actual = anchor
            .translate_coordinates(&strip.scroller, 0, 0)
            .unwrap()
            .0;
        assert!(
            (actual - position).abs() <= 1,
            "visible thumbnail moved: {position} -> {actual}"
        );
    };
    write(
        &directory.path().join("Neue Aufnahme.png"),
        2000,
        [40, 200, 20, 255],
    );
    strip.refresh();
    settle();
    assert_eq!(strip.manifest.borrow().len(), ITEMS);
    check_anchor();
    let opened = Rc::new(RefCell::new(None));
    let result = opened.clone();
    strip.connect_open(move |path| *result.borrow_mut() = Some(path.to_path_buf()));
    anchor
        .clone()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    assert_eq!(opened.borrow().as_ref(), Some(&anchor_path));

    let changed_path = strip.manifest.borrow()[3].path.clone();
    let changed_tile = strip.row.children()[3].clone();
    let old_pixels = strip.images.borrow()[3].pixbuf().unwrap();
    write(&changed_path, 3000, [20, 40, 200, 255]);
    strip.refresh();
    settle();
    check_anchor();
    assert_eq!(strip.manifest.borrow()[0].path, changed_path);
    assert_eq!(strip.row.children()[0], changed_tile);
    let new_pixels = strip.images.borrow()[0].pixbuf().unwrap();
    assert_ne!(
        new_pixels, old_pixels,
        "modified files need a fresh thumbnail"
    );
    assert_eq!(
        &new_pixels.read_pixel_bytes().as_ref()[..4],
        &[20, 40, 200, 255]
    );
    let removed = strip.manifest.borrow()[1].path.clone();
    fs::remove_file(removed).unwrap();
    strip.refresh();
    settle();
    check_anchor();
    strip.toggle.set_active(false);
    settle();
    inspect(&strip);
    strip.toggle.set_active(true);
    settle();
    inspect(&strip);
    check_anchor();

    // The same strip moves between the editor canvas and the capture page.
    // Reparenting must preserve loaded previews, scroll position and collapse.
    let root = window.child().unwrap().downcast::<gtk::Box>().unwrap();
    let other_host = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.pack_start(&other_host, false, false, 0);
    other_host.show();
    strip.attach_to(&other_host);
    settle();
    check_anchor();
    strip.toggle.set_active(false);
    strip.attach_to(parent);
    settle();
    assert!(!strip.scroller.is_visible());
    strip.toggle.set_active(true);
    settle();
    check_anchor();
    root.remove(&other_host);
    window.resize(1000, 700);
    settle();

    let next_index = strip
        .manifest
        .borrow()
        .iter()
        .position(|s| s.path == anchor_path)
        .unwrap()
        + 1;
    let next = strip.row.children()[next_index].clone();
    let next_position = next.translate_coordinates(&strip.scroller, 0, 0).unwrap().0;
    fs::remove_file(&anchor_path).unwrap();
    strip.refresh();
    settle();
    assert!(!strip
        .manifest
        .borrow()
        .iter()
        .any(|s| s.path == anchor_path));
    assert!(
        (next.translate_coordinates(&strip.scroller, 0, 0).unwrap().0 - next_position).abs() <= 1,
        "when the first visible image disappears, keep its neighbor in place"
    );

    strip.scroller.hadjustment().set_value(0.);
    write(
        &directory.path().join("Noch eine Aufnahme.png"),
        4000,
        [40, 20, 200, 255],
    );
    strip.refresh();
    settle();
    assert_eq!(
        strip.scroller.hadjustment().value(),
        0.,
        "at the start, keep newest files visible"
    );
    let surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        window.allocated_width(),
        window.allocated_height(),
    )
    .unwrap();
    window.draw(&cairo::Context::new(&surface).unwrap());
    surface
        .write_to_png(&mut fs::File::create("target/gtk-history-many.png").unwrap())
        .unwrap();
    strip.stop();
    parent.remove(&strip.root);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_invalidation_corruption_and_scan() {
        let dir = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let path = dir.path().join("Bild mit Leerzeichen.PNG");
        image::RgbaImage::from_pixel(400, 200, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        fs::write(dir.path().join("ignore.txt"), "x").unwrap();
        let first = scan(dir.path()).unwrap();
        assert_eq!(first.len(), 1);
        let thumb = thumbnail(&first[0], cache.path()).unwrap();
        assert_eq!((thumb.width, thumb.height), (160, 80));
        let (png, meta) = cache_paths(cache.path(), &first[0]);
        assert!(meta.exists());
        assert_eq!(
            fs::metadata(&png).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::write(&png, b"corrupt").unwrap();
        assert_eq!(
            thumbnail(&first[0], cache.path()).unwrap().pixels,
            thumb.pixels
        );
        image::RgbaImage::from_pixel(100, 400, image::Rgba([0, 0, 255, 255]))
            .save(&path)
            .unwrap();
        let second = scan(dir.path()).unwrap();
        assert_ne!(first, second);
        let new = thumbnail(&second[0], cache.path()).unwrap();
        assert_eq!((new.width, new.height), (24, 96));
        assert!(scan(&dir.path().join("missing")).unwrap().is_empty());
        fs::remove_file(path).unwrap();
        assert!(scan(dir.path()).unwrap().is_empty());
    }
    #[test]
    fn cache_pruning_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..CACHE_ITEMS + 3 {
            fs::write(dir.path().join(format!("{i:016x}.png")), b"x").unwrap();
            fs::write(dir.path().join(format!("{i:016x}.json")), b"x").unwrap();
        }
        prune(dir.path());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), CACHE_ITEMS * 2);
    }
}
