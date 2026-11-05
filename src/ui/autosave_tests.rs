use super::*;
use clap::Parser;

fn document() -> Document {
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 64, 48).unwrap();
    pixels.fill(0x203040ff);
    Document::new(pixels, Vec::new()).unwrap()
}
fn edit(e: &Editor, name: &str) {
    e.state
        .borrow_mut()
        .document
        .as_mut()
        .unwrap()
        .add(Annotation {
            effects: Default::default(),
            clip: None,
            label: name.into(),
            font_size: None,
            tool: Tool::Arrow,
            points: vec![Point::new(2., 2.), Point::new(30., 20.)],
            color: (1., 0., 0.),
            width: 3.,
            text: String::new(),
            visible: true,
        });
    e.refresh();
}

#[test]
#[ignore = "requires a private GTK display"]
fn gtk_autosave_workflow() {
    gtk::init().unwrap();
    let app = gtk::Application::new(
        Some("io.github.untushot.AutosaveTest"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    app.register(gio::Cancellable::NONE).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let e = build(&app, Args::parse_from(["untushot"]));
    e.load_preferences(&directory.path().join("preferences.json"));
    e.preferences.borrow_mut().export_directory = Some(directory.path().join("Screenshots"));
    e.session.replace(Some(
        recovery::Session::open(&directory.path().join("recovery"), false)
            .unwrap()
            .0,
    ));
    let prompts = Rc::new(Cell::new(0));
    let count = prompts.clone();
    let guard = glib::timeout_add_local(Duration::from_millis(100), move || {
        for w in gtk::Window::list_toplevels() {
            if let Ok(d) = w.downcast::<gtk::MessageDialog>() {
                if d.is_visible() && d.is_modal() {
                    count.set(count.get() + 1);
                    d.response(gtk::ResponseType::Cancel);
                }
            }
        }
        glib::ControlFlow::Continue
    });

    // A capture without a filename gets a durable file before a new capture.
    e.set_document(document(), "Capture A");
    edit(&e, "First arrow");
    assert!(e.can_discard());
    let first = e.project_path.borrow().clone().unwrap();
    assert!(first.starts_with(directory.path().join("Screenshots")));
    assert_eq!(
        open_document(&first).unwrap().0.annotations()[1].label,
        "First arrow"
    );
    assert_eq!(
        e.storage_caption.text(),
        crate::i18n::tr("Automatisch gesichert")
    );
    assert!(!e.project_save_button.is_visible());
    e.set_document(document(), "Capture B");
    edit(&e, "Second arrow");
    assert!(e.can_discard());
    assert_ne!(e.project_path.borrow().as_ref(), Some(&first));
    assert_eq!(open_document(&first).unwrap().0.annotations().len(), 2);

    // Exported images retain editable layers beside them; subsequent edits
    // update the sidecar without changing the PNG being shared.
    let png = directory.path().join("Shot.png");
    save_png(
        &e.state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .export()
            .unwrap(),
        &png,
    )
    .unwrap();
    e.mark_exported(&png);
    e.refresh();
    assert!(e.can_discard());
    let pixels = std::fs::read(&png).unwrap();
    let sidecar = autosave::sidecar_path(&png);
    assert_eq!(e.project_path.borrow().as_ref(), Some(&sidecar));
    edit(&e, "Extra arrow");
    e.save_recovery().unwrap();
    e.session.borrow().as_ref().unwrap().flush().unwrap();
    e.poll_recovery();
    assert_eq!(open_document(&sidecar).unwrap().0.annotations().len(), 3);
    assert_eq!(std::fs::read(&png).unwrap(), pixels);
    assert!(autosave::matches_image(&png));
    assert!(e.open_path(&png));
    assert_eq!(
        e.state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .len(),
        3
    );

    // Undo and redo are persisted too, including a return to the original.
    edit(&e, "Undo me");
    e.history(false);
    e.flush_autosave().unwrap();
    assert_eq!(open_document(&sidecar).unwrap().0.annotations().len(), 3);
    e.history(true);
    e.flush_autosave().unwrap();
    assert_eq!(open_document(&sidecar).unwrap().0.annotations().len(), 4);

    // Restart retains the association, and concurrent writers are refused.
    assert!(autosave::Target::open(sidecar.clone(), Some(png.clone()), true).is_err());
    assert!(e.shutdown());
    let (session, restored) =
        recovery::Session::open(&directory.path().join("recovery"), true).unwrap();
    assert_eq!(restored.unwrap().annotations().len(), 4);
    let location = session.restored_autosave.clone().unwrap();
    let target = autosave::Target::restore(&location).unwrap();
    assert_eq!(target.path, sidecar);
    drop(target);
    drop(session);
    e.window.close();

    // A changed external file must never be silently overwritten. A failed
    // autosave blocks replacement but still preserves recovery, then Save As
    // lets the user rescue their edits without being stuck in a modal loop.
    let e = build(&app, Args::parse_from(["untushot"]));
    e.load_preferences(&directory.path().join("preferences-2.json"));
    e.session.replace(Some(
        recovery::Session::open(&directory.path().join("recovery-2"), false)
            .unwrap()
            .0,
    ));
    assert!(e.open_path(&png));
    e.flush_autosave().unwrap();
    std::fs::write(&sidecar, b"external change").unwrap();
    edit(&e, "Keep on failure");
    assert!(!e.can_discard());
    assert!(e.error_dialog.borrow().is_some());
    assert_eq!(
        e.state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .len(),
        5
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"external change");
    let recovery = e.session.borrow().as_ref().unwrap().path.clone();
    assert_eq!(open_document(&recovery).unwrap().0.annotations().len(), 5);
    assert!(e.save_project_to(&directory.path().join("Rescued.untushot")));
    assert!(e.can_discard());
    e.dismiss_error();

    // An existing unrelated sidecar is preserved, and a changed PNG is opened
    // as an image rather than replaced by an outdated editable snapshot.
    assert!(!autosave::matches_image(&png));
    assert!(e.open_path(&png));
    assert!(
        e.state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .len()
            == 1
    );
    e.preferences.borrow_mut().export_directory = Some(directory.path().join("Screenshots"));
    edit(&e, "New version");
    assert!(e.can_discard());
    assert_ne!(e.project_path.borrow().as_ref(), Some(&sidecar));
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"external change");

    // Even destination-creation failures must leave a current recovery copy.
    e.set_document(document(), "Unwritable destination");
    let bad = directory.path().join("not-a-folder");
    std::fs::write(&bad, b"keep").unwrap();
    e.preferences.borrow_mut().export_directory = Some(bad);
    edit(&e, "Recovery despite destination failure");
    assert!(!e.can_discard());
    assert_eq!(
        open_document(&recovery).unwrap().0.annotations()[1].label,
        "Recovery despite destination failure"
    );
    assert!(e.save_project_to(&directory.path().join("Rescued-again.untushot")));
    assert!(e.can_discard());
    assert!(e.shutdown());
    e.window.close();
    // Exercise the actual periodic timer and startup restoration, not just the
    // explicit flush used at document boundaries.
    assert!(recovery::default_directory()
        .to_string_lossy()
        .starts_with("/tmp/untushot-clipboard-"));
    let e = launch(&app, Args::parse_from(["untushot"]));
    e.preferences.borrow_mut().export_directory = Some(directory.path().join("Screenshots"));
    e.set_document(document(), "Periodic autosave");
    edit(&e, "Saved by the timer");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while e.state.borrow().dirty && std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!e.state.borrow().dirty);
    let saved = e.project_path.borrow().clone().unwrap();
    assert_eq!(
        open_document(&saved).unwrap().0.annotations()[1].label,
        "Saved by the timer"
    );
    assert!(e.shutdown());
    e.window.close();
    let e = launch(&app, Args::parse_from(["untushot"]));
    assert_eq!(e.project_path.borrow().as_ref(), Some(&saved));
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations()[1].label,
        "Saved by the timer"
    );
    edit(&e, "After restart");
    assert!(e.can_discard());
    assert_eq!(open_document(&saved).unwrap().0.annotations().len(), 3);
    assert!(e.shutdown());
    e.window.close();
    guard.remove();
    assert_eq!(
        prompts.get(),
        0,
        "routine editing must not ask to save a project"
    );
}
