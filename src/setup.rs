use crate::{capture, icons, monitor_map::MonitorMap, Args, Backend};
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

fn direct_backend(backend: Backend) -> bool {
    !capture::uses_desktop_chooser(backend)
}

fn source_hint(direct: bool, system: bool) -> &'static str {
    if system {
        crate::i18n::tr("Wähle ein Fenster oder einen Bildschirm im Desktopdialog. Fensteraufnahme hängt von deinem Desktop ab; alternativ einen Bildschirm aufnehmen und zuschneiden.")
    } else if direct {
        crate::i18n::tr("Einzelner Monitor oder gemeinsame Aufnahme über alle Displays.")
    } else {
        crate::i18n::tr("Dein Desktop öffnet seine Aufnahmeauswahl. Bestätige dort das Bild; danach kannst du es hier zuschneiden.")
    }
}

// Keep the native selector readable without letting a long monitor name force
// the entire window wider. The complete selected name remains in the tooltip.
pub fn source_selector(width: i32, height: i32) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    combo.set_hexpand(true);
    combo.set_size_request(width, height);
    for cell in combo.cells() {
        if let Ok(text) = cell.downcast::<gtk::CellRendererText>() {
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.set_width_chars(1);
            text.set_max_width_chars(60);
        }
    }
    combo.connect_changed(|c| c.set_tooltip_text(c.active_text().as_deref()));
    combo
}

#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    Region,
    Full,
    System,
}

impl Mode {
    pub fn action_label(self, delay: i32) -> String {
        let action = match self {
            Self::Region => crate::i18n::tr("Bereich auswählen"),
            Self::Full => crate::i18n::tr("Bildschirm aufnehmen"),
            Self::System => crate::i18n::tr("Fenster oder Bildschirm wählen"),
        };
        if delay > 0 {
            crate::i18n::format!("{action} · {delay} s")
        } else {
            action.into()
        }
    }
}

pub struct Setup {
    pub root: gtk::Box,
    pub options: gtk::Expander,
    pub history: gtk::Box,
    pub source: gtk::ComboBoxText,
    pub delay: gtk::SpinButton,
    pub region: gtk::RadioButton,
    pub full: gtk::RadioButton,
    pub system: gtk::RadioButton,
    pub start: gtk::Button,
    pub demo: gtk::Button,
    pub web: gtk::Button,
    pub cancel: gtk::Button,
    pub progress: gtk::Label,
    pub map: MonitorMap,
    direct: Rc<Cell<bool>>,
    hint: gtk::Label,
    choices: gtk::Box,
    source_card: gtk::Box,
    update_action: Rc<dyn Fn()>,
}

impl Setup {
    pub fn new(args: &Args) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);
        root.set_size_request(540, -1);
        root.set_border_width(20);
        let title = gtk::Label::new(Some(crate::i18n::tr("Screenshot aufnehmen")));
        title.style_context().add_class("hero");
        title.set_margin_bottom(18);
        root.pack_start(&title, false, false, 0);
        let choices = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        choices.set_homogeneous(true);
        let mut modes: Vec<gtk::RadioButton> = Vec::new();
        for (icon, name, detail) in [
            (
                "crop",
                crate::i18n::tr("Bereich"),
                crate::i18n::tr("Ausschnitt aufziehen"),
            ),
            (
                "monitor",
                crate::i18n::tr("Bildschirm"),
                crate::i18n::tr("Bildschirm vollständig aufnehmen"),
            ),
            (
                "camera",
                crate::i18n::tr("Fenster / Bildschirm …"),
                crate::i18n::tr("Mit dem Desktop auswählen"),
            ),
        ] {
            let radio = gtk::RadioButton::new();
            if let Some(first) = modes.first() {
                radio.join_group(Some(first));
            }
            radio.set_mode(false);
            radio.style_context().add_class("capture-card");
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            content.set_halign(gtk::Align::Center);
            content.pack_start(&icons::widget(icon), false, false, 0);
            let label = gtk::Label::new(Some(name));
            label.style_context().add_class("card-title");
            content.pack_start(&label, false, false, 0);
            radio.set_tooltip_text(Some(detail));
            icons::accessible_name(&radio, name);
            radio.add(&content);
            choices.pack_start(&radio, true, true, 0);
            modes.push(radio);
        }
        modes[0].set_active(true);
        root.pack_start(&choices, false, false, 8);
        let options = gtk::Expander::new(Some(crate::i18n::tr("Optionen")));
        options.set_tooltip_text(Some(crate::i18n::tr("Bildschirm und Verzögerung")));
        let details = gtk::Box::new(gtk::Orientation::Vertical, 14);
        details.set_margin_top(12);
        details.set_margin_bottom(8);
        options.add(&details);
        root.pack_start(&options, false, false, 0);
        let source_card = gtk::Box::new(gtk::Orientation::Vertical, 10);
        details.pack_start(&source_card, false, false, 0);
        let source_label = gtk::Label::new(Some(crate::i18n::tr("Bildschirm / Quelle")));
        source_label.set_xalign(0.0);
        source_label.style_context().add_class("caption");
        source_card.pack_start(&source_label, false, false, 0);
        let map = MonitorMap::new(
            capture::desktop_layout()
                .map(|l| l.monitors)
                .unwrap_or_default(),
        );
        map.area.set_no_show_all(true);
        map.area.set_visible(!map.monitors().is_empty());
        source_card.pack_start(&map.area, false, false, 0);
        let source = source_selector(440, 42);
        let direct = direct_backend(args.backend);
        options.set_tooltip_text(Some(if direct {
            crate::i18n::tr("Bildschirm und Verzögerung")
        } else {
            crate::i18n::tr("Verzögerung")
        }));
        source.append(
            Some("all"),
            if direct {
                crate::i18n::tr("Alle verfügbaren Bildschirme")
            } else {
                crate::i18n::tr("Vom Desktop bereitgestellter Bildschirm")
            },
        );
        if direct {
            if let Some(layout) = capture::desktop_layout() {
                for (i, m) in layout.monitors.iter().enumerate() {
                    source.append(
                        Some(&i.to_string()),
                        &crate::i18n::format!(
                            "Bildschirm {} · {} · {} × {}",
                            i + 1,
                            m.name,
                            m.rect.w,
                            m.rect.h
                        ),
                    );
                }
            }
        }
        source.set_active_id(Some("all"));
        if let Some(index) = args.monitor {
            source.set_active_id(Some(&(index - 1).to_string()));
        }
        source_card.pack_start(&source, false, false, 0);
        let hint = gtk::Label::new(Some(source_hint(direct, false)));
        hint.set_xalign(0.0);
        hint.style_context().add_class("muted");
        hint.set_line_wrap(true);
        hint.set_max_width_chars(62);
        source_card.pack_start(&hint, false, false, 0);
        let direct = Rc::new(Cell::new(direct));
        let toggle_direct = direct.clone();
        let source_clone = source.clone();
        let source_map = map.area.clone();
        let mode_hint = hint.clone();
        modes[2].connect_toggled(move |radio| {
            mode_hint.set_text(source_hint(toggle_direct.get(), radio.is_active()));
            source_clone.set_sensitive(!radio.is_active());
            if toggle_direct.get() {
                source_map.set_sensitive(!radio.is_active());
            }
        });
        let timer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        timer.pack_start(
            &icons::content("timer", crate::i18n::tr("Verzögerung")),
            false,
            false,
            0,
        );
        let delay = gtk::SpinButton::with_range(0., 60., 1.);
        delay.set_value(args.delay as f64);
        delay.set_width_chars(3);
        timer.pack_start(&delay, false, false, 0);
        timer.pack_start(
            &gtk::Label::new(Some(crate::i18n::tr("Sekunden · 0 = sofort"))),
            false,
            false,
            0,
        );
        details.pack_start(&timer, false, false, 0);
        let history = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details.pack_start(&history, false, false, 0);
        let start = gtk::Button::new();
        let start_content = icons::content("camera", "");
        let start_caption = gtk::Label::new(None);
        start_content.pack_start(&start_caption, false, false, 0);
        start_content.set_halign(gtk::Align::Center);
        start.add(&start_content);
        start.style_context().add_class("suggested-action");
        start.set_margin_top(8);
        start.set_size_request(-1, 42);
        root.pack_start(&start, false, false, 0);
        let region_weak = modes[0].downgrade();
        let full_weak = modes[1].downgrade();
        let delay_weak = delay.downgrade();
        let caption_weak = start_caption.downgrade();
        let action_direct = direct.clone();
        let update: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(move || {
            if let (Some(region), Some(full), Some(delay), Some(caption)) = (
                region_weak.upgrade(),
                full_weak.upgrade(),
                delay_weak.upgrade(),
                caption_weak.upgrade(),
            ) {
                let mode = if region.is_active() {
                    Mode::Region
                } else if full.is_active() {
                    Mode::Full
                } else {
                    Mode::System
                };
                let seconds = delay.value_as_int();
                caption.set_text(&if action_direct.get() {
                    mode.action_label(seconds)
                } else {
                    let action = crate::i18n::tr("Screenshot aufnehmen");
                    if seconds > 0 {
                        crate::i18n::format!("{action} · {seconds} s")
                    } else {
                        action.into()
                    }
                });
            }
        });
        for radio in &modes {
            let update = update.clone();
            radio.connect_toggled(move |_| update());
        }
        let update_delay = update.clone();
        delay.connect_value_changed(move |_| update_delay());
        update();
        let progress = gtk::Label::new(None);
        progress.set_no_show_all(true);
        progress.connect_label_notify(|label| label.set_visible(!label.text().is_empty()));
        progress.set_line_wrap(true);
        progress.set_max_width_chars(62);
        progress.style_context().add_class("muted");
        root.pack_start(&progress, false, false, 0);
        let select_source = source.clone();
        let system = modes[2].clone();
        let progress_label = progress.clone();
        let map_direct = direct.clone();
        map.connect_selected(move |index| {
            if map_direct.get() {
                select_source.set_active_id(Some(&index.to_string()));
            } else {
                system.set_active(true);
                progress_label.set_text(crate::i18n::tr(
                    "Den gewünschten Bildschirm bitte im Desktopdialog auswählen.",
                ));
            }
        });
        let selected_map = map.clone();
        source.connect_changed(move |combo| {
            selected_map.select(combo.active_id().and_then(|id| id.parse().ok()))
        });
        let demo = gtk::Button::with_label(crate::i18n::tr("Mit Demo ausprobieren"));
        let web = gtk::Button::new();
        web.add(&icons::button_content(
            "web",
            crate::i18n::tr("Webseite übernehmen …"),
        ));
        web.style_context().add_class("flat");
        web.set_tooltip_text(Some(crate::i18n::tr(
            "Eine ganze Seite im Browser kopieren und mit Vorschau übernehmen",
        )));
        demo.style_context().add_class("flat");
        let cancel = gtk::Button::with_label(crate::i18n::tr("Zurück zum Editor"));
        cancel.style_context().add_class("flat");
        cancel.set_halign(gtk::Align::Center);
        root.pack_start(&cancel, false, false, 0);
        // A custom monitor remains visible in the collapsed summary. A delay
        // is already included in the primary capture button's label.
        let option_weak = options.downgrade();
        let source_weak = source.downgrade();
        let system_weak = modes[2].downgrade();
        let summarize: Rc<dyn Fn()> = Rc::new(move || {
            if let (Some(options), Some(source), Some(system)) = (
                option_weak.upgrade(),
                source_weak.upgrade(),
                system_weak.upgrade(),
            ) {
                let monitor = source.active_id().and_then(|id| id.parse::<usize>().ok());
                let label = match monitor.filter(|_| !system.is_active()) {
                    Some(index) => crate::i18n::format!("Optionen · Bildschirm {}", index + 1),
                    None => crate::i18n::tr("Optionen").into(),
                };
                options.set_label(Some(&label));
            }
        });
        let update_source = summarize.clone();
        source.connect_changed(move |_| update_source());
        let update_mode = summarize.clone();
        modes[2].connect_toggled(move |_| update_mode());
        summarize();
        // show_all must not bring back controls delegated to the desktop.
        choices.show_all();
        choices.set_no_show_all(true);
        choices.set_visible(direct.get());
        source_card.show_all();
        source_card.set_no_show_all(true);
        source_card.set_visible(direct.get());
        Self {
            root,
            options,
            history,
            source,
            delay,
            region: modes[0].clone(),
            full: modes[1].clone(),
            system: modes[2].clone(),
            start,
            demo,
            web,
            cancel,
            progress,
            map,
            direct,
            hint,
            choices,
            source_card,
            update_action: update,
        }
    }
    pub fn set_backend(&self, backend: Backend) {
        let direct = direct_backend(backend);
        if self.direct.replace(direct) == direct {
            return;
        }
        self.options.set_tooltip_text(Some(if direct {
            crate::i18n::tr("Bildschirm und Verzögerung")
        } else {
            crate::i18n::tr("Verzögerung")
        }));
        self.choices.set_visible(direct);
        self.source_card.set_visible(direct);
        (self.update_action)();
        self.rebuild_monitors(self.map.monitors());
        self.map
            .area
            .set_sensitive(!direct || !self.system.is_active());
        self.hint
            .set_text(source_hint(direct, self.system.is_active()));
    }
    pub fn refresh_layout(&self) -> bool {
        self.update_monitors(
            capture::desktop_layout()
                .map(|l| l.monitors)
                .unwrap_or_default(),
        )
    }
    pub fn update_monitors(&self, monitors: Vec<crate::geometry::Monitor>) -> bool {
        if self.map.monitors() == monitors {
            return false;
        }
        self.rebuild_monitors(monitors);
        true
    }
    fn rebuild_monitors(&self, monitors: Vec<crate::geometry::Monitor>) {
        let previous = self
            .source
            .active_id()
            .and_then(|id| id.parse::<usize>().ok())
            .and_then(|i| self.map.monitors().get(i).cloned());
        let selected = previous
            .and_then(|old| crate::geometry::monitor_index(&monitors, &old))
            .filter(|_| self.direct.get());
        self.map.area.set_visible(!monitors.is_empty());
        self.map.set_monitors(monitors.clone());
        self.source.remove_all();
        self.source.append(
            Some("all"),
            if self.direct.get() {
                crate::i18n::tr("Alle verfügbaren Bildschirme")
            } else {
                crate::i18n::tr("Vom Desktop bereitgestellter Bildschirm")
            },
        );
        if self.direct.get() {
            for (i, m) in monitors.iter().enumerate() {
                self.source.append(
                    Some(&i.to_string()),
                    &crate::i18n::format!(
                        "Bildschirm {} · {} · {} × {}",
                        i + 1,
                        m.name,
                        m.rect.w,
                        m.rect.h
                    ),
                );
            }
        }
        self.source.set_active_id(Some(
            &selected
                .map(|i| i.to_string())
                .unwrap_or_else(|| "all".into()),
        ));
    }
    pub fn mode(&self) -> Mode {
        if !self.direct.get() {
            Mode::System
        } else if self.region.is_active() {
            Mode::Region
        } else if self.full.is_active() {
            Mode::Full
        } else {
            Mode::System
        }
    }
    pub fn uses_desktop_chooser(&self) -> bool {
        !self.direct.get()
    }
}
