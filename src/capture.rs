use crate::{
    geometry::{Layout, Monitor, Rect},
    Backend,
};
use anyhow::{Context, Result};
use gdk::prelude::MonitorExt;
use gdk_pixbuf::Pixbuf;
use gio::prelude::*;
use glib::variant::ToVariant;
use std::{
    cell::RefCell,
    collections::HashMap,
    process::{Command, Stdio},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Cancellation {
    stopped: Arc<AtomicBool>,
    request: Rc<RefCell<Option<(gio::DBusConnection, String)>>>,
}
impl Cancellation {
    pub fn cancel(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some((bus, path)) = self.request.borrow().as_ref() {
            close_request(bus, path);
        }
    }
    fn check(&self) -> Result<()> {
        crate::i18n::ensure!(
            !self.stopped.load(Ordering::Relaxed),
            "Aufnahme abgebrochen"
        );
        Ok(())
    }
}
fn close_request(bus: &gio::DBusConnection, path: &str) {
    bus.call(
        Some("org.freedesktop.portal.Desktop"),
        path,
        "org.freedesktop.portal.Request",
        "Close",
        None,
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
        |_| {},
    );
}

pub struct Capture {
    pub image: Pixbuf,
    pub monitors: Vec<Monitor>,
    pub note: String,
}

pub fn desktop_layout() -> Option<Layout> {
    let display = gdk::Display::default()?;
    Layout::new(
        (0..display.n_monitors())
            .filter_map(|i| display.monitor(i))
            .map(|m| {
                let r = m.geometry();
                Monitor {
                    name: m
                        .model()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| crate::i18n::tr("Display").into()),
                    rect: Rect::new(
                        r.x() as f64,
                        r.y() as f64,
                        r.width() as f64,
                        r.height() as f64,
                    ),
                    scale: m.scale_factor() as f64,
                }
            })
            .collect(),
    )
}

/// Run the external capture off the GTK thread. A private temporary file avoids
/// pipe deadlocks and is removed on success, failure and timeout.
fn grim(layout: &Layout, stopped: &AtomicBool) -> Result<Vec<u8>> {
    crate::i18n::ensure!(!stopped.load(Ordering::Relaxed), "Aufnahme abgebrochen");
    let file = tempfile::Builder::new().suffix(".png").tempfile()?;
    let stderr = tempfile::tempfile()?;
    let r = layout.bounds;
    let scale = layout.monitors.iter().map(|m| m.scale).fold(1.0, f64::max);
    let mut child = Command::new("grim")
        .args([
            "-t",
            "png",
            "-s",
            &scale.to_string(),
            "-g",
            &crate::i18n::format!(
                "{},{} {}x{}",
                r.x as i32,
                r.y as i32,
                r.w as i32,
                r.h as i32
            ),
        ])
        .arg(file.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr.try_clone()?)
        .spawn()
        .context(crate::i18n::tr("grim konnte nicht gestartet werden"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                use std::io::{Read, Seek, SeekFrom};
                let mut stderr = stderr;
                stderr.seek(SeekFrom::Start(0))?;
                let mut message = String::new();
                stderr.take(4096).read_to_string(&mut message)?;
                crate::i18n::bail!("grim: {}", message.trim());
            }
            return std::fs::read(file.path())
                .context(crate::i18n::tr("Screenshot konnte nicht gelesen werden"));
        }
        if stopped.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            crate::i18n::bail!("Aufnahme abgebrochen");
        }
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            crate::i18n::bail!("grim: Zeitüberschreitung nach 20 Sekunden");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn ensure_layout(expected: &Option<Layout>, actual: &Option<Layout>) -> Result<()> {
    crate::i18n::ensure!(
        expected == actual,
        "Monitor-Anordnung hat sich geändert. Bitte Quelle prüfen und erneut aufnehmen."
    );
    Ok(())
}

pub async fn take(
    backend: Backend,
    interactive: bool,
    cancel: Cancellation,
    expected_layout: Option<Layout>,
) -> Result<Capture> {
    cancel.check()?;
    let layout = desktop_layout();
    // The setup choice predates the timer and any save dialog. Never apply that
    // old numeric monitor index to a different topology captured afterwards.
    ensure_layout(&expected_layout, &layout)?;
    if backend != Backend::Portal && !interactive {
        if let Some(capture_layout) = layout.clone() {
            let (tx, rx) = futures_channel::oneshot::channel();
            let stopped = cancel.stopped.clone();
            std::thread::spawn(move || {
                let _ = tx.send(grim(&capture_layout, &stopped));
            });
            let result = rx.await?;
            cancel.check()?;
            match result {
                Ok(bytes) => {
                    if desktop_layout() != layout {
                        crate::i18n::bail!("Monitor-Anordnung hat sich während der Aufnahme geändert. Bitte erneut aufnehmen.");
                    }
                    let image = crate::pixels::from_bytes(&bytes)
                        .context(crate::i18n::tr("grim lieferte kein Bild"))?;
                    let monitors = layout.as_ref().and_then(|l| l.image_monitors(image.width(),image.height()))
                        .context(crate::i18n::tr("Monitor-Geometrie hat sich während der Aufnahme geändert. Bitte erneut aufnehmen."))?;
                    return Ok(Capture {
                        image,
                        monitors,
                        note: crate::i18n::tr("Alle Monitore · grim").into(),
                    });
                }
                Err(error) if backend == Backend::Grim => return Err(error),
                Err(_) => {}
            }
        } else if backend == Backend::Grim {
            crate::i18n::bail!("Keine Monitor-Geometrie verfügbar");
        }
    }
    let image = portal(interactive, &cancel).await?;
    cancel.check()?;
    ensure_layout(&layout, &desktop_layout())?;
    // The portal returns a URI, not monitor coordinates. Do not invent reliable
    // output mappings from an image's aspect ratio alone.
    Ok(Capture { image, monitors: Vec::new(), note:
        crate::i18n::tr("Desktop-Portal · Bildumfang vom Desktop bestimmt; monitorübergreifende Auswahl im gelieferten Bild möglich").into() })
}

async fn portal(interactive: bool, cancel: &Cancellation) -> Result<Pixbuf> {
    const SERVICE: &str = "org.freedesktop.portal.Desktop";
    const REQUEST: &str = "org.freedesktop.portal.Request";
    let connection = gio::bus_get_future(gio::BusType::Session)
        .await
        .context(crate::i18n::tr("Kein Session-D-Bus erreichbar"))?;
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let token = crate::i18n::format!(
        "untushot_{}_{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    );
    let sender = connection
        .unique_name()
        .context(crate::i18n::tr("D-Bus ohne eindeutigen Namen"))?
        .trim_start_matches(':')
        .replace('.', "_");
    let path = crate::i18n::format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let (tx, rx) = futures_channel::oneshot::channel();
    let tx = Rc::new(RefCell::new(Some(tx)));
    let subscription = connection.signal_subscribe(
        Some(SERVICE),
        Some(REQUEST),
        Some("Response"),
        Some(&path),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, response| {
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(response.clone());
            }
        },
    );
    let options = HashMap::from([
        ("handle_token", token.to_variant()),
        ("interactive", interactive.to_variant()),
        ("modal", false.to_variant()),
    ]);
    *cancel.request.borrow_mut() = Some((connection.clone(), path.clone()));
    let response = async {
        cancel.check()?;
        let handle = connection.call_future(Some(SERVICE), "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Screenshot", "Screenshot", Some(&("", options).to_variant()),
            None, gio::DBusCallFlags::NONE, 30_000).await
            .context(crate::i18n::tr("Screenshot-Portal nicht verfügbar. Passendes xdg-desktop-portal-Backend installieren oder --backend grim verwenden."))?;
        let returned = handle.child_value(0).str().unwrap_or("").to_string();
        if returned != path { crate::i18n::bail!("Portal verwendet einen unerwarteten Request-Pfad: {returned}"); }
        cancel.check()?;
        let cancellation=async {let started=Instant::now();loop{cancel.check()?;if started.elapsed()>Duration::from_secs(120){crate::i18n::bail!("Screenshot-Freigabe nach 120 Sekunden abgelaufen");}glib::timeout_future(Duration::from_millis(50)).await;}};
        match futures_util::future::select(Box::pin(rx), Box::pin(cancellation)).await {
            futures_util::future::Either::Left((reply, _)) => Ok(reply?),
            futures_util::future::Either::Right((result,_)) => result,
        }
    }.await;
    connection.signal_unsubscribe(subscription);
    cancel.request.borrow_mut().take();
    if response.is_err() {
        close_request(&connection, &path);
    }
    let response = response?;
    let (code, results) = response
        .get::<(u32, HashMap<String, glib::Variant>)>()
        .context(crate::i18n::tr("Ungültige Portal-Antwort"))?;
    match code {
        0 => {}
        1 => crate::i18n::bail!("Screenshot abgebrochen"),
        _ => crate::i18n::bail!("Desktop-Portal hat die Aufnahme abgelehnt"),
    }
    let uri = results
        .get("uri")
        .and_then(|v| v.str())
        .context(crate::i18n::tr("Portal lieferte keine Bild-URI"))?;
    let path = gio::File::for_uri(uri)
        .path()
        .ok_or_else(|| crate::i18n::anyhow!("Portal lieferte keine lokale Datei"))?;
    crate::pixels::open(&path).context(crate::i18n::tr(
        "Portal-Screenshot konnte nicht geöffnet werden",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_plan_rejects_reordered_removed_moved_or_rescaled_monitors() {
        let a = Monitor {
            name: "Same model".into(),
            rect: Rect::new(-1080., 0., 1080., 1920.),
            scale: 1.,
        };
        let b = Monitor {
            name: "Same model".into(),
            rect: Rect::new(0., 752., 2560., 1440.),
            scale: 2.,
        };
        let expected = Layout::new(vec![a.clone(), b.clone()]);
        assert!(ensure_layout(&expected, &expected.clone()).is_ok());
        assert!(ensure_layout(&expected, &Layout::new(vec![b.clone(), a.clone()])).is_err());
        assert!(ensure_layout(&expected, &Layout::new(vec![b.clone()])).is_err());
        assert!(ensure_layout(&expected, &None).is_err());
        let mut moved = b.clone();
        moved.rect.y = 0.;
        assert!(ensure_layout(&expected, &Layout::new(vec![a.clone(), moved])).is_err());
        let mut scaled = b;
        scaled.scale = 1.;
        assert!(ensure_layout(&expected, &Layout::new(vec![a, scaled])).is_err());
        assert!(ensure_layout(&None, &None).is_ok());
    }
}
