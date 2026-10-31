//! One desktop process; CLI/hotkey invocations are forwarded by GApplication.
use crate::{capture, setup::Mode, tray, ui, Args, Backend};
use anyhow::{Context, Result};
use clap::Parser;
use glib::translate::*;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    ffi::CString,
    rc::Rc,
    time::Duration,
};

struct Runtime {
    app: gtk::Application,
    editor: RefCell<Option<Rc<ui::Editor>>>,
    indicator: RefCell<Option<tray::Indicator>>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    handling: Cell<bool>,
    backend: Cell<Backend>,
    status: gtk::MenuItem,
    actions: Vec<gtk::MenuItem>,
    cancel: gtk::MenuItem,
    menu: gtk::Menu,
}
impl Runtime {
    fn new(app: &gtk::Application) -> Rc<Self> {
        let menu = gtk::Menu::new();
        let status = gtk::MenuItem::with_label(crate::i18n::tr("Bereit · Strg+Shift+X"));
        status.set_sensitive(false);
        menu.append(&status);
        menu.append(&gtk::SeparatorMenuItem::new());
        let actions: Vec<_> = [
            crate::i18n::tr("Bereich auswählen"),
            crate::i18n::tr("Bereich auswählen · 3 Sekunden"),
            crate::i18n::tr("Bildschirm"),
            crate::i18n::tr("Fenster oder Bildschirm wählen"),
            crate::i18n::tr("Editor öffnen"),
        ]
        .into_iter()
        .map(|label| {
            let item = gtk::MenuItem::with_label(label);
            item.show_all();
            item.set_no_show_all(true);
            menu.append(&item);
            item
        })
        .collect();
        let cancel = gtk::MenuItem::with_label(crate::i18n::tr("Aufnahme abbrechen"));
        cancel.show_all();
        cancel.set_no_show_all(true);
        cancel.hide();
        cancel.set_sensitive(false);
        menu.append(&cancel);
        menu.append(&gtk::SeparatorMenuItem::new());
        let quit = gtk::MenuItem::with_label(crate::i18n::tr("UntuShot beenden"));
        menu.append(&quit);
        menu.show_all();
        let runtime = Rc::new(Self {
            app: app.clone(),
            editor: RefCell::new(None),
            indicator: RefCell::new(None),
            hold: RefCell::new(None),
            handling: Cell::new(false),
            backend: Cell::new(Backend::Auto),
            status,
            actions,
            cancel,
            menu,
        });
        runtime.update_capture_actions();
        for (i, item) in runtime.actions.iter().enumerate() {
            let weak = Rc::downgrade(&runtime);
            item.connect_activate(move |_| {
                if let Some(r) = weak.upgrade() {
                    let mut args = Args::parse_from(["untushot"]);
                    args.backend = r.backend.get();
                    args.capture = match i {
                        0 | 1 if capture::uses_desktop_chooser(args.backend) => Some(Mode::System),
                        0 | 1 => Some(Mode::Region),
                        2 => Some(Mode::Full),
                        3 => Some(Mode::System),
                        _ => None,
                    };
                    if i == 1 {
                        args.delay = 3;
                    }
                    if let Err(error) = r.request(args) {
                        crate::i18n::eprintln!("UntuShot: {error:#}");
                    }
                }
            });
        }
        let weak = Rc::downgrade(&runtime);
        runtime.cancel.connect_activate(move |_| {
            if let Some(r) = weak.upgrade() {
                if let Some(e) = r.editor.borrow().clone() {
                    e.cancel_capture();
                }
            }
        });
        let weak = Rc::downgrade(&runtime);
        quit.connect_activate(move |_| {
            if let Some(r) = weak.upgrade() {
                let mut args = Args::parse_from(["untushot"]);
                args.quit = true;
                if let Err(error) = r.request(args) {
                    crate::i18n::eprintln!("UntuShot: {error:#}");
                }
            }
        });
        let weak = Rc::downgrade(&runtime);
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(r) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let e = r.editor.borrow().clone();
            let busy = e.as_ref().is_some_and(|e| e.is_capturing());
            let label = e
                .as_ref()
                .map(|e| e.capture_status())
                .unwrap_or_else(|| crate::i18n::tr("Bereit · Strg+Shift+X").into());
            if r.status.label().as_deref() != Some(&label) {
                r.status.set_label(&label);
            }
            for item in &r.actions {
                item.set_sensitive(!busy && !r.handling.get());
            }
            r.cancel.set_sensitive(busy);
            r.cancel.set_visible(busy);
            glib::ControlFlow::Continue
        });
        runtime
    }
    fn set_backend(&self, backend: Backend) {
        self.backend.set(backend);
        self.update_capture_actions();
    }
    fn update_capture_actions(&self) {
        let desktop = capture::uses_desktop_chooser(self.backend.get());
        self.actions[0].set_label(if desktop {
            crate::i18n::tr("Screenshot aufnehmen")
        } else {
            crate::i18n::tr("Bereich auswählen")
        });
        self.actions[1].set_label(if desktop {
            crate::i18n::tr("Aufnahmeauswahl in 3 Sekunden")
        } else {
            crate::i18n::tr("Bereich auswählen · 3 Sekunden")
        });
        for (index, item) in self.actions.iter().enumerate() {
            item.set_visible(!desktop || !matches!(index, 2 | 3));
        }
    }
    fn ensure_indicator(&self) -> Result<()> {
        if self.indicator.borrow().is_some() {
            return Ok(());
        }
        if !tray::host_available().unwrap_or(false) {
            crate::i18n::eprintln!("UntuShot: Warte auf AppIndicator-Unterstützung des Desktops.");
        }
        let indicator = tray::Indicator::new(&self.menu)?;
        *self.hold.borrow_mut() = Some(self.app.hold());
        *self.indicator.borrow_mut() = Some(indicator);
        if let Some(e) = self.editor.borrow().as_ref() {
            e.set_resident(true);
        }
        Ok(())
    }
    fn request(self: &Rc<Self>, args: Args) -> Result<String> {
        if args.status {
            let e = self.editor.borrow().clone();
            return Ok(serde_json::json!({"running":self.indicator.borrow().is_some(),"tray_connected":self.indicator.borrow().as_ref().is_some_and(|i|i.connected()),"capturing":e.as_ref().is_some_and(|e|e.is_capturing()),"pid":std::process::id()}).to_string());
        }
        crate::i18n::ensure!(
            !self.handling.replace(true),
            "Ein Dialog oder Befehl ist bereits aktiv"
        );
        let result = self.handle(args);
        self.handling.set(false);
        result
    }
    fn handle(self: &Rc<Self>, args: Args) -> Result<String> {
        let existing = self.editor.borrow().clone().filter(|e| !e.is_closed());
        if !args.cancel {
            crate::i18n::ensure!(
                !gtk::Window::list_toplevels()
                    .iter()
                    .filter_map(|w| w.clone().downcast::<gtk::Window>().ok())
                    .any(|w| w.is_visible() && w.is_modal()),
                "Bitte zuerst den geöffneten Dialog abschließen"
            );
        }
        if args.quit {
            if let Some(e) = existing {
                crate::i18n::ensure!(
                    e.shutdown(),
                    "Beenden abgebrochen: Sitzung konnte nicht gesichert werden"
                );
            }
            self.indicator.borrow_mut().take();
            self.hold.borrow_mut().take();
            self.app.quit();
            return Ok(crate::i18n::tr("UntuShot beendet").into());
        }
        if args.cancel {
            if let Some(e) = existing {
                e.cancel_capture();
            }
            return Ok(crate::i18n::tr("Aufnahme abgebrochen").into());
        }
        // Do not change a document while its file chooser/text dialog is running.
        crate::i18n::ensure!(
            !self
                .app
                .windows()
                .iter()
                .any(|w| w.is_visible() && w.is_modal()),
            "Bitte zuerst den geöffneten Dialog abschließen"
        );
        if let Err(error) = self.ensure_indicator() {
            if args.background {
                return Err(error);
            }
            crate::i18n::eprintln!("UntuShot ohne Tray: {error:#}");
        }
        if args.background {
            self.set_backend(args.backend);
            return Ok(crate::i18n::tr("UntuShot wartet im Hintergrund").into());
        }
        if existing.as_ref().is_some_and(|e| e.is_capturing()) {
            crate::i18n::bail!("Eine Aufnahme läuft bereits");
        }
        self.set_backend(args.backend);
        let new = existing.is_none();
        let e = existing.unwrap_or_else(|| {
            let e = ui::launch(&self.app, args.clone());
            *self.editor.borrow_mut() = Some(e.clone());
            e
        });
        e.set_resident(self.indicator.borrow().is_some());
        if new && args.open.is_some() {
            e.present();
        } else {
            crate::i18n::ensure!(e.request(args), "Aktion abgebrochen");
        }
        Ok(crate::i18n::tr("UntuShot bereit").into())
    }
}

fn reply(line: &gio::ApplicationCommandLine, text: &str, error: bool) {
    let text = CString::new(text.replace('\0', "�")).expect("sanitized");
    // Constant printf format; caller-controlled paths/text are only data.
    unsafe {
        if error {
            gio::ffi::g_application_command_line_printerr(
                line.to_glib_none().0,
                b"%s\n\0".as_ptr().cast(),
                text.as_ptr(),
            );
        } else {
            gio::ffi::g_application_command_line_print(
                line.to_glib_none().0,
                b"%s\n\0".as_ptr().cast(),
                text.as_ptr(),
            );
        }
    }
}
pub fn run(mut args: Args) -> glib::ExitCode {
    // Resolve paths in the invoking process, not in the daemon's working directory.
    if let Ok(cwd) = std::env::current_dir() {
        for path in [&mut args.open, &mut args.output].into_iter().flatten() {
            if path.is_relative() {
                *path = cwd.join(&*path);
            }
        }
    }
    let payload = match serde_json::to_string(&args) {
        Ok(value) => value,
        Err(error) => {
            crate::i18n::eprintln!("UntuShot: {error}");
            return glib::ExitCode::FAILURE;
        }
    };
    let app = gtk::Application::new(
        Some("io.github.untushot.UntuShot"),
        gio::ApplicationFlags::HANDLES_COMMAND_LINE,
    );
    // Parse CLI with clap locally and forward a typed request in GApplication's
    // options dictionary. GTK never has to parse UntuShot-specific flags.
    app.connect_handle_local_options(move |_, options| {
        options.insert("untushot-request", payload.clone());
        -1
    });
    let runtime = Runtime::new(&app);
    let r = runtime.clone();
    app.connect_command_line(move |_, line| {
        let result = (|| -> Result<String> {
            let payload = line
                .options_dict()
                .lookup::<String>("untushot-request")?
                .context(crate::i18n::tr("Kein UntuShot-Befehl"))?;
            let args: Args = serde_json::from_str(&payload)?;
            r.request(args)
        })();
        match result {
            Ok(message) => {
                reply(line, &message, false);
                0
            }
            Err(error) => {
                reply(line, &crate::i18n::format!("UntuShot: {error:#}"), true);
                1
            }
        }
    });
    let r = runtime.clone();
    app.connect_activate(move |_| {
        if let Err(error) = r.request(Args::parse_from(["untushot"])) {
            crate::i18n::eprintln!("UntuShot: {error:#}");
        }
    });
    let r = runtime;
    app.connect_shutdown(move |_| {
        r.indicator.borrow_mut().take();
        r.hold.borrow_mut().take();
    });
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
pub(crate) fn showcase_menu(app: &gtk::Application) -> gtk::Menu {
    Runtime::new(app).menu.clone()
}

#[cfg(test)]
pub(crate) fn assert_quit_with_error(app: &gtk::Application, editor: &Rc<ui::Editor>) {
    let runtime = Runtime::new(app);
    *runtime.editor.borrow_mut() = Some(editor.clone());
    runtime
        .request(Args::parse_from(["untushot", "--quit"]))
        .expect("an error popup must not block the quit command");
    assert!(editor.is_closed());
}

#[cfg(test)]
pub(crate) fn start_capture_for_test(app: &gtk::Application, args: Args) -> Rc<ui::Editor> {
    let runtime = Runtime::new(app);
    runtime.request(args).expect("cold-start capture command");
    let editor = runtime.editor.borrow().clone().unwrap();
    editor
}

#[cfg(test)]
pub(crate) fn start_tray_capture_for_test(app: &gtk::Application, delayed: bool) -> Rc<ui::Editor> {
    let runtime = Runtime::new(app);
    runtime.set_backend(Backend::Grim);
    runtime.menu.show_all();
    // The menu itself stays unmapped until opened by the tray host.
    assert_eq!(
        runtime.actions.iter().filter(|i| i.get_visible()).count(),
        5
    );
    assert_eq!(
        runtime.actions[0].label().as_deref(),
        Some(crate::i18n::tr("Bereich auswählen"))
    );
    runtime.set_backend(Backend::Portal);
    runtime.menu.show_all();
    assert_eq!(
        runtime.actions.iter().filter(|i| i.get_visible()).count(),
        3
    );
    assert_eq!(
        runtime.actions[0].label().as_deref(),
        Some(crate::i18n::tr("Screenshot aufnehmen"))
    );
    assert_eq!(
        runtime.actions[1].label().as_deref(),
        Some(crate::i18n::tr("Aufnahmeauswahl in 3 Sekunden"))
    );
    assert!(!runtime.actions[2].get_visible() && !runtime.actions[3].get_visible());
    assert!(runtime.actions[4].get_visible());
    assert!(runtime.actions[0].child().unwrap().get_visible());
    assert!(!runtime.cancel.get_visible());
    runtime.actions[usize::from(delayed)].activate();
    let editor = runtime
        .editor
        .borrow()
        .clone()
        .expect("tray action started the editor");
    editor
}
