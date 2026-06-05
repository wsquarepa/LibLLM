//! Shared helpers for local time and 0o600-mode file creation used across the diagnostics module.

use std::fs::{File, OpenOptions};
use std::path::Path;

use time::UtcOffset;

pub(super) fn create_output_file(
    path: &Path,
    create_new: bool,
    truncate: bool,
) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    if create_new {
        options.create_new(true);
    }
    if truncate {
        options.truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub(super) fn local_now() -> time::OffsetDateTime {
    let now = time::OffsetDateTime::now_utc();
    match UtcOffset::current_local_offset() {
        Ok(offset) => now.to_offset(offset),
        Err(_) => now,
    }
}
