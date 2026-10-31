//! Editable sidecars, with exclusive writers and atomic, conflict-checked saves.
use crate::document::SavedEdits;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceStamp {
    bytes: u64,
    sha256: String,
}
impl SourceStamp {
    pub fn read(path: &Path) -> Result<Self> {
        let m = std::fs::metadata(path)?;
        ensure!(
            m.is_file() && m.len() <= 512 * 1024 * 1024,
            "Invalid sidecar image source"
        );
        let mut file = File::open(path)?;
        let mut checksum =
            glib::Checksum::new(glib::ChecksumType::Sha256).context("SHA-256 unavailable")?;
        let mut buffer = [0; 65536];
        loop {
            let size = file.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            checksum.update(&buffer[..size]);
        }
        Ok(Self {
            bytes: m.len(),
            sha256: checksum.string().context("Image checksum unavailable")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Stamp {
    bytes: u64,
    modified: u128,
    inode: u64,
}
fn stamp(path: &Path) -> Result<Option<Stamp>> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => {
            ensure!(
                m.is_file(),
                "Autosave destination is not a regular file: {}",
                path.display()
            );
            Ok(Some(Stamp {
                bytes: m.len(),
                modified: m
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos(),
                inode: m.ino(),
            }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Location {
    pub path: PathBuf,
    stamp: Option<Stamp>,
    pub source: Option<PathBuf>,
    source_stamp: Option<SourceStamp>,
}

pub fn sidecar_path(image: &Path) -> PathBuf {
    let mut name = image.as_os_str().to_os_string();
    name.push(".untushot");
    name.into()
}

#[derive(Default, Deserialize)]
pub struct Header {
    pub sidecar_source: Option<SourceStamp>,
    pub autosave: Option<Location>,
}
pub fn header(path: &Path) -> Result<Header> {
    Ok(serde_json::from_reader(
        std::io::BufReader::new(File::open(path)?).take(512 * 1024 * 1024),
    )?)
}
pub fn matches_image(image: &Path) -> bool {
    SourceStamp::read(image).ok().is_some_and(|s| {
        header(&sidecar_path(image))
            .ok()
            .and_then(|h| h.sidecar_source)
            == Some(s)
    })
}

pub struct Target {
    pub path: PathBuf,
    source: Option<PathBuf>,
    source_stamp: Option<SourceStamp>,
    expected: Mutex<Option<Stamp>>,
    _lock: File,
}
impl Target {
    pub fn open(path: PathBuf, source: Option<PathBuf>, allow_existing: bool) -> Result<Arc<Self>> {
        // Keep the lock inode stable across writers, but out of the normal
        // screenshot folder listing.
        let mut lock_name = std::ffi::OsString::from(".");
        lock_name.push(path.file_name().context("Autosave path needs a filename")?);
        lock_name.push(".lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path.with_file_name(lock_name))?;
        ensure!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "Editable file is already open in another window: {}",
            path.display()
        );
        let expected = stamp(&path)?;
        ensure!(
            allow_existing || expected.is_none(),
            "Autosave would replace an existing file: {}",
            path.display()
        );
        let source_stamp = source
            .as_deref()
            .map(SourceStamp::read)
            .transpose()?
            .or_else(|| header(&path).ok().and_then(|h| h.sidecar_source));
        Ok(Arc::new(Self {
            path,
            source,
            source_stamp,
            expected: Mutex::new(expected),
            _lock: lock,
        }))
    }
    pub fn restore(location: &Location) -> Result<Arc<Self>> {
        let mut target = Self::open(location.path.clone(), location.source.clone(), true)?;
        ensure!(
            *target.expected.lock().unwrap() == location.stamp,
            "Autosaved file changed outside UntuShot"
        );
        Arc::get_mut(&mut target).unwrap().source_stamp = location.source_stamp.clone();
        Ok(target)
    }
    pub fn location(&self) -> Location {
        Location {
            path: self.path.clone(),
            stamp: self.expected.lock().unwrap().clone(),
            source: self.source.clone(),
            source_stamp: self.source_stamp.clone(),
        }
    }
    pub fn write(&self, edits: &SavedEdits, background: &str) -> Result<()> {
        let mut expected = self.expected.lock().unwrap();
        ensure!(
            stamp(&self.path)? == *expected,
            "Autosaved file changed outside UntuShot: {}",
            self.path.display()
        );
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temp = tempfile::Builder::new()
            .prefix(".untushot-")
            .tempfile_in(parent)?;
        edits.write_with_metadata(background, temp.as_file(), self.source_stamp.as_ref(), None)?;
        temp.as_file().sync_all()?;
        // Check again after encoding, and never replace a newly appeared file.
        ensure!(
            stamp(&self.path)? == *expected,
            "Autosaved file changed during saving: {}",
            self.path.display()
        );
        if expected.is_none() {
            temp.persist_noclobber(&self.path).map_err(|e| e.error)?;
        } else {
            temp.persist(&self.path).map_err(|e| e.error)?;
        }
        *expected = stamp(&self.path)?;
        File::open(parent)?
            .sync_all()
            .context("Could not sync autosave directory")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_collisions_and_rejects_stale_restore_locations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png.untushot");
        let target = Target::open(path.clone(), None, false).unwrap();
        let location = target.location();
        std::fs::write(&path, b"another writer").unwrap();
        let doc = crate::document::demo().unwrap();
        assert!(target.write(&doc.recovery_edits(), "unused").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"another writer");
        drop(target);
        assert!(Target::restore(&location).is_err());
        assert!(Target::open(path.clone(), None, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"another writer");
    }

    #[test]
    fn changed_images_do_not_reopen_old_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        std::fs::write(&png, b"original image").unwrap();
        let doc = crate::document::demo().unwrap();
        let target = Target::open(sidecar_path(&png), Some(png.clone()), false).unwrap();
        target.write(&doc.recovery_edits(), "unused").unwrap();
        assert!(matches_image(&png));
        // The association survives copying both files, regardless of timestamps.
        let copy = dir.path().join("copy.png");
        std::fs::copy(&png, &copy).unwrap();
        std::fs::copy(sidecar_path(&png), sidecar_path(&copy)).unwrap();
        assert!(matches_image(&copy));
        std::fs::write(&png, b"different image").unwrap();
        assert!(!matches_image(&png));
        assert_ne!(sidecar_path(&png), sidecar_path(&png.with_extension("jpg")));
    }
}
