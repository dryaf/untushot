//! Exercise the real asynchronous portal call on a private D-Bus session.
use super::*;
use clap::Parser;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

#[test]
#[ignore = "requires the private portal test runner"]
fn gtk_portal_capture() {
    assert_eq!(
        std::env::var("UNTUSHOT_TEST_PRIVATE_PORTAL").as_deref(),
        Ok("1")
    );
    let state = std::env::var("XDG_STATE_HOME").unwrap();
    assert!(state.starts_with("/tmp/untushot-clipboard-"));
    gtk::init().unwrap();
    let app = gtk::Application::new(
        Some("io.github.untushot.PortalTest"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    app.register(gio::Cancellable::NONE).unwrap();
    let context = glib::MainContext::default();
    let bus = context
        .block_on(gio::bus_get_future(gio::BusType::Session))
        .unwrap();
    let name = context
        .block_on(bus.call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&("org.freedesktop.portal.Desktop", 4u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
        ))
        .unwrap();
    assert_eq!(name.get::<(u32,)>().unwrap().0, 1);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.png");
    let image = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 32, 24).unwrap();
    image.fill(0x3280c0ff);
    crate::pixels::write_png(&image, std::fs::File::create(&path).unwrap()).unwrap();
    let uri = gio::File::for_path(&path).uri().to_string();
    let responses = Arc::new(AtomicU32::new(0));
    let reply_code = responses.clone();
    let requests = Arc::new(AtomicU32::new(0));
    let received = requests.clone();
    let interface = gio::DBusNodeInfo::for_xml(r#"<node><interface name="org.freedesktop.portal.Screenshot"><method name="Screenshot"><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method></interface></node>"#).unwrap().lookup_interface("org.freedesktop.portal.Screenshot").unwrap();
    let registration = bus
        .register_object(
            "/org/freedesktop/portal/desktop",
            &interface,
            move |bus, sender, _, _, method, parameters, invocation| {
                assert_eq!(method, "Screenshot");
                let (_, options) = parameters
                    .get::<(String, HashMap<String, glib::Variant>)>()
                    .unwrap();
                let interactive = options
                    .get("interactive")
                    .and_then(|v| v.get::<bool>())
                    .unwrap_or(false);
                // Reproduce GNOME's rejection of a hidden app's non-interactive
                // permission prompt. Only an explicit desktop chooser succeeds.
                let code = if interactive {
                    reply_code.load(Ordering::SeqCst)
                } else {
                    2
                };
                received.fetch_add(1, Ordering::SeqCst);
                let token = options["handle_token"].str().unwrap();
                let sender_path = sender.trim_start_matches(':').replace('.', "_");
                let request_path =
                    format!("/org/freedesktop/portal/desktop/request/{sender_path}/{token}");
                let handle = glib::variant::ObjectPath::try_from(request_path.as_str()).unwrap();
                invocation.return_value(Some(&(handle,).to_variant()));
                let mut result = HashMap::<String, glib::Variant>::new();
                if code == 0 {
                    result.insert("uri".into(), uri.to_variant());
                }
                bus.emit_signal(
                    Some(sender),
                    &request_path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    Some(&(code, result).to_variant()),
                )
                .unwrap();
            },
            |_, _, _, _, _| unreachable!(),
            |_, _, _, _, _, _| false,
        )
        .unwrap();

    for mode in ["region", "full", "system"] {
        let args = Args::parse_from(["untushot", "--backend", "portal", "--capture", mode]);
        let e = crate::runtime::start_capture_for_test(&app, args);
        assert!(e.is_capturing());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while e.is_capturing() && std::time::Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(3));
        }
        assert!(!e.is_capturing(), "portal request did not finish");
        assert!(
            e.error_dialog.borrow().is_none(),
            "cold-start {mode} capture was rejected"
        );
        let s = e.state.borrow();
        let doc = s.document.as_ref().expect("portal image reached editor");
        assert_eq!((doc.width(), doc.height()), (32, 24));
        assert!(
            !s.selecting_region,
            "desktop-confirmed {mode} capture must open directly in the editor"
        );
        assert!(
            doc.selection().is_none(),
            "preserve the complete desktop-selected image"
        );
        drop(s);
        assert_eq!(e.stack.visible_child_name().as_deref(), Some("editor"));
        assert!(e.export_button.is_sensitive());
        assert!(e.copy.is_sensitive());
        e.mark_project_saved();
        assert!(e.shutdown());
        e.window.close();
    }
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    let settle_capture = |e: &Editor| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while e.is_capturing() && std::time::Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(3));
        }
        assert!(!e.is_capturing());
    };
    let e = build(
        &app,
        Args::parse_from(["untushot", "--demo", "--backend", "portal"]),
    );
    e.show_setup();
    e.window.show_all();
    assert!(!e.setup.region.parent().unwrap().is_visible());
    assert!(!e.setup.source.parent().unwrap().is_visible());
    assert_eq!(e.setup.mode(), Mode::System);
    e.set_document(
        Document::new(image.clone(), Vec::new()).unwrap(),
        "Original",
    );
    for keyboard in [false, true] {
        e.setup.delay.set_value(60.); // New screenshot is immediate; options are explicit.
        if keyboard {
            e.key_input(gdk::keys::constants::n, gdk::ModifierType::CONTROL_MASK);
        } else {
            e.new_capture.emit_clicked();
        }
        assert!(e.is_capturing());
        assert_eq!(e.stack.visible_child_name().as_deref(), Some("editor"));
        assert_eq!(e.setup.delay.value_as_int(), 0);
        e.new_capture.emit_clicked(); // Repeated activation must not queue a second chooser.
        settle_capture(&e);
        assert!(e.error_dialog.borrow().is_none());
        assert!(!e.state.borrow().selecting_region);
    }
    assert_eq!(requests.load(Ordering::SeqCst), 5);
    let crop = Rect::new(1., 2., 10., 12.);
    e.state
        .borrow_mut()
        .document
        .as_mut()
        .unwrap()
        .select(Some(crop));
    e.mark_project_saved();
    let before_cancels = requests.load(Ordering::SeqCst);
    // GNOME returns 2 when its screenshot chooser closes without a file;
    // standard portal cancellation returns 1. Neither should open an error.
    for code in [1, 2] {
        responses.store(code, Ordering::SeqCst);
        e.new_capture.emit_clicked();
        settle_capture(&e);
        assert_eq!(e.stack.visible_child_name().as_deref(), Some("editor"));
        assert_eq!(
            e.state.borrow().document.as_ref().unwrap().selection(),
            Some(crop)
        );
        assert!(e.error_dialog.borrow().is_none());
        if code == 2 {
            assert_eq!(
                e.status.text(),
                crate::i18n::tr("Keine Aufnahme übernommen.")
            );
        }
    }
    e.window.close();

    // Cancelling the first capture leaves a simple retry screen, without
    // automatically reopening the chooser or showing misleading mode buttons.
    let empty = build(&app, Args::parse_from(["untushot", "--backend", "portal"]));
    for code in [1, 2] {
        responses.store(code, Ordering::SeqCst);
        empty.new_screenshot();
        settle_capture(&empty);
        assert_eq!(empty.stack.visible_child_name().as_deref(), Some("setup"));
        assert!(empty.state.borrow().document.is_none());
        assert!(empty.error_dialog.borrow().is_none());
        assert!(!empty.setup.region.parent().unwrap().is_visible());
        assert!(empty.setup.start.is_sensitive());
    }
    assert_eq!(requests.load(Ordering::SeqCst), before_cancels + 4);
    responses.store(0, Ordering::SeqCst);
    empty.setup.start.emit_clicked();
    settle_capture(&empty);
    assert_eq!(empty.stack.visible_child_name().as_deref(), Some("editor"));
    // Unknown failures still surface, and preserve the successfully loaded image.
    responses.store(99, Ordering::SeqCst);
    empty.new_screenshot();
    settle_capture(&empty);
    assert!(empty.error_dialog.borrow().is_some());
    assert!(empty.state.borrow().document.is_some());
    empty.dismiss_error();
    responses.store(0, Ordering::SeqCst);
    empty.window.close();

    let direct = build(
        &app,
        Args::parse_from(["untushot", "--demo", "--backend", "grim"]),
    );
    direct.new_capture.emit_clicked();
    assert!(!direct.is_capturing());
    assert_eq!(direct.stack.visible_child_name().as_deref(), Some("setup"));
    assert!(direct.setup.region.parent().unwrap().is_visible());
    assert!(direct.setup.source.parent().unwrap().is_visible());
    direct.window.close();
    // The decision comes from the capture result, not the requested backend or
    // mode: Auto may fall back to the portal after a direct capture fails.
    let e = build(&app, Args::parse_from(["untushot", "--demo"]));
    let result = |desktop_confirmed| capture::Capture {
        image: image.clone(),
        monitors: Vec::new(),
        note: "Synthetic capture".into(),
        desktop_confirmed,
    };
    e.finish_capture(Ok(result(false)), Mode::Region, None);
    assert!(
        e.state.borrow().selecting_region,
        "direct capture still needs a region"
    );
    assert!(!e.confirm_region.is_sensitive());
    e.finish_capture(Ok(result(true)), Mode::Region, Some(99));
    assert!(!e.state.borrow().selecting_region);
    assert!(e.state.borrow().region_bounds.is_none());
    assert!(e
        .state
        .borrow()
        .document
        .as_ref()
        .unwrap()
        .selection()
        .is_none());
    assert!(e.export_button.is_sensitive());
    // Further cropping is optional and remains available in the editor.
    e.tools[1].set_active(true);
    assert_eq!(e.state.borrow().tool, Tool::Select);
    assert!(!e.state.borrow().selecting_region);
    e.mark_project_saved();
    e.window.close();
    // The simplified tray starts the same native chooser. Its delay applies to
    // opening that chooser, and remains cancellable before any portal request.
    let before = requests.load(Ordering::SeqCst);
    let tray = crate::runtime::start_tray_capture_for_test(&app, false);
    assert_eq!(tray.args.borrow().backend, crate::Backend::Portal);
    assert_eq!(tray.args.borrow().capture, Some(Mode::System));
    settle_capture(&tray);
    assert_eq!(requests.load(Ordering::SeqCst), before + 1);
    assert!(tray.error_dialog.borrow().is_none());
    assert_eq!(tray.stack.visible_child_name().as_deref(), Some("editor"));
    assert!(!tray.state.borrow().selecting_region);
    assert!(tray.shutdown());
    tray.window.close();
    let delayed = crate::runtime::start_tray_capture_for_test(&app, true);
    assert_eq!(delayed.args.borrow().capture, Some(Mode::System));
    assert_eq!(delayed.setup.delay.value_as_int(), 3);
    assert!(delayed.is_capturing());
    delayed.cancel_capture();
    while context.pending() {
        context.iteration(false);
    }
    assert_eq!(requests.load(Ordering::SeqCst), before + 1);
    assert!(!delayed.is_capturing());
    assert!(delayed.shutdown());
    delayed.window.close();
    for code in [1, 2, 99] {
        responses.store(code, Ordering::SeqCst);
        let error = context
            .block_on(capture::take(
                crate::Backend::Portal,
                false,
                capture::Cancellation::default(),
                capture::desktop_layout(),
            ))
            .err()
            .unwrap();
        let expected = match code {
            1 => capture::PortalFailure::Cancelled,
            2 => capture::PortalFailure::NoImage,
            _ => capture::PortalFailure::Rejected,
        };
        assert_eq!(
            error.downcast_ref::<capture::PortalFailure>(),
            Some(&expected)
        );
    }
    bus.unregister_object(registration).unwrap();
}
