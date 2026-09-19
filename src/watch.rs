//! Noticing that files changed underneath the reader.
//!
//! The watcher runs on its own thread and drops paths into a channel; the UI
//! drains that channel on a timer. Failing to start a watcher is not fatal —
//! `Ctrl+R` still reloads by hand.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

pub struct Watch {
    watcher: RecommendedWatcher,
    changes: Receiver<PathBuf>,
    roots: Vec<PathBuf>,
}

impl Watch {
    pub fn new() -> Option<Self> {
        let (tx, changes) = channel();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            // Access times and metadata churn are not content changes.
            if !matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                return;
            }
            for path in event.paths {
                if tx.send(path).is_err() {
                    return;
                }
            }
        })
        .ok()?;
        Some(Self {
            watcher,
            changes,
            roots: Vec::new(),
        })
    }

    /// Watch exactly these locations, dropping any previously watched ones.
    pub fn observe(&mut self, roots: impl IntoIterator<Item = PathBuf>) {
        let wanted: Vec<PathBuf> = roots.into_iter().filter(|p| p.exists()).collect();
        if wanted == self.roots {
            return;
        }
        for old in self.roots.drain(..) {
            let _ = self.watcher.unwatch(&old);
        }
        for root in wanted {
            let mode = if root.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            if self.watcher.watch(&root, mode).is_ok() {
                self.roots.push(root);
            }
        }
    }

    /// Every path that changed since the last call. A single save often
    /// produces several events, so they are collapsed into a set.
    pub fn drain(&self) -> HashSet<PathBuf> {
        let mut changed = HashSet::new();
        while let Ok(path) = self.changes.try_recv() {
            changed.insert(path);
        }
        changed
    }
}

/// Whether a set of changed paths touches one particular file.
pub fn touches(changed: &HashSet<PathBuf>, path: &Path) -> bool {
    changed.iter().any(|c| c == path)
}
