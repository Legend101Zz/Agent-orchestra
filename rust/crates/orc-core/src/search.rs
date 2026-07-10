use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};

use crate::model::RunMeta;

const MAX_LOG_SEARCH_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub run_id: String,
    pub line: usize,
    pub text: String,
}

#[must_use]
pub fn search_runs(runs: &[RunMeta], query: &str, limit: usize) -> Vec<SearchHit> {
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for run in runs {
        if hits.len() >= limit {
            break;
        }
        let metadata_match = run.id.to_lowercase().contains(&needle)
            || run.task.to_lowercase().contains(&needle)
            || run
                .session
                .as_deref()
                .is_some_and(|session| session.to_lowercase().contains(&needle));
        if metadata_match {
            hits.push(SearchHit {
                run_id: run.id.clone(),
                line: 0,
                text: run.task.chars().take(160).collect(),
            });
            continue;
        }
        let Some(path) = run.run_dir.as_ref().map(|dir| dir.join("output.log")) else {
            continue;
        };
        let Ok(mut file) = File::open(path) else {
            continue;
        };
        let length = file.metadata().map_or(0, |metadata| metadata.len());
        let offset = length.saturating_sub(MAX_LOG_SEARCH_BYTES);
        if offset > 0 {
            if file.seek(SeekFrom::Start(offset)).is_err() {
                continue;
            }
        }
        let mut reader = BufReader::new(file);
        if offset > 0 {
            let mut partial = String::new();
            if reader.read_line(&mut partial).is_err() {
                continue;
            }
        }
        for (line_number, line) in reader.lines().enumerate() {
            let Ok(line) = line else { continue };
            if line.to_lowercase().contains(&needle) {
                hits.push(SearchHit {
                    run_id: run.id.clone(),
                    line: line_number + 1,
                    text: line.chars().take(160).collect(),
                });
                break;
            }
        }
    }
    hits
}
