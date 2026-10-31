use super::*;
use clap::Parser;

fn control_key(dialog: &gtk::Dialog, key: gdk::keys::Key) {
    use glib::translate::IntoGlib;
    let mut event = gdk::Event::new(gdk::EventType::KeyPress)
        .downcast::<gdk::EventKey>()
        .unwrap();
    event.as_mut().keyval = key.into_glib();
    event.as_mut().state = gdk::ModifierType::CONTROL_MASK.bits();
    assert!(dialog.emit_by_name::<bool>("key-press-event", &[&*event]));
}

fn settle() {
    let until = std::time::Instant::now() + Duration::from_millis(180);
    while std::time::Instant::now() < until {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(3));
    }
}
fn snapshot(widget: &impl IsA<gtk::Widget>, name: &str) {
    settle();
    let widget = widget.as_ref();
    let surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        widget.allocated_width(),
        widget.allocated_height(),
    )
    .unwrap();
    widget.draw(&cairo::Context::new(&surface).unwrap());
    surface
        .write_to_png(&mut std::fs::File::create(format!("target/ux-{name}.png")).unwrap())
        .unwrap();
}

#[test]
#[ignore = "requires a private GTK display"]
fn gtk_ux_changes() {
    gtk::init().unwrap();
    let app = gtk::Application::new(
        Some("io.github.untushot.UxChanges"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    app.register(gio::Cancellable::NONE).unwrap();
    let e = build(&app, Args::parse_from(["untushot", "--demo"]));
    let dir = tempfile::tempdir().unwrap();
    let prefs = dir.path().join("preferences.json");
    e.load_preferences(&prefs);
    settle();

    // An empty layer list gives the canvas its space back. Explicitly opening
    // it wins, and automatically hiding it must not persist as a user choice.
    assert!(!e.right_panel.is_visible());
    assert!(e.auto_layers_hidden.get());
    e.save_preferences().unwrap();
    assert!(Preferences::load(&prefs).unwrap().layers_visible);
    e.show_layers.set_active(true);
    e.refresh();
    assert!(e.right_panel.is_visible());
    e.layers_manually_shown.set(false);
    e.refresh();
    assert!(!e.right_panel.is_visible());
    snapshot(&e.window, "empty-canvas");
    e.tools[3].set_active(true); // Line
    let view = e.state.borrow().view;
    e.press(
        view.x + 100. * view.scale,
        view.y + 100. * view.scale,
        false,
    );
    e.release(view.x + 300. * view.scale, view.y + 200. * view.scale);
    assert!(e.right_panel.is_visible());
    e.mark_project_saved();

    // Native focus navigation and control keys must not invoke canvas actions.
    e.export_button.grab_focus();
    assert_eq!(
        e.window.focused_widget().as_ref(),
        Some(e.export_button.upcast_ref())
    );
    for key in [
        gdk::keys::constants::Tab,
        gdk::keys::constants::ISO_Left_Tab,
        gdk::keys::constants::Return,
        gdk::keys::constants::space,
        gdk::keys::constants::Right,
    ] {
        assert_eq!(
            e.key_input(key, gdk::ModifierType::empty()),
            glib::Propagation::Proceed
        );
    }
    // Broadway has no physical seat: deliver the focus event a desktop would
    // send before testing GTK's native traversal.
    let mut focus = gdk::Event::new(gdk::EventType::FocusChange)
        .downcast::<gdk::EventFocus>()
        .unwrap();
    focus.as_mut().in_ = 1;
    e.export_button.send_focus_change(&focus);
    e.window
        .emit_by_name::<()>("move-focus", &[&gtk::DirectionType::TabForward]);
    assert_ne!(
        e.window.focused_widget().as_ref(),
        Some(e.export_button.upcast_ref())
    );
    e.key_input(gdk::keys::constants::F6, gdk::ModifierType::empty());
    assert_eq!(
        e.window.focused_widget().as_ref(),
        Some(e.area.upcast_ref())
    );
    e.tools[2].grab_focus();
    e.tools[2].set_active(true);
    assert_eq!(
        e.window.focused_widget().as_ref(),
        Some(e.tools[2].upcast_ref())
    );
    e.key_input(gdk::keys::constants::F6, gdk::ModifierType::CONTROL_MASK);
    assert!(!e.left_panel.is_visible() && !e.right_panel.is_visible());
    e.key_input(gdk::keys::constants::F6, gdk::ModifierType::CONTROL_MASK);
    assert!(e.left_panel.is_visible() && e.right_panel.is_visible());
    e.show_shortcuts();
    e.show_shortcuts();
    assert_eq!(
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .filter(
                |w| w.is_visible() && w.title().as_deref() == Some(crate::i18n::tr("Tastenkürzel"))
            )
            .count(),
        1,
        "shortcut help must be a singleton"
    );
    let help = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Dialog>().ok())
        .find(|d| d.title().as_deref() == Some(crate::i18n::tr("Tastenkürzel")))
        .unwrap();
    snapshot(&help, "shortcuts");
    snapshot(&help, &format!("shortcuts-{}", crate::i18n::language()));
    let settings = gtk::Settings::default().unwrap();
    let settings_font: String = settings.property("gtk-font-name");
    let dark: bool = settings.property("gtk-application-prefer-dark-theme");
    settings.set_property("gtk-font-name", "Sans 16");
    settings.set_property("gtk-application-prefer-dark-theme", true);
    snapshot(
        &help,
        &format!("shortcuts-large-{}", crate::i18n::language()),
    );
    let scroll = help.content_area().children()[0]
        .clone()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let viewport = scroll.child().unwrap().downcast::<gtk::Viewport>().unwrap();
    let grid = viewport.child().unwrap().downcast::<gtk::Grid>().unwrap();
    assert!(grid.allocated_width() <= scroll.allocated_width());
    for label in grid
        .children()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
    {
        let (width, _) = label.layout().unwrap().pixel_size();
        assert!(
            width <= label.allocated_width() + 1,
            "help text is clipped: {}",
            label.text()
        );
    }
    assert!(
        help.allocated_width() <= 1000,
        "large-text help fits a laptop screen"
    );
    settings.set_property("gtk-font-name", settings_font.as_str());
    settings.set_property("gtk-application-prefer-dark-theme", dark);
    control_key(&help, gdk::keys::constants::w);
    assert!(e.shortcuts_dialog.borrow().is_none());

    e.setup.set_backend(crate::Backend::Grim);
    // Failure actions preserve the previous document. Retry is cancelled during
    // its timer, so this test never calls the desktop capture service.
    e.show_setup();
    settle();
    assert!(!e.setup.options.is_expanded());
    let compact_height = e.setup.root.allocated_height();
    assert!(
        compact_height < 400,
        "default capture controls stay compact"
    );
    assert!(!e.setup.demo.is_mapped() && !e.setup.web.is_mapped());
    assert!(!e.setup.progress.is_visible());
    snapshot(
        &e.window,
        &format!("capture-simple-{}", crate::i18n::language()),
    );
    e.setup.options.set_expanded(true);
    settle();
    assert!(e.setup.root.allocated_height() > compact_height + 100);
    assert!(e.setup.source.is_mapped() && e.setup.delay.is_mapped());
    e.setup.source.append(Some("1"), "Second screen");
    e.setup.source.set_active_id(Some("1"));
    assert_eq!(
        e.setup.options.label().as_deref(),
        Some(crate::i18n::format!("Optionen · Bildschirm {}", 2).as_str())
    );
    e.setup.options.set_expanded(false);
    e.setup.delay.set_value(3.);
    {
        use gtk::atk::prelude::*;
        assert!(e
            .setup
            .start
            .accessible()
            .unwrap()
            .name()
            .unwrap()
            .contains("3 s"));
    }
    e.setup.source.set_active_id(Some("all"));
    e.setup.delay.set_value(0.);
    let before = e.state.borrow().document.as_ref().unwrap().content_id();
    e.finish_capture(
        Err(capture::PortalFailure::Cancelled.into()),
        Mode::Full,
        None,
    );
    assert!(e.error_dialog.borrow().is_none());
    e.finish_capture(
        Err(capture::PortalFailure::Rejected.into()),
        Mode::Full,
        None,
    );
    let popup = e.error_dialog.borrow().clone().unwrap();
    let details = popup
        .content_area()
        .children()
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Expander>().ok())
        .unwrap();
    assert!(!details.is_expanded());
    assert!(!popup.is_modal());
    snapshot(&popup, "capture-error");
    popup.response(gtk::ResponseType::Other(1));
    assert!(e.error_dialog.borrow().is_none());
    assert_eq!(e.stack.visible_child_name().as_deref(), Some("setup"));
    assert_eq!(
        e.window.focused_widget().as_ref(),
        Some(e.setup.region.upcast_ref())
    );
    e.setup.system.set_active(true);
    {
        use gtk::atk::prelude::*;
        assert_eq!(
            e.setup.start.accessible().unwrap().name().as_deref(),
            Some(Mode::System.action_label(0).as_str())
        );
    }
    assert!(!e.setup.source.is_sensitive());
    snapshot(&e.window, "capture-choices");
    e.setup.region.set_active(true);
    e.setup.delay.set_value(1.);
    e.show_shortcuts();
    let capture_help = e.shortcuts_dialog.borrow().clone().unwrap();
    e.capture_error(capture::PortalFailure::Rejected.into());
    let popup = e.error_dialog.borrow().clone().unwrap();
    popup.response(gtk::ResponseType::Other(2));
    assert!(e.is_capturing());
    assert!(!capture_help.is_visible());
    assert!(e.shortcuts_dialog.borrow().is_none());
    {
        use gtk::atk::prelude::*;
        assert_eq!(
            e.setup.cancel.accessible().unwrap().name().as_deref(),
            Some(crate::i18n::tr("Countdown abbrechen"))
        );
    }
    // Reopening help during a countdown must not put it into the capture.
    // Stop as soon as the window hides, before the 350 ms capture wait ends.
    e.show_shortcuts();
    let late_help = e.shortcuts_dialog.borrow().clone().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while e.window.is_visible() && std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(3));
    }
    let hidden = !e.window.is_visible();
    e.cancel_capture();
    assert!(hidden, "the countdown must finish");
    assert!(!late_help.is_visible());
    assert!(!e.is_capturing());
    assert_eq!(
        before,
        e.state.borrow().document.as_ref().unwrap().content_id()
    );

    // On a compact window, layers retain space and keyboard focus can reveal
    // the last property control through its independent scroller.
    let mut doc = document::demo().unwrap();
    let tile = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 80, 60).unwrap();
    tile.fill(0x4389baff);
    let index = doc
        .insert_image(tile, Rect::new(100., 100., 160., 120.), "Image".into())
        .unwrap();
    e.set_document(doc, "Test image");
    e.select_new_image(index, "Image properties");
    e.shadow_enabled.set_active(true);
    let geometry = e.object_properties.children()[0]
        .clone()
        .downcast::<gtk::Expander>()
        .unwrap();
    geometry.set_expanded(true);
    e.window.resize(1000, 700);
    settle();
    let split = e
        .property_scroll
        .parent()
        .unwrap()
        .downcast::<gtk::Paned>()
        .unwrap();
    split.set_position(240);
    e.shadow_offset.grab_focus();
    settle();
    assert!(e.layer_scroll.allocated_height() >= 120);
    let adjustment = e.property_scroll.vadjustment();
    assert!(adjustment.upper() > adjustment.page_size());
    let bottom = e
        .shadow_offset
        .translate_coordinates(&e.property_scroll, 0, 0)
        .unwrap()
        .1
        + e.shadow_offset.allocated_height();
    assert!(
        bottom <= e.property_scroll.allocated_height(),
        "focused property is scrolled into view"
    );
    snapshot(&e.window, "resizable-inspector");
    e.mark_project_saved();

    // Both exit choices are explicit and persist; the explanation is one-time.
    e.set_resident(true);
    assert!(!e.close_to_tray.is_active());
    assert!(e.close_notice.is_visible());
    snapshot(&e.window, "close-explanation");
    settings.set_property("gtk-font-name", "Sans 16");
    e.file_menu.set_active(true);
    settle();
    let menu = e.file_menu.popover().unwrap();
    snapshot(
        &e.window,
        &format!("menu-large-{}", crate::i18n::language()),
    );
    assert!(
        menu.allocated_height() < e.window.allocated_height() - 60,
        "application menu must fit below the header at large text sizes"
    );
    e.close_to_tray.grab_focus();
    settle();
    let (_, y) = e.close_to_tray.translate_coordinates(&menu, 0, 0).unwrap();
    assert!(
        y >= 0 && y + e.close_to_tray.allocated_height() <= menu.allocated_height(),
        "keyboard navigation must reveal the background-mode option"
    );
    e.file_menu.set_active(false);
    settings.set_property("gtk-font-name", settings_font.as_str());
    e.close_to_tray.set_active(true);
    e.close_notice.response(gtk::ResponseType::Close);
    e.save_preferences().unwrap();
    let saved = Preferences::load(&prefs).unwrap();
    assert!(saved.close_to_tray && saved.close_behavior_explained);
    e.set_resident(true);
    assert!(!e.close_notice.is_visible());
    e.show_shortcuts();
    let background_help = e.shortcuts_dialog.borrow().clone().unwrap();
    e.capture_error(capture::PortalFailure::Rejected.into());
    e.window.close();
    settle();
    assert!(!e.window.is_visible());
    assert!(!background_help.is_visible());
    assert!(e.shortcuts_dialog.borrow().is_none());
    assert!(e.error_dialog.borrow().is_none());
    assert!(!e.is_closed());
    e.present();
    e.close_to_tray.set_active(false);
    e.window.close();
    settle();
    assert!(e.is_closed());
    assert!(!Preferences::load(&prefs).unwrap().close_to_tray);
    let quitting = build(&app, Args::parse_from(["untushot", "--demo"]));
    quitting.show_shortcuts();
    let help = quitting.shortcuts_dialog.borrow().clone().unwrap();
    control_key(&help, gdk::keys::constants::q);
    assert!(quitting.is_closed());
    assert!(!help.is_visible());
    quitting.window.close();
}
