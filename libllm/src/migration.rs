//! Database schema versioning and automatic migration on startup.

/// Outcome of a migration attempt: how many changes were applied and any non-fatal warnings.
pub struct MigrationResult {
    pub changed_count: usize,
    pub warnings: Vec<String>,
}

pub fn migrate_config_path() -> MigrationResult {
    crate::config::migrate_config();
    MigrationResult {
        changed_count: 0,
        warnings: Vec::new(),
    }
}
