//! Retention thinning policy for pruning old backup snapshots.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{Datelike, Duration, IsoWeek, Utc};

use crate::BackupConfig;
use crate::index::{BackupEntry, BackupIndex, BackupType};

/// Returns IDs of entries that should be pruned based on the thinning schedule.
///
/// Thinning tiers (all thresholds measured in days from now):
/// - Age < keep_all_days: keep everything.
/// - keep_all_days <= age < keep_daily_days: keep the most recent entry per calendar day.
/// - keep_daily_days <= age < keep_weekly_days: keep the most recent entry per ISO week.
/// - age >= keep_weekly_days: keep the most recent entry per year-month.
///
/// A base entry is never pruned if it is the only base in the index.
/// When a base is pruned, all diffs that reference it are pruned as well.
/// Bases referenced by surviving diffs are preserved even if they would otherwise be pruned.
pub fn compute_prunable_entries(index: &BackupIndex, config: &BackupConfig) -> Vec<String> {
    let now = Utc::now();
    let keep_all_cutoff = now - Duration::days(config.keep_all_days as i64);
    let keep_daily_cutoff = now - Duration::days(config.keep_daily_days as i64);
    let keep_weekly_cutoff = now - Duration::days(config.keep_weekly_days as i64);

    #[derive(PartialEq, Eq, Hash)]
    enum PeriodKey {
        Daily(i32, u32),      // (year, ordinal day)
        Weekly(i32, IsoWeek), // (iso_year, iso_week)
        Monthly(i32, u32),    // (year, month)
    }

    let mut period_winners: HashMap<PeriodKey, &BackupEntry> = HashMap::new();

    for entry in &index.entries {
        let age_date = entry.created_at;

        if age_date >= keep_all_cutoff {
            continue;
        }

        let key = if age_date >= keep_daily_cutoff {
            PeriodKey::Daily(age_date.year(), age_date.ordinal())
        } else if age_date >= keep_weekly_cutoff {
            let iso = age_date.iso_week();
            PeriodKey::Weekly(iso.year(), iso)
        } else {
            PeriodKey::Monthly(age_date.year(), age_date.month())
        };

        period_winners
            .entry(key)
            .and_modify(|winner| {
                if entry.created_at > winner.created_at {
                    *winner = entry;
                }
            })
            .or_insert(entry);
    }

    let surviving_ids: HashSet<&str> = period_winners.values().map(|e| e.id.as_str()).collect();

    let base_ids: Vec<&str> = index
        .entries
        .iter()
        .filter(|e| e.entry_type == BackupType::Base)
        .map(|e| e.id.as_str())
        .collect();

    let mut prunable: HashSet<&str> = index
        .entries
        .iter()
        .filter(|e| {
            let age_date = e.created_at;
            age_date < keep_all_cutoff && !surviving_ids.contains(e.id.as_str())
        })
        .map(|e| e.id.as_str())
        .collect();

    if base_ids.len() == 1 {
        let sole_base_id = base_ids[0];
        prunable.remove(sole_base_id);
    }

    let mut changed = true;
    while changed {
        changed = false;

        for entry in &index.entries {
            if entry.entry_type != BackupType::Diff {
                continue;
            }

            let base_id = match entry.base_id.as_deref() {
                Some(id) => id,
                None => continue,
            };

            if prunable.contains(base_id) && !prunable.contains(entry.id.as_str()) {
                prunable.insert(entry.id.as_str());
                changed = true;
            }

            if !prunable.contains(entry.id.as_str()) && prunable.contains(base_id) {
                prunable.remove(base_id);
                changed = true;
            }
        }
    }

    prunable.iter().map(|id| id.to_string()).collect()
}

/// Removes entries from the index and deletes their backing files from disk.
///
/// For each prunable entry, the disk file is deleted first. Only after the delete succeeds
/// is the entry removed from the in-memory index, keeping the on-disk and in-memory views
/// aligned. Missing files (NotFound) are treated as already deleted and the entry is still
/// removed from the index.
///
/// On `Err`, the in-memory index reflects all deletions that completed before the failure.
pub fn apply_prune(
    index: &mut BackupIndex,
    prunable_ids: &[String],
    backups_dir: &Path,
) -> Result<()> {
    let id_set: HashSet<&str> = prunable_ids.iter().map(String::as_str).collect();

    let prunable_entries: Vec<(usize, String)> = index
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| id_set.contains(e.id.as_str()))
        .map(|(i, e)| (i, e.filename.clone()))
        .collect();

    let mut removed_indices: HashSet<usize> = HashSet::new();

    for (idx, filename) in prunable_entries {
        let file_path = backups_dir.join(&filename);
        match std::fs::remove_file(&file_path) {
            Ok(()) => {
                removed_indices.insert(idx);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                removed_indices.insert(idx);
            }
            Err(err) => {
                // Flush already-deleted entries into the in-memory index before
                // returning so the index is consistent with the disk state.
                let mut i = 0;
                index.entries.retain(|_| {
                    let keep = !removed_indices.contains(&i);
                    i += 1;
                    keep
                });
                return Err(err)
                    .with_context(|| format!("delete pruned backup {}", file_path.display()));
            }
        }
    }

    let mut i = 0;
    index.entries.retain(|_| {
        let keep = !removed_indices.contains(&i);
        i += 1;
        keep
    });

    Ok(())
}

/// Computes prunable entries and removes them from the index and disk in one step.
pub fn run_retention(index: &mut BackupIndex, config: &BackupConfig, backups_dir: &Path) -> Result<()> {
    let prunable = compute_prunable_entries(index, config);
    apply_prune(index, &prunable, backups_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};
    use tempfile::TempDir;

    fn make_entry(
        id: &str,
        entry_type: BackupType,
        base_id: Option<&str>,
        created_at: DateTime<Utc>,
    ) -> BackupEntry {
        let filename = match entry_type {
            BackupType::Base => format!("{id}-base.bak"),
            BackupType::Diff => format!("{id}-diff.bak"),
        };
        BackupEntry {
            id: id.to_string(),
            entry_type,
            filename,
            base_id: base_id.map(str::to_string),
            plaintext_hash: "aabb".to_string(),
            file_hash: "ccdd".to_string(),
            plaintext_size: 1024,
            stored_size: 512,
            encrypted: false,
            created_at,
            wrapped_dek: None,
            kek_fingerprint: None,
        }
    }

    fn default_config() -> BackupConfig {
        BackupConfig {
            enabled: true,
            keep_all_days: 7,
            keep_daily_days: 30,
            keep_weekly_days: 90,
            rebase_threshold_percent: 50,
            rebase_hard_ceiling: 10,
        }
    }

    #[test]
    fn keeps_all_recent_backups() {
        let config = default_config();
        let now = Utc::now();

        let mut index = BackupIndex::new();
        // Two base entries within keep_all_days -- neither should be pruned.
        index.entries.push(make_entry(
            "b1",
            BackupType::Base,
            None,
            now - Duration::days(1),
        ));
        index.entries.push(make_entry(
            "b2",
            BackupType::Base,
            None,
            now - Duration::hours(6),
        ));

        let prunable = compute_prunable_entries(&index, &config);
        assert!(
            prunable.is_empty(),
            "recent entries should not be pruned: {prunable:?}"
        );
    }

    #[test]
    fn thins_daily_tier() {
        let config = default_config();
        let now = Utc::now();

        let day_15_ago = (now - Duration::days(15))
            .date_naive()
            .and_hms_opt(1, 0, 0)
            .unwrap()
            .and_utc();
        let day_15_ago_later = day_15_ago + Duration::hours(2);

        let mut index = BackupIndex::new();
        // Older entry on that day.
        index
            .entries
            .push(make_entry("older", BackupType::Base, None, day_15_ago));
        // More recent entry on the same day -- this should survive.
        index.entries.push(make_entry(
            "newer",
            BackupType::Base,
            None,
            day_15_ago_later,
        ));

        let prunable = compute_prunable_entries(&index, &config);
        assert!(
            prunable.contains(&"older".to_string()),
            "older same-day entry should be pruned"
        );
        assert!(
            !prunable.contains(&"newer".to_string()),
            "newer same-day entry should survive"
        );
    }

    #[test]
    fn thins_weekly_tier() {
        let config = default_config();
        let now = Utc::now();

        // Three entries in the same ISO week, in the weekly tier (between keep_daily and keep_weekly).
        // Pick a Monday in that range so all three days fall in the same ISO week.
        let week_base = now - Duration::days(60);
        // Find the Monday of that week.
        let weekday_offset = week_base.weekday().num_days_from_monday() as i64;
        let monday = week_base - Duration::days(weekday_offset);

        let e1 = monday;
        let e2 = monday + Duration::days(1);
        let e3 = monday + Duration::days(2);

        let mut index = BackupIndex::new();
        index
            .entries
            .push(make_entry("e1", BackupType::Base, None, e1));
        index
            .entries
            .push(make_entry("e2", BackupType::Base, None, e2));
        index
            .entries
            .push(make_entry("e3", BackupType::Base, None, e3));

        let prunable = compute_prunable_entries(&index, &config);

        assert!(prunable.contains(&"e1".to_string()), "e1 should be pruned");
        assert!(prunable.contains(&"e2".to_string()), "e2 should be pruned");
        assert!(
            !prunable.contains(&"e3".to_string()),
            "e3 (most recent) should survive"
        );
    }

    #[test]
    fn never_prunes_sole_base() {
        let config = default_config();
        let now = Utc::now();

        // Single base entry that is very old -- should be protected.
        let ancient = now - Duration::days(365);

        let mut index = BackupIndex::new();
        index
            .entries
            .push(make_entry("sole_base", BackupType::Base, None, ancient));

        let prunable = compute_prunable_entries(&index, &config);
        assert!(
            !prunable.contains(&"sole_base".to_string()),
            "sole base must never be pruned"
        );
    }

    #[test]
    fn prunes_orphaned_diffs_when_base_pruned() {
        let config = default_config();
        let now = Utc::now();

        // Two bases, old and new. Old base has a diff. The old base should be pruned
        // (it loses the daily-tier election to the new base on the same day -- actually
        // put them in different months so the old one is in monthly tier and clearly prunable).
        let old_time = now - Duration::days(200);
        let new_time = now - Duration::days(1);

        let mut index = BackupIndex::new();
        // old base and its diff -- both should be pruned.
        index
            .entries
            .push(make_entry("old_base", BackupType::Base, None, old_time));
        index.entries.push(make_entry(
            "old_diff",
            BackupType::Diff,
            Some("old_base"),
            old_time + Duration::hours(1),
        ));
        // new base (sole recent backup) -- must survive.
        index
            .entries
            .push(make_entry("new_base", BackupType::Base, None, new_time));

        let prunable = compute_prunable_entries(&index, &config);

        assert!(
            prunable.contains(&"old_base".to_string()),
            "old base should be pruned"
        );
        assert!(
            prunable.contains(&"old_diff".to_string()),
            "diff of pruned base should also be pruned"
        );
        assert!(
            !prunable.contains(&"new_base".to_string()),
            "new base should survive"
        );
    }

    #[test]
    fn apply_prune_removes_files_and_entries() {
        let dir = TempDir::new().unwrap();
        let backups_dir = dir.path();
        let config = default_config();
        let now = Utc::now();

        // Place both old entries in the same month within the monthly tier so old_base loses
        // the period election to old_base_newer and becomes prunable. Anchor to day 15 so the
        // +1d neighbor never crosses a month boundary regardless of when the test runs.
        let old_base_time = (now - Duration::days(200))
            .date_naive()
            .with_day(15)
            .unwrap()
            .and_hms_opt(1, 0, 0)
            .unwrap()
            .and_utc();
        let old_base_newer_time = old_base_time + Duration::days(1);
        // new_base is in the keep-all tier.
        let new_time = now - Duration::days(1);

        let mut index = BackupIndex::new();
        index.entries.push(make_entry(
            "old_base",
            BackupType::Base,
            None,
            old_base_time,
        ));
        index.entries.push(make_entry(
            "old_base_newer",
            BackupType::Base,
            None,
            old_base_newer_time,
        ));
        index
            .entries
            .push(make_entry("new_base", BackupType::Base, None, new_time));

        // Write dummy files to disk.
        std::fs::write(backups_dir.join("old_base-base.bak"), b"data").unwrap();
        std::fs::write(backups_dir.join("old_base_newer-base.bak"), b"data").unwrap();
        std::fs::write(backups_dir.join("new_base-base.bak"), b"data").unwrap();

        let prunable = compute_prunable_entries(&index, &config);
        assert!(
            prunable.contains(&"old_base".to_string()),
            "old_base should be prunable"
        );
        assert!(
            !prunable.contains(&"old_base_newer".to_string()),
            "old_base_newer should survive as monthly winner"
        );
        assert!(
            !prunable.contains(&"new_base".to_string()),
            "new_base should survive (keep-all tier)"
        );

        apply_prune(&mut index, &prunable, backups_dir).unwrap();

        assert!(
            !backups_dir.join("old_base-base.bak").exists(),
            "old file should be deleted"
        );
        assert!(
            backups_dir.join("old_base_newer-base.bak").exists(),
            "monthly winner file should remain"
        );
        assert!(
            backups_dir.join("new_base-base.bak").exists(),
            "new file should remain"
        );
        assert_eq!(index.entries.len(), 2);
        let remaining_ids: Vec<&str> = index.entries.iter().map(|e| e.id.as_str()).collect();
        assert!(
            remaining_ids.contains(&"old_base_newer"),
            "old_base_newer must remain in index"
        );
        assert!(
            remaining_ids.contains(&"new_base"),
            "new_base must remain in index"
        );
    }

    #[test]
    fn apply_prune_partial_failure_removes_succeeded_entries_from_index() {
        let dir = TempDir::new().unwrap();
        let backups_dir = dir.path();
        let now = Utc::now();
        let old_time = now - Duration::days(200);

        let mut index = BackupIndex::new();
        index.entries.push(make_entry("first", BackupType::Base, None, old_time));
        index.entries.push(make_entry("fails", BackupType::Base, None, old_time + Duration::hours(1)));
        index.entries.push(make_entry("third", BackupType::Base, None, old_time + Duration::hours(2)));

        // Write "first" as a normal file that will be successfully deleted.
        std::fs::write(backups_dir.join("first-base.bak"), b"data").unwrap();
        // Write "fails" as a directory so remove_file returns EISDIR, not NotFound.
        std::fs::create_dir(backups_dir.join("fails-base.bak")).unwrap();
        // Write "third" as a normal file.
        std::fs::write(backups_dir.join("third-base.bak"), b"data").unwrap();

        let prunable = vec![
            "first".to_string(),
            "fails".to_string(),
            "third".to_string(),
        ];

        let result = apply_prune(&mut index, &prunable, backups_dir);

        assert!(result.is_err(), "apply_prune must return Err when a deletion fails");

        // The first file was successfully deleted before the error.
        assert!(
            !backups_dir.join("first-base.bak").exists(),
            "first file must be gone from disk"
        );
        // The in-memory index must not contain the entry whose file was already deleted.
        let remaining_ids: Vec<&str> = index.entries.iter().map(|e| e.id.as_str()).collect();
        assert!(
            !remaining_ids.contains(&"first"),
            "first entry must be removed from index after its file was deleted; remaining: {remaining_ids:?}"
        );
        // The entry that caused the error and anything after it must remain in the index.
        assert!(
            remaining_ids.contains(&"fails"),
            "fails entry must remain in index; remaining: {remaining_ids:?}"
        );
    }

    #[test]
    fn apply_prune_tolerates_missing_files() {
        let dir = TempDir::new().unwrap();
        let backups_dir = dir.path();
        let config = default_config();
        let now = Utc::now();

        // Same time-placement strategy as apply_prune_removes_files_and_entries:
        // anchor to day 15 so the +1d neighbor never crosses a month boundary.
        let old_base_time = (now - Duration::days(200))
            .date_naive()
            .with_day(15)
            .unwrap()
            .and_hms_opt(1, 0, 0)
            .unwrap()
            .and_utc();
        let old_base_newer_time = old_base_time + Duration::days(1);
        let new_time = now - Duration::days(1);

        let mut index = BackupIndex::new();
        index.entries.push(make_entry(
            "old_base",
            BackupType::Base,
            None,
            old_base_time,
        ));
        index.entries.push(make_entry(
            "old_base_newer",
            BackupType::Base,
            None,
            old_base_newer_time,
        ));
        index
            .entries
            .push(make_entry("new_base", BackupType::Base, None, new_time));

        // Write both non-prunable files to disk; intentionally omit old_base's file.
        std::fs::write(backups_dir.join("old_base_newer-base.bak"), b"data").unwrap();
        std::fs::write(backups_dir.join("new_base-base.bak"), b"data").unwrap();

        let prunable = compute_prunable_entries(&index, &config);
        assert!(
            prunable.contains(&"old_base".to_string()),
            "old_base must be in prunable set for this test to be meaningful"
        );

        // apply_prune must succeed even though old_base's backing file does not exist.
        apply_prune(&mut index, &prunable, backups_dir).unwrap();

        // The surviving file remains intact.
        assert!(
            backups_dir.join("old_base_newer-base.bak").exists(),
            "monthly winner file should remain"
        );
        assert!(
            backups_dir.join("new_base-base.bak").exists(),
            "new file should remain"
        );

        // Both prunable entries are removed from the index even though one had no file.
        assert_eq!(index.entries.len(), 2);
        let remaining_ids: Vec<&str> = index.entries.iter().map(|e| e.id.as_str()).collect();
        assert!(
            !remaining_ids.contains(&"old_base"),
            "old_base must be removed from index"
        );
        assert!(
            remaining_ids.contains(&"old_base_newer"),
            "old_base_newer must remain in index"
        );
        assert!(
            remaining_ids.contains(&"new_base"),
            "new_base must remain in index"
        );
    }
}
