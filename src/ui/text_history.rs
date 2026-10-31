//! Session-local text undo. GTK user actions group selection replacement,
//! paste and input-method commits without changing the document's history.
use super::*;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    text: String,
    size: Option<f64>,
    color: (f64, f64, f64),
    cursor: i32,
    bound: i32,
}

impl Snapshot {
    fn same_content(&self, other: &Self) -> bool {
        self.text == other.text && self.size == other.size && self.color == other.color
    }
}

#[derive(Default)]
pub(super) struct History {
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
    pending: Option<Snapshot>,
    depth: usize,
    restoring: bool,
}

impl History {
    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn prepare(&mut self, before: Snapshot) {
        if !self.restoring && self.pending.is_none() {
            self.pending = Some(before);
        }
    }

    fn finish(&mut self, after: &Snapshot) {
        if self.restoring || self.depth != 0 {
            return;
        }
        if let Some(before) = self.pending.take() {
            if !before.same_content(after) {
                self.undo.push_back(before);
                self.redo.clear();
                self.trim(true);
            }
        }
    }

    fn step(&mut self, redo: bool, current: Snapshot) -> Option<Snapshot> {
        let target = if redo {
            let target = self.redo.pop_back()?;
            self.undo.push_back(current);
            target
        } else {
            let target = self.undo.pop_back()?;
            self.redo.push_back(current);
            target
        };
        self.trim(redo);
        Some(target)
    }

    fn trim(&mut self, latest_is_undo: bool) {
        // Retain the nearest step even for an unusually large pasted text.
        // Otherwise cap both count and the retained UTF-8 payload.
        let mut bytes: usize = self
            .undo
            .iter()
            .chain(&self.redo)
            .map(|s| s.text.len())
            .sum();
        while self.undo.len() + self.redo.len() > 1
            && (self.undo.len() + self.redo.len() > 100 || bytes > 8 * 1024 * 1024)
        {
            let removed = if self.undo.len() > self.redo.len()
                || (self.undo.len() == self.redo.len() && !latest_is_undo)
            {
                self.undo.pop_front()
            } else {
                self.redo.pop_front()
            };
            bytes -= removed.unwrap().text.len();
        }
    }
}

pub(super) fn connect(editor: &Rc<Editor>) {
    let buffer = editor.inline_input.buffer().unwrap();
    let weak = Rc::downgrade(editor);
    buffer.connect_begin_user_action(move |_| {
        if let Some(e) = weak.upgrade() {
            e.prepare_text_change();
            if let Some(edit) = e.inline_edit.borrow_mut().as_mut() {
                edit.history.depth += 1;
            }
        }
    });
    let weak = Rc::downgrade(editor);
    buffer.connect_end_user_action(move |_| {
        if let Some(e) = weak.upgrade() {
            if let Some(edit) = e.inline_edit.borrow_mut().as_mut() {
                edit.history.depth = edit.history.depth.saturating_sub(1);
            }
            e.finish_text_change();
        }
    });
    let weak = Rc::downgrade(editor);
    buffer.connect_insert_text(move |_, _, _| {
        if let Some(e) = weak.upgrade() {
            e.prepare_text_change();
        }
    });
    let weak = Rc::downgrade(editor);
    buffer.connect_delete_range(move |_, _, _| {
        if let Some(e) = weak.upgrade() {
            e.prepare_text_change();
        }
    });
    let weak = Rc::downgrade(editor);
    buffer.connect_changed(move |buffer| {
        if let Some(e) = weak.upgrade() {
            if let Some(edit) = e.inline_edit.borrow_mut().as_mut() {
                edit.annotation.text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .unwrap_or_default()
                    .replace('\0', "");
            }
            e.finish_text_change();
            e.area.queue_draw();
        }
    });
}

impl Editor {
    fn text_snapshot(&self) -> Option<Snapshot> {
        let editing = self.inline_edit.borrow();
        let edit = editing.as_ref()?;
        if edit.history.restoring {
            return None;
        }
        let buffer = self.inline_input.buffer().unwrap();
        Some(Snapshot {
            text: edit.annotation.text.clone(),
            size: edit.annotation.font_size,
            color: edit.annotation.color,
            cursor: buffer.iter_at_mark(&buffer.get_insert().unwrap()).offset(),
            bound: buffer
                .iter_at_mark(&buffer.selection_bound().unwrap())
                .offset(),
        })
    }

    pub(super) fn prepare_text_change(&self) {
        if let Some(before) = self.text_snapshot() {
            self.inline_edit
                .borrow_mut()
                .as_mut()
                .unwrap()
                .history
                .prepare(before);
        }
    }

    pub(super) fn finish_text_change(&self) {
        if let Some(after) = self.text_snapshot() {
            let restoring = {
                let mut editing = self.inline_edit.borrow_mut();
                let history = &mut editing.as_mut().unwrap().history;
                history.finish(&after);
                history.restoring
            };
            if !restoring {
                self.update_history_controls();
            }
        }
    }

    pub(super) fn pending_text_size(&self) -> bool {
        self.font_size
            .text()
            .replace(',', ".")
            .parse::<f64>()
            .is_ok_and(|value| {
                value.is_finite()
                    && value >= self.font_size.adjustment().lower()
                    && value <= self.font_size.adjustment().upper()
                    && value != self.font_size.value()
            })
    }

    pub(super) fn text_history(&self, redo: bool) {
        self.inline_input.reset_im_context();
        self.font_size.update();
        let Some(current) = self.text_snapshot() else {
            return;
        };
        let target = {
            let mut editing = self.inline_edit.borrow_mut();
            let edit = editing.as_mut().unwrap();
            let Some(target) = edit.history.step(redo, current) else {
                return;
            };
            edit.history.restoring = true;
            edit.annotation.text = target.text.clone();
            edit.annotation.font_size = target.size;
            edit.annotation.color = target.color;
            target
        };
        let buffer = self.inline_input.buffer().unwrap();
        buffer.set_text(&target.text);
        buffer.select_range(
            &buffer.iter_at_offset(target.cursor),
            &buffer.iter_at_offset(target.bound),
        );
        let size = self
            .inline_edit
            .borrow()
            .as_ref()
            .unwrap()
            .annotation
            .text_size();
        {
            let mut s = self.state.borrow_mut();
            s.font_size = size;
            s.color = target.color;
        }
        let syncing = self.syncing.replace(true);
        self.font_size.set_value(size);
        self.color.set_rgba(&gdk::RGBA::new(
            target.color.0,
            target.color.1,
            target.color.2,
            1.,
        ));
        self.syncing.set(syncing);
        self.update_inline_style();
        self.inline_input.grab_focus();
        self.inline_input.scroll_to_iter(
            &mut buffer.iter_at_offset(target.cursor),
            0.1,
            false,
            0.,
            0.,
        );
        self.inline_edit
            .borrow_mut()
            .as_mut()
            .unwrap()
            .history
            .restoring = false;
        self.update_history_controls();
        self.update_storage();
        self.area.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str) -> Snapshot {
        Snapshot {
            text: text.into(),
            size: Some(24.),
            color: (0., 0., 0.),
            cursor: text.chars().count() as i32,
            bound: text.chars().count() as i32,
        }
    }

    #[test]
    fn replacements_group_and_navigation_does_not_destroy_redo() {
        let mut h = History::default();
        let mut before = snapshot("Grüße 🙂");
        before.cursor = 2;
        before.bound = 5;
        h.prepare(before.clone());
        h.depth = 1;
        h.prepare(snapshot("Gr 🙂"));
        h.finish(&snapshot("Gr 🙂"));
        assert!(!h.can_undo());
        let after = snapshot("Gr世界 🙂");
        h.depth = 0;
        h.finish(&after);
        assert_eq!(h.step(false, after.clone()), Some(before.clone()));
        assert!(!h.can_undo());
        let mut moved = before.clone();
        moved.cursor = 0;
        moved.bound = 0;
        h.prepare(before);
        h.finish(&moved);
        assert!(!h.can_undo());
        assert!(h.can_redo());
        assert_eq!(h.step(true, moved), Some(after.clone()));
        h.step(false, after);
        h.prepare(snapshot("Grüße 🙂"));
        h.finish(&snapshot("Neuer Zweig"));
        assert!(!h.can_redo());
    }

    #[test]
    fn history_retains_nearest_steps_with_count_and_payload_limits() {
        let mut h = History::default();
        for i in 0..120 {
            h.prepare(snapshot(&i.to_string()));
            h.finish(&snapshot(&(i + 1).to_string()));
        }
        assert_eq!(h.undo.len(), 100);
        for i in (20..120).rev() {
            assert_eq!(
                h.step(false, snapshot(&(i + 1).to_string())),
                Some(snapshot(&i.to_string()))
            );
        }
        assert!(!h.can_undo());
        assert_eq!(h.redo.len(), 100);
        let large = snapshot(&"x".repeat(3 * 1024 * 1024));
        h.undo.clear();
        h.redo.clear();
        for _ in 0..5 {
            h.undo.push_back(large.clone());
        }
        h.trim(true);
        assert_eq!(h.undo.len(), 2);
        // A single action stays reversible even if its text exceeds the budget.
        let huge = snapshot(&"y".repeat(9 * 1024 * 1024));
        h.undo.push_back(huge.clone());
        h.trim(true);
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.step(false, snapshot("")), Some(huge));
        assert!(h.can_redo());
    }
}
