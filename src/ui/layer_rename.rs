//! A single name editor, temporarily hosted inside the selected layer row.
use super::*;

impl Editor {
    pub(super) fn layer_rename_valid(&self) -> bool {
        let Some((index, revision, epoch)) = self.layer_rename.get() else {
            return false;
        };
        let s = self.state.borrow();
        !s.capturing
            && !s.selecting_region
            && s.indices() == [index]
            && self.clipboard_epoch.get() == epoch
            && s.document
                .as_ref()
                .is_some_and(|d| d.revision() == revision && index < d.annotations().len())
    }

    pub(super) fn begin_layer_rename(&self) {
        self.commit_pending_edits();
        layer_drag::cancel(self);
        let (index, revision, name) = {
            let s = self.state.borrow();
            let indices = s.indices();
            let [index] = indices.as_slice() else {
                return;
            };
            let Some(doc) = s.document.as_ref() else {
                return;
            };
            if doc.annotations()[*index].locked {
                return;
            }
            (*index, doc.revision(), doc.annotations()[*index].name())
        };
        self.show_layers.set_active(true);
        let title = self.layer_rows.borrow()[index].title.clone();
        self.layer_rows.borrow()[index].row.set_can_focus(false);
        self.layer_name.set_text(&name);
        title.add_named(&self.layer_name, "edit");
        self.layer_rename
            .set(Some((index, revision, self.clipboard_epoch.get())));
        self.layer_name.show();
        title.set_visible_child_name("edit");
        self.reveal_layer.set(true);
        self.reveal_selected_layer();
        self.layer_name.grab_focus();
        self.layer_name.select_region(0, -1);
        self.status.set_text(crate::i18n::tr(
            "Ebenenname ändern · Enter bestätigen · Esc abbrechen",
        ));
        self.update_history_controls();
        self.update_storage();
    }

    pub(super) fn finish_layer_rename(&self, commit: bool) {
        let valid = self.layer_rename_valid();
        let Some((index, _, _)) = self.layer_rename.take() else {
            return;
        };
        let label = self.layer_name.text().trim().to_string();
        if let Some(row) = self.layer_name.ancestor(gtk::ListBoxRow::static_type()) {
            row.set_can_focus(true);
        }
        // Clear the session before detaching: removing a focused entry can
        // synchronously emit focus-out and must not commit a second time.
        if let Some(parent) = self
            .layer_name
            .parent()
            .and_then(|p| p.downcast::<gtk::Stack>().ok())
        {
            parent.set_visible_child_name("name");
            parent.remove(&self.layer_name);
        }
        self.layer_name.hide();
        let original = self
            .state
            .borrow()
            .document
            .as_ref()
            .and_then(|d| d.annotations().get(index))
            .map(|a| a.name())
            .unwrap_or_default();
        if commit && valid && label != original {
            self.edit_selected(|a| a.label = label.clone());
            self.status
                .set_text(crate::i18n::tr("Ebene umbenannt · Strg+Z: rückgängig"));
        } else {
            self.layer_name.set_text(&original);
            self.status.set_text(if commit {
                crate::i18n::tr("Bereit")
            } else {
                crate::i18n::tr("Umbenennen abgebrochen")
            });
        }
        self.update_history_controls();
        self.update_storage();
    }
}
