//! Bounded cache for flattened vector path geometry.

use crate::{FlattenedPath, Path, PathError};

#[derive(Debug, Clone)]
struct Entry {
    key: u64,
    tolerance_bits: u32,
    last_used: u64,
    geometry: FlattenedPath,
}

/// Reuses flattened path geometry across frames.
///
/// The caller supplies a stable key and must invalidate it when the source path
/// changes. Tolerance is part of the cache key. The cache has a fixed entry
/// limit and evicts the least recently used entry when full.
#[derive(Debug)]
pub struct PathGeometryCache {
    capacity: usize,
    clock: u64,
    hits: u64,
    misses: u64,
    entries: Vec<Entry>,
}

impl PathGeometryCache {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            clock: 0,
            hits: 0,
            misses: 0,
            entries: Vec::with_capacity(capacity),
        }
    }

    pub fn get_or_flatten(
        &mut self,
        key: u64,
        path: &Path,
        tolerance: f32,
    ) -> Result<&FlattenedPath, PathError> {
        self.clock = self.clock.wrapping_add(1);
        let tolerance_bits = tolerance.to_bits();
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.key == key && entry.tolerance_bits == tolerance_bits
        }) {
            self.hits = self.hits.saturating_add(1);
            self.entries[index].last_used = self.clock;
            return Ok(&self.entries[index].geometry);
        }

        self.misses = self.misses.saturating_add(1);
        let geometry = path.flatten(tolerance)?;
        if self.entries.len() == self.capacity {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| index)
                .unwrap();
            self.entries.swap_remove(oldest);
        }
        self.entries.push(Entry {
            key,
            tolerance_bits,
            last_used: self.clock,
            geometry,
        });
        Ok(&self.entries.last().unwrap().geometry)
    }

    pub fn invalidate(&mut self, key: u64) {
        self.entries.retain(|entry| entry.key != key);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[must_use]
    pub fn hit_count(&self) -> u64 {
        self.hits
    }

    #[must_use]
    pub fn miss_count(&self) -> u64 {
        self.misses
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vector;

    fn triangle() -> Path {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(4.0, 0.0))
            .line_to(Vector::new(2.0, 4.0))
            .close();
        path
    }

    #[test]
    fn reuses_geometry_and_tracks_cache_hits() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(2);
        let first = cache.get_or_flatten(7, &path, 0.25).unwrap().clone();
        let second = cache.get_or_flatten(7, &path, 0.25).unwrap();
        assert_eq!(&first, second);
        assert_eq!(cache.hit_count(), 1);
        assert_eq!(cache.miss_count(), 1);
    }

    #[test]
    fn tolerance_is_part_of_key_and_invalidation_removes_entries() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(2);
        cache.get_or_flatten(7, &path, 0.25).unwrap();
        cache.get_or_flatten(7, &path, 0.5).unwrap();
        assert_eq!(cache.len(), 2);
        cache.invalidate(7);
        assert!(cache.is_empty());
    }

    #[test]
    fn evicts_least_recently_used_entry() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(2);
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.get_or_flatten(2, &path, 0.25).unwrap();
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.get_or_flatten(3, &path, 0.25).unwrap();
        assert!(cache.entries.iter().any(|entry| entry.key == 1));
        assert!(!cache.entries.iter().any(|entry| entry.key == 2));
    }
}
