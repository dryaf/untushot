//! Explicit clipboard import; no background observation of browser or clipboard.
use crate::{icons, pixels};
use gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub enum Outcome {
    Image(Pixbuf),
    Cancel,
    Quit,
}

/// Direct layer paste uses the same explicit PNG/JPEG transport as the preview,
/// with the layer decoder's size limits and decoding off the GTK thread.
pub fn request_layer(callback: impl FnOnce(anyhow::Result<Pixbuf>) + 'static) {
    let callback = Rc::new(RefCell::new(Some(callback)));
    let timed = callback.clone();
    glib::timeout_add_local_once(Duration::from_secs(15), move || {
        if let Some(done) = timed.borrow_mut().take() {
            done(Err(crate::i18n::anyhow!(
                "Die Zwischenablage antwortet nicht. Bitte erneut kopieren."
            )));
        }
    });
    request_layer_format(callback, false);
}

fn request_layer_format<F: FnOnce(anyhow::Result<Pixbuf>) + 'static>(
    callback: Rc<RefCell<Option<F>>>,
    jpeg: bool,
) {
    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).request_contents(
        &gdk::Atom::intern(if jpeg { "image/jpeg" } else { "image/png" }), move |_, selection| {
            if callback.borrow().is_none() { return; }
            let bytes = selection.data();
            if bytes.is_empty() {
                if !jpeg { request_layer_format(callback, true); }
                else if let Some(done) = callback.borrow_mut().take() {
                    done(Err(crate::i18n::anyhow!("Kein PNG- oder JPEG-Bild in der Zwischenablage · Zuerst das Bild selbst kopieren")));
                }
                return;
            }
            let (tx, rx) = futures_channel::oneshot::channel();
            std::thread::spawn(move || { let _ = tx.send(pixels::decode_layer(&bytes)); });
            glib::MainContext::default().spawn_local(async move {
                let decoded = rx.await;
                if let Some(done) = callback.borrow_mut().take() {
                    done(decoded.map_err(|_| crate::i18n::anyhow!("Das Bild konnte nicht geladen werden. Bitte erneut kopieren.")).and_then(|r| r).and_then(pixels::from_rgba));
                }
            });
        },
    );
}

struct Preview {
    dialog: gtk::Dialog,
    image: RefCell<Option<Pixbuf>>,
    view: gtk::Image,
    scroller: gtk::ScrolledWindow,
    status: gtk::Label,
    read: gtk::Button,
    original_size: gtk::ToggleButton,
    help: Option<gtk::Expander>,
    accept: gtk::Widget,
    pending: Cell<bool>,
    generation: Cell<u64>,
    closed: Cell<bool>,
}

impl Preview {
    fn new(parent: &gtk::ApplicationWindow, browser: bool) -> Rc<Self> {
        let dialog = gtk::Dialog::with_buttons(
            Some(if browser {
                crate::i18n::tr("Webseite übernehmen")
            } else {
                crate::i18n::tr("Bild aus Zwischenablage")
            }),
            Some(parent),
            gtk::DialogFlags::MODAL,
            &[
                (crate::i18n::tr("Abbrechen"), gtk::ResponseType::Cancel),
                (
                    crate::i18n::tr("Im Editor öffnen"),
                    gtk::ResponseType::Accept,
                ),
            ],
        );
        dialog.set_default_size(760, 640);
        dialog.set_widget_name("untushot");
        dialog.set_resizable(true);
        dialog.set_default_response(gtk::ResponseType::Accept);
        let accept = dialog
            .widget_for_response(gtk::ResponseType::Accept)
            .unwrap();
        accept.style_context().add_class("suggested-action");
        accept.set_sensitive(false);
        let content = dialog.content_area();
        content.set_spacing(12);
        content.set_border_width(18);
        let heading = gtk::Label::new(Some(if browser {
            crate::i18n::tr("Ganze Webseite aus deinem Browser")
        } else {
            crate::i18n::tr("Vorschau prüfen und übernehmen")
        }));
        heading.set_xalign(0.);
        heading.style_context().add_class("card-title");
        content.pack_start(&heading, false, false, 0);
        let instructions = gtk::Label::new(Some(if browser {
            crate::i18n::tr("1. In Firefox einen Rechtsklick auf eine freie Stelle der Seite machen und „Bildschirmfoto aufnehmen“ wählen.\n2. Die gesamte Seite auswählen und das Bild kopieren.\n3. Hier „Bild einfügen“ anklicken und die Vorschau prüfen.")
        } else {
            crate::i18n::tr("Füge ein kopiertes Bild ein. Dein bisheriges Bild bleibt bis zum Übernehmen geöffnet.")
        }));
        instructions.set_xalign(0.);
        instructions.set_line_wrap(true);
        instructions.set_max_width_chars(76);
        let guide = gtk::Box::new(gtk::Orientation::Vertical, 8);
        guide.pack_start(&instructions, false, false, 0);
        if browser {
            let hint = gtk::Label::new(Some(crate::i18n::tr("Nachladende Inhalte vorher im Browser laden. Der Browser erstellt die Aufnahme; UntuShot übernimmt das fertige Bild.")));
            hint.set_xalign(0.);
            hint.set_line_wrap(true);
            hint.set_max_width_chars(76);
            hint.style_context().add_class("muted");
            guide.pack_start(&hint, false, false, 0);
        }
        let help = if browser {
            let help = gtk::Expander::new(Some(crate::i18n::tr("Anleitung für Firefox")));
            help.add(&guide);
            help.set_expanded(true);
            content.pack_start(&help, false, false, 0);
            Some(help)
        } else {
            content.pack_start(&guide, false, false, 0);
            None
        };
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let read = gtk::Button::new();
        read.add(&icons::button_content(
            "paste",
            crate::i18n::tr("Bild einfügen"),
        ));
        read.set_tooltip_text(Some(crate::i18n::tr(
            "Bild aus der Zwischenablage laden · Strg+V",
        )));
        controls.pack_start(&read, false, false, 0);
        let original_size = gtk::ToggleButton::with_label("100 %");
        original_size.set_tooltip_text(Some(crate::i18n::tr("Originalgröße prüfen; ausgeschaltet passt das Bild in die Breite. Lange Seiten lassen sich vertikal scrollen.")));
        original_size.set_sensitive(false);
        controls.pack_end(&original_size, false, false, 0);
        content.pack_start(&controls, false, false, 0);
        let scroller = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scroller.set_min_content_height(160);
        scroller.set_hexpand(true);
        scroller.set_vexpand(true);
        scroller.set_shadow_type(gtk::ShadowType::In);
        let view =
            gtk::Image::from_icon_name(Some("image-x-generic-symbolic"), gtk::IconSize::Dialog);
        view.set_halign(gtk::Align::Center);
        view.set_valign(gtk::Align::Start);
        scroller.add(&view);
        content.pack_start(&scroller, true, true, 0);
        let status = gtk::Label::new(Some(crate::i18n::tr("Noch kein Bild eingefügt.")));
        status.set_xalign(0.);
        status.set_line_wrap(true);
        status.set_max_width_chars(76);
        status.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
        content.pack_start(&status, false, false, 0);
        let preview = Rc::new(Self {
            dialog,
            image: RefCell::new(None),
            view,
            scroller,
            status,
            read,
            original_size,
            help,
            accept,
            pending: Cell::new(false),
            generation: Cell::new(0),
            closed: Cell::new(false),
        });
        let weak = Rc::downgrade(&preview);
        preview.read.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.load();
            }
        });
        let weak = Rc::downgrade(&preview);
        preview.original_size.connect_toggled(move |_| {
            if let Some(p) = weak.upgrade() {
                p.resize();
            }
        });
        let weak = Rc::downgrade(&preview);
        preview.scroller.connect_size_allocate(move |_, _| {
            if let Some(p) = weak.upgrade() {
                p.resize();
            }
        });
        let weak = Rc::downgrade(&preview);
        preview
            .dialog
            .connect_key_press_event(move |dialog, event| {
                use gdk::keys::constants as key;
                if event.state().contains(gdk::ModifierType::CONTROL_MASK) {
                    match event.keyval() {
                        key::v | key::V => {
                            if let Some(p) = weak.upgrade() {
                                p.load();
                            }
                        }
                        key::w | key::W => dialog.response(gtk::ResponseType::Cancel),
                        key::q | key::Q => dialog.response(gtk::ResponseType::Other(1)),
                        _ => return glib::Propagation::Proceed,
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        preview
    }

    fn current(&self, generation: u64) -> bool {
        !self.closed.get() && self.pending.get() && self.generation.get() == generation
    }
    fn load(self: &Rc<Self>) {
        if self.pending.replace(true) {
            return;
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        let generation = self.generation.get();
        self.accept.set_sensitive(false);
        self.read.set_sensitive(false);
        self.original_size.set_sensitive(false);
        self.status.style_context().remove_class("error");
        self.status.set_text(crate::i18n::tr("Bild wird geladen …"));
        self.request(generation, false);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(15), move || {
            if let Some(p) = weak.upgrade() {
                if p.current(generation) {
                    p.fail(crate::i18n::tr("Die Zwischenablage antwortet nicht. Kopiere das Bild erneut und versuche es noch einmal."));
                }
            }
        });
    }
    fn request(self: &Rc<Self>, generation: u64, jpeg: bool) {
        let weak = Rc::downgrade(self);
        gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).request_contents(
            &gdk::Atom::intern(if jpeg { "image/jpeg" } else { "image/png" }), move |_, selection| {
                let Some(p) = weak.upgrade().filter(|p| p.current(generation)) else { return; };
                let bytes = selection.data();
                if bytes.is_empty() {
                    if !jpeg { p.request(generation, true); }
                    else { p.fail(crate::i18n::tr("Kein PNG- oder JPEG-Bild in der Zwischenablage. Bitte das Bild selbst kopieren, nicht seine Adresse.")); }
                    return;
                }
                p.decode(generation, bytes);
            },
        );
    }
    fn decode(self: &Rc<Self>, generation: u64, bytes: Vec<u8>) {
        let (tx, rx) = futures_channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(pixels::decode_import(&bytes));
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = rx.await;
            let Some(p) = weak.upgrade().filter(|p| p.current(generation)) else {
                return;
            };
            match result {
                Ok(Ok(rgba)) => match pixels::from_rgba(rgba) {
                    Ok(image) => p.show_image(image),
                    Err(e) => p.fail(&crate::i18n::format!("{e:#}")),
                },
                Ok(Err(e)) => p.fail(&crate::i18n::format!("{e:#}")),
                Err(_) => p.fail(crate::i18n::tr(
                    "Das Bild konnte nicht geladen werden. Bitte erneut kopieren.",
                )),
            }
        });
    }
    fn fail(&self, message: &str) {
        if let Some(help) = &self.help {
            help.set_expanded(true);
        }
        self.pending.set(false);
        self.image.borrow_mut().take();
        self.view.clear();
        self.accept.set_sensitive(false);
        self.original_size.set_sensitive(false);
        self.read.set_sensitive(true);
        self.status.style_context().add_class("error");
        self.status.set_text(message);
    }
    fn show_image(&self, image: Pixbuf) {
        if let Some(help) = &self.help {
            help.set_expanded(false);
        }
        self.pending.set(false);
        self.status.style_context().remove_class("error");
        self.status.set_text(&crate::i18n::format!(
            "{} × {} px · Vorschau scrollen, um auch das Seitenende zu prüfen.",
            image.width(),
            image.height()
        ));
        self.image.replace(Some(image));
        self.view.clear();
        self.accept.set_sensitive(true);
        self.read.set_sensitive(true);
        self.original_size.set_sensitive(true);
        self.scroller.vadjustment().set_value(0.);
        self.scroller.hadjustment().set_value(0.);
        self.resize();
        if self.dialog.is_active() {
            self.accept.grab_focus();
        }
    }
    fn resize(&self) {
        let image = self.image.borrow();
        let Some(image) = image.as_ref() else {
            return;
        };
        let width = if self.original_size.is_active() {
            image.width()
        } else {
            image
                .width()
                .min((self.scroller.allocated_width() - 24).max(1))
        };
        let height =
            (i64::from(image.height()) * i64::from(width) / i64::from(image.width())).max(1) as i32;
        if self
            .view
            .pixbuf()
            .is_some_and(|p| p.width() == width && p.height() == height)
        {
            return;
        }
        let scaled = if width == image.width() {
            Some(image.clone())
        } else {
            image.scale_simple(width, height, gdk_pixbuf::InterpType::Bilinear)
        };
        self.view.set_from_pixbuf(scaled.as_ref());
    }
    fn finish(&self, response: gtk::ResponseType) -> Outcome {
        self.closed.set(true);
        self.dialog.hide();
        self.dialog.close();
        if response == gtk::ResponseType::Other(1) {
            return Outcome::Quit;
        }
        if response == gtk::ResponseType::Accept && !self.pending.get() {
            if let Some(image) = self.image.borrow_mut().take() {
                return Outcome::Image(image);
            }
        }
        Outcome::Cancel
    }
}

pub fn run(parent: &gtk::ApplicationWindow, browser: bool) -> Outcome {
    let preview = Preview::new(parent, browser);
    preview.dialog.show_all();
    if !browser {
        preview.load();
    }
    let response = preview.dialog.run();
    preview.finish(response)
}

/// Run by the existing GTK smoke test. It never reads or writes the user's clipboard.
#[cfg(test)]
pub(crate) fn gtk_smoke(parent: &gtk::ApplicationWindow) -> Pixbuf {
    use image::ImageEncoder;
    let rgba = image::RgbaImage::from_fn(900, 2400, |x, y| {
        image::Rgba(if y < 180 {
            [36, 43, 54, 255]
        } else if y > 2280 {
            [30, 100, 200, 255]
        } else if (80..820).contains(&x) && y % 450 < 300 {
            [210, 220, 230, 255]
        } else {
            [248, 249, 250, 255]
        })
    });
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, 900, 2400, image::ColorType::Rgba8)
        .unwrap();
    let preview = Preview::new(parent, true);
    preview.dialog.show_all();
    preview.pending.set(true);
    preview.generation.set(1);
    preview.decode(1, png);
    let context = glib::MainContext::default();
    let settle = || {
        let until = std::time::Instant::now() + Duration::from_millis(200);
        while std::time::Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while preview.pending.get() {
        assert!(std::time::Instant::now() < deadline);
        settle();
    }
    settle();
    assert!(preview.accept.is_sensitive(), "{}", preview.status.text());
    assert!(preview.view.pixbuf().unwrap().width() < 900);
    assert!(preview.scroller.vadjustment().upper() > preview.scroller.vadjustment().page_size());
    let surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        preview.dialog.allocated_width(),
        preview.dialog.allocated_height(),
    )
    .unwrap();
    preview.dialog.draw(&cairo::Context::new(&surface).unwrap());
    surface
        .write_to_png(&mut std::fs::File::create("target/gtk-web-import.png").unwrap())
        .unwrap();
    preview.original_size.set_active(true);
    settle();
    assert_eq!(preview.view.pixbuf().unwrap().width(), 900);
    assert_eq!(preview.view.pixbuf().unwrap().height(), 2400);
    preview.scroller.vadjustment().set_value(2400.);
    settle();
    assert!(preview.scroller.vadjustment().value() > 0.);
    // A replacement of identical dimensions must refresh pixels, not reuse the
    // previous thumbnail merely because the size is unchanged.
    let replacement = pixels::from_rgba(image::RgbaImage::from_pixel(
        900,
        2400,
        image::Rgba([20, 180, 70, 255]),
    ))
    .unwrap();
    preview.show_image(replacement);
    assert_eq!(
        &preview.view.pixbuf().unwrap().read_pixel_bytes().as_ref()[..4],
        &[20, 180, 70, 255]
    );
    assert!(matches!(
        preview.finish(gtk::ResponseType::Cancel),
        Outcome::Cancel
    ));
    assert!(!preview.current(1));
    let accept = Preview::new(parent, true);
    accept.show_image(pixels::from_rgba(rgba).unwrap());
    let image = match accept.finish(gtk::ResponseType::Accept) {
        Outcome::Image(image) => image,
        _ => panic!("missing image"),
    };
    let failed = Preview::new(parent, false);
    failed.show_image(image.clone());
    failed.pending.set(true);
    failed.fail("Test: kein Bild");
    assert!(!failed.accept.is_sensitive());
    assert!(matches!(
        failed.finish(gtk::ResponseType::Accept),
        Outcome::Cancel
    ));
    let cancelled = Preview::new(parent, false);
    cancelled.pending.set(true);
    cancelled.generation.set(7);
    let mut encoded = Vec::new();
    pixels::write_png(&image, &mut encoded).unwrap();
    cancelled.decode(7, encoded);
    assert!(matches!(
        cancelled.finish(gtk::ResponseType::Cancel),
        Outcome::Cancel
    ));
    settle();
    assert!(cancelled.image.borrow().is_none());
    let quit = Preview::new(parent, false);
    assert!(matches!(
        quit.finish(gtk::ResponseType::Other(1)),
        Outcome::Quit
    ));
    image
}

#[cfg(test)]
pub(crate) fn record_preview(
    parent: &gtk::ApplicationWindow,
    image: &Pixbuf,
    mut frame: impl FnMut(&gtk::Dialog, &str),
) -> Pixbuf {
    // Clipboard belongs to the private Broadway display used by the recorder.
    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_image(image);
    let preview = Preview::new(parent, true);
    preview.dialog.show_all();
    frame(
        &preview.dialog,
        "Copy a full-page image in your browser, then paste it here",
    );
    preview.read.emit_clicked();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while preview.pending.get() {
        assert!(std::time::Instant::now() < deadline);
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(preview.image.borrow().is_some());
    frame(
        &preview.dialog,
        "Fit to width · Preview the complete imported image",
    );
    preview.original_size.set_active(true);
    frame(&preview.dialog, "100% · Inspect the original pixels");
    preview.original_size.set_active(false);
    for i in 1..=6 {
        let a = preview.scroller.vadjustment();
        a.set_value((a.upper() - a.page_size()) * i as f64 / 6.);
        frame(
            &preview.dialog,
            "Scroll to the bottom · Check that nothing is missing",
        );
    }
    match preview.finish(gtk::ResponseType::Accept) {
        Outcome::Image(image) => image,
        _ => panic!("preview not accepted"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a private Broadway display; never run against the desktop clipboard"]
    fn clipboard_transport_isolated() {
        assert_eq!(std::env::var("GDK_BACKEND").as_deref(), Ok("broadway"));
        assert!(std::env::var("XDG_RUNTIME_DIR")
            .unwrap()
            .starts_with("/tmp/untushot-clipboard-"));
        gtk::init().unwrap();
        assert_eq!(
            gdk::Display::default().unwrap().type_().name(),
            "GdkBroadwayDisplay"
        );
        let app = gtk::Application::new(
            Some("io.github.untushot.ClipboardTest"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gio::Cancellable::NONE).unwrap();
        let window = gtk::ApplicationWindow::new(&app);
        window.show_all();
        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        let image = pixels::from_rgba(image::RgbaImage::from_pixel(
            64,
            1024,
            image::Rgba([30, 100, 200, 255]),
        ))
        .unwrap();
        clipboard.set_image(&image);
        let wait = |preview: &Preview| {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            while preview.pending.get() {
                assert!(std::time::Instant::now() < deadline);
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let preview = Preview::new(&window, false);
        preview.dialog.show_all();
        preview.load();
        wait(&preview);
        assert!(preview.accept.is_sensitive(), "{}", preview.status.text());
        match preview.finish(gtk::ResponseType::Accept) {
            Outcome::Image(p) => assert_eq!((p.width(), p.height()), (64, 1024)),
            _ => panic!("clipboard image was lost"),
        }
        clipboard.set_text("Not an image");
        let empty = Preview::new(&window, false);
        empty.dialog.show_all();
        empty.load();
        wait(&empty);
        assert!(!empty.accept.is_sensitive());
        assert!(empty.status.text().contains("Kein PNG- oder JPEG-Bild"));
        empty.finish(gtk::ResponseType::Cancel);
        clipboard.clear();
        window.close();
    }
}
