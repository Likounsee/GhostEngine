//! Bounded, ID-addressable cache for immutable CPU textures.

use std::collections::HashMap;

use crate::Texture;

/// Stable handle returned when a texture is inserted into a cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(u64);

impl TextureHandle {
    /// Returns the cache-local numeric identifier.
    #[must_use]
    pub const fn id(self) -> u64 {
        self.0
    }
}

/// Memory usage and eviction counters for a texture cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextureCacheStats {
    pub entries: usize,
    pub used_bytes: usize,
    pub budget_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

/// Errors returned when a texture cannot be inserted into a cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureCacheError {
    /// The texture alone is larger than the configured cache budget.
    TextureExceedsBudget {
        texture_bytes: usize,
        budget_bytes: usize,
    },
    /// The cache-local handle counter has reached its maximum value.
    HandleExhausted,
    /// The cache cannot fit the texture without evicting retained entries.
    CacheFullyRetained,
}

#[derive(Debug)]
struct Entry {
    texture: Texture,
    bytes: usize,
    last_used: u64,
    retain_count: usize,
}

/// A bounded LRU cache for immutable textures.
///
/// The cache owns its textures and accounts for their pixel storage. Inserting
/// an item may evict least-recently-used entries until the new item fits.
/// Handles are monotonically allocated and are never reused during the cache's
/// lifetime, so an evicted handle cannot accidentally refer to a later texture.
#[derive(Debug)]
pub struct TextureCache {
    entries: HashMap<TextureHandle, Entry>,
    budget_bytes: usize,
    used_bytes: usize,
    next_handle: u64,
    clock: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl TextureCache {
    /// Creates an empty cache with a maximum pixel-storage budget.
    #[must_use]
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            budget_bytes,
            used_bytes: 0,
            next_handle: 1,
            clock: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Inserts a texture, evicting least-recently-used entries if necessary.
    ///
    /// The size is calculated from the owned RGBA pixel buffer. Hash-map and
    /// allocator overhead are intentionally not included in this accounting.
    pub fn insert(&mut self, texture: Texture) -> Result<TextureHandle, TextureCacheError> {
        let bytes = texture
            .pixels()
            .len()
            .saturating_mul(std::mem::size_of::<crate::Color>());
        if bytes > self.budget_bytes {
            return Err(TextureCacheError::TextureExceedsBudget {
                texture_bytes: bytes,
                budget_bytes: self.budget_bytes,
            });
        }

        let handle = TextureHandle(self.next_handle);
        let next_handle = self
            .next_handle
            .checked_add(1)
            .ok_or(TextureCacheError::HandleExhausted)?;

        let required_bytes = self
            .used_bytes
            .saturating_add(bytes)
            .saturating_sub(self.budget_bytes);
        let reclaimable_bytes = self
            .entries
            .values()
            .filter(|entry| entry.retain_count == 0)
            .map(|entry| entry.bytes)
            .fold(0usize, usize::saturating_add);
        if reclaimable_bytes < required_bytes {
            return Err(TextureCacheError::CacheFullyRetained);
        }
        while self.used_bytes > self.budget_bytes - bytes {
            let evicted = self.evict_least_recently_used();
            debug_assert!(evicted, "reclaimable cache capacity was precomputed");
        }

        self.advance_clock();
        self.entries.insert(
            handle,
            Entry {
                texture,
                bytes,
                last_used: self.clock,
                retain_count: 0,
            },
        );
        self.used_bytes += bytes;
        self.next_handle = next_handle;
        Ok(handle)
    }

    /// Gets a texture and marks it as recently used.
    pub fn get(&mut self, handle: TextureHandle) -> Option<&Texture> {
        self.advance_clock();
        match self.entries.get_mut(&handle) {
            Some(entry) => {
                entry.last_used = self.clock;
                self.hits = self.hits.saturating_add(1);
                Some(&entry.texture)
            }
            None => {
                self.misses = self.misses.saturating_add(1);
                None
            }
        }
    }

    /// Retains a cached texture so automatic eviction cannot remove it.
    ///
    /// Call release when the component or resource owner no longer needs it.
    pub fn retain(&mut self, handle: TextureHandle) -> bool {
        let Some(entry) = self.entries.get_mut(&handle) else {
            return false;
        };
        let Some(retain_count) = entry.retain_count.checked_add(1) else {
            return false;
        };
        entry.retain_count = retain_count;
        true
    }

    /// Releases one retained reference. Returns false for missing or unretained handles.
    pub fn release(&mut self, handle: TextureHandle) -> bool {
        let Some(entry) = self.entries.get_mut(&handle) else {
            return false;
        };
        if entry.retain_count == 0 {
            return false;
        }
        entry.retain_count -= 1;
        true
    }

    /// Returns the number of active retained references for a handle.
    #[must_use]
    pub fn retain_count(&self, handle: TextureHandle) -> Option<usize> {
        self.entries.get(&handle).map(|entry| entry.retain_count)
    }

    /// Reads a texture without changing LRU order or hit/miss counters.
    #[must_use]
    pub fn peek(&self, handle: TextureHandle) -> Option<&Texture> {
        self.entries.get(&handle).map(|entry| &entry.texture)
    }

    /// Removes an entry and returns ownership of its texture.
    pub fn remove(&mut self, handle: TextureHandle) -> Option<Texture> {
        let entry = self.entries.remove(&handle)?;
        self.used_bytes -= entry.bytes;
        Some(entry.texture)
    }

    /// Changes the memory budget, immediately evicting entries if required.
    pub fn set_budget_bytes(&mut self, budget_bytes: usize) -> Result<(), TextureCacheError> {
        let required_bytes = self.used_bytes.saturating_sub(budget_bytes);
        let reclaimable_bytes = self
            .entries
            .values()
            .filter(|entry| entry.retain_count == 0)
            .map(|entry| entry.bytes)
            .fold(0usize, usize::saturating_add);
        if reclaimable_bytes < required_bytes {
            return Err(TextureCacheError::CacheFullyRetained);
        }

        self.budget_bytes = budget_bytes;
        while self.used_bytes > self.budget_bytes {
            let evicted = self.evict_least_recently_used();
            debug_assert!(evicted, "reclaimable cache capacity was precomputed");
        }
        Ok(())
    }

    /// Removes every cached texture while preserving counters and handle IDs.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.used_bytes = 0;
    }

    /// Returns the current cache metrics.
    #[must_use]
    pub fn stats(&self) -> TextureCacheStats {
        TextureCacheStats {
            entries: self.entries.len(),
            used_bytes: self.used_bytes,
            budget_bytes: self.budget_bytes,
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
        }
    }

    /// Returns the number of currently cached textures.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no textures are currently cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Advances the LRU clock, rebasing timestamps before integer overflow.
    fn advance_clock(&mut self) {
        if self.clock == u64::MAX {
            let mut ordered: Vec<_> = self
                .entries
                .iter()
                .map(|(handle, entry)| (entry.last_used, *handle))
                .collect();
            ordered.sort_unstable_by_key(|(last_used, handle)| (*last_used, handle.id()));

            for (index, (_, handle)) in ordered.into_iter().enumerate() {
                if let Some(entry) = self.entries.get_mut(&handle) {
                    entry.last_used = index as u64 + 1;
                }
            }
            self.clock = self.entries.len() as u64;
        }
        self.clock += 1;
    }

    fn evict_least_recently_used(&mut self) -> bool {
        let candidate = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.retain_count == 0)
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(handle, _)| *handle);
        let Some(handle) = candidate else {
            return false;
        };
        if let Some(entry) = self.entries.remove(&handle) {
            self.used_bytes -= entry.bytes;
            self.evictions = self.evictions.saturating_add(1);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    fn texture(color: Color, pixels: usize) -> Texture {
        Texture::new(pixels as u32, 1, vec![color; pixels]).unwrap()
    }

    #[test]
    fn inserts_and_retrieves_with_stable_handles() {
        let mut cache = TextureCache::new(64);
        let handle = cache.insert(texture(Color::RED, 4)).unwrap();

        assert_eq!(handle.id(), 1);
        assert_eq!(cache.get(handle).unwrap().get_pixel(0, 0), Some(Color::RED));
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().used_bytes, 16);
    }

    #[test]
    fn evicts_least_recently_used_texture_to_respect_budget() {
        let mut cache = TextureCache::new(16);
        let first = cache.insert(texture(Color::RED, 2)).unwrap();
        let second = cache.insert(texture(Color::BLUE, 2)).unwrap();

        assert!(cache.get(first).is_some());
        let third = cache.insert(texture(Color::GREEN, 2)).unwrap();

        assert!(cache.peek(first).is_some());
        assert!(cache.peek(second).is_none());
        assert!(cache.peek(third).is_some());
        assert_eq!(cache.stats().used_bytes, 16);
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn rejects_oversized_texture_without_evicting_existing_entries() {
        let mut cache = TextureCache::new(8);
        let retained = cache.insert(texture(Color::RED, 2)).unwrap();

        assert_eq!(
            cache.insert(texture(Color::BLUE, 3)),
            Err(TextureCacheError::TextureExceedsBudget {
                texture_bytes: 12,
                budget_bytes: 8,
            })
        );
        assert!(cache.peek(retained).is_some());
        assert_eq!(cache.stats().entries, 1);
    }

    #[test]
    fn shrinking_budget_evicts_oldest_entries_immediately() {
        let mut cache = TextureCache::new(32);
        let first = cache.insert(texture(Color::RED, 2)).unwrap();
        let second = cache.insert(texture(Color::BLUE, 2)).unwrap();
        cache.set_budget_bytes(8).unwrap();

        assert!(cache.peek(first).is_none());
        assert!(cache.peek(second).is_some());
        assert_eq!(cache.stats().used_bytes, 8);
        assert_eq!(cache.stats().budget_bytes, 8);
    }

    #[test]
    fn shrinking_budget_fails_atomically_when_retained_entries_prevent_it() {
        let mut cache = TextureCache::new(16);
        let first = cache.insert(texture(Color::RED, 2)).unwrap();
        let second = cache.insert(texture(Color::BLUE, 2)).unwrap();
        assert!(cache.retain(first));
        assert!(cache.retain(second));

        assert_eq!(
            cache.set_budget_bytes(8),
            Err(TextureCacheError::CacheFullyRetained)
        );
        assert_eq!(cache.stats().budget_bytes, 16);
        assert_eq!(cache.stats().used_bytes, 16);
        assert_eq!(cache.stats().entries, 2);
    }

    #[test]
    fn remove_and_clear_release_accounted_storage() {
        let mut cache = TextureCache::new(64);
        let first = cache.insert(texture(Color::RED, 2)).unwrap();
        cache.insert(texture(Color::BLUE, 2)).unwrap();

        assert_eq!(
            cache.remove(first).unwrap().get_pixel(0, 0),
            Some(Color::RED)
        );
        assert_eq!(cache.stats().used_bytes, 8);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.stats().used_bytes, 0);
    }

    #[test]
    fn peek_does_not_change_lru_or_access_counters() {
        let mut cache = TextureCache::new(16);
        let first = cache.insert(texture(Color::RED, 2)).unwrap();
        let second = cache.insert(texture(Color::BLUE, 2)).unwrap();

        assert!(cache.peek(first).is_some());
        assert_eq!(cache.stats().hits, 0);
        cache.insert(texture(Color::GREEN, 2)).unwrap();

        assert!(cache.peek(first).is_none());
        assert!(cache.peek(second).is_some());
    }

    #[test]
    fn retained_textures_are_not_evicted_until_released() {
        let mut cache = TextureCache::new(16);
        let retained = cache.insert(texture(Color::RED, 2)).unwrap();
        let evictable = cache.insert(texture(Color::BLUE, 2)).unwrap();
        assert!(cache.retain(retained));
        let newest = cache.insert(texture(Color::GREEN, 2)).unwrap();

        assert!(cache.peek(retained).is_some());
        assert!(cache.peek(evictable).is_none());
        assert!(cache.peek(newest).is_some());
        assert_eq!(cache.retain_count(retained), Some(1));
        assert!(cache.release(retained));
        assert_eq!(cache.retain_count(retained), Some(0));
    }

    #[test]
    fn insertion_fails_without_evicting_when_all_entries_are_retained() {
        let mut cache = TextureCache::new(16);
        let first = cache.insert(texture(Color::RED, 2)).unwrap();
        let second = cache.insert(texture(Color::BLUE, 2)).unwrap();
        assert!(cache.retain(first));
        assert!(cache.retain(second));

        assert_eq!(
            cache.insert(texture(Color::GREEN, 2)),
            Err(TextureCacheError::CacheFullyRetained)
        );
        assert!(cache.peek(first).is_some());
        assert!(cache.peek(second).is_some());
        assert_eq!(cache.stats().entries, 2);
        assert_eq!(cache.stats().used_bytes, 16);
    }

    #[test]
    fn retain_rejects_reference_count_overflow_without_changing_entry() {
        let mut cache = TextureCache::new(16);
        let handle = cache.insert(texture(Color::RED, 2)).unwrap();
        cache.entries.get_mut(&handle).unwrap().retain_count = usize::MAX;

        assert!(!cache.retain(handle));
        assert_eq!(cache.retain_count(handle), Some(usize::MAX));
        assert!(cache.peek(handle).is_some());
    }

    #[test]
    fn explicit_removal_releases_retained_texture() {
        let mut cache = TextureCache::new(16);
        let handle = cache.insert(texture(Color::RED, 2)).unwrap();
        assert!(cache.retain(handle));
        assert_eq!(
            cache.remove(handle).unwrap().get_pixel(0, 0),
            Some(Color::RED)
        );
        assert_eq!(cache.stats().used_bytes, 0);
    }

    #[test]
    fn lru_clock_rebases_without_losing_eviction_order() {
        let mut cache = TextureCache::new(16);
        let first = cache.insert(texture(Color::RED, 2)).unwrap();
        let second = cache.insert(texture(Color::BLUE, 2)).unwrap();

        cache.clock = u64::MAX;
        assert!(cache.get(first).is_some());
        let third = cache.insert(texture(Color::GREEN, 2)).unwrap();

        assert!(cache.peek(first).is_some());
        assert!(cache.peek(second).is_none());
        assert!(cache.peek(third).is_some());
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn missing_handle_increments_miss_counter() {
        let mut cache = TextureCache::new(16);
        assert!(cache.get(TextureHandle(999)).is_none());
        assert_eq!(cache.stats().misses, 1);
    }
}
