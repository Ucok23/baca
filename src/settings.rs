//! What baca remembers between runs.
//!
//! Everything here is a convenience, never a correctness requirement: a
//! missing, unreadable or half-written settings file simply yields defaults.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The library folder last in use.
    pub root: Option<PathBuf>,
    /// The document last being read.
    pub open: Option<PathBuf>,
    pub theme: String,
    /// Whether the theme tracks the desktop's light/dark preference.
    pub follow_system_theme: bool,
    pub scale: f32,
    /// Whether the outline panel is showing.
    pub outline: bool,
    /// Vertical reading position per document, so reopening a long note lands
    /// where it was left rather than at the top.
    pub positions: BTreeMap<PathBuf, f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            root: None,
            open: None,
            theme: "Paper".into(),
            follow_system_theme: true,
            scale: 1.0,
            outline: true,
            positions: BTreeMap::new(),
        }
    }
}

/// Keep the remembered positions from growing without bound.
const MAX_POSITIONS: usize = 200;

impl Settings {
    pub fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("baca").join("settings.json"))
    }

    pub fn load() -> Self {
        let mut settings = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<Self>(&raw).ok())
            .unwrap_or_default();
        // A folder that has since been moved or deleted should not strand the
        // reader on an empty shelf with no explanation.
        settings.root = settings.root.filter(|p| p.is_dir());
        settings.open = settings.open.filter(|p| p.is_file());
        // Normalise as well as clamp, so a value written by an older build
        // does not persist as 1.3000001 forever.
        settings.scale = ((settings.scale * 10.).round() / 10.).clamp(0.6, 2.5);
        settings
    }

    pub fn remember_position(&mut self, path: &Path, offset: f32) {
        if self.positions.len() >= MAX_POSITIONS && !self.positions.contains_key(path) {
            // Drop an arbitrary old entry rather than tracking access times;
            // losing one remembered position is not worth more bookkeeping.
            if let Some(victim) = self.positions.keys().next().cloned() {
                self.positions.remove(&victim);
            }
        }
        self.positions.insert(path.to_path_buf(), offset);
    }

    pub fn position(&self, path: &Path) -> Option<f32> {
        self.positions.get(path).copied()
    }

    /// Write via a temporary file so an interrupted save cannot leave the
    /// settings truncated.
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let Some(dir) = path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let Ok(body) = serde_json::to_string_pretty(self) else {
            return;
        };
        let scratch = path.with_extension("json.tmp");
        if std::fs::write(&scratch, body).is_ok() {
            let _ = std::fs::rename(&scratch, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_survive_a_round_trip() {
        let raw = serde_json::to_string(&Settings::default()).unwrap();
        let back: Settings = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.theme, "Paper");
        assert!(back.follow_system_theme);
        assert_eq!(back.scale, 1.0);
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let back: Settings =
            serde_json::from_str(r#"{"theme":"Kraft","future_field":42}"#).unwrap();
        assert_eq!(back.theme, "Kraft");
        assert_eq!(back.scale, 1.0, "a missing field falls back to its default");
    }

    #[test]
    fn remembered_positions_are_bounded() {
        let mut settings = Settings::default();
        for i in 0..MAX_POSITIONS + 25 {
            settings.remember_position(Path::new(&format!("/notes/{i}.md")), i as f32);
        }
        assert!(settings.positions.len() <= MAX_POSITIONS);
    }

    #[test]
    fn a_position_comes_back() {
        let mut settings = Settings::default();
        let path = Path::new("/notes/a.md");
        settings.remember_position(path, -420.5);
        assert_eq!(settings.position(path), Some(-420.5));
        assert_eq!(settings.position(Path::new("/notes/b.md")), None);
    }
}
