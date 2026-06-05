//! Database schema versioning and automatic migration on startup.

pub fn migrate_config_path() {
    crate::config::migrate_config();
}
