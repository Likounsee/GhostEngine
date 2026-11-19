//! Bounded cache for flattened vector path geometry.

use crate::{FlattenedPath, Path, PathError};

#[derive(Debug, Clone)]
struct Entry {
    key: u64,
    tolerance_bits: u32,
    last_used: u64,
    geometry: FlattenedPath,
}

/// Errors returned when a path geometry cache cannot be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathGeometryCacheError {
    /// The cache could not reserve its initial entry storage.
    AllocationFailed,
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
    /// Creates a cache while reporting failure to reserve its initial storage.
    pub fn try_new(capacity: usize) -> Result<Self, PathGeometryCacheError> {
        let capacity = capacity.max(1);
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(capacity)
            .map_err(|_| PathGeometryCacheError::AllocationFailed)?;
        Ok(Self {
            capacity,
            clock: 0,
            hits: 0,
            misses: 0,
            entries,
        })
    }

    /// Creates a cache with preallocated entry storage.
    ///
    /// Prefer [`Self::try_new`] when allocation failure must be handled.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self::try_new(capacity).expect("failed to allocate path geometry cache")
    }

    fn advance_clock(&mut self) {
        if self.clock == u64::MAX {
            // Rebase timestamps by recency rank before incrementing. Subtracting
            // the oldest timestamp alone is insufficient when it is already zero.
            self.entries.sort_unstable_by_key(|entry| entry.last_used);
            for (index, entry) in self.entries.iter_mut().enumerate() {
                entry.last_used = index as u64 + 1;
            }
            self.clock = self.entries.len() as u64 + 1;
        } else {
            self.clock += 1;
        }
    }

    pub fn get_or_flatten(
        &mut self,
        key: u64,
        path: &Path,
        tolerance: f32,
    ) -> Result<&FlattenedPath, PathError> {
        self.advance_clock();
        let tolerance_bits = tolerance.to_bits();
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.key == key && entry.tolerance_bits == tolerance_bits)
        {
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
    fn fallible_constructor_preallocates_requested_capacity() {
        let cache = PathGeometryCache::try_new(4).unwrap();
        assert_eq!(cache.capacity, 4);
        assert!(cache.is_empty());
        assert_eq!(cache.hit_count(), 0);
        assert_eq!(cache.miss_count(), 0);
    }

    #[test]
    fn fallible_constructor_keeps_minimum_capacity_of_one() {
        let cache = PathGeometryCache::try_new(0).unwrap();
        assert_eq!(cache.capacity, 1);
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
    fn lru_clock_rebases_without_losing_eviction_order() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(2);
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.get_or_flatten(2, &path, 0.25).unwrap();
        cache.entries[0].last_used = 0;
        cache.entries[1].last_used = u64::MAX - 1;
        cache.clock = u64::MAX;

        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.get_or_flatten(3, &path, 0.25).unwrap();

        assert!(cache.entries.iter().any(|entry| entry.key == 1));
        assert!(!cache.entries.iter().any(|entry| entry.key == 2));
        assert!(cache.entries.iter().any(|entry| entry.key == 3));
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
