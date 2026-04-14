pub mod index;
pub mod hash;
pub mod crypto;
pub mod diff;
pub mod export;
pub mod snapshot;
pub mod retention;
pub mod restore;
pub mod verify;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    pub enabled: bool,
    pub keep_all_days: u32,
    pub keep_daily_days: u32,
    pub keep_weekly_days: u32,
    pub rebase_threshold_percent: u32,
    pub rebase_hard_ceiling: u32,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_all_days: 7,
            keep_daily_days: 30,
            keep_weekly_days: 90,
            rebase_threshold_percent: 50,
            rebase_hard_ceiling: 10,
        }
    }
}
