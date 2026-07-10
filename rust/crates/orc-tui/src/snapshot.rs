use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;
use orc_core::model::RunMeta;
use orc_core::registry::{pid_alive, read_meta, runs_dir};

#[derive(Clone, Debug)]
struct Cached {
    modified: SystemTime,
    len: u64,
    meta: RunMeta,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    entries: HashMap<PathBuf, Cached>,
    pub runs: Vec<RunMeta>,
}

impl Snapshot {
    pub fn refresh(&mut self) -> Result<bool> {
        let root = runs_dir()?;
        let mut seen = HashSet::new();
        let mut changed = false;
        for entry in fs::read_dir(root)?.flatten() {
            let run_dir = entry.path();
            let meta_path = run_dir.join("meta.json");
            let Ok(stat) = fs::metadata(&meta_path) else {
                continue;
            };
            seen.insert(run_dir.clone());
            let modified = stat.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let is_current = self
                .entries
                .get(&run_dir)
                .is_some_and(|cached| cached.modified == modified && cached.len == stat.len());
            if is_current {
                continue;
            }
            let Ok(mut meta) = read_meta(&run_dir) else {
                continue;
            };
            meta.run_dir = Some(run_dir.clone());
            if meta.status == "running" && !pid_alive(meta.pid) {
                // View-only projection. The TUI never repairs meta.json.
                meta.status = "orphaned".to_owned();
            }
            self.entries.insert(
                run_dir,
                Cached {
                    modified,
                    len: stat.len(),
                    meta,
                },
            );
            changed = true;
        }
        let before = self.entries.len();
        self.entries.retain(|path, _| seen.contains(path));
        changed |= self.entries.len() != before;
        if changed || self.runs.is_empty() {
            self.runs = self
                .entries
                .values()
                .map(|cached| cached.meta.clone())
                .collect();
            self.runs.sort_by(|left, right| {
                right
                    .created_ts
                    .total_cmp(&left.created_ts)
                    .then_with(|| right.id.cmp(&left.id))
            });
        }
        Ok(changed)
    }

    #[must_use]
    pub fn from_runs(runs: Vec<RunMeta>) -> Self {
        Self {
            entries: HashMap::new(),
            runs,
        }
    }
}
