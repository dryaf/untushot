mod capture;
mod color;
mod document;
mod effects;
mod font_size;
mod geometry;
mod history;
mod i18n;
mod icons;
mod monitor_map;
mod pixels;
mod preferences;
mod preview;
mod recovery;
mod runtime;
mod selection;
mod setup;
mod snapping;
mod stroke;
mod theme;
mod tray;
mod ui;
mod web_import;

use clap::{Parser, ValueEnum};
use gtk::prelude::*;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Backend {
    Auto,
    Portal,
    Grim,
}

#[derive(Clone, Debug, Parser, serde::Serialize, serde::Deserialize)]
#[command(
    version,
    about = crate::i18n::tr("UntuShot – Screenshot und Annotation für Linux/Wayland")
)]
pub struct Args {
    /// Auto: grim versuchen, sonst XDG Desktop Portal
    #[arg(help = crate::i18n::tr("Auto: grim versuchen, sonst XDG Desktop Portal"))]
    #[arg(long, value_enum, default_value = "auto")]
    backend: Backend,
    /// Bilddatei aus „Öffnen mit“ (alternativ --open)
    #[arg(help = crate::i18n::tr("Bilddatei aus „Öffnen mit“ (alternativ --open)"))]
    #[arg(value_name="FILE",conflicts_with_all=["open","demo","capture","background","quit","status","cancel","list_monitors"])]
    file: Option<PathBuf>,
    /// Ein vorhandenes Bild bearbeiten
    #[arg(help = crate::i18n::tr("Ein vorhandenes Bild bearbeiten"))]
    #[arg(long, conflicts_with = "demo")]
    open: Option<PathBuf>,
    /// Zielpfad für den Speichern-Dialog
    #[arg(help = crate::i18n::tr("Zielpfad für den Speichern-Dialog"))]
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Verzögerung vor der Aufnahme in Sekunden (0–60)
    #[arg(help = crate::i18n::tr("Verzögerung vor der Aufnahme in Sekunden (0–60)"))]
    #[arg(long, default_value = "0", value_parser = clap::value_parser!(u64).range(0..=60))]
    delay: u64,
    /// Monitor direkt auswählen (1-basiert, siehe --list-monitors)
    #[arg(help = crate::i18n::tr("Monitor direkt auswählen (1-basiert, siehe --list-monitors)"))]
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..), conflicts_with_all = ["open", "demo"])]
    monitor: Option<u32>,
    /// Erkannte Monitore mit logischen Koordinaten anzeigen
    #[arg(help = crate::i18n::tr("Erkannte Monitore mit logischen Koordinaten anzeigen"))]
    #[arg(long)]
    list_monitors: bool,
    /// Editor als Fenster statt im Vollbild öffnen
    #[arg(help = crate::i18n::tr("Editor als Fenster statt im Vollbild öffnen"))]
    #[arg(long)]
    windowed: bool,
    /// Synthetischen Multimonitor-Desktop öffnen, ohne Bildschirmaufnahme
    #[arg(help = crate::i18n::tr("Synthetischen Multimonitor-Desktop öffnen, ohne Bildschirmaufnahme"))]
    #[arg(long)]
    demo: bool,
    /// Im Hintergrund mit Tray-Symbol starten
    #[arg(help = crate::i18n::tr("Im Hintergrund mit Tray-Symbol starten"))]
    #[arg(long, conflicts_with_all=["capture","open","demo","quit","status","cancel"]) ]
    background: bool,
    /// Direkt aufnehmen: region, full oder system
    #[arg(help = crate::i18n::tr("Direkt aufnehmen: region, full oder system"))]
    #[arg(long,value_enum,conflicts_with_all=["open","demo","quit","status","cancel"])]
    capture: Option<setup::Mode>,
    /// Hintergrundprozess nach dem Sichern beenden
    #[arg(help = crate::i18n::tr("Hintergrundprozess nach dem Sichern beenden"))]
    #[arg(long,conflicts_with_all=["open","demo","status","cancel"])]
    quit: bool,
    /// Status des Hintergrundprozesses ausgeben
    #[arg(help = crate::i18n::tr("Status des Hintergrundprozesses ausgeben"))]
    #[arg(long,conflicts_with_all=["open","demo","cancel"])]
    status: bool,
    /// Laufende Aufnahme abbrechen
    #[arg(help = crate::i18n::tr("Laufende Aufnahme abbrechen"))]
    #[arg(long,conflicts_with_all=["open","demo"])]
    cancel: bool,
}

fn main() -> glib::ExitCode {
    let mut args = Args::parse();
    if let Some(file) = args.file.take() {
        args.open = Some(file);
    }
    if let Err(error) = gtk::init() {
        crate::i18n::eprintln!("UntuShot: GTK konnte keinen Bildschirm öffnen: {error}\nIn einer grafischen Wayland-Sitzung starten.");
        return glib::ExitCode::FAILURE;
    }
    if args.list_monitors {
        if let Some(layout) = capture::desktop_layout() {
            for (i, m) in layout.monitors.iter().enumerate() {
                crate::i18n::println!(
                    "{}: {} — {}x{} bei {},{} · Skalierung {}",
                    i + 1,
                    m.name,
                    m.rect.w,
                    m.rect.h,
                    m.rect.x,
                    m.rect.y,
                    m.scale
                );
            }
            return glib::ExitCode::SUCCESS;
        }
        crate::i18n::eprintln!("Keine Monitore gefunden.");
        return glib::ExitCode::FAILURE;
    }
    if args.demo {
        let app = gtk::Application::new(
            Some("io.github.untushot.Demo"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        app.connect_activate(move |app| {
            ui::launch(app, args.clone());
        });
        return app.run_with_args::<&str>(&[]);
    }
    runtime::run(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_file_argument_handles_spaces_and_empty_launch() {
        let args = Args::try_parse_from(["untushot", "--", "/tmp/Screenshot mit Leerzeichen.png"])
            .unwrap();
        assert_eq!(
            args.file,
            Some(PathBuf::from("/tmp/Screenshot mit Leerzeichen.png"))
        );
        assert!(Args::try_parse_from(["untushot", "--"]).is_ok());
        assert!(
            Args::try_parse_from(["untushot", "--capture", "region", "/tmp/image.png"]).is_err()
        );
    }
    #[test]
    fn quick_capture_cli_and_forwarded_request() {
        let args =
            Args::try_parse_from(["untushot", "--capture", "region", "--delay", "3"]).unwrap();
        assert_eq!(args.capture, Some(setup::Mode::Region));
        assert_eq!(args.delay, 3);
        let forwarded: Args = serde_json::from_str(&serde_json::to_string(&args).unwrap()).unwrap();
        assert_eq!(forwarded.capture, args.capture);
        assert_eq!(forwarded.delay, 3);
        assert!(Args::try_parse_from(["untushot", "--background", "--capture", "region"]).is_err());
        assert!(Args::try_parse_from(["untushot", "--quit", "--capture", "full"]).is_err());
        assert!(
            Args::try_parse_from(["untushot", "--capture", "region", "--delay", "61"]).is_err()
        );
    }
}
