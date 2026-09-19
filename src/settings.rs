//! What baca remembers between runs.
//!
//! Everything here is a convenience, never a correctness requirement: a
//! missing, unreadable or half-written settings file simply yields defaults.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A document the reader has opened, for the home screen's recent list.
#[derive(Clone, Serialize, Deserialize)]
pub struct Recent {
    pub path: PathBuf,
    pub title: String,
    /// The folder it came from, shown as context beside the title.
    pub collection: String,
    /// Seconds since the epoch, when it was last opened.
    pub at: u64,
    /// How far through it the reader got, 0.0 to 1.0.
    pub progress: f32,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// A human reading of how long ago something was.
pub fn ago(at: u64) -> String {
    let seconds = now().saturating_sub(at);
    match seconds {
        0..=90 => "just now".into(),
        91..=5400 => format!("{} min ago", seconds / 60),
        5401..=79200 => format!("{} hr ago", seconds / 3600),
        79201..=172_800 => "yesterday".into(),
        172_801..=604_800 => format!("{} days ago", seconds / 86400),
        604_801..=2_592_000 => format!("{} weeks ago", seconds / 604_800),
        _ => format!("{} months ago", (seconds / 2_592_000).max(1)),
    }
}

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
    /// Folders the reader added by hand, on top of the ones baca finds.
    pub folders: Vec<PathBuf>,
    /// Documents opened recently, newest first.
    pub recent: Vec<Recent>,
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
            folders: Vec::new(),
            recent: Vec::new(),
            positions: BTreeMap::new(),
        }
    }
}

/// Keep the remembered positions from growing without bound.
const MAX_POSITIONS: usize = 200;

/// How many documents the home screen remembers.
const MAX_RECENT: usize = 30;

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
        settings.folders.retain(|p| p.is_dir());
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

    /// Note that a document was opened, or move it back to the front.
    pub fn remember_read(&mut self, path: &Path, title: &str, collection: &str, progress: f32) {
        self.recent.retain(|r| r.path != path);
        self.recent.insert(
            0,
            Recent {
                path: path.to_path_buf(),
                title: title.to_string(),
                collection: collection.to_string(),
                at: now(),
                progress: progress.clamp(0., 1.),
            },
        );
        self.recent.truncate(MAX_RECENT);
    }

    /// Update the progress of whatever is at the front, without reordering.
    pub fn update_progress(&mut self, path: &Path, progress: f32) {
        if let Some(entry) = self.recent.iter_mut().find(|r| r.path == path) {
            entry.progress = progress.clamp(0., 1.);
            entry.at = now();
        }
    }

    /// Recent documents that still exist on disk.
    pub fn recent_reads(&self) -> Vec<&Recent> {
        self.recent.iter().filter(|r| r.path.is_file()).collect()
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

#[cfg(test)]
mod recent_tests {
    use super::*;

    #[test]
    fn reading_something_again_moves_it_to_the_front() {
        let mut settings = Settings::default();
        settings.remember_read(Path::new("/a.md"), "A", "notes", 0.1);
        settings.remember_read(Path::new("/b.md"), "B", "notes", 0.2);
        settings.remember_read(Path::new("/a.md"), "A", "notes", 0.3);
        let paths: Vec<_> = settings.recent.iter().map(|r| r.path.clone()).collect();
        assert_eq!(paths, vec![PathBuf::from("/a.md"), PathBuf::from("/b.md")]);
        assert_eq!(settings.recent.len(), 2, "no duplicate entry is kept");
    }

    #[test]
    fn the_recent_list_is_bounded() {
        let mut settings = Settings::default();
        for i in 0..MAX_RECENT + 10 {
            settings.remember_read(Path::new(&format!("/{i}.md")), "t", "c", 0.);
        }
        assert_eq!(settings.recent.len(), MAX_RECENT);
    }

    #[test]
    fn progress_is_updated_without_reordering() {
        let mut settings = Settings::default();
        settings.remember_read(Path::new("/a.md"), "A", "notes", 0.1);
        settings.remember_read(Path::new("/b.md"), "B", "notes", 0.2);
        settings.update_progress(Path::new("/a.md"), 0.9);
        assert_eq!(settings.recent[0].path, PathBuf::from("/b.md"));
        let a = settings
            .recent
            .iter()
            .find(|r| r.path == Path::new("/a.md"));
        assert_eq!(a.unwrap().progress, 0.9);
    }

    #[test]
    fn progress_stays_within_range() {
        let mut settings = Settings::default();
        settings.remember_read(Path::new("/a.md"), "A", "c", 4.2);
        assert_eq!(settings.recent[0].progress, 1.0);
    }

    #[test]
    fn elapsed_time_reads_as_words() {
        let now = now();
        assert_eq!(ago(now), "just now");
        assert_eq!(ago(now - 600), "10 min ago");
        assert_eq!(ago(now - 7200), "2 hr ago");
        assert_eq!(ago(now - 90_000), "yesterday");
        assert_eq!(ago(now - 259_200), "3 days ago");
    }
}
