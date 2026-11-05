//! In-list pointer gestures. A held press shows destinations; only a completed drag
//! changes the document. No platform drag-and-drop transport is involved.
use super::*;

pub(super) struct Session {
    id: u64,
    index: usize,
    revision: u64,
    epoch: u64,
    start: (f64, f64),
    pointer: (f64, f64),
    dragging: bool,
    guides: bool,
    pressed_at: std::time::Instant,
    target: Option<usize>,
    last_frame: Option<i64>,
}

fn valid(e: &Editor) -> bool {
    let drag = e.layer_drag.borrow();
    let Some(drag) = drag.as_ref() else {
        return false;
    };
    let s = e.state.borrow();
    !s.capturing
        && !s.selecting_region
        && e.clipboard_epoch.get() == drag.epoch
        && s.document
            .as_ref()
            .is_some_and(|d| d.revision() == drag.revision)
}

fn clear_marker(e: &Editor) {
    let changed = e
        .layer_drag
        .borrow_mut()
        .as_mut()
        .is_some_and(|d| d.target.take().is_some());
    if changed {
        e.layers.queue_draw();
    }
}

pub(super) fn cancel(e: &Editor) {
    if e.layer_drag.borrow_mut().take().is_some() {
        e.layers.queue_draw();
        if let Some(window) = e.layer_scroll.window() {
            window.set_cursor(None);
        }
    }
}

fn inside(e: &Editor, (x, y): (f64, f64)) -> bool {
    x >= 0.
        && y >= 0.
        && x < f64::from(e.layer_scroll.allocated_width())
        && y < f64::from(e.layer_scroll.allocated_height())
}

// Each gap has exactly one position, halfway between adjacent rows. The same
// gap must not jump between the preceding row's bottom and the next row's top.
fn gaps(e: &Editor) -> Vec<(usize, f64)> {
    let rows: Vec<_> = e
        .layers
        .children()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::ListBoxRow>().ok())
        .collect();
    let Some(first) = rows.first() else {
        return vec![];
    };
    let Some((_, offset)) = e.layers.translate_coordinates(&e.layer_scroll, 0, 0) else {
        return vec![];
    };
    let mut result = vec![(rows.len(), f64::from(first.allocation().y() + offset))];
    for (i, row) in rows.iter().enumerate() {
        let r = row.allocation();
        let bottom = r.y() + r.height();
        let y = rows.get(i + 1).map_or(f64::from(bottom), |next| {
            f64::from(bottom + next.allocation().y()) / 2.
        });
        result.push((rows.len() - i - 1, y + f64::from(offset)));
    }
    result
}

#[cfg(test)]
fn destination(e: &Editor, y: i32) -> Option<(usize, f64)> {
    gaps(e).into_iter().min_by(|a, b| {
        (a.1 - f64::from(y))
            .abs()
            .total_cmp(&(b.1 - f64::from(y)).abs())
    })
}

// A small dead band prevents pointer noise at a row midpoint from alternating
// two targets. The visible target is also the target used on release.
fn stable_target(gaps: &[(usize, f64)], previous: Option<usize>, y: f64) -> Option<usize> {
    let nearest = gaps
        .iter()
        .min_by(|a, b| (a.1 - y).abs().total_cmp(&(b.1 - y).abs()))?;
    if let Some(old) = gaps.iter().find(|(index, _)| Some(*index) == previous) {
        if (old.1 - y).abs() <= (nearest.1 - y).abs() + 6. {
            return Some(old.0);
        }
    }
    Some(nearest.0)
}

fn paint(e: &Editor, cr: &cairo::Context) -> Result<()> {
    let drag = e.layer_drag.borrow();
    let Some(drag) = drag.as_ref() else {
        return Ok(());
    };
    if !drag.guides {
        return Ok(());
    }
    let palette = crate::theme::Palette::for_widget(&e.layers);
    let right = f64::from(
        e.layers
            .allocated_width()
            .min(e.layer_scroll.allocated_width()),
    ) - 8.;
    let height = f64::from(e.layer_scroll.allocated_height());
    if height <= 2. || right <= 8. {
        return Ok(());
    }
    for (index, y) in gaps(e) {
        if y < 0. || y > height {
            continue;
        }
        let y = y.round().clamp(1., height - 1.) + 0.5;
        let active = drag.target == Some(index);
        crate::theme::source(
            cr,
            if active {
                &palette.accent
            } else {
                &palette.foreground
            },
            if active { 1. } else { 0.25 },
        );
        cr.set_line_width(if active { 3. } else { 1. });
        cr.set_line_cap(cairo::LineCap::Round);
        cr.move_to(8., y);
        cr.line_to(right, y);
        cr.stroke()?;
    }
    Ok(())
}

fn hit(e: &Editor, x: f64, y: f64) -> Option<usize> {
    if !inside(e, (x, y)) {
        return None;
    }
    if let Some(bar) = e.layer_scroll.vscrollbar().filter(|b| b.is_visible()) {
        if let Some((bx, by)) = bar.translate_coordinates(&e.layer_scroll, 0, 0) {
            if x >= f64::from(bx)
                && x < f64::from(bx + bar.allocated_width())
                && y >= f64::from(by)
                && y < f64::from(by + bar.allocated_height())
            {
                return None;
            }
        }
    }
    let (lx, ly) = e
        .layer_scroll
        .translate_coordinates(&e.layers, x as i32, y as i32)?;
    let row = e.layers.row_at_y(ly)?;
    let index = row.widget_name().parse::<usize>().ok()?;
    let rows = e.layer_rows.borrow();
    if e.layer_name.is_mapped() {
        if let Some((ex, ey)) = e.layer_name.translate_coordinates(&e.layers, 0, 0) {
            if lx >= ex
                && lx < ex + e.layer_name.allocated_width()
                && ly >= ey
                && ly < ey + e.layer_name.allocated_height()
            {
                return None;
            }
        }
    }
    let controls = rows.get(index)?;
    for button in [&controls.visibility, &controls.lock] {
        let (bx, by) = button.translate_coordinates(&e.layers, 0, 0)?;
        if lx >= bx
            && lx < bx + button.allocated_width()
            && ly >= by
            && ly < by + button.allocated_height()
        {
            return None;
        }
    }
    Some(index)
}

fn mark(e: &Editor, y: i32) {
    let positions = gaps(e);
    let mut drag = e.layer_drag.borrow_mut();
    let Some(drag) = drag.as_mut() else {
        return;
    };
    let target = stable_target(&positions, drag.target, f64::from(y));
    if drag.target != target {
        drag.target = target;
        e.layers.queue_draw();
    }
}

fn scroll_speed(height: f64, y: f64) -> f64 {
    let edge = (height / 3.).clamp(1., 36.);
    if y < edge {
        -360. * ((edge - y) / edge).clamp(0., 1.).powi(2)
    } else if y > height - edge {
        360. * ((y - height + edge) / edge).clamp(0., 1.).powi(2)
    } else {
        0.
    }
}

fn scroll(e: &Editor, y: i32, seconds: f64) {
    let delta = scroll_speed(f64::from(e.layer_scroll.allocated_height()), f64::from(y))
        * seconds.clamp(0., 0.05);
    if delta != 0. {
        let a = e.layer_scroll.vadjustment();
        let value =
            (a.value() + delta).clamp(a.lower(), (a.upper() - a.page_size()).max(a.lower()));
        if a.value() != value {
            a.set_value(value);
        }
    }
    mark(e, y);
}

fn select(e: &Editor, index: usize, modifiers: gdk::ModifierType) {
    e.tools[0].set_active(true);
    let mut s = e.state.borrow_mut();
    let mut indices = s.indices();
    if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        let anchor = s.selected.unwrap_or(index);
        let range = anchor.min(index)..=anchor.max(index);
        if !modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
            indices.clear();
        }
        indices.extend(range);
        s.selected = Some(anchor);
    } else if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
        if indices.contains(&index) {
            indices.retain(|i| *i != index);
            s.selected = indices.first().copied();
        } else {
            indices.push(index);
            s.selected = Some(index);
        }
    } else {
        indices = vec![index];
        s.selected = Some(index);
    }
    indices.sort_unstable_by(|a, b| b.cmp(a));
    indices.dedup();
    s.selected_many = indices;
    s.pixel_selection = None;
    s.draft = None;
    s.drag = None;
    s.crop_drag = None;
    drop(s);
    e.refresh();
    e.area.grab_focus();
}

fn press(e: &Editor, count: i32, x: f64, y: f64, modifiers: gdk::ModifierType) -> bool {
    cancel(e);
    let Some(index) = hit(e, x, y) else {
        return false;
    };
    if e.state.borrow().capturing || e.state.borrow().selecting_region {
        return false;
    }
    e.commit_pending_edits();
    let extending =
        modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK);
    if count == 2 && !extending {
        e.select_new_image(
            index,
            crate::i18n::tr("Ebenenname ändern · Enter bestätigen · Esc abbrechen"),
        );
        e.begin_layer_rename();
        return true;
    }
    if extending {
        select(e, index, modifiers);
        return true;
    }
    let s = e.state.borrow();
    let Some(doc) = s
        .document
        .as_ref()
        .filter(|d| index < d.annotations().len())
    else {
        return false;
    };
    if doc.annotations()[index].locked {
        drop(s);
        select(e, index, modifiers);
        return true;
    }
    let id = e.layer_drag_serial.get().wrapping_add(1);
    e.layer_drag_serial.set(id);
    *e.layer_drag.borrow_mut() = Some(Session {
        id,
        index,
        revision: doc.revision(),
        epoch: e.clipboard_epoch.get(),
        start: (x, y),
        pointer: (x, y),
        dragging: false,
        guides: false,
        pressed_at: std::time::Instant::now(),
        target: None,
        last_frame: None,
    });
    drop(s);
    select(e, index, modifiers);
    e.reveal_layer.set(false);
    let weak = e.weak_self.clone();
    e.layer_scroll.add_tick_callback(move |_, clock| {
        let Some(e) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if e.layer_drag.borrow().as_ref().map(|d| d.id) != Some(id) {
            return glib::ControlFlow::Break;
        }
        if !valid(&e) {
            cancel(&e);
            return glib::ControlFlow::Break;
        }
        let (point, seconds) = {
            let mut drag = e.layer_drag.borrow_mut();
            let d = drag.as_mut().unwrap();
            if !d.guides && d.pressed_at.elapsed() >= Duration::from_millis(150) {
                d.guides = true;
                e.layers.queue_draw();
            }
            let now = clock.frame_time();
            let seconds = d
                .last_frame
                .replace(now)
                .map_or(0., |before| (now - before) as f64 / 1_000_000.);
            (d.dragging.then_some(d.pointer), seconds)
        };
        if let Some(point) = point.filter(|p| inside(&e, *p)) {
            scroll(&e, point.1 as i32, seconds);
        }
        glib::ControlFlow::Continue
    });
    true
}

fn motion(e: &Editor, x: f64, y: f64) -> bool {
    if !valid(e) {
        cancel(e);
        return false;
    }
    let mut drag = e.layer_drag.borrow_mut();
    let d = drag.as_mut().unwrap();
    d.pointer = (x, y);
    if !d.dragging
        && e.layer_scroll.drag_check_threshold(
            d.start.0 as i32,
            d.start.1 as i32,
            x as i32,
            y as i32,
        )
    {
        d.dragging = true;
        d.guides = true;
        e.layers.queue_draw();
        if let Some(window) = e.layer_scroll.window() {
            window
                .set_cursor(gdk::Cursor::from_name(&e.layer_scroll.display(), "grabbing").as_ref());
        }
    }
    let dragging = d.dragging;
    drop(drag);
    if dragging && inside(e, (x, y)) {
        mark(e, y as i32);
    } else {
        clear_marker(e);
    }
    dragging
}

fn release(e: &Editor, x: f64, y: f64) {
    let pending = if valid(e) && inside(e, (x, y)) {
        mark(e, y as i32);
        e.layer_drag
            .borrow()
            .as_ref()
            .filter(|d| d.dragging)
            .and_then(|d| d.target.map(|gap| (d.index, gap, d.revision, d.epoch)))
    } else {
        None
    };
    cancel(e);
    if let Some((index, gap, revision, epoch)) = pending {
        e.drop_layer(index, gap, revision, epoch);
    }
}

pub(super) fn connect(editor: &Rc<Editor>) {
    let weak = editor.weak_self.clone();
    // Paint on the list's own surface, after its rows. An
    // additional DrawingArea can own an input window and intercept row clicks,
    // even inside a pass-through overlay.
    editor.layers.connect_local("draw", true, move |values| {
        if let Some(e) = weak.upgrade() {
            let cr = values[1].get::<cairo::Context>().expect("draw context");
            if cr.save().is_ok() {
                if let Some((x, y)) = e.layers.translate_coordinates(&e.layer_scroll, 0, 0) {
                    cr.translate(-f64::from(x), -f64::from(y));
                    let _ = paint(&e, &cr);
                }
                let _ = cr.restore();
            }
        }
        Some(false.to_value())
    });
    let weak = editor.weak_self.clone();
    editor
        .layer_scroll
        .vadjustment()
        .connect_value_changed(move |_| {
            if let Some(e) = weak.upgrade() {
                if e.layer_drag.borrow().is_some() {
                    e.layers.queue_draw();
                }
            }
        });
    let drag = gtk::GestureDrag::new(&editor.layer_scroll);
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = editor.weak_self.clone();
    drag.connect_drag_update(move |g, dx, dy| {
        if let (Some(e), Some((x, y))) = (weak.upgrade(), g.start_point()) {
            if motion(&e, x + dx, y + dy) {
                g.set_state(gtk::EventSequenceState::Claimed);
            }
        }
    });
    let weak = editor.weak_self.clone();
    drag.connect_drag_end(move |g, dx, dy| {
        if let Some(e) = weak.upgrade() {
            if let Some((x, y)) = g.start_point() {
                release(&e, x + dx, y + dy);
            } else {
                cancel(&e);
            }
        }
    });
    let weak = editor.weak_self.clone();
    drag.connect_cancel(move |_, _| {
        if let Some(e) = weak.upgrade() {
            cancel(&e);
        }
    });
    let clicks = gtk::GestureMultiPress::new(&editor.layer_scroll);
    clicks.group_with(&drag);
    clicks.set_button(1);
    clicks.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = editor.weak_self.clone();
    clicks.connect_pressed(move |g, count, x, y| {
        if let Some(e) = weak.upgrade() {
            let modifiers = gtk::current_event_state().unwrap_or_else(gdk::ModifierType::empty);
            // Own the whole row interaction: GTK's native multiple selection
            // would otherwise add/toggle rows a second time.
            if press(&e, count, x, y, modifiers) {
                g.set_state(gtk::EventSequenceState::Claimed);
            }
        }
    });
    let weak = editor.weak_self.clone();
    clicks.connect_released(move |_, _, x, y| {
        if let Some(e) = weak.upgrade() {
            release(&e, x, y);
        }
    });
    *editor.layer_gestures.borrow_mut() = vec![drag.upcast(), clicks.upcast()];
    let weak = editor.weak_self.clone();
    editor.window.connect_focus_out_event(move |_, _| {
        if let Some(e) = weak.upgrade() {
            cancel(&e);
        }
        glib::Propagation::Proceed
    });
}
impl Editor {
    fn drop_layer(&self, index: usize, gap: usize, revision: u64, epoch: u64) -> bool {
        let mut s = self.state.borrow_mut();
        if s.capturing || s.selecting_region || epoch != self.clipboard_epoch.get() {
            return false;
        }
        let Some(doc) = s.document.as_mut().filter(|d| d.revision() == revision) else {
            return false;
        };
        let Some(target) = doc.move_layer(index, gap) else {
            return false;
        };
        if target == index {
            return true;
        }
        s.selected = Some(target);
        s.selected_many = vec![target];
        self.property_edit.set(None);
        drop(s);
        // Keep the existing widgets allocated: changing the stack only updates
        // their content and selection, without an empty-list/layout flash.
        self.refresh();
        self.status.set_text(crate::i18n::tr(
            "Ebenenreihenfolge geändert · Strg+Z: rückgängig",
        ));
        true
    }
}

#[cfg(test)]
pub(super) fn verify_locked_row(e: &Editor, index: usize) {
    let row = e.layer_rows.borrow()[index].row.clone();
    let (x, y) = row
        .translate_coordinates(&e.layer_scroll, 30, row.allocated_height() / 2)
        .unwrap();
    assert!(press(
        e,
        1,
        f64::from(x),
        f64::from(y),
        gdk::ModifierType::empty()
    ));
    assert_eq!(e.state.borrow().selected, Some(index));
    assert!(
        e.layer_drag.borrow().is_none(),
        "a locked row must not arm a drag"
    );
    assert!(press(
        e,
        2,
        f64::from(x),
        f64::from(y),
        gdk::ModifierType::empty()
    ));
    assert!(e.layer_rename.get().is_none());
    let controls = e.layer_rows.borrow();
    for button in [&controls[index].lock, &controls[index].visibility] {
        let (x, y) = button
            .translate_coordinates(
                &e.layer_scroll,
                button.allocated_width() / 2,
                button.allocated_height() / 2,
            )
            .unwrap();
        assert_eq!(
            hit(e, f64::from(x), f64::from(y)),
            None,
            "row gestures must leave buttons clickable"
        );
    }
}

#[cfg(test)]
pub(super) fn verify(e: &Rc<Editor>) {
    let settle = || {
        let deadline = std::time::Instant::now() + Duration::from_millis(150);
        while std::time::Instant::now() < deadline {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    };
    settle();
    selection_checks(e);
    e.select_new_image(0, "Auswahl geprüft");
    settle();
    let original = e
        .state
        .borrow()
        .document
        .as_ref()
        .unwrap()
        .annotations()
        .to_vec();
    let revision = e.state.borrow().document.as_ref().unwrap().revision();
    let epoch = e.clipboard_epoch.get();
    let count = original.len();
    let adjustment = e.layer_scroll.vadjustment();
    adjustment.set_value(0.);
    settle();
    assert_eq!(destination(e, 0).unwrap().0, count);
    let top = e.layers.row_at_index(0).unwrap();
    let (_, bottom_half) = top
        .translate_coordinates(&e.layer_scroll, 0, top.allocated_height() - 2)
        .unwrap();
    assert_eq!(destination(e, bottom_half).unwrap().0, count - 1);
    for _ in 0..200 {
        scroll(e, e.layer_scroll.allocated_height() - 1, 1. / 30.);
    }
    assert!(adjustment.value() > 0.);
    assert_eq!(
        destination(e, e.layer_scroll.allocated_height() - 1)
            .unwrap()
            .0,
        0
    );
    for _ in 0..200 {
        scroll(e, 0, 1. / 30.);
    }
    assert_eq!(adjustment.value(), 0.);
    // Equal elapsed time produces equal travel, independent of display refresh rate.
    let edge_y = e.layer_scroll.allocated_height() - 12;
    for _ in 0..60 {
        scroll(e, edge_y, 1. / 60.);
    }
    let distance_60 = adjustment.value();
    adjustment.set_value(0.);
    for _ in 0..120 {
        scroll(e, edge_y, 1. / 120.);
    }
    assert!((adjustment.value() - distance_60).abs() < 0.01);
    adjustment.set_value(0.);
    // Dispatch button/motion events through GTK, including capture-phase gestures.
    // This catches wiring failures that direct calls to drop_layer cannot detect.
    let row = e.layers.row_at_index(0).unwrap();
    let (x, y) = row
        .translate_coordinates(&e.layer_scroll, 30, row.allocated_height() / 2)
        .unwrap();
    let previous_rows = e.layers.children();
    let previous_allocations: Vec<_> = previous_rows.iter().map(|r| r.allocation()).collect();
    pointer(e, gdk::EventType::ButtonPress, x, y, 1000);
    assert!(
        e.layer_drag.borrow().is_some(),
        "mouse-down must arm the drag gesture"
    );
    assert!(e.layer_scroll.is_mapped());
    let image = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        e.window.allocated_width(),
        e.window.allocated_height(),
    )
    .unwrap();
    settle();
    settle();
    assert!(
        e.layer_drag.borrow().as_ref().unwrap().guides,
        "holding a press must reveal the insertion guides"
    );
    assert_eq!(
        e.layers.children(),
        previous_rows,
        "pressing must not replace layer widgets"
    );
    assert_eq!(
        e.layers
            .children()
            .iter()
            .map(|r| r.allocation())
            .collect::<Vec<_>>(),
        previous_allocations,
        "guides must not alter row layout"
    );
    e.window.draw(&cairo::Context::new(&image).unwrap());
    image
        .write_to_png(&mut std::fs::File::create("target/gtk-layer-drop-targets.png").unwrap())
        .unwrap();
    pointer(e, gdk::EventType::ButtonRelease, x, y, 1050);
    assert!(e.layer_drag.borrow().is_none());

    assert_eq!(
        e.state.borrow().selected,
        Some(count - 1),
        "a normal click still selects its row"
    );
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().revision(),
        revision
    );
    // GTK's multi-press timeout uses the main loop, not just event timestamps.
    for _ in 0..3 {
        settle();
    }
    pointer(e, gdk::EventType::ButtonPress, x, y, 2000);
    assert!(
        e.layer_drag.borrow().is_some(),
        "the next independent press must start a drag"
    );
    let bottom = e.layer_scroll.allocated_height() - 2;
    // Exercise the same insertion gap from both sides of the row boundary.
    let boundary = gaps(e)[1].1 as i32;
    pointer(e, gdk::EventType::MotionNotify, x, boundary - 2, 2020);
    let target = e.layer_drag.borrow().as_ref().unwrap().target;
    let render_overlay = || {
        let surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            e.layer_scroll.allocated_width(),
            e.layer_scroll.allocated_height(),
        )
        .unwrap();
        paint(e, &cairo::Context::new(&surface).unwrap()).unwrap();
        let mut bytes = vec![];
        surface.with_data(|v| bytes.extend_from_slice(v)).unwrap();
        bytes
    };
    let stable_pixels = render_overlay();
    for (i, dy) in [-1, 0, 1, 2, 0, -2].into_iter().enumerate() {
        pointer(
            e,
            gdk::EventType::MotionNotify,
            x,
            boundary + dy,
            2021 + i as u32,
        );
        assert_eq!(e.layer_drag.borrow().as_ref().unwrap().target, target);
        assert_eq!(
            render_overlay(),
            stable_pixels,
            "the same gap must not flicker between adjacent row edges"
        );
    }
    pointer(e, gdk::EventType::MotionNotify, x, bottom, 2050);
    assert!(
        e.layer_drag.borrow().as_ref().is_some_and(|d| d.dragging),
        "motion must claim the drag gesture"
    );
    let before_hold = adjustment.value();
    settle();
    assert!(
        adjustment.value() > before_hold,
        "holding at the edge must scroll via the frame clock without further mouse events"
    );
    assert_eq!(e.layers.children(), previous_rows);
    for _ in 0..200 {
        scroll(e, bottom, 1. / 30.);
    }
    pointer(e, gdk::EventType::ButtonRelease, x, bottom, 2100);
    assert!(e.layer_drag.borrow().is_none());
    assert_eq!(
        e.layers.children(),
        previous_rows,
        "dropping must reuse the list widgets"
    );
    assert!(
        render_overlay().iter().all(|b| *b == 0),
        "no drag graphics remain after release"
    );
    settle();
    assert_eq!(e.state.borrow().selected, Some(0));
    assert_eq!(
        e.state
            .borrow()
            .document
            .as_ref()
            .unwrap()
            .annotations()
            .first(),
        original.last()
    );
    assert_eq!(
        e.layers
            .row_at_index(count as i32 - 1)
            .unwrap()
            .widget_name(),
        "0"
    );
    assert!(
        !e.drop_layer(0, count, revision, epoch),
        "stale drag cannot move a replacement layer"
    );
    let revision = e.state.borrow().document.as_ref().unwrap().revision();
    assert!(!e.drop_layer(0, count, revision, epoch.wrapping_add(1)));
    assert!(e.drop_layer(0, 0, revision, epoch));
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().revision(),
        revision
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Reordered.untushot");
    assert!(e.save_project_to(&path));
    let reopened = Document::read_project(std::fs::File::open(path).unwrap()).unwrap();
    assert_eq!(
        reopened.annotations(),
        e.state.borrow().document.as_ref().unwrap().annotations()
    );
    e.history(false);
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations(),
        original
    );
    e.history(true);
    let revision = e.state.borrow().document.as_ref().unwrap().revision();
    assert!(e.drop_layer(0, count, revision, epoch));
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations(),
        original
    );
    e.select_new_image(0, "Ebenenreihenfolge geprüft");
    settle();
    let row = e.layers.row_at_index(count as i32 - 1).unwrap();
    let (x, y) = row
        .translate_coordinates(&e.layer_scroll, 30, row.allocated_height() / 2)
        .unwrap();
    // Escape and releasing outside the list leave the stack untouched.
    for time in [2400, 2700] {
        for _ in 0..3 {
            settle();
        }
        pointer(e, gdk::EventType::ButtonPress, x, y, time);
        pointer(e, gdk::EventType::MotionNotify, x, y - 35, time + 25);
        if time == 2400 {
            e.key_input(gdk::keys::constants::Escape, gdk::ModifierType::empty());
            pointer(e, gdk::EventType::ButtonRelease, x, y - 35, time + 50);
        } else {
            pointer(e, gdk::EventType::ButtonRelease, -20, y, time + 50);
        }
        assert!(e.layer_drag.borrow().is_none());

        assert_eq!(
            e.state.borrow().document.as_ref().unwrap().annotations(),
            original
        );
    }
    for _ in 0..3 {
        settle();
    }
    let name_row_height = row.allocated_height();
    for time in [3000, 3100] {
        pointer(e, gdk::EventType::ButtonPress, x, y, time);
        pointer(e, gdk::EventType::ButtonRelease, x, y, time + 25);
    }
    assert!(
        e.layer_name.is_focus(),
        "double-click must focus the layer name"
    );
    assert_eq!(
        e.layer_name.selection_bounds(),
        Some((0, e.layer_name.text().chars().count() as i32))
    );
    settle();
    assert_eq!(
        row.allocated_height(),
        name_row_height,
        "renaming must not resize the row"
    );
    assert_eq!(
        e.layer_name.parent().as_ref(),
        Some(e.layer_rows.borrow()[0].title.upcast_ref())
    );
    let (ex, ey) = e
        .layer_name
        .translate_coordinates(&e.layer_scroll, 8, e.layer_name.allocated_height() / 2)
        .unwrap();
    pointer(e, gdk::EventType::ButtonPress, ex, ey, 3300);
    pointer(e, gdk::EventType::ButtonRelease, ex, ey, 3325);
    settle();
    assert!(
        e.layer_rename_valid(),
        "clicking in the name must preserve the editing session: focus={:?}, session={:?}, selection={:?}",
        e.window.focused_widget().map(|w| (w.type_().name().to_owned(), w.widget_name().to_string())),
        e.layer_rename.get(), e.state.borrow().indices()
    );
    assert_eq!(e.state.borrow().indices(), vec![0]);
    let snapshot = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        e.window.allocated_width(),
        e.window.allocated_height(),
    )
    .unwrap();
    e.window.draw(&cairo::Context::new(&snapshot).unwrap());
    snapshot
        .write_to_png(&mut std::fs::File::create("target/gtk-inline-layer-name.png").unwrap())
        .unwrap();
    assert!(e.layer_drag.borrow().is_none());
    e.layer_name.set_text("Nicht übernehmen");
    e.key_input(gdk::keys::constants::Escape, gdk::ModifierType::empty());
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations(),
        original
    );
    assert_eq!(e.layer_name.text(), original[0].name());
    e.begin_layer_rename();
    e.layer_name.set_text("Per Doppelklick umbenannt");
    e.layer_name.emit_activate();
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations()[0].name(),
        "Per Doppelklick umbenannt"
    );
    e.history(false);
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations(),
        original
    );
    e.select_new_image(0, "Umbenennen prüfen");
    e.key_input(gdk::keys::constants::F2, gdk::ModifierType::empty());
    assert!(
        e.layer_name.is_focus(),
        "F2 must edit the selected row directly"
    );
    e.layer_name.set_text("Bei Fokuswechsel übernehmen");
    e.area.grab_focus();
    settle();
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations()[0].name(),
        "Bei Fokuswechsel übernehmen"
    );
    e.history(false);
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations(),
        original
    );
    let styled = original.iter().position(|a| a.effects.border).unwrap();
    e.select_new_image(styled, "Veraltete Eingabe prüfen");
    e.begin_layer_rename();
    e.layer_name.set_text("Vor Eigenschaftsänderung übernehmen");
    let border = e.border_width.value();
    e.border_width.grab_focus();
    e.border_width.set_value(border + 1.);
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations()[styled].name(),
        "Vor Eigenschaftsänderung übernehmen"
    );
    e.area.grab_focus();
    e.history(false);
    e.history(false);
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations(),
        original,
        "rename and property change must remain separate undo steps"
    );
    e.select_new_image(0, "Veraltete Eingabe prüfen");
    e.begin_layer_rename();
    e.layer_name
        .set_text("Nicht nach Dokumentänderung übernehmen");
    e.nudge(1., 0., false);
    assert!(e.layer_rename.get().is_none());
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations()[0].name(),
        original[0].name()
    );
    e.history(false);
    e.select_new_image(0, "Ebenenreihenfolge geprüft");
    e.begin_layer_rename();
    e.layer_name.set_text("Vor Werkzeugwechsel übernehmen");
    e.tools[2].set_active(true);
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations()[0].name(),
        "Vor Werkzeugwechsel übernehmen"
    );
    e.history(false);
    assert_eq!(
        e.state.borrow().document.as_ref().unwrap().annotations(),
        original
    );
    e.select_new_image(0, "Ebenenreihenfolge geprüft");
    settle();
}

#[cfg(test)]
fn pointer(e: &Editor, kind: gdk::EventType, x: i32, y: i32, time: u32) {
    pointer_with_modifiers(e, kind, x, y, time, gdk::ModifierType::empty());
}

#[cfg(test)]
fn pointer_with_modifiers(
    e: &Editor,
    kind: gdk::EventType,
    x: i32,
    y: i32,
    time: u32,
    modifiers: gdk::ModifierType,
) {
    use glib::translate::ToGlibPtr;
    thread_local! { static GRAB: RefCell<Option<gdk::Window>> = const { RefCell::new(None) }; }
    let root = e.window.window().unwrap();
    let (mut x, mut y) = e
        .layer_scroll
        .translate_coordinates(&e.window, x, y)
        .unwrap();
    // Resolve the visible input window instead of addressing a layer directly.
    // This detects decorative windows that accidentally intercept mouse input.
    let window = if kind == gdk::EventType::ButtonPress {
        let window = input_window_at(&root, x, y).unwrap();
        GRAB.with(|grab| *grab.borrow_mut() = Some(window.clone()));
        window
    } else {
        GRAB.with(|grab| grab.borrow().clone())
            .unwrap_or_else(|| input_window_at(&root, x, y).unwrap())
    };
    let mut parent = window.clone();
    while parent != root {
        let (dx, dy) = parent.position();
        x -= dx;
        y -= dy;
        parent = parent.parent().unwrap();
    }
    let device = e
        .window
        .display()
        .default_seat()
        .unwrap()
        .pointer()
        .unwrap();
    if kind == gdk::EventType::MotionNotify {
        let mut event = gdk::Event::new(kind)
            .downcast::<gdk::EventMotion>()
            .unwrap();
        event.as_mut().window = window.to_glib_full();
        event.as_mut().x = f64::from(x);
        event.as_mut().y = f64::from(y);
        event.as_mut().state = (modifiers | gdk::ModifierType::BUTTON1_MASK).bits();
        event.as_mut().time = time;
        event.set_device(Some(&device));
        gtk::main_do_event(&mut event);
    } else {
        let mut event = gdk::Event::new(kind)
            .downcast::<gdk::EventButton>()
            .unwrap();
        event.as_mut().window = window.to_glib_full();
        event.as_mut().x = f64::from(x);
        event.as_mut().y = f64::from(y);
        event.as_mut().button = 1;
        event.as_mut().state = modifiers.bits()
            | if kind == gdk::EventType::ButtonRelease {
                gdk::ModifierType::BUTTON1_MASK.bits()
            } else {
                0
            };
        event.as_mut().time = time;
        event.set_device(Some(&device));
        gtk::main_do_event(&mut event);
    }
    if kind == gdk::EventType::ButtonRelease {
        GRAB.with(|grab| grab.borrow_mut().take());
    }
}

#[cfg(test)]
fn input_window_at(window: &gdk::Window, x: i32, y: i32) -> Option<gdk::Window> {
    for child in window.children() {
        let (cx, cy) = child.position();
        if child.is_visible()
            && x >= cx
            && y >= cy
            && x < cx + child.width()
            && y < cy + child.height()
        {
            if let Some(hit) = input_window_at(&child, x - cx, y - cy) {
                return Some(hit);
            }
        }
    }
    (!window.is_pass_through()).then(|| window.clone())
}

#[cfg(test)]
fn selection_checks(e: &Rc<Editor>) {
    let settle = || {
        let until = std::time::Instant::now() + Duration::from_millis(300);
        while std::time::Instant::now() < until {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    };
    let n = e.layers.children().len();
    let click = |visual: i32, modifiers: gdk::ModifierType| {
        e.layer_scroll.vadjustment().set_value(0.);
        settle();
        let row = e.layers.row_at_index(visual).unwrap();
        let (x, y) = row
            .translate_coordinates(&e.layer_scroll, 30, row.allocated_height() / 2)
            .unwrap();
        pointer_with_modifiers(e, gdk::EventType::ButtonPress, x, y, 5000, modifiers);
        assert!(
            e.layer_drag.borrow().as_ref().map_or(true, |d| !d.guides),
            "quick selection presses must not flash insertion lines"
        );
        pointer_with_modifiers(e, gdk::EventType::ButtonRelease, x, y, 5050, modifiers);
        settle();
        assert!(
            e.layer_drag.borrow().is_none(),
            "release must always clear drag feedback"
        );
    };
    click(0, gdk::ModifierType::empty());
    assert_eq!(
        e.state.borrow().indices(),
        vec![n - 1],
        "plain click must select exactly one layer"
    );
    click(1, gdk::ModifierType::empty());
    assert_eq!(
        e.state.borrow().indices(),
        vec![n - 2],
        "plain clicks must not accumulate selected layers"
    );
    click(0, gdk::ModifierType::CONTROL_MASK);
    let mut selected = e.state.borrow().indices();
    selected.sort_unstable();
    assert_eq!(selected, vec![n - 2, n - 1]);
    click(1, gdk::ModifierType::CONTROL_MASK);
    assert_eq!(
        e.state.borrow().indices(),
        vec![n - 1],
        "Ctrl-click must deselect a selected layer"
    );
    click(0, gdk::ModifierType::CONTROL_MASK);
    assert!(
        e.state.borrow().indices().is_empty(),
        "the last selected layer must be deselectable"
    );
    click(0, gdk::ModifierType::empty());
    click(2, gdk::ModifierType::SHIFT_MASK);
    assert_eq!(
        e.state.borrow().indices(),
        vec![n - 1, n - 2, n - 3],
        "Shift-click selects the range from the anchor"
    );
    click(1, gdk::ModifierType::SHIFT_MASK);
    assert_eq!(
        e.state.borrow().indices(),
        vec![n - 1, n - 2],
        "Shift-click can shrink the range"
    );
    e.key_input(gdk::keys::constants::Escape, gdk::ModifierType::empty());
    assert!(e.state.borrow().indices().is_empty());
    assert!(
        e.layers.selected_rows().is_empty(),
        "Escape must also remove GTK selection highlights"
    );
    e.layer_scroll.vadjustment().set_value(0.);
    settle();
    for visual in [0, 1, 0, 1, 0, 1] {
        let row = e.layers.row_at_index(visual).unwrap();
        let (x, y) = row
            .translate_coordinates(&e.layer_scroll, 30, row.allocated_height() / 2)
            .unwrap();
        pointer(e, gdk::EventType::ButtonPress, x, y, 6000);
        pointer(e, gdk::EventType::ButtonRelease, x, y, 6050);
        assert_eq!(
            e.state.borrow().indices(),
            vec![n - 1 - visual as usize],
            "rapid clicks on different rows must remain exclusive"
        );
        assert_eq!(e.layers.selected_rows().len(), 1);
        assert!(e.layer_drag.borrow().is_none());
    }
}

/// Exercise the same pointer handlers as the layer list when recording docs.
#[cfg(test)]
pub(super) fn showcase_click(e: &Editor, index: usize, modifiers: gdk::ModifierType) {
    let row = e.layer_rows.borrow()[index].row.clone();
    let (x, y) = row
        .translate_coordinates(&e.layer_scroll, 12, row.allocated_height() / 2)
        .unwrap();
    press(e, 1, x as f64, y as f64, modifiers);
    release(e, x as f64, y as f64);
}
#[cfg(test)]
pub(super) fn showcase_reorder(e: &Editor, mut frame: impl FnMut()) {
    let first = e.layers.row_at_index(0).unwrap();
    let last = e
        .layers
        .row_at_index(e.layers.children().len() as i32 - 1)
        .unwrap();
    let (x, y) = first
        .translate_coordinates(&e.layer_scroll, 12, first.allocated_height() / 2)
        .unwrap();
    let (_, end) = last
        .translate_coordinates(&e.layer_scroll, 12, last.allocated_height() - 2)
        .unwrap();
    assert!(press(e, 1, x as f64, y as f64, gdk::ModifierType::empty()));
    for i in 1..=12 {
        motion(e, x as f64, y as f64 + (end - y) as f64 * i as f64 / 12.);
        frame();
    }
    release(e, x as f64, end as f64);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insertion_target_resists_small_pointer_jitter() {
        let gaps = [(3, 0.), (2, 54.), (1, 108.), (0, 162.)];
        let mut target = Some(2);
        for y in [79., 81., 83., 80., 82.] {
            target = stable_target(&gaps, target, y);
            assert_eq!(target, Some(2));
        }
        target = stable_target(&gaps, target, 86.);
        assert_eq!(target, Some(1));
        for y in [83., 81., 79., 82.] {
            target = stable_target(&gaps, target, y);
            assert_eq!(target, Some(1));
        }
        assert_eq!(stable_target(&gaps, target, 76.), Some(2));
        assert_eq!(stable_target(&gaps, target, -30.), Some(3));
        assert_eq!(stable_target(&gaps, target, 200.), Some(0));
    }
}
