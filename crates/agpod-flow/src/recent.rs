//! Recent task scoring: git trailers + doc updated_at + git file mtime.
//!
//! Collects evidence from three sources in as few passes as possible:
//! 1. Git commit trailers (single `git log` call)
//! 2. Doc frontmatter `updated_at` (single scan, shared with source 3)
//! 3. Git file mtime (single `git log` call over all doc roots)
//!
//! Keywords: recent tasks, scoring, time decay, git trailer, evidence

use crate::config::FlowDocsConfig;
use crate::error::FlowResult;
use crate::frontmatter::parse_frontmatter;
use crate::scanner;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

const WEIGHT_COMMIT_TRAILER: f64 = 100.0;
const WEIGHT_DOC_UPDATED_AT: f64 = 60.0;
const WEIGHT_GIT_FILE_MTIME: f64 = 30.0;
const DECAY_HALF_LIFE_DAYS: f64 = 7.0;
const MAX_EVIDENCE_PER_TASK: usize = 5;

#[derive(Debug, Clone, Serialize)]
pub struct RecentTask {
    pub task_id: String,
    pub score: f64,
    pub last_seen_at: String,
    pub evidence: Vec<String>,
    pub suggested_command: String,
}

/// Accumulates scores per task without intermediate allocations.
#[derive(Default)]
struct Scorer {
    scores: HashMap<String, f64>,
    last_seen: HashMap<String, DateTime<Utc>>,
    evidence: HashMap<String, Vec<String>>,
    /// Dedup key: (evidence_type, task_id, detail_hash) to avoid counting
    /// the same commit/doc twice.
    seen: HashSet<(u8, String, String)>,
}

// Evidence type tags for dedup
const TAG_TRAILER: u8 = 0;
const TAG_DOC: u8 = 1;
const TAG_MTIME: u8 = 2;

impl Scorer {
    fn add(
        &mut self,
        tag: u8,
        task_id: &str,
        ts: DateTime<Utc>,
        dedup_key: &str,
        summary: String,
        weight: f64,
        now: DateTime<Utc>,
    ) {
        if !self
            .seen
            .insert((tag, task_id.to_string(), dedup_key.to_string()))
        {
            return;
        }

        let decay = time_decay(now, ts);
        *self.scores.entry(task_id.to_string()).or_default() += weight * decay;

        self.last_seen
            .entry(task_id.to_string())
            .and_modify(|prev| {
                if ts > *prev {
                    *prev = ts;
                }
            })
            .or_insert(ts);

        let ev = self.evidence.entry(task_id.to_string()).or_default();
        if ev.len() < MAX_EVIDENCE_PER_TASK {
            ev.push(summary);
        }
    }

    fn into_results(mut self, limit: usize) -> Vec<RecentTask> {
        let mut results: Vec<RecentTask> = self
            .scores
            .drain()
            .map(|(task_id, score)| {
                let last_seen = self
                    .last_seen
                    .get(&task_id)
                    .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                    .unwrap_or_default();
                let evidence = self.evidence.remove(&task_id).unwrap_or_default();
                RecentTask {
                    suggested_command: format!("agpod flow -s <id> focus --task {task_id}"),
                    task_id,
                    score: (score * 10000.0).round() / 10000.0,
                    last_seen_at: last_seen,
                    evidence,
                }
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
                .then_with(|| a.task_id.cmp(&b.task_id))
        });

        results.truncate(limit);
        results
    }
}

fn time_decay(now: DateTime<Utc>, ts: DateTime<Utc>) -> f64 {
    let age_days = (now - ts).num_seconds().max(0) as f64 / 86400.0;
    0.5_f64.powf(age_days / DECAY_HALF_LIFE_DAYS)
}

/// Compute recent tasks with scoring.
pub fn recent_tasks(
    repo_root: &Path,
    config: &FlowDocsConfig,
    limit: usize,
    days: u32,
) -> FlowResult<Vec<RecentTask>> {
    let now = Utc::now();
    let cutoff = now - chrono::Duration::days(days as i64);
    let mut scorer = Scorer::default();

    // --- Source 1: git commit trailers (single git call) ---
    collect_trailer_evidence(repo_root, &cutoff, now, &mut scorer);

    // --- Source 2 + 3: scan docs once, then batch git mtime ---
    // Single scan pass: read each file once, extract task_id + updated_at
    let files = scanner::scan_documents(repo_root, config)?;
    // rel_path -> task_id mapping for git mtime cross-reference
    let mut path_to_task: HashMap<String, String> = HashMap::new();

    for file_path in &files {
        let rel = file_path
            .strip_prefix(repo_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let fm = match parse_frontmatter(&content, &rel) {
            Ok(fm) => fm,
            Err(_) => continue,
        };
        let task_id = match &fm.task_id {
            Some(id) => id.clone(),
            None => continue,
        };

        path_to_task.insert(rel.clone(), task_id.clone());

        // Source 2: doc frontmatter updated_at
        if let Some(updated) = &fm.updated_at {
            if let Ok(parsed) = DateTime::parse_from_rfc3339(updated) {
                let ts = parsed.with_timezone(&Utc);
                if ts >= cutoff {
                    scorer.add(
                        TAG_DOC,
                        &task_id,
                        ts,
                        &rel,
                        format!("doc updated_at in {rel}"),
                        WEIGHT_DOC_UPDATED_AT,
                        now,
                    );
                }
            }
        }
    }

    // --- Source 3: batch git file mtime (single git call) ---
    // `git log --since=<cutoff> --name-only --format=<date>` gives us
    // all recently-touched files with their commit dates in one shot.
    collect_batch_mtime_evidence(repo_root, &cutoff, now, &path_to_task, &mut scorer);

    Ok(scorer.into_results(limit))
}

/// Single `git log` call to extract commit trailers.
fn collect_trailer_evidence(
    repo_root: &Path,
    cutoff: &DateTime<Utc>,
    now: DateTime<Utc>,
    scorer: &mut Scorer,
) {
    let since = cutoff.format("%Y-%m-%d").to_string();
    // %x00 as record separator to avoid ambiguity with trailer values
    let format = "%H%x00%aI%x00%(trailers:key=Task-Id,valueonly,separator=%x01)%x00%(trailers:key=Root-Task-Id,valueonly,separator=%x01)";
    let output = Command::new("git")
        .args(["log", "--since", &since, &format!("--format={format}")])
        .current_dir(repo_root)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return,
    };

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.split('\0').collect();
        if parts.len() < 4 {
            continue;
        }
        let sha = parts[0].trim();
        let short_sha = &sha[..sha.len().min(7)];
        let ts = match DateTime::parse_from_rfc3339(parts[1].trim()) {
            Ok(d) => d.with_timezone(&Utc),
            Err(_) => continue,
        };

        // Task-Id trailer(s)
        for task_id in parts[2]
            .split('\x01')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            scorer.add(
                TAG_TRAILER,
                task_id,
                ts,
                sha,
                format!("commit {short_sha} trailer Task-Id"),
                WEIGHT_COMMIT_TRAILER,
                now,
            );
        }

        // Root-Task-Id trailer(s)
        for root_id in parts[3]
            .split('\x01')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            scorer.add(
                TAG_TRAILER,
                root_id,
                ts,
                sha,
                format!("commit {short_sha} trailer Root-Task-Id"),
                WEIGHT_COMMIT_TRAILER,
                now,
            );
        }
    }
}

/// Single `git log` call to get all recently modified files + dates,
/// then cross-reference with the doc->task mapping.
fn collect_batch_mtime_evidence(
    repo_root: &Path,
    cutoff: &DateTime<Utc>,
    now: DateTime<Utc>,
    path_to_task: &HashMap<String, String>,
    scorer: &mut Scorer,
) {
    if path_to_task.is_empty() {
        return;
    }

    let since = cutoff.format("%Y-%m-%d").to_string();
    // --name-only outputs blank-line-separated records:
    //   <date>
    //   <empty line>
    //   file1
    //   file2
    //   <empty line>
    let output = Command::new("git")
        .args([
            "log",
            "--since",
            &since,
            "--name-only",
            "--format=%aI",
            "--diff-filter=AMCR",
        ])
        .current_dir(repo_root)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return,
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut current_ts: Option<DateTime<Utc>> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Try parsing as date first — each commit block starts with a date
        if let Ok(parsed) = DateTime::parse_from_rfc3339(line) {
            current_ts = Some(parsed.with_timezone(&Utc));
            continue;
        }

        // Otherwise it's a file path — look up in our doc mapping
        let Some(ts) = current_ts else { continue };
        let Some(task_id) = path_to_task.get(line) else {
            continue;
        };

        scorer.add(
            TAG_MTIME,
            task_id,
            ts,
            line,
            format!("git modified {line}"),
            WEIGHT_GIT_FILE_MTIME,
            now,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_at_zero_is_one() {
        let now = Utc::now();
        assert!((time_decay(now, now) - 1.0).abs() < 0.001);
    }

    #[test]
    fn decay_at_half_life() {
        let now = Utc::now();
        let ts = now - chrono::Duration::days(7);
        assert!((time_decay(now, ts) - 0.5).abs() < 0.01);
    }

    #[test]
    fn decay_at_two_half_lives() {
        let now = Utc::now();
        let ts = now - chrono::Duration::days(14);
        assert!((time_decay(now, ts) - 0.25).abs() < 0.01);
    }

    #[test]
    fn scorer_dedup_same_evidence() {
        let now = Utc::now();
        let mut scorer = Scorer::default();
        scorer.add(TAG_DOC, "T-001", now, "doc.md", "test".into(), 60.0, now);
        scorer.add(TAG_DOC, "T-001", now, "doc.md", "test".into(), 60.0, now);
        let results = scorer.into_results(10);
        assert_eq!(results.len(), 1);
        // Only counted once
        assert!((results[0].score - 60.0).abs() < 0.01);
    }

    #[test]
    fn scorer_sorts_by_score_desc() {
        let now = Utc::now();
        let mut scorer = Scorer::default();
        scorer.add(TAG_DOC, "T-low", now, "a", "a".into(), 10.0, now);
        scorer.add(TAG_DOC, "T-high", now, "b", "b".into(), 100.0, now);
        let results = scorer.into_results(10);
        assert_eq!(results[0].task_id, "T-high");
        assert_eq!(results[1].task_id, "T-low");
    }
}
