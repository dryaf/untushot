//! Reproducible recordings of real GTK widgets on an isolated display.
//! Sample image data is generated here; no personal files or desktop captures.
use super::*;
use cairo::{Context, Format, ImageSurface};
use clap::Parser;
use std::{fs, path::Path, time::Instant};

fn settle(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(3));
    }
}
fn text(cr: &Context, x: f64, y: f64, size: f64, content: &str, color: (f64, f64, f64)) {
    cr.set_source_rgb(color.0, color.1, color.2);
    cr.select_font_face(
        "sans-serif",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    cr.set_font_size(size);
    cr.move_to(x, y);
    cr.show_text(content).unwrap();
}
fn rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, color: (f64, f64, f64)) {
    cr.set_source_rgb(color.0, color.1, color.2);
    cr.rectangle(x, y, w, h);
    cr.fill().unwrap();
}
fn sample(page: usize, tall: bool) -> gdk_pixbuf::Pixbuf {
    let height = if tall { 1800 } else { 640 };
    let surface = ImageSurface::create(Format::ARgb32, 1000, height).unwrap();
    let cr = Context::new(&surface).unwrap();
    rect(&cr, 0., 0., 1000., height as f64, (0.95, 0.96, 0.98));
    rect(&cr, 0., 0., 1000., 70., (0.07, 0.11, 0.19));
    text(&cr, 36., 45., 24., "NORTHSTAR", (1., 1., 1.));
    text(
        &cr,
        560.,
        44.,
        16.,
        "Overview     Projects     Team     Settings",
        (0.70, 0.77, 0.88),
    );
    text(
        &cr,
        38.,
        125.,
        14.,
        "WORKSPACE / PRODUCT TEAM",
        (0.38, 0.45, 0.57),
    );
    text(
        &cr,
        36.,
        180.,
        38.,
        [
            "A good week. In one view.",
            "Ready for the next step.",
            "Everything, in perspective.",
        ][page % 3],
        (0.07, 0.11, 0.19),
    );
    text(
        &cr,
        38.,
        218.,
        18.,
        "A sample dashboard for a clearer conversation.",
        (0.38, 0.45, 0.57),
    );
    for (i, (label, value, sub)) in [
        ("Active projects", "12", "+3 this week"),
        ("Tasks completed", "86%", "Ahead of schedule"),
        ("Team sentiment", "Great", "Keep the momentum"),
    ]
    .iter()
    .enumerate()
    {
        let x = 38. + i as f64 * 313.;
        rect(&cr, x, 254., 296., 152., (1., 1., 1.));
        rect(
            &cr,
            x,
            254.,
            296.,
            4.,
            [(0.20, 0.44, 0.89), (0.16, 0.64, 0.48), (0.98, 0.44, 0.24)][i],
        );
        text(&cr, x + 20., 289., 15., label, (0.4, 0.46, 0.56));
        text(&cr, x + 20., 343., 36., value, (0.08, 0.14, 0.24));
        text(&cr, x + 20., 380., 15., sub, (0.16, 0.53, 0.40));
    }
    rect(&cr, 38., 434., 924., 164., (1., 1., 1.));
    text(&cr, 60., 470., 20., "Next up", (0.08, 0.14, 0.24));
    for (i, line) in [
        "Review the onboarding experience",
        "Share the prototype with the team",
        "Contact: alex@example.test",
    ]
    .iter()
    .enumerate()
    {
        text(
            &cr,
            62.,
            507. + i as f64 * 33.,
            17.,
            line,
            (0.30, 0.37, 0.48),
        );
        text(
            &cr,
            800.,
            507. + i as f64 * 33.,
            14.,
            ["TODAY", "TOMORROW", "PRIVATE"][i],
            (0.4, 0.48, 0.60),
        );
    }
    if tall {
        for i in 0..4 {
            let y = 650. + i as f64 * 260.;
            rect(&cr, 38., y, 924., 230., (1., 1., 1.));
            text(
                &cr,
                62.,
                y + 46.,
                26.,
                [
                    "Make the details visible.",
                    "Keep your context.",
                    "Share the result.",
                    "You reached the end.",
                ][i],
                (0.08, 0.14, 0.24),
            );
            for j in 0..3 {
                rect(
                    &cr,
                    62.,
                    y + 80. + j as f64 * 32.,
                    730. - j as f64 * 80.,
                    12.,
                    (0.88, 0.91, 0.95),
                );
            }
        }
        text(
            &cr,
            38.,
            1770.,
            16.,
            "END OF PAGE · FULL RESOLUTION PRESERVED",
            (0.2, 0.45, 0.72),
        );
    }
    let mut png = Vec::new();
    surface.write_to_png(&mut png).unwrap();
    crate::pixels::from_bytes(&png).unwrap()
}
fn document() -> Document {
    Document::new(sample(0, false), vec![]).unwrap()
}
fn tool(e: &Editor, t: Tool) {
    let i = Tool::ALL.iter().position(|(v, _, _)| *v == t).unwrap();
    e.tools[i].set_active(true);
    settle(40);
}
fn canvas(e: &Editor, x: f64, y: f64) -> (f64, f64) {
    let v = e.state.borrow().view;
    (v.x + x * v.scale, v.y + y * v.scale)
}
fn reset(e: &Editor) {
    e.mark_project_saved();
    e.set_document(document(), "Demo image · No desktop capture");
    e.show_tools.set_active(true);
    e.show_layers.set_active(true);
    e.state.borrow_mut().color = (0.96, 0.29, 0.15);
    e.width.set_value(5.);
    e.font_size.set_value(34.);
    tool(e, Tool::Move);
    e.window.resize(1180, 820);
    e.refresh();
    settle(100);
}
struct Recorder {
    root: PathBuf,
    title: String,
    frames: Vec<serde_json::Value>,
    pointer: Option<(f64, f64)>,
}
impl Recorder {
    fn new(slug: &str, title: &str) -> Self {
        let root = Path::new("target/showcase-frames").join(slug);
        fs::create_dir_all(&root).unwrap();
        Self {
            root,
            title: title.into(),
            frames: vec![],
            pointer: None,
        }
    }
    fn point(&mut self, e: &Editor, widget: &impl IsA<gtk::Widget>) {
        self.pointer = widget
            .as_ref()
            .translate_coordinates(
                &e.window,
                widget.as_ref().allocated_width() / 2,
                widget.as_ref().allocated_height() / 2,
            )
            .map(|(x, y)| (x as f64, y as f64));
    }
    fn shot(&mut self, e: &Editor, caption: &str, duration: f64) {
        self.widget(&e.window, Some(e), caption, duration);
    }
    fn widget(
        &mut self,
        widget: &impl IsA<gtk::Widget>,
        editor: Option<&Editor>,
        caption: &str,
        duration: f64,
    ) {
        settle(if duration >= 0.6 { 210 } else { 45 });
        let surface = ImageSurface::create(Format::ARgb32, 1220, 980).unwrap();
        let cr = Context::new(&surface).unwrap();
        rect(&cr, 0., 0., 1220., 980., (0.035, 0.055, 0.095));
        rect(&cr, 24., 28., 5., 41., (1., 0.36, 0.16));
        text(
            &cr,
            42.,
            43.,
            14.,
            "UNTUSHOT  /  FEATURE TOUR",
            (0.56, 0.65, 0.80),
        );
        text(&cr, 42., 73., 27., &self.title, (0.96, 0.97, 1.));
        text(&cr, 1008., 48., 14., "NATIVE LINUX APP", (0.56, 0.65, 0.80));
        // Broadway without a browser has no physical screen for popup placement.
        // Allocate real menu widgets at their natural size before drawing them.
        if let Some(menu) = widget.as_ref().downcast_ref::<gtk::Menu>() {
            let (_, natural) = menu.preferred_size();
            menu.size_allocate(&gtk::Allocation::new(0, 0, natural.width, natural.height));
        }
        let w = widget.as_ref().allocated_width() as f64;
        let h = widget.as_ref().allocated_height() as f64;
        assert!(
            w >= 100. && h >= 60.,
            "recorded widget must be allocated: {w}x{h}"
        );
        let maximum = if widget.as_ref().is::<gtk::Menu>() {
            2.
        } else {
            1.
        };
        let scale = (1180. / w).min(820. / h).min(maximum);
        let x = 20. + (1180. - w * scale) / 2.;
        let y = 94. + (820. - h * scale) / 2.;
        cr.save().unwrap();
        cr.translate(x, y);
        cr.scale(scale, scale);
        widget.draw(&cr);
        if let Some(e) = editor {
            for button in [
                &e.file_menu,
                &e.storage_button,
                &e.width_picker,
                &e.font_picker,
                &e.color.button,
                &e.border_color.button,
                &e.monitor_button,
            ] {
                if let Some(pop) = button.popover().filter(|p| p.is_mapped()) {
                    if let Some((px, py)) = pop.translate_coordinates(&e.window, 0, 0) {
                        cr.save().unwrap();
                        cr.translate(px as f64, py as f64);
                        pop.draw(&cr);
                        cr.restore().unwrap();
                    }
                }
            }
            if let Some((px, py)) = self.pointer {
                cr.set_source_rgba(1., 0.32, 0.12, 0.22);
                cr.arc(px, py, 15., 0., std::f64::consts::TAU);
                cr.fill().unwrap();
                cr.set_source_rgb(1., 0.4, 0.18);
                cr.set_line_width(2.);
                cr.arc(px, py, 9., 0., std::f64::consts::TAU);
                cr.stroke().unwrap();
            }
        }
        cr.restore().unwrap();
        text(&cr, 28., 950., 19., caption, (0.94, 0.95, 0.98));
        let file = format!("{:04}.png", self.frames.len());
        surface
            .write_to_png(&mut fs::File::create(self.root.join(&file)).unwrap())
            .unwrap();
        self.frames
            .push(serde_json::json!({"file":file,"duration":duration}));
    }
    fn finish(self) {
        assert!(self.frames.len() > 1, "each demo must animate");
        fs::write(
            self.root.join("frames.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"title":self.title,"language":"en","frames":self.frames}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
fn draw_gesture(
    e: &Editor,
    r: &mut Recorder,
    t: Tool,
    from: (f64, f64),
    to: (f64, f64),
    caption: &str,
) {
    tool(e, t);
    let (x, y) = canvas(e, from.0, from.1);
    e.press(x, y, false);
    for i in 1..=10 {
        let f = i as f64 / 10.;
        let (px, py) = canvas(
            e,
            from.0 + (to.0 - from.0) * f,
            from.1 + (to.1 - from.1) * f,
        );
        e.motion(px, py);
        r.pointer = e
            .area
            .translate_coordinates(&e.window, px as i32, py as i32)
            .map(|(x, y)| (x as f64, y as f64));
        r.shot(e, caption, 0.09);
    }
    let (x, y) = canvas(e, to.0, to.1);
    e.release(x, y);
    r.pointer = None;
    r.shot(e, caption, 1.2);
}
fn seed(e: &Editor) {
    let mut d = document();
    d.add(Annotation {
        tool: Tool::Rectangle,
        points: vec![Point::new(343., 249.), Point::new(650., 412.)],
        color: (0.96, 0.29, 0.15),
        width: 5.,
        text: String::new(),
        font_size: None,
        visible: true,
        label: "Completion rate".into(),
        clip: None,
        effects: Default::default(),
    });
    d.add(Annotation {
        tool: Tool::Arrow,
        points: vec![Point::new(730., 210.), Point::new(610., 280.)],
        color: (0.96, 0.29, 0.15),
        width: 5.,
        text: String::new(),
        font_size: None,
        visible: true,
        label: "Look here".into(),
        clip: None,
        effects: Default::default(),
    });
    d.add(Annotation {
        tool: Tool::Text,
        points: vec![Point::new(675., 135.)],
        color: (0.13, 0.36, 0.74),
        width: 5.,
        text: "Great progress".into(),
        font_size: Some(26.),
        visible: true,
        label: String::new(),
        clip: None,
        effects: Default::default(),
    });
    e.set_document(d, "Every annotation is an editable layer");
    tool(e, Tool::Move);
    settle(100);
}

#[test]
#[ignore = "requires a private GTK display; generates documentation GIF frames"]
fn record_feature_showcase() {
    assert_eq!(
        crate::i18n::language(),
        "en",
        "documentation recordings must be English"
    );
    gtk::init().unwrap();
    let settings = gtk::Settings::default().unwrap();
    settings.set_property("gtk-theme-name", "Yaru");
    settings.set_property("gtk-application-prefer-dark-theme", false);
    let app = gtk::Application::new(
        Some("io.github.untushot.Showcase"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    app.register(gio::Cancellable::NONE).unwrap();
    let e = build(&app, Args::parse_from(["untushot", "--demo"]));
    reset(&e);
    let output = tempfile::tempdir().unwrap();
    if std::env::var("UNTUSHOT_SHOWCASE_SECTION").as_deref() == Ok("menus") {
        menu_features(&e, &app);
        e.mark_project_saved();
        e.window.close();
        return;
    }
    // Basic tools, all using the production gesture handlers.
    let mut r = Recorder::new("01-annotate", "Make the important part impossible to miss.");
    r.shot(&e, "A clean screenshot. Seven ways to explain it.", 1.4);
    draw_gesture(
        &e,
        &mut r,
        Tool::Rectangle,
        (343., 249.),
        (650., 412.),
        "Rectangle · Frame the result",
    );
    draw_gesture(
        &e,
        &mut r,
        Tool::Arrow,
        (760., 200.),
        (620., 282.),
        "Arrow · Point to the detail",
    );
    draw_gesture(
        &e,
        &mut r,
        Tool::Ellipse,
        (38., 252.),
        (337., 407.),
        "Ellipse · Circle a key number",
    );
    e.state.borrow_mut().color = (0.95, 0.71, 0.08);
    draw_gesture(
        &e,
        &mut r,
        Tool::Highlight,
        (57., 485.),
        (460., 513.),
        "Highlighter · Keep the words readable",
    );
    e.state.borrow_mut().color = (0.15, 0.4, 0.82);
    draw_gesture(
        &e,
        &mut r,
        Tool::Line,
        (56., 546.),
        (480., 546.),
        "Line · Underline the next step",
    );
    tool(&e, Tool::Pen);
    let (x, y) = canvas(&e, 680., 455.);
    e.press(x, y, false);
    for i in 0..16 {
        let (x, y) = canvas(
            &e,
            680. + i as f64 * 13.,
            455. + (i as f64 * 0.6).sin() * 14.,
        );
        e.motion(x, y);
        r.shot(&e, "Pen · Draw freely", 0.1);
    }
    let (x, y) = canvas(&e, 875., 461.);
    e.release(x, y);
    r.shot(&e, "Pen · Draw freely", 1.1);
    draw_gesture(
        &e,
        &mut r,
        Tool::Redact,
        (130., 551.),
        (364., 578.),
        "Redact · Opaque pixels in the exported image",
    );
    r.finish();
    reset(&e);
    let mut r = Recorder::new("02-crop", "Choose exactly what you want to share.");
    r.shot(&e, "Crop without changing the original image", 1.2);
    draw_gesture(
        &e,
        &mut r,
        Tool::Select,
        (28., 240.),
        (972., 420.),
        "Drag a crop · Eight handles keep it precise",
    );
    e.confirm_action();
    r.shot(&e, "Enter · Fit the selected area into the editor", 1.6);
    e.history(false);
    r.shot(&e, "Ctrl+Z · Bring the complete image back", 1.5);
    r.finish();
    reset(&e);
    let mut r = Recorder::new("03-detail", "Lift a detail. Give it room to shine.");
    r.shot(&e, "M · Select pixels while keeping your export area", 1.1);
    draw_gesture(
        &e,
        &mut r,
        Tool::Marquee,
        (351., 258.),
        (642., 403.),
        "Select the metric you want to explain",
    );
    e.selection_to_layer();
    r.shot(&e, "Ctrl+J · Copy the selection into its own layer", 1.5);
    e.properties[2].set_value(500.);
    e.properties[0].set_value(245.);
    e.properties[1].set_value(340.);
    e.commit_pending_edits();
    r.shot(
        &e,
        "Resize proportionally · The original stays in place",
        1.5,
    );
    e.border_enabled.set_active(true);
    e.border_width.set_value(5.);
    e.shadow_enabled.set_active(true);
    e.shadow_blur.set_value(24.);
    e.shadow_offset.set_value(14.);
    r.shot(&e, "Add a border and a soft shadow", 2.);
    r.finish();
    reset(&e);
    seed(&e);
    let mut r = Recorder::new("04-text", "Edit the words. Keep your place.");
    e.select_new_image(2, "Edit text directly on the canvas");
    e.text(Point::default());
    r.shot(&e, "Double-click a text layer to edit in place", 1.4);
    let buffer = e.inline_input.buffer().unwrap();
    buffer.place_cursor(&buffer.end_iter());
    for word in [" —", " ship", " it!"] {
        buffer.begin_user_action();
        buffer.insert_at_cursor(word);
        buffer.end_user_action();
        r.shot(&e, "Type naturally · Shift+Enter adds a new line", 0.65);
    }
    e.history(false);
    r.shot(&e, "Ctrl+Z · Undo just the last typing step", 1.3);
    e.history(true);
    r.shot(&e, "Ctrl+Shift+Z · Restore text and cursor", 1.3);
    e.font_picker.set_active(true);
    r.point(&e, &e.font_picker);
    r.shot(&e, "Font size · Browse the Sample previews", 1.8);
    e.font_picker.set_active(false);
    e.font_size.set_value(32.);
    e.finish_text(true);
    r.shot(
        &e,
        "Apply · The finished edit is one document undo step",
        1.6,
    );
    r.finish();
    more_features(&e, output.path());
    menu_features(&e, &app);
    e.mark_project_saved();
    e.window.close();
}

#[test]
#[ignore = "requires a private GTK display; run once per supported language"]
fn gtk_localized_ui() {
    gtk::init().unwrap();
    let app = gtk::Application::new(
        Some("io.github.untushot.Localization"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    app.register(gio::Cancellable::NONE).unwrap();
    let e = build(&app, Args::parse_from(["untushot", "--demo"]));
    reset(&e);
    assert_eq!(e.layers_heading.text(), crate::i18n::tr("Ebenen"));
    assert_eq!(
        e.storage_caption.text(),
        crate::i18n::tr("Projekt: ungespeichert")
    );
    seed(&e);
    e.select_new_image(0, "Test");
    e.begin_layer_rename();
    e.layer_name
        .set_text("Meine Ebene / My layer / Mi capa / Mon calque");
    e.finish_layer_rename(true);
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations()[0].label,
        "Meine Ebene / My layer / Mi capa / Mon calque"
    );
    e.file_menu.set_active(true);
    settle(250);
    let surface = ImageSurface::create(
        Format::ARgb32,
        e.window.allocated_width(),
        e.window.allocated_height(),
    )
    .unwrap();
    e.window.draw(&Context::new(&surface).unwrap());
    surface
        .write_to_png(
            &mut fs::File::create(format!("target/locale-{}.png", crate::i18n::language()))
                .unwrap(),
        )
        .unwrap();
    e.file_menu.set_active(false);
    e.mark_project_saved();
    e.window.close();
}

fn children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut result = vec![widget.as_ref().clone()];
    if let Some(container) = widget.as_ref().dynamic_cast_ref::<gtk::Container>() {
        for child in container.children() {
            result.extend(children(&child));
        }
    }
    result
}
fn expand_properties(e: &Editor) {
    if let Some(widget) = e.properties[0].ancestor(gtk::Expander::static_type()) {
        widget
            .downcast::<gtk::Expander>()
            .unwrap()
            .set_expanded(true);
    }
    settle(120);
}
fn more_features(e: &Rc<Editor>, directory: &Path) {
    reset(e);
    seed(e);
    e.select_new_image(0, "Choose a style without leaving the canvas");
    let mut r = Recorder::new("05-style", "Your colors. Your line weight.");
    r.shot(e, "Select a shape to change its appearance", 1.1);
    e.width_picker.set_active(true);
    r.point(e, &e.width_picker);
    r.shot(
        e,
        "Stroke presets · Real thickness previews beside the control",
        1.8,
    );
    crate::stroke::preset(&e.width_picker, 8).emit_clicked();
    r.shot(e, "Pick a preset or type an exact pixel value", 1.2);
    e.color.button.set_active(true);
    r.point(e, &e.color.button);
    r.shot(e, "Colors open right beside the button", 1.7);
    let popup = e.color.button.popover().unwrap();
    let chooser = children(&popup)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::ColorChooserWidget>().ok())
        .unwrap();
    chooser.set_rgba(&gdk::RGBA::new(0.12, 0.55, 0.85, 1.));
    let apply = children(&popup)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.label().as_deref() == Some("Apply"))
        .unwrap();
    apply.emit_clicked();
    r.pointer = None;
    r.shot(e, "Apply · Update the selected object", 1.7);
    r.finish();

    reset(e);
    let image = sample(1, false);
    let index = e
        .state
        .borrow_mut()
        .document
        .as_mut()
        .unwrap()
        .insert_image(
            image,
            Rect::new(460., 340., 460., 294.),
            "Focus card".into(),
        )
        .unwrap();
    e.refresh();
    e.select_new_image(index, "Style an image layer");
    let mut r = Recorder::new("06-effects", "A little depth. A lot more clarity.");
    r.shot(e, "Select an image layer", 1.1);
    e.border_enabled.set_active(true);
    e.border_width.set_value(6.);
    r.point(e, &e.border_enabled);
    r.shot(e, "Border · Adjustable width and color", 1.5);
    e.shadow_enabled.set_active(true);
    r.point(e, &e.shadow_enabled);
    r.shot(e, "Shadow · Give the detail a clear edge", 1.5);
    for blur in [10., 18., 28., 36.] {
        e.shadow_blur.set_value(blur);
        r.shot(e, "Live controls · Blur, opacity and offset", 0.4);
    }
    e.shadow_opacity.set_value(50.);
    e.shadow_offset.set_value(15.);
    r.pointer = None;
    r.shot(e, "Every style change remains editable", 1.8);
    r.finish();

    reset(e);
    seed(e);
    let mut r = Recorder::new("07-layers", "Every mark is a layer. Every layer is yours.");
    r.shot(
        e,
        "Click to select · Ctrl-click to add or remove a layer",
        1.4,
    );
    layer_drag::showcase_click(e, 0, gdk::ModifierType::empty());
    r.shot(e, "Select one layer", 0.9);
    layer_drag::showcase_click(e, 1, gdk::ModifierType::CONTROL_MASK);
    r.shot(e, "Ctrl-click · Add another layer", 1.2);
    layer_drag::showcase_click(e, 1, gdk::ModifierType::CONTROL_MASK);
    layer_drag::showcase_click(e, 2, gdk::ModifierType::SHIFT_MASK);
    r.shot(e, "Shift-click · Select a contiguous range", 1.2);
    e.key_input(gdk::keys::constants::Escape, gdk::ModifierType::empty());
    r.shot(e, "Esc · Clear the selection", 1.0);
    layer_drag::showcase_click(e, 0, gdk::ModifierType::empty());
    e.begin_layer_rename();
    r.point(e, &e.layer_name);
    r.shot(e, "Double-click or F2 · Rename directly in the list", 1.4);
    e.layer_name.set_text("Our strongest result");
    e.layer_name.emit_activate();
    r.pointer = None;
    r.shot(e, "Enter · Keep the new name", 1.1);
    layer_drag::showcase_reorder(e, || {
        r.shot(
            e,
            "Drag to reorder · The insertion line shows where it will land",
            0.12,
        )
    });
    r.shot(e, "Drop · One complete undo step", 1.3);
    let eye = e.layer_rows.borrow()[0].visibility.clone();
    eye.emit_clicked();
    r.point(e, &eye);
    r.shot(e, "Eye icon · Hide a layer without deleting it", 1.4);
    eye.emit_clicked();
    r.shot(e, "Show it again · All content is preserved", 1.2);
    r.finish();

    reset(e);
    seed(e);
    let mut r = Recorder::new("08-groups", "Move and align objects together.");
    layer_drag::showcase_click(e, 0, gdk::ModifierType::empty());
    layer_drag::showcase_click(e, 2, gdk::ModifierType::CONTROL_MASK);
    r.shot(e, "Ctrl-click · Select multiple layers", 1.5);
    e.duplicate.emit_clicked();
    r.point(e, &e.duplicate);
    r.shot(e, "Ctrl+D · Duplicate the selected objects together", 1.4);
    e.align_layers(Alignment::Left);
    r.shot(e, "Align left · Keep your layout consistent", 1.4);
    e.nudge(0., 100., false);
    r.pointer = None;
    r.shot(e, "Move the group · Relative positions stay intact", 1.4);
    e.delete.emit_clicked();
    r.shot(e, "Delete a group in one action", 1.1);
    e.history(false);
    r.shot(e, "Ctrl+Z · Restore the whole group", 1.5);
    r.finish();

    reset(e);
    seed(e);
    e.select_new_image(0, "Position and size");
    expand_properties(e);
    let mut r = Recorder::new("09-precision", "Place it exactly where you mean.");
    r.shot(e, "Position and size · Editable numeric fields", 1.4);
    for x in [360., 380., 400.] {
        e.properties[0].set_value(x);
        r.point(e, &e.properties[0]);
        r.shot(e, "X / Y · Immediate visual feedback", 0.45);
    }
    e.properties[2].set_value(380.);
    r.point(e, &e.properties[2]);
    r.shot(e, "Width / height · Work in original image pixels", 1.6);
    e.commit_pending_edits();
    let a = e.state.borrow().document.as_ref().unwrap().annotations()[0].bounds();
    draw_gesture(
        e,
        &mut r,
        Tool::Move,
        (a.right(), a.bottom()),
        (a.right() + 70., a.bottom() + 35.),
        "Or resize with the eight on-canvas handles",
    );
    r.finish();

    reset(e);
    seed(e);
    e.snapping.set_active(true);
    e.select_new_image(2, "Snap to object edges and centers");
    let mut r = Recorder::new("10-snapping", "Let the guides do the lining up.");
    r.point(e, &e.snapping);
    r.shot(e, "S · Enable snapping to edges and centers", 1.5);
    let b = e.state.borrow().document.as_ref().unwrap().annotations()[2].bounds();
    let (x, y) = canvas(e, b.x + b.w / 2., b.y + b.h / 2.);
    e.press(x, y, false);
    for i in 1..=12 {
        let (px, py) = canvas(
            e,
            b.x + b.w / 2. - (b.x - 343.) * i as f64 / 12.,
            b.y + b.h / 2. + 80. * i as f64 / 12.,
        );
        e.motion(px, py);
        r.pointer = None;
        r.shot(
            e,
            "Alignment guides appear as the object snaps into place",
            0.13,
        );
    }
    r.shot(
        e,
        "Guides are only editing aids · They never appear in your export",
        1.7,
    );
    e.cancel_action();
    e.snapping.set_active(false);
    r.shot(e, "Alt while dragging temporarily bypasses snapping", 1.4);
    r.finish();

    reset(e);
    seed(e);
    let mut r = Recorder::new("11-workspace", "Room to work. Precision when you need it.");
    r.shot(e, "Fit the image to your workspace", 1.1);
    for _ in 0..5 {
        e.zoom(Some(1.22));
        r.shot(e, "Ctrl+wheel · Zoom around your point of interest", 0.2);
    }
    {
        e.state.borrow_mut().space = true;
    }
    let (x, y) = (
        e.area.allocated_width() as f64 / 2.,
        e.area.allocated_height() as f64 / 2.,
    );
    e.press(x, y, false);
    for i in 1..=8 {
        e.motion(x - 120. * i as f64 / 8., y + 45. * i as f64 / 8.);
        r.shot(e, "Space + drag · Pan without moving any layers", 0.12);
    }
    e.release(x - 120., y + 45.);
    e.state.borrow_mut().space = false;
    e.zoom(None);
    e.show_tools.set_active(false);
    e.show_layers.set_active(false);
    r.shot(e, "Tab · Give the image the whole workspace", 1.6);
    e.show_tools.set_active(true);
    e.show_layers.set_active(true);
    for pos in [820, 780, 740] {
        e.workspace_split.set_position(pos);
        r.shot(e, "Drag the divider · Make room for long layer names", 0.4);
    }
    r.finish();

    reset(e);
    seed(e);
    let mut r = Recorder::new("12-projects", "Save the edit. Not just the pixels.");
    e.file_menu.set_active(true);
    r.point(e, &e.file_menu);
    r.shot(
        e,
        "Save project · Keep the original, crop and every layer",
        1.8,
    );
    e.file_menu.set_active(false);
    let project = directory.join("Product-review.untushot");
    assert!(e.save_project_to(&project));
    r.pointer = None;
    r.shot(e, "A .untushot file keeps your work fully editable", 1.6);
    e.select_new_image(0, "Keep editing");
    e.nudge(40., 0., false);
    r.shot(
        e,
        "The project status tracks changes separately from PNG export",
        1.5,
    );
    e.history(false);
    e.mark_project_saved();
    assert!(e.open_path(&project));
    r.shot(
        e,
        "Open project · All layers return as individual objects",
        1.8,
    );
    e.select_new_image(2, "Editable text restored");
    e.text(Point::default());
    r.shot(e, "Pick up exactly where you left off", 1.8);
    e.finish_text(false);
    r.finish();

    reset(e);
    seed(e);
    let mut r = Recorder::new("13-sharing", "Ready to share. Easy to keep editing.");
    r.point(e, &e.copy);
    r.shot(e, "Copy · Put the rendered image on the clipboard", 1.2);
    e.copy();
    r.shot(e, "Paste into your message, issue or document", 1.6);
    let png = directory.join("Product-review.png");
    *e.export_path.borrow_mut() = Some(png.clone());
    assert!(e.save());
    r.point(e, &e.export_button);
    r.shot(e, "Export PNG · Save the finished result", 1.6);
    assert!(png.is_file());
    e.storage_button.set_active(true);
    r.point(e, &e.storage_button);
    r.shot(
        e,
        "Image, project and autosave have separate status indicators",
        2.,
    );
    e.storage_button.set_active(false);
    r.finish();

    reset(e);
    let mut r = Recorder::new("14-history", "Your recent captures, within reach.");
    let history_dir = directory.join("screenshots");
    fs::create_dir_all(&history_dir).unwrap();
    for i in 0..5 {
        crate::pixels::write_png(
            &sample(i, false),
            fs::File::create(history_dir.join(format!("Review-{:02}.png", i + 1))).unwrap(),
        )
        .unwrap();
    }
    let strip = crate::history::Strip::new(Some(history_dir.clone()));
    strip.attach_to(&e.history_canvas);
    strip.set_available(true);
    strip.refresh();
    settle(600);
    r.shot(
        e,
        "Screenshot history · Browse the latest images below the canvas",
        1.8,
    );
    strip.toggle.set_active(false);
    r.point(e, &strip.toggle);
    r.shot(
        e,
        "Collapse the history without shrinking your layer controls",
        1.4,
    );
    strip.toggle.set_active(true);
    r.shot(
        e,
        "Expand it again · Your place in the list is preserved",
        1.3,
    );
    e.mark_project_saved();
    e.open_path(&history_dir.join("Review-03.png"));
    r.pointer = None;
    r.shot(e, "Open a capture · The original file stays untouched", 1.7);
    strip.root.hide();
    strip.stop();
    e.history_canvas.remove(&strip.root);
    r.finish();

    reset(e);
    let mut r = Recorder::new("15-composition", "Put the whole story in one image.");
    r.shot(e, "Start with one screenshot", 1.2);
    let second = directory.join("Step-02.png");
    crate::pixels::write_png(&sample(1, false), fs::File::create(&second).unwrap()).unwrap();
    e.append_image_path(&second);
    r.shot(e, "Ctrl+I · Append a second screenshot on the right", 1.8);
    let third = directory.join("Step-03.png");
    crate::pixels::write_png(&sample(2, false), fs::File::create(&third).unwrap()).unwrap();
    e.append_image_path(&third);
    r.shot(
        e,
        "The canvas grows · Every image remains its own layer",
        1.8,
    );
    e.history(false);
    r.shot(
        e,
        "Ctrl+Z · Undo the append and canvas expansion together",
        1.4,
    );
    e.history(true);
    r.shot(e, "Redo · Continue building your visual walkthrough", 1.4);
    r.finish();

    reset(e);
    let mut r = Recorder::new("16-web-pages", "Long page. Complete picture.");
    let imported =
        crate::web_import::record_preview(&e.window, &sample(0, true), |dialog, caption| {
            r.widget(dialog, None, caption, 1.2)
        });
    e.set_document(
        Document::new(imported, vec![]).unwrap(),
        "Full-page image imported from the clipboard",
    );
    r.shot(
        e,
        "Open in editor · Keep the original image resolution",
        1.7,
    );
    r.finish();

    reset(e);
    seed(e);
    let mut r = Recorder::new("17-recovery", "Your session has a second chance.");
    let recovery_dir = directory.join("recovery");
    let (session, _) = recovery::Session::open(&recovery_dir, false).unwrap();
    *e.session.borrow_mut() = Some(session);
    e.save_recovery().unwrap();
    e.session.borrow().as_ref().unwrap().flush().unwrap();
    e.poll_recovery();
    e.storage_button.set_active(true);
    r.shot(
        e,
        "Automatically saved · Confirmed only after the write succeeds",
        2.,
    );
    e.storage_button.set_active(false);
    let original = e
        .state
        .borrow()
        .document
        .as_ref()
        .unwrap()
        .annotations()
        .to_vec();
    drop(e.session.borrow_mut().take());
    e.set_document(document(), "Reloading the saved session");
    r.shot(
        e,
        "A recovery file stores the original image and all annotations",
        1.3,
    );
    let (session, restored) = recovery::Session::open(&recovery_dir, true).unwrap();
    let restored = restored.unwrap();
    assert_eq!(restored.annotations(), original);
    e.set_document(
        restored,
        "Session restored · Original image, layers and crop",
    );
    *e.session.borrow_mut() = Some(session);
    r.shot(
        e,
        "Restore the session · Continue editing individual layers",
        1.9,
    );
    e.save_recovery().unwrap();
    e.session.borrow().as_ref().unwrap().flush().unwrap();
    drop(e.session.borrow_mut().take());
    r.finish();

    reset(e);
    seed(e);
    let mut r = Recorder::new("18-appearance", "At home on your desktop.");
    let settings = gtk::Settings::default().unwrap();
    r.shot(e, "Light theme · Native GTK controls", 1.5);
    settings.set_property("gtk-theme-name", "Yaru-dark");
    settle(180);
    r.shot(e, "Dark theme · Follows your desktop appearance", 1.8);
    settings.set_property("gtk-theme-name", "Yaru-sage-dark");
    settle(180);
    r.shot(
        e,
        "System accent · Consistent controls and selection colors",
        1.8,
    );
    settings.set_property("gtk-theme-name", "Yaru");
    settle(180);
    r.shot(
        e,
        "Four interface languages · Automatically follows your system language",
        1.8,
    );
    r.finish();

    reset(e);
    e.mark_project_saved();
    e.show_setup();
    let mut r = Recorder::new("19-capture", "From capture to canvas, in a few steps.");
    r.shot(e, "Region, entire screen or system selection", 1.6);
    e.setup.full.set_active(true);
    r.shot(e, "Entire screen · Start with the full picture", 1.3);
    e.setup.system.set_active(true);
    r.shot(
        e,
        "System selection · Let your desktop handle the choice",
        1.4,
    );
    e.setup.region.set_active(true);
    e.setup.delay.set_value(3.);
    r.point(e, &e.setup.delay);
    r.shot(e, "Set a delay · Give yourself time to prepare", 1.5);
    e.capture();
    settle(100);
    r.pointer = None;
    r.shot(e, "The countdown can be cancelled", 1.2);
    e.cancel_capture();
    r.shot(e, "Cancel · Keep your existing document", 1.4);
    e.accept_capture(
        document(),
        "Demo capture result · No desktop screen was recorded",
        Mode::Region,
        None,
    );
    r.shot(e, "Select the region before entering the editor", 1.3);
    draw_gesture(
        e,
        &mut r,
        Tool::Select,
        (28., 239.),
        (973., 419.),
        "Draw the region · Confirm when it is right",
    );
    e.magnifier.set_active(true);
    let (mx, my) = canvas(e, 520., 320.);
    e.motion(mx, my);
    r.shot(e, "Pixel magnifier · Inspect the original pixels", 1.5);
    e.finish_region();
    r.shot(e, "Enter · Ready to annotate", 1.5);
    r.finish();

    reset(e);
    let mut d = document();
    d.monitors = vec![
        crate::geometry::Monitor {
            name: "Left display".into(),
            rect: Rect::new(0., 0., 500., 640.),
            scale: 1.,
        },
        crate::geometry::Monitor {
            name: "Right display".into(),
            rect: Rect::new(500., 0., 500., 640.),
            scale: 1.,
        },
    ];
    e.set_document(d, "Illustrated monitor layout · Synthetic demo");
    let mut r = Recorder::new("20-monitors", "One canvas. More than one display.");
    e.monitor_guides.set_active(true);
    r.shot(
        e,
        "Monitor guides show the boundaries in the combined image",
        1.5,
    );
    e.monitor_button.set_active(true);
    r.point(e, &e.monitor_button);
    r.shot(e, "Choose a monitor from the visual overview", 1.7);
    e.monitor_button.set_active(false);
    e.monitors.set_active_id(Some("0"));
    r.shot(e, "Choose one display as the export area", 1.4);
    e.monitors.set_active_id(Some("all"));
    r.pointer = None;
    draw_gesture(
        e,
        &mut r,
        Tool::Select,
        (280., 230.),
        (760., 430.),
        "Or crop across display boundaries",
    );
    e.finish_region();
    r.shot(e, "One continuous, editable crop", 1.6);
    r.finish();
}

fn menu_features(e: &Rc<Editor>, app: &gtk::Application) {
    reset(e);
    let menu = crate::runtime::showcase_menu(app);
    menu.set_property("tearoff-state", true);
    menu.show_all();
    settle(120);
    let mut r = Recorder::new("21-tray", "Capture again without reopening the editor.");
    r.widget(
        &menu,
        None,
        "Application tray menu · Actual menu preview, not a desktop recording",
        1.8,
    );
    let items = menu.children();
    menu.select_item(&items[2]);
    r.widget(
        &menu,
        None,
        "Region capture · Also available with the desktop shortcut Ctrl+Shift+X",
        1.8,
    );
    menu.select_item(&items[3]);
    r.widget(
        &menu,
        None,
        "Delayed capture · Three seconds to prepare",
        1.8,
    );
    menu.select_item(items.last().unwrap());
    r.widget(
        &menu,
        None,
        "Quit UntuShot · Save the session before leaving",
        1.7,
    );
    menu.set_property("tearoff-state", false);
    menu.popdown();
    r.finish();

    reset(e);
    seed(e);
    let mut r = Recorder::new(
        "22-object-actions",
        "The right action, right at the object.",
    );
    e.select_new_image(0, "Right-click an object");
    let (x, y) = canvas(e, 350., 260.);
    let menu = e.menu_at_object(x, y).unwrap();
    menu.set_property("tearoff-state", true);
    menu.show_all();
    settle(100);
    r.widget(
        &menu,
        None,
        "Right-click · Duplicate, reorder or delete the selected object",
        1.8,
    );
    menu.set_property("tearoff-state", false);
    menu.popdown();
    e.duplicate.emit_clicked();
    r.shot(
        e,
        "Duplicate · The copy remains independently editable",
        1.6,
    );
    e.history(false);
    r.shot(e, "Undo · Return to the previous arrangement", 1.6);
    r.finish();
    reset(e);
    {
        let mut state = e.state.borrow_mut();
        let doc = state.document.as_mut().unwrap();
        doc.insert_image(
            sample(1, false),
            Rect::new(520., 360., 400., 256.),
            "Highlighted screenshot detail".into(),
        )
        .unwrap();
        for i in 0..24 {
            doc.add(Annotation {
                tool: Tool::Rectangle,
                points: vec![
                    Point::new(40. + (i % 6) as f64 * 140., 260. + (i / 6) as f64 * 70.),
                    Point::new(160. + (i % 6) as f64 * 140., 310. + (i / 6) as f64 * 70.),
                ],
                color: (0.2, 0.45, 0.8),
                width: 2.,
                text: String::new(),
                font_size: None,
                visible: true,
                label: format!("Review detail {:02} — an editable annotation", i + 1),
                clip: None,
                effects: Default::default(),
            });
        }
    }
    e.refresh();
    e.select_new_image(0, "Many layers, all editable");
    e.border_enabled.set_active(true);
    e.shadow_enabled.set_active(true);
    let mut r = Recorder::new("23-layer-space", "Keep the controls. Scroll the layers.");
    r.shot(e, "25 layers · Image effects keep the room they need", 1.6);
    let adjustment = e.layer_scroll.vadjustment();
    adjustment.set_value(0.);
    r.shot(
        e,
        "The layer list scrolls independently above the properties",
        1.4,
    );
    for i in 1..=12 {
        adjustment.set_value((adjustment.upper() - adjustment.page_size()) * i as f64 / 12.);
        r.shot(
            e,
            "Browse the stack without losing your effects controls",
            0.12,
        );
    }
    e.window.resize(1080, 720);
    r.shot(
        e,
        "Compact window · The selected layer stays reachable",
        1.7,
    );
    e.window.resize(1180, 820);
    r.shot(e, "Resize the workspace to fit the task", 1.4);
    r.finish();
}
