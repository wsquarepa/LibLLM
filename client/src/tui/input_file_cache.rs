//! In-memory cache of `@<token>` resolutions kept while the user is
//! composing input, so the Est. token counter can sum file
//! contributions without re-reading disk on every keystroke.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CachedResolution {
    pub estimated_tokens: usize,
}

#[derive(Debug, Default)]
pub struct InputFileCache {
    entries: HashMap<PathBuf, CachedResolution>,
}

impl InputFileCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, canonical_path: PathBuf, res: CachedResolution) {
        self.entries.insert(canonical_path, res);
    }

    pub fn lookup(&self, canonical_path: &std::path::Path) -> Option<&CachedResolution> {
        self.entries.get(canonical_path)
    }

    pub fn retain_paths(&mut self, live: &HashSet<PathBuf>) {
        self.entries.retain(|k, _| live.contains(k));
    }

    pub fn sum_estimated_tokens(&self) -> usize {
        self.entries.values().map(|r| r.estimated_tokens).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn insert_and_lookup() {
        let mut cache = InputFileCache::new();
        cache.insert(
            PathBuf::from("/tmp/a.md"),
            CachedResolution { estimated_tokens: 3 },
        );
        assert_eq!(
            cache.lookup(Path::new("/tmp/a.md")).map(|r| r.estimated_tokens),
            Some(3)
        );
    }

    #[test]
    fn retain_paths_evicts_missing() {
        let mut cache = InputFileCache::new();
        cache.insert(
            PathBuf::from("/tmp/a.md"),
            CachedResolution { estimated_tokens: 1 },
        );
        cache.insert(
            PathBuf::from("/tmp/b.md"),
            CachedResolution { estimated_tokens: 1 },
        );
        let mut live = HashSet::new();
        live.insert(PathBuf::from("/tmp/a.md"));
        cache.retain_paths(&live);
        assert!(cache.lookup(Path::new("/tmp/a.md")).is_some());
        assert!(cache.lookup(Path::new("/tmp/b.md")).is_none());
    }

    #[test]
    fn sum_estimated_tokens_adds_entries() {
        let mut cache = InputFileCache::new();
        cache.insert(
            PathBuf::from("/tmp/a.md"),
            CachedResolution { estimated_tokens: 3 },
        );
        cache.insert(
            PathBuf::from("/tmp/b.md"),
            CachedResolution { estimated_tokens: 5 },
        );
        assert_eq!(cache.sum_estimated_tokens(), 8);
    }
}
