//! Bounded cache for flattened vector path geometry.

use crate::{FlattenedPath, Path, PathError};

#[derive(Debug, Clone)]
struct Entry {
    key: u64,
    tolerance_bits: u32,
    last_used: u64,
    geometry_bytes: usize,
    geometry: FlattenedPath,
}

/// Errors returned when a path geometry cache cannot be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathGeometryCacheError {
    /// The cache could not reserve its initial entry storage.
    AllocationFailed,
}

/// Snapshot of path-geometry cache usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PathGeometryCacheStats {
    pub entries: usize,
    pub capacity: usize,
    /// Estimated bytes held by the cache's entry and geometry vector capacities.
    pub allocated_bytes: usize,
    /// Bytes held by cached flattened geometry, excluding entry-table overhead.
    pub geometry_bytes: usize,
    /// Maximum cached flattened-geometry bytes. Oversized paths are returned
    /// uncached.
    pub budget_bytes: usize,
    pub hits: u64,
    pub misses: u64,
}

/// Reuses flattened path geometry across frames.
///
/// The caller supplies a stable key and must invalidate it when the source path
/// changes. Tolerance is part of the cache key. The cache has a fixed entry
/// limit and an optional geometry-memory budget. Least-recently-used entries
/// are evicted when either limit is reached. A single geometry larger than
/// the memory budget is returned to the caller but is not retained in the cache.
#[derive(Debug)]
pub struct PathGeometryCache {
    capacity: usize,
    budget_bytes: usize,
    uncached_geometry: Option<FlattenedPath>,
    clock: u64,
    hits: u64,
    misses: u64,
    entries: Vec<Entry>,
    geometry_bytes: usize,
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
            budget_bytes: usize::MAX,
            uncached_geometry: None,
            clock: 0,
            hits: 0,
            misses: 0,
            entries,
            geometry_bytes: 0,
        })
    }

    /// Creates a cache with entry-count and geometry-memory limits.
    ///
    /// Geometry larger than the budget is returned normally but not retained.
    pub fn try_with_budget(
        capacity: usize,
        budget_bytes: usize,
    ) -> Result<Self, PathGeometryCacheError> {
        let mut cache = Self::try_new(capacity)?;
        cache.budget_bytes = budget_bytes;
        Ok(cache)
    }

    /// Creates a cache with entry-count and geometry-memory limits.
    #[must_use]
    pub fn with_budget(capacity: usize, budget_bytes: usize) -> Self {
        Self::try_with_budget(capacity, budget_bytes)
            .expect("failed to allocate path geometry cache")
    }

    /// Creates a cache with preallocated entry storage.
    ///
    /// Prefer [`Self::try_new`] when allocation failure must be handled.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self::try_new(capacity).expect("failed to allocate path geometry cache")
    }

    /// Changes the maximum number of cached geometries.
    ///
    /// Growing the cache reserves storage before changing its configured
    /// capacity. Shrinking evicts least-recently-used entries immediately.
    /// A zero capacity is normalized to one, matching construction.
    pub fn set_capacity(&mut self, capacity: usize) -> Result<(), PathGeometryCacheError> {
        let capacity = capacity.max(1);
        if capacity > self.capacity {
            self.entries
                .try_reserve_exact(capacity.saturating_sub(self.entries.len()))
                .map_err(|_| PathGeometryCacheError::AllocationFailed)?;
        }
        if capacity < self.entries.len() {
            self.evict_oldest_until(capacity, usize::MAX);
        }
        self.capacity = capacity;
        Ok(())
    }

    /// Returns a consistent snapshot of cache occupancy and lookup counters.
    #[must_use]
    pub fn stats(&self) -> PathGeometryCacheStats {
        PathGeometryCacheStats {
            entries: self.entries.len(),
            capacity: self.capacity,
            allocated_bytes: self.allocated_bytes(),
            geometry_bytes: self.cached_geometry_bytes(),
            budget_bytes: self.budget_bytes,
            hits: self.hits,
            misses: self.misses,
        }
    }

    /// Resets lookup counters without discarding cached geometry.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    /// Estimates memory retained by the cache's vector capacities.
    ///
    /// This includes the entry vector and flattened geometry vectors. It
    /// excludes allocator metadata and unrelated memory.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.entries
            .capacity()
            .saturating_mul(core::mem::size_of::<Entry>())
            .saturating_add(self.cached_geometry_bytes())
            .saturating_add(
                self.uncached_geometry
                    .as_ref()
                    .map_or(0, Self::flattened_geometry_bytes),
            )
    }

    /// Returns the geometry payload bytes currently retained in cache entries.
    #[must_use]
    pub fn cached_geometry_bytes(&self) -> usize {
        self.geometry_bytes
    }

    fn flattened_geometry_bytes(geometry: &FlattenedPath) -> usize {
        let subpaths_bytes = geometry
            .subpaths
            .capacity()
            .saturating_mul(core::mem::size_of::<crate::FlattenedSubpath>());
        geometry
            .subpaths
            .iter()
            .fold(subpaths_bytes, |bytes, subpath| {
                bytes.saturating_add(
                    subpath
                        .points
                        .capacity()
                        .saturating_mul(core::mem::size_of::<crate::Vector>()),
                )
            })
    }

    /// Changes the maximum cached geometry payload and evicts least-recently
    /// used entries immediately until the new budget is satisfied.
    pub fn set_budget_bytes(&mut self, budget_bytes: usize) {
        // A transient oversized result is no longer needed once the caller
        // changes cache policy, so do not retain it across the budget change.
        self.uncached_geometry = None;
        self.budget_bytes = budget_bytes;
        self.evict_oldest_until(self.capacity, budget_bytes);
    }

    /// Drops cached geometries while retaining the entry vector for reuse.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.geometry_bytes = 0;
        self.uncached_geometry = None;
    }

    /// Drops cached geometries and releases the cache's retained entry storage.
    ///
    /// Lookup counters and the configured entry limit are preserved.
    pub fn release(&mut self) {
        self.entries.clear();
        self.geometry_bytes = 0;
        self.entries.shrink_to_fit();
        self.uncached_geometry = None;
    }

    /// Removes the least-recently-used prefix until both cache limits fit.
    ///
    /// Entries are ordered once per trimming operation. This avoids repeatedly
    /// searching for the minimum timestamp and recomputing the full geometry
    /// payload after every eviction.
    fn evict_oldest_until(&mut self, max_entries: usize, max_geometry_bytes: usize) {
        if self.entries.len() <= max_entries && self.geometry_bytes <= max_geometry_bytes {
            return;
        }

        self.entries.sort_unstable_by_key(|entry| entry.last_used);
        let mut remove_count = 0usize;
        let mut remaining_bytes = self.geometry_bytes;
        while remove_count < self.entries.len()
            && (self.entries.len() - remove_count > max_entries
                || remaining_bytes > max_geometry_bytes)
        {
            remaining_bytes =
                remaining_bytes.saturating_sub(self.entries[remove_count].geometry_bytes);
            remove_count += 1;
        }
        if remove_count > 0 {
            self.entries.drain(..remove_count);
            self.geometry_bytes = remaining_bytes;
        }
        debug_assert!(self.entries.len() <= max_entries);
        debug_assert!(self.geometry_bytes <= max_geometry_bytes);
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
        // Oversized results live only until the next lookup.
        self.uncached_geometry = None;
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
        let geometry_bytes = Self::flattened_geometry_bytes(&geometry);
        if geometry_bytes > self.budget_bytes {
            self.uncached_geometry = Some(geometry);
            return Ok(self.uncached_geometry.as_ref().unwrap());
        }

        self.evict_oldest_until(
            self.capacity.saturating_sub(1),
            self.budget_bytes - geometry_bytes,
        );
        self.entries.push(Entry {
            key,
            tolerance_bits,
            last_used: self.clock,
            geometry_bytes,
            geometry,
        });
        self.geometry_bytes = self.geometry_bytes.saturating_add(geometry_bytes);
        Ok(&self.entries.last().unwrap().geometry)
    }

    pub fn invalidate(&mut self, key: u64) {
        let mut removed_bytes = 0usize;
        self.entries.retain(|entry| {
            if entry.key == key {
                removed_bytes = removed_bytes.saturating_add(entry.geometry_bytes);
                false
            } else {
                true
            }
        });
        self.geometry_bytes = self.geometry_bytes.saturating_sub(removed_bytes);
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
    fn memory_accounting_tracks_geometry_clear_and_release() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(4);
        let empty_allocation = cache.allocated_bytes();
        assert!(empty_allocation >= 4 * core::mem::size_of::<Entry>());

        cache.get_or_flatten(1, &path, 0.25).unwrap();
        let populated = cache.allocated_bytes();
        assert!(populated > empty_allocation);
        assert_eq!(cache.stats().allocated_bytes, populated);

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.allocated_bytes(), empty_allocation);

        cache.get_or_flatten(2, &path, 0.25).unwrap();
        assert!(cache.allocated_bytes() > empty_allocation);
        cache.release();
        assert_eq!(cache.allocated_bytes(), 0);
        assert_eq!(cache.stats().entries, 0);
        assert_eq!(cache.stats().capacity, 4);
    }

    #[test]
    fn retained_memory_tracks_lru_eviction_resize_and_invalidation() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(3);
        let empty_bytes = cache.allocated_bytes();

        cache.get_or_flatten(1, &path, 0.25).unwrap();
        let one_entry_bytes = cache.allocated_bytes();
        cache.get_or_flatten(2, &path, 0.25).unwrap();
        let two_entry_bytes = cache.allocated_bytes();
        assert!(one_entry_bytes > empty_bytes);
        assert!(two_entry_bytes > one_entry_bytes);

        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.get_or_flatten(3, &path, 0.25).unwrap();
        let three_entry_bytes = cache.allocated_bytes();
        assert!(three_entry_bytes > two_entry_bytes);

        cache.get_or_flatten(4, &path, 0.25).unwrap();
        assert_eq!(cache.len(), 3);
        assert!(cache.allocated_bytes() <= three_entry_bytes);

        cache.invalidate(1);
        assert_eq!(cache.len(), 2);
        assert!(cache.allocated_bytes() < three_entry_bytes);

        cache.set_capacity(1).unwrap();
        assert_eq!(cache.len(), 1);
        assert!(cache.allocated_bytes() < three_entry_bytes);
    }

    #[test]
    fn geometry_budget_evicts_lru_entries_and_reports_payload_bytes() {
        let path = triangle();
        let mut cache = PathGeometryCache::with_budget(4, usize::MAX);
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        let first_bytes = cache.cached_geometry_bytes();
        cache.get_or_flatten(2, &path, 0.25).unwrap();
        assert_eq!(cache.cached_geometry_bytes(), first_bytes * 2);

        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.set_budget_bytes(first_bytes);
        assert_eq!(cache.cached_geometry_bytes(), first_bytes);
        assert!(cache.entries.iter().any(|entry| entry.key == 1));
        assert!(!cache.entries.iter().any(|entry| entry.key == 2));
        assert_eq!(cache.stats().geometry_bytes, first_bytes);
        assert_eq!(cache.stats().budget_bytes, first_bytes);
    }

    #[test]
    fn batched_eviction_keeps_geometry_byte_accounting_consistent() {
        let path = triangle();
        let mut cache = PathGeometryCache::with_budget(8, usize::MAX);
        for key in 1..=6 {
            cache.get_or_flatten(key, &path, 0.25).unwrap();
        }
        let one = cache.cached_geometry_bytes() / 6;
        cache.get_or_flatten(2, &path, 0.25).unwrap();
        cache.get_or_flatten(4, &path, 0.25).unwrap();
        cache.get_or_flatten(6, &path, 0.25).unwrap();

        // Keep only the three most recently used geometries.
        cache.set_capacity(3).unwrap();
        assert_eq!(cache.len(), 3);
        assert!(cache.entries.iter().any(|entry| entry.key == 2));
        assert!(cache.entries.iter().any(|entry| entry.key == 4));
        assert!(cache.entries.iter().any(|entry| entry.key == 6));
        assert_eq!(cache.cached_geometry_bytes(), one * 3);
        assert_eq!(
            cache.cached_geometry_bytes(),
            cache.entries.iter().map(|entry| entry.geometry_bytes).sum()
        );

        cache.invalidate(4);
        assert_eq!(cache.cached_geometry_bytes(), one * 2);
        cache.clear();
        assert_eq!(cache.cached_geometry_bytes(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn insertion_evicts_multiple_oldest_geometries_in_one_trim() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(8);
        for key in 1..=5 {
            cache.get_or_flatten(key, &path, 0.25).unwrap();
        }
        let one = cache.cached_geometry_bytes() / 5;
        cache.get_or_flatten(2, &path, 0.25).unwrap();
        cache.get_or_flatten(4, &path, 0.25).unwrap();
        cache.set_budget_bytes(one * 3);

        // Key 6 fits only after evicting two older entries.
        cache.get_or_flatten(6, &path, 0.25).unwrap();
        assert_eq!(cache.len(), 3);
        assert!(cache.entries.iter().any(|entry| entry.key == 2));
        assert!(cache.entries.iter().any(|entry| entry.key == 4));
        assert!(cache.entries.iter().any(|entry| entry.key == 6));
        assert_eq!(cache.cached_geometry_bytes(), one * 3);
    }

    #[test]
    fn oversized_geometry_is_returned_but_not_retained() {
        let path = triangle();
        let mut unlimited = PathGeometryCache::new(2);
        unlimited.get_or_flatten(1, &path, 0.25).unwrap();
        let geometry_bytes = unlimited.cached_geometry_bytes();
        let mut bounded = PathGeometryCache::with_budget(2, geometry_bytes - 1);

        let first = bounded.get_or_flatten(1, &path, 0.25).unwrap().clone();
        assert_eq!(first, path.flatten(0.25).unwrap());
        assert!(bounded.is_empty());
        assert_eq!(bounded.cached_geometry_bytes(), 0);
        assert!(bounded.allocated_bytes() > 0);

        let second = bounded.get_or_flatten(1, &path, 0.25).unwrap();
        assert_eq!(second, &first);
        assert_eq!(bounded.stats().misses, 2);
        assert_eq!(bounded.stats().entries, 0);
    }

    #[test]
    fn changing_budget_releases_previous_uncached_result() {
        let path = triangle();
        let mut unlimited = PathGeometryCache::new(2);
        unlimited.get_or_flatten(1, &path, 0.25).unwrap();
        let geometry_bytes = unlimited.cached_geometry_bytes();
        let mut cache = PathGeometryCache::with_budget(2, geometry_bytes - 1);
        let empty_bytes = cache.allocated_bytes();

        cache.get_or_flatten(1, &path, 0.25).unwrap();
        assert!(cache.allocated_bytes() > empty_bytes);
        cache.set_budget_bytes(0);
        assert_eq!(cache.allocated_bytes(), empty_bytes);
        assert_eq!(cache.cached_geometry_bytes(), 0);
    }

    #[test]
    fn geometry_budget_zero_disables_retention_and_clear_drops_transient_geometry() {
        let path = triangle();
        let mut cache = PathGeometryCache::with_budget(2, 0);
        let empty_bytes = cache.allocated_bytes();
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        assert!(cache.is_empty());
        assert!(cache.allocated_bytes() > empty_bytes);
        cache.clear();
        assert_eq!(cache.allocated_bytes(), empty_bytes);
        assert_eq!(cache.cached_geometry_bytes(), 0);
    }

    #[test]
    fn resizing_cache_evicts_oldest_entries_and_preserves_recent_geometry() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(3);
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.get_or_flatten(2, &path, 0.25).unwrap();
        cache.get_or_flatten(3, &path, 0.25).unwrap();
        cache.get_or_flatten(1, &path, 0.25).unwrap();

        cache.set_capacity(2).unwrap();

        assert_eq!(cache.stats().entries, 2);
        assert_eq!(cache.stats().capacity, 2);
        assert!(cache.entries.iter().any(|entry| entry.key == 1));
        assert!(!cache.entries.iter().any(|entry| entry.key == 2));
        assert!(cache.entries.iter().any(|entry| entry.key == 3));
    }

    #[test]
    fn growing_cache_retains_entries_and_zero_capacity_normalizes_to_one() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(1);
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.set_capacity(4).unwrap();
        cache.get_or_flatten(2, &path, 0.25).unwrap();
        assert_eq!(cache.stats().capacity, 4);
        assert_eq!(cache.len(), 2);

        cache.set_capacity(0).unwrap();
        assert_eq!(cache.stats().capacity, 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn resetting_stats_does_not_discard_cached_geometry() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(2);
        cache.get_or_flatten(7, &path, 0.25).unwrap();
        cache.get_or_flatten(7, &path, 0.25).unwrap();
        cache.reset_stats();

        assert_eq!(
            cache.stats(),
            PathGeometryCacheStats {
                entries: 1,
                capacity: 2,
                allocated_bytes: cache.allocated_bytes(),
                geometry_bytes: cache.cached_geometry_bytes(),
                budget_bytes: cache.budget_bytes,
                hits: 0,
                misses: 0,
            }
        );
        cache.get_or_flatten(7, &path, 0.25).unwrap();
        assert_eq!(cache.hit_count(), 1);
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
    fn repeated_cache_lifecycle_keeps_capacity_and_lookup_accounting_consistent() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(4);

        for step in 0..2_048_u64 {
            let key = step % 23;
            let tolerance = match step % 3 {
                0 => 0.125,
                1 => 0.25,
                _ => 0.5,
            };
            cache.get_or_flatten(key, &path, tolerance).unwrap();

            if step % 11 == 0 {
                cache.invalidate(key);
            }
            if step % 29 == 0 {
                cache.set_capacity((step as usize % 7) + 1).unwrap();
            }
            if step % 97 == 0 {
                cache.clear();
            }

            let stats = cache.stats();
            assert!(stats.capacity >= 1);
            assert!(stats.entries <= stats.capacity);
            assert_eq!(stats.entries, cache.len());
            assert_eq!(
                stats.geometry_bytes,
                cache
                    .entries
                    .iter()
                    .map(|entry| entry.geometry_bytes)
                    .sum::<usize>()
            );
            assert_eq!(stats.hits + stats.misses, step + 1);
        }

        cache.reset_stats();
        assert_eq!(cache.hit_count(), 0);
        assert_eq!(cache.miss_count(), 0);
        assert!(cache.len() <= cache.stats().capacity);
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

    #[test]
    fn failed_flatten_does_not_evict_valid_geometry_or_corrupt_lru_state() {
        let valid = triangle();
        let mut invalid = Path::new();
        invalid.line_to(Vector::new(1.0, 1.0));
        let mut cache = PathGeometryCache::new(1);

        cache.get_or_flatten(10, &valid, 0.25).unwrap();
        let before = cache.stats();

        assert!(cache.get_or_flatten(20, &invalid, 0.25).is_err());
        assert_eq!(cache.stats().entries, before.entries);
        assert_eq!(cache.stats().hits, before.hits);
        assert_eq!(cache.stats().misses, before.misses + 1);

        // A failed miss must not replace the existing entry or make it older.
        cache.get_or_flatten(10, &valid, 0.25).unwrap();
        assert_eq!(cache.hit_count(), 1);
        assert_eq!(cache.len(), 1);

        // A later successful miss still performs the expected bounded eviction.
        cache.get_or_flatten(30, &valid, 0.5).unwrap();
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.stats().misses, 3);
        assert!(cache.entries.iter().any(|entry| entry.key == 30));
        assert!(!cache.entries.iter().any(|entry| entry.key == 10));
    }

    #[test]
    fn clear_preserves_lookup_counters_and_cache_remains_reusable() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(2);
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.get_or_flatten(1, &path, 0.25).unwrap();
        cache.get_or_flatten(2, &path, 0.25).unwrap();

        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 2);

        cache.get_or_flatten(3, &path, 0.25).unwrap();
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.stats().misses, 3);
        cache.get_or_flatten(3, &path, 0.25).unwrap();
        assert_eq!(cache.stats().hits, 2);
    }

    #[test]
    fn tolerance_bits_are_isolated_and_invalid_tolerances_never_enter_cache() {
        let path = triangle();
        let mut cache = PathGeometryCache::new(4);

        cache.get_or_flatten(7, &path, 0.25).unwrap();
        cache.get_or_flatten(7, &path, 0.5).unwrap();
        assert_eq!(cache.len(), 2);

        for tolerance in [0.0, -0.25, f32::NAN, f32::INFINITY] {
            assert!(cache.get_or_flatten(9, &path, tolerance).is_err());
        }

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.stats().misses, 6);
        assert_eq!(cache.stats().hits, 0);
        cache.get_or_flatten(7, &path, 0.25).unwrap();
        assert_eq!(cache.stats().hits, 1);
    }
}
