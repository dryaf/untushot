//! Small, atomic user preferences; document contents live in recovery/project files.
use crate::{document::Tool, geometry::Monitor};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub tool: Tool,
    pub color: (f64, f64, f64),
    pub width: f64,
    pub font_size: f64,
    pub snapping: bool,
    pub tools_visible: bool,
    pub layers_visible: bool,
    pub layer_panel_width: i32,
    pub export_directory: Option<PathBuf>,
    pub history_visible: bool,
    pub monitor: Option<Monitor>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            tool: Tool::Move,
            color: (0.98, 0.32, 0.27),
            width: 4.,
            font_size: 28.,
            snapping: true,
            tools_visible: true,
            layers_visible: true,
            layer_panel_width: 252,
            export_directory: None,
            history_visible: true,
            monitor: None,
        }
    }
}
impl Preferences {
    pub fn path() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            })
            .join("untushot/preferences.json")
    }
    pub fn load(path: &Path) -> Result<Self> {
        let mut p: Self = match std::fs::File::open(path) {
            Ok(file) => serde_json::from_reader(file)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        crate::i18n::ensure!(
            p.width.is_finite()
                && (1. ..=100.).contains(&p.width)
                && p.font_size.is_finite()
                && (6. ..=512.).contains(&p.font_size)
                && [p.color.0, p.color.1, p.color.2]
                    .iter()
                    .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
            "Ungültige Werkzeugeinstellungen"
        );
        if matches!(p.tool, Tool::Select | Tool::Marquee) {
            p.tool = Tool::Move;
        }
        p.export_directory = p.export_directory.filter(|p| p.is_absolute() && p.is_dir());
        p.layer_panel_width = p.layer_panel_width.clamp(252, 16384);
        Ok(p)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| crate::i18n::anyhow!("Kein Einstellungsverzeichnis"))?;
        std::fs::create_dir_all(parent)?;
        let tmp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(tmp.as_file(), self)?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
    pub fn monitor_index(&self, monitors: &[Monitor]) -> Option<usize> {
        let selected = self.monitor.as_ref()?;
        crate::geometry::monitor_index(monitors, selected)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    #[test]
    fn preferences_roundtrip_and_monitor_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        let m = Monitor {
            name: "Portrait".into(),
            rect: Rect::new(-1080., 0., 1080., 1920.),
            scale: 1.,
        };
        assert!(serde_json::from_str::<Preferences>("{}").unwrap().snapping);
        let p = Preferences {
            snapping: false,
            tool: Tool::Arrow,
            width: 7.,
            layer_panel_width: 380,
            font_size: 42.,
            tools_visible: false,
            export_directory: Some(dir.path().to_path_buf()),
            monitor: Some(m.clone()),
            ..Default::default()
        };
        p.save(&path).unwrap();
        let loaded = Preferences::load(&path).unwrap();
        assert_eq!(p, loaded);
        let mut other = m.clone();
        other.name = "Other".into();
        assert_eq!(loaded.monitor_index(&[other, m.clone()]), Some(1));
        assert_eq!(loaded.monitor_index(&[]), None);
        assert_eq!(loaded.monitor_index(&[m.clone(), m]), None);
        std::fs::write(&path, r#"{"width":0}"#).unwrap();
        assert!(Preferences::load(&path).is_err());
    }
}
