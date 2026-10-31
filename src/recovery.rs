//! Per-window, locked recovery files. PNG encoding and disk writes run off GTK.
use crate::document::{Document, RawBackground, SavedEdits};
use anyhow::{Context, Result};
use base64::Engine;
use image::ImageEncoder;
use std::{
    fs::{File, OpenOptions},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

enum Job {
    Save(
        u64,
        SavedEdits,
        Option<RawBackground>,
        Result<Option<std::sync::Arc<crate::autosave::Target>>, String>,
    ),
    Flush(mpsc::Sender<Result<(), String>>),
    Stop,
}
pub struct Completion {
    pub ticket: u64,
    pub result: Result<(), String>,
}
pub struct Session {
    tx: mpsc::Sender<Job>,
    completions: mpsc::Receiver<Completion>,
    next_ticket: std::cell::Cell<u64>,
    worker: Option<thread::JoinHandle<()>>,
    _lock: File,
    pub path: PathBuf,
    pub restored_autosave: Option<crate::autosave::Location>,
}

fn lock(path: &Path) -> Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path.with_extension("lock"))?;
    // flock only operates on our open recovery file; closing releases the lock.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        Ok(Some(file))
    } else {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            Ok(None)
        } else {
            Err(error.into())
        }
    }
}

pub fn default_directory() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("untushot/recovery")
}

impl Session {
    pub fn open(directory: &Path, restore: bool) -> Result<(Self, Option<Document>)> {
        std::fs::create_dir_all(directory)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        let mut candidates: Vec<_> = std::fs::read_dir(directory)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|v| v == "untushot" || v == "rustshot")
            })
            .collect();
        candidates
            .sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));
        let mut restored = None;
        let mut selected = None;
        if restore {
            for entry in candidates {
                if let Some(guard) = lock(&entry.path())? {
                    restored = Some(
                        Document::read_project(std::io::BufReader::new(File::open(entry.path())?))
                            .with_context(|| {
                                crate::i18n::format!(
                                    "Sitzung konnte nicht wiederhergestellt werden: {}",
                                    entry.path().display()
                                )
                            })?,
                    );
                    selected = Some((entry.path(), guard));
                    break;
                }
            }
        }
        let (path, guard) = if let Some(pair) = selected {
            pair
        } else {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let path = directory.join(crate::i18n::format!(
                "session-{stamp}-{}.untushot",
                std::process::id()
            ));
            let guard = lock(&path)?.context(crate::i18n::tr("Sitzung wird bereits verwendet"))?;
            (path, guard)
        };
        let restored_autosave = if restored.is_some() {
            crate::autosave::header(&path).ok().and_then(|h| h.autosave)
        } else {
            None
        };
        let (tx, rx) = mpsc::channel();
        let (completion_tx, completions) = mpsc::channel();
        let target = path.clone();
        let worker = thread::spawn(move || {
            let mut background = None;
            let mut last_error = None;
            for job in rx {
                match job {
                    Job::Save(ticket, edits, raw, autosave) => {
                        let result = (|| -> Result<()> {
                            if let Some(raw) = raw {
                                let mut png = Vec::new();
                                image::codecs::png::PngEncoder::new(&mut png).write_image(
                                    &raw.pixels,
                                    raw.width,
                                    raw.height,
                                    image::ColorType::Rgba8,
                                )?;
                                background =
                                    Some(base64::engine::general_purpose::STANDARD.encode(png));
                            }
                            let background = background.as_deref().context(crate::i18n::tr(
                                "Kein Hintergrund für die Autosicherung",
                            ))?;
                            let parent = target
                                .parent()
                                .context(crate::i18n::tr("Kein Sicherungsverzeichnis"))?;
                            let tmp = tempfile::Builder::new()
                                .prefix(".recovery-")
                                .tempfile_in(parent)?;
                            let mirrored = match &autosave {
                                Ok(Some(target)) => target.write(&edits, background),
                                Ok(None) => Ok(()),
                                Err(error) => Err(anyhow::Error::msg(error.clone())),
                            };
                            let location = if mirrored.is_ok() {
                                autosave
                                    .as_ref()
                                    .ok()
                                    .and_then(|t| t.as_ref())
                                    .map(|t| t.location())
                            } else {
                                None
                            };
                            // Keep recovery even if the named destination is unwritable.
                            edits.write_with_metadata(
                                background,
                                tmp.as_file(),
                                None,
                                location.as_ref(),
                            )?;
                            tmp.as_file().sync_all()?;
                            tmp.persist(&target).map_err(|e| e.error)?;
                            File::open(parent)?.sync_all()?;
                            mirrored?;
                            Ok(())
                        })();
                        last_error = result.err().map(|e| crate::i18n::format!("{e:#}"));
                        let _ = completion_tx.send(Completion {
                            ticket,
                            result: last_error.clone().map_or(Ok(()), Err),
                        });
                    }
                    Job::Flush(reply) => {
                        let _ = reply.send(last_error.clone().map_or(Ok(()), Err));
                    }
                    Job::Stop => break,
                }
            }
        });
        Ok((
            Self {
                tx,
                completions,
                next_ticket: std::cell::Cell::new(1),
                worker: Some(worker),
                _lock: guard,
                path,
                restored_autosave,
            },
            restored,
        ))
    }
    #[cfg(test)]
    pub fn save(&self, doc: &Document, include_background: bool) -> Result<u64> {
        self.save_with_sidecar(doc, include_background, Ok(None))
    }
    pub fn save_with_sidecar(
        &self,
        doc: &Document,
        include_background: bool,
        target: Result<Option<std::sync::Arc<crate::autosave::Target>>, String>,
    ) -> Result<u64> {
        let raw = if include_background {
            Some(doc.raw_background()?)
        } else {
            None
        };
        let ticket = self.next_ticket.get();
        self.next_ticket.set(ticket + 1);
        self.tx
            .send(Job::Save(ticket, doc.recovery_edits(), raw, target))
            .map_err(|_| crate::i18n::anyhow!("Autosicherung wurde beendet"))?;
        Ok(ticket)
    }
    pub fn flush(&self) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Job::Flush(tx))
            .map_err(|_| crate::i18n::anyhow!("Autosicherung wurde beendet"))?;
        rx.recv()
            .context(crate::i18n::tr("Autosicherung antwortet nicht"))?
            .map_err(anyhow::Error::msg)
    }
    pub fn completions(&self) -> Vec<Completion> {
        self.completions.try_iter().collect()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Job::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    #[test]
    fn restores_latest_document_and_excludes_live_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let (first, restored) = Session::open(dir.path(), true).unwrap();
        assert!(restored.is_none());
        let mut doc = crate::document::demo().unwrap();
        first.save(&doc, true).unwrap();
        first.flush().unwrap();
        let (second, restored) = Session::open(dir.path(), true).unwrap();
        assert!(restored.is_none());
        assert_ne!(first.path, second.path);
        drop(second);
        doc.select(Some(Rect::new(100., 200., 300., 400.)));
        first.save(&doc, false).unwrap();
        first.flush().unwrap();
        drop(first);
        let (reopened, restored) = Session::open(dir.path(), true).unwrap();
        let restored = restored.unwrap();
        assert_eq!(restored.selection(), doc.selection());
        assert_eq!(
            restored.export().unwrap().read_pixel_bytes(),
            doc.export().unwrap().read_pixel_bytes()
        );
        assert_eq!(
            std::fs::metadata(reopened.path.as_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn reports_write_failure_instead_of_silently_claiming_success() {
        let dir = tempfile::tempdir().unwrap();
        let (session, _) = Session::open(dir.path(), false).unwrap();
        session
            .save(&crate::document::demo().unwrap(), false)
            .unwrap();
        assert!(session.flush().is_err());
        assert!(session.completions().iter().any(|c| c.result.is_err()));
        assert!(!session.path.exists());
    }
}
