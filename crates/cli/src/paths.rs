//! Filesystem path helpers shared across the client binary.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Append a literal suffix to a path's existing OS string.
///
/// Unlike [`Path::with_extension`] which REPLACES the existing extension, this
/// preserves the full path and appends the suffix verbatim. Used for building
/// sidecar paths like `<dbpath>-wal` or temp paths like `<file>.tmp` where the
/// suffix must follow the original path exactly.
pub(crate) fn append_suffix(path: &Path, suffix: impl AsRef<OsStr>) -> PathBuf {
    let mut bytes = path.as_os_str().to_owned();
    bytes.push(suffix);
    PathBuf::from(bytes)
}
