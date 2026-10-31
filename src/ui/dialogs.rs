//! Non-blocking feedback and help. Error messages never run a nested GTK loop.
use super::*;

impl Editor {
    pub(super) fn dismiss_error(&self) {
        let previous = self.error_dialog.borrow_mut().take();
        if let Some(dialog) = previous {
            dialog.hide();
            dialog.close();
        }
    }

    pub(super) fn error(&self, error: anyhow::Error) {
        self.show_error(
            crate::i18n::tr("Die Aktion konnte nicht abgeschlossen werden."),
            &error.to_string(),
            error,
            false,
        );
    }

    pub(super) fn capture_error(&self, error: anyhow::Error) {
        let (title, help) = match error.downcast_ref::<capture::PortalFailure>() {
            Some(capture::PortalFailure::Rejected) => (
                crate::i18n::tr("Der Desktop hat kein Bildschirmfoto geliefert."),
                crate::i18n::tr("Die Aufnahme wurde nicht freigegeben oder konnte nicht abgeschlossen werden. Versuche es erneut und bestätige die Aufnahme im Desktopdialog. Dein bisheriges Bild bleibt erhalten."),
            ),
            Some(capture::PortalFailure::Unavailable) => (
                crate::i18n::tr("Die Bildschirmaufnahme ist nicht verfügbar."),
                crate::i18n::tr("Der Aufnahmedienst deines Desktops ist nicht erreichbar. Du kannst es erneut versuchen oder ein vorhandenes Bild öffnen. Dein bisheriges Bild bleibt erhalten."),
            ),
            Some(capture::PortalFailure::TimedOut) => (
                crate::i18n::tr("Die Freigabe hat zu lange gedauert."),
                crate::i18n::tr("Versuche es erneut und bestätige die Aufnahme im Desktopdialog. Dein bisheriges Bild bleibt erhalten."),
            ),
            _ => (
                crate::i18n::tr("Die Bildschirmaufnahme ist fehlgeschlagen."),
                crate::i18n::tr("Versuche es erneut oder wähle eine andere Aufnahmeart. Dein bisheriges Bild bleibt erhalten."),
            ),
        };
        self.show_error(title, help, error, true);
    }

    fn show_error(&self, title: &str, help: &str, error: anyhow::Error, capture: bool) {
        self.dismiss_error();
        self.status.set_text(title);
        self.status
            .set_tooltip_text(Some(&crate::i18n::format!("{error:#}")));
        let dialog = gtk::MessageDialog::new(
            Some(&self.window),
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            gtk::MessageType::Error,
            gtk::ButtonsType::Close,
            title,
        );
        dialog.set_title("UntuShot");
        dialog.set_secondary_text(Some(help));
        let details = gtk::Expander::new(Some(crate::i18n::tr("Technische Details")));
        let text = gtk::Label::new(Some(&crate::i18n::format!("{error:#}")));
        text.set_selectable(true);
        text.set_line_wrap(true);
        text.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.set_max_width_chars(65);
        text.set_xalign(0.);
        details.add(&text);
        dialog.content_area().add(&details);
        if capture {
            dialog.add_button(
                if self.setup.uses_desktop_chooser() {
                    crate::i18n::tr("Aufnahmeoptionen …")
                } else {
                    crate::i18n::tr("Aufnahmeart wählen")
                },
                gtk::ResponseType::Other(1),
            );
            dialog.add_button(
                crate::i18n::tr("Erneut versuchen"),
                gtk::ResponseType::Other(2),
            );
        }
        dialog.set_default_response(gtk::ResponseType::Close);
        let weak = self.weak_self.clone();
        dialog.connect_response(move |dialog, response| {
            dialog.hide();
            dialog.close();
            if let Some(e) = weak.upgrade().filter(|e| !e.closed.get()) {
                let current = e.error_dialog.borrow().as_ref() == Some(dialog);
                if !current {
                    return;
                }
                e.error_dialog.borrow_mut().take();
                match response {
                    gtk::ResponseType::Other(1) if capture => {
                        e.show_setup();
                        e.present();
                        if e.setup.uses_desktop_chooser() {
                            e.setup.options.set_expanded(true);
                            e.setup.start.grab_focus();
                        } else {
                            e.setup.region.grab_focus();
                        }
                    }
                    gtk::ResponseType::Other(2) if capture => e.capture(),
                    _ => {}
                }
            }
        });
        self.connect_dialog_keys(dialog.upcast_ref());
        *self.error_dialog.borrow_mut() = Some(dialog.clone());
        dialog.show_all();
        dialog.present();
    }

    pub(super) fn update_close_notice(&self) {
        let prefs = self.preferences.borrow();
        if !self.resident.get()
            || prefs.close_behavior_explained
            || self.stack.visible_child_name().as_deref() != Some("editor")
        {
            self.close_notice.hide();
            return;
        }
        self.close_notice_text.set_text(if self.close_to_tray.is_active() {
            crate::i18n::tr("Beim Schließen läuft UntuShot im Hintergrund weiter. Zum vollständigen Beenden nutze „UntuShot beenden“ oder Strg+Q.")
        } else {
            crate::i18n::tr("Fenster schließen beendet UntuShot. Für Aufnahmekürzel im Hintergrund aktiviere „Beim Schließen im Hintergrund weiterlaufen“ im Menü.")
        });
        self.close_notice.content_area().show_all();
        if let Some(actions) = self.close_notice.action_area() {
            actions.show_all();
        }
        self.close_notice.show();
    }

    pub(super) fn dismiss_shortcuts(&self) {
        let previous = self.shortcuts_dialog.borrow_mut().take();
        if let Some(dialog) = previous {
            dialog.hide();
            dialog.close();
        }
    }

    fn connect_dialog_keys(&self, dialog: &gtk::Dialog) {
        let weak = self.weak_self.clone();
        dialog.connect_key_press_event(move |dialog, event| {
            use gdk::keys::constants as key;
            if event.state().contains(gdk::ModifierType::CONTROL_MASK) {
                match event.keyval() {
                    key::q | key::Q => {
                        if let Some(e) = weak.upgrade() {
                            e.quit();
                        }
                        return glib::Propagation::Stop;
                    }
                    key::w | key::W => {
                        dialog.response(gtk::ResponseType::Close);
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
            glib::Propagation::Proceed
        });
    }

    pub(super) fn show_shortcuts(&self) {
        if let Some(dialog) = self.shortcuts_dialog.borrow().as_ref() {
            dialog.present();
            return;
        }
        let dialog = gtk::Dialog::with_buttons(
            Some(crate::i18n::tr("Tastenkürzel")),
            Some(&self.window),
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[(crate::i18n::tr("Schließen"), gtk::ResponseType::Close)],
        );
        dialog.set_widget_name("untushot-shortcuts");
        dialog.set_default_size(640, 540);
        let grid = gtk::Grid::new();
        grid.set_column_spacing(24);
        grid.set_row_spacing(10);
        grid.set_margin_start(24);
        grid.set_margin_end(24);
        grid.set_margin_top(18);
        grid.set_margin_bottom(18);
        let shortcuts = [
            (
                crate::i18n::tr("Tab / Umschalt+Tab"),
                crate::i18n::tr("Bedienelemente durchgehen"),
            ),
            (
                crate::i18n::tr("F6"),
                crate::i18n::tr("Zeichenfläche fokussieren"),
            ),
            (
                crate::i18n::tr("F8 / F9"),
                crate::i18n::tr("Werkzeuge / Ebenen umschalten"),
            ),
            (
                crate::i18n::tr("Strg+F6"),
                crate::i18n::tr("Beide Seitenleisten umschalten"),
            ),
            (crate::i18n::tr("Strg+N"), crate::i18n::tr("Neue Aufnahme")),
            (
                crate::i18n::tr("Strg+O"),
                crate::i18n::tr("Bild oder Projekt öffnen"),
            ),
            (
                crate::i18n::tr("Strg+S"),
                crate::i18n::tr("Bild exportieren"),
            ),
            (
                crate::i18n::tr("Strg+Alt+S"),
                crate::i18n::tr("Bearbeitbares Projekt speichern"),
            ),
            (
                crate::i18n::tr("Strg+C / Strg+V"),
                crate::i18n::tr("Bild kopieren / als Ebene einfügen"),
            ),
            (
                crate::i18n::tr("Strg+Z / Strg+Umschalt+Z"),
                crate::i18n::tr("Rückgängig / Wiederholen"),
            ),
            (
                crate::i18n::tr("V / M / 1–9"),
                crate::i18n::tr("Werkzeug wählen (Zeichenfläche)"),
            ),
            (
                crate::i18n::tr("Pfeile / Umschalt+Pfeile"),
                crate::i18n::tr("1 / 10 Pixel verschieben"),
            ),
            (
                crate::i18n::tr("Enter / Esc"),
                crate::i18n::tr("Bestätigen / Abbrechen"),
            ),
            (
                crate::i18n::tr("Strg+W"),
                crate::i18n::tr("Fenster schließen"),
            ),
            (
                crate::i18n::tr("Strg+Q"),
                crate::i18n::tr("UntuShot beenden"),
            ),
        ];
        for (row, (keys, action)) in shortcuts.into_iter().enumerate() {
            let keys = gtk::Label::new(Some(keys));
            keys.set_xalign(1.);
            keys.set_valign(gtk::Align::Start);
            keys.style_context().add_class("shortcut-key");
            let action = gtk::Label::new(Some(action));
            action.set_xalign(0.);
            action.set_hexpand(true);
            action.set_line_wrap(true);
            action.set_max_width_chars(38);
            grid.attach(&keys, 0, row as i32, 1, 1);
            grid.attach(&action, 1, row as i32, 1, 1);
        }
        let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_min_content_height(240);
        scroll.set_overlay_scrolling(false);
        scroll.add(&grid);
        dialog.content_area().pack_start(&scroll, true, true, 0);
        let weak = self.weak_self.clone();
        dialog.connect_response(move |dialog, _| {
            if let Some(e) = weak.upgrade() {
                if e.shortcuts_dialog.borrow().as_ref() == Some(dialog) {
                    e.shortcuts_dialog.borrow_mut().take();
                }
            }
            dialog.hide();
            dialog.close();
        });
        self.connect_dialog_keys(&dialog);
        *self.shortcuts_dialog.borrow_mut() = Some(dialog.clone());
        dialog.show_all();
        dialog.present();
    }
}
