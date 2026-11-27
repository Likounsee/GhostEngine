//! Runtime CPU rendering configuration.
//!
//! These options let applications choose between automatic parallelism,
//! a fixed worker limit, and serial rendering without changing global state.

/// Per-call policy for CPU rasterization operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    /// Maximum number of worker threads. Zero selects automatic detection.
    /// A value of one forces serial execution.
    pub max_threads: usize,
    /// Minimum clipped pixel area required before workers are spawned.
    /// Set to zero to disable the automatic size threshold.
    pub parallel_threshold_pixels: u64,
}

impl RenderOptions {
    /// Uses the engine defaults: automatic worker detection and a 16K-pixel
    /// threshold to avoid thread overhead for small drawing operations.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_threads: 0,
            parallel_threshold_pixels: 16_384,
        }
    }

    /// Forces serial execution.
    #[must_use]
    pub const fn serial() -> Self {
        Self {
            max_threads: 1,
            parallel_threshold_pixels: 0,
        }
    }

    /// Limits this operation to at most the requested number of workers.
    /// Zero restores automatic worker detection.
    #[must_use]
    pub const fn with_max_threads(mut self, max_threads: usize) -> Self {
        self.max_threads = max_threads;
        self
    }

    /// Changes the minimum clipped pixel area for parallel execution.
    #[must_use]
    pub const fn with_parallel_threshold(mut self, pixels: u64) -> Self {
        self.parallel_threshold_pixels = pixels;
        self
    }

    /// Resolves the worker count for an operation with the supplied area.
    #[must_use]
    pub fn worker_count(self, pixel_area: u64) -> usize {
        if pixel_area < self.parallel_threshold_pixels {
            return 1;
        }

        if self.max_threads == 1 {
            return 1;
        }

        let available = automatic_worker_count();
        if self.max_threads == 0 {
            available
        } else {
            available.min(self.max_threads)
        }
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn automatic_worker_count() -> usize {
    #[cfg(feature = "parallel-rendering")]
    {
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
    }
    #[cfg(not(feature = "parallel-rendering"))]
    {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::RenderOptions;

    #[test]
    fn serial_option_always_uses_one_worker() {
        assert_eq!(RenderOptions::serial().worker_count(u64::MAX), 1);
    }

    #[test]
    fn threshold_keeps_small_operations_serial() {
        let options = RenderOptions::new().with_parallel_threshold(100);
        assert_eq!(options.worker_count(99), 1);
    }

    #[test]
    fn explicit_worker_limit_is_respected() {
        let options = RenderOptions::new()
            .with_max_threads(2)
            .with_parallel_threshold(0);
        assert!(options.worker_count(1) <= 2);
        assert!(options.worker_count(1) >= 1);
    }

    #[test]
    fn default_options_are_stable() {
        assert_eq!(RenderOptions::default(), RenderOptions::new());
        assert_eq!(RenderOptions::new().parallel_threshold_pixels, 16_384);
    }
}
