//! Ordered CPU layer composition with offsets, visibility, and opacity.

use std::collections::{BTreeMap, BTreeSet};

use crate::texture::sample_pixels_bilinear;
use crate::{Color, Framebuffer, FramebufferError, Rect, TextureFilter, Transform2D, Vector};

// Subtracts the intersection with the cutter while preserving exact coverage.
fn subtract_rect(source: Rect, cutter: Rect, output: &mut Vec<Rect>) {
    let Some(intersection) = source.intersection(cutter) else {
        output.push(source);
        return;
    };

    let source_right = u64::from(source.x) + u64::from(source.width);
    let source_bottom = u64::from(source.y) + u64::from(source.height);
    let intersection_right = u64::from(intersection.x) + u64::from(intersection.width);
    let intersection_bottom = u64::from(intersection.y) + u64::from(intersection.height);

    if intersection.y > source.y {
        output.push(Rect::new(
            source.x,
            source.y,
            source.width,
            intersection.y - source.y,
        ));
    }
    if intersection_bottom < source_bottom {
        output.push(Rect::new(
            source.x,
            intersection_bottom as u32,
            source.width,
            (source_bottom - intersection_bottom) as u32,
        ));
    }
    if intersection.x > source.x {
        output.push(Rect::new(
            source.x,
            intersection.y,
            intersection.x - source.x,
            intersection.height,
        ));
    }
    if intersection_right < source_right {
        output.push(Rect::new(
            intersection_right as u32,
            intersection.y,
            (source_right - intersection_right) as u32,
            intersection.height,
        ));
    }
}

/// Failure returned when a damage region cannot reserve storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageRegionError {
    AllocationFailed,
}

impl core::fmt::Display for DamageRegionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("could not allocate damage-region storage")
    }
}

impl std::error::Error for DamageRegionError {}

/// A bounded collection of framebuffer regions that need recomposition.
///
/// Added rectangles are clipped to the dimensions supplied at construction.
/// Overlapping rectangles are represented as an exact, disjoint union.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageRegion {
    width: u32,
    height: u32,
    rectangles: Vec<Rect>,
}

impl DamageRegion {
    /// Creates an empty damage region bounded to a framebuffer of `width` × `height`.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rectangles: Vec::new(),
        }
    }

    /// Creates an empty damage region with storage reserved for the requested number of rectangles.
    #[must_use]
    pub fn with_capacity(width: u32, height: u32, capacity: usize) -> Self {
        Self {
            width,
            height,
            rectangles: Vec::with_capacity(capacity),
        }
    }

    /// Reserves additional storage without panicking on allocation failure.
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), DamageRegionError> {
        self.rectangles
            .try_reserve(additional)
            .map_err(|_| DamageRegionError::AllocationFailed)
    }

    /// Reserves additional storage for at least the requested number of more rectangles.
    ///
    /// This convenience method follows Vec::reserve and may panic if allocation fails.
    pub fn reserve(&mut self, additional: usize) {
        self.rectangles.reserve(additional);
    }

    /// Returns the number of rectangles that can be stored without reallocating.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.rectangles.capacity()
    }

    /// Marks both the previous and current bounds of an object as damaged.
    ///
    /// This is the preferred invalidation operation for moving, resizing,
    /// showing, or hiding scene content. Passing `None` for either bound
    /// handles objects that have just appeared or disappeared. Each bound is
    /// clipped and coalesced through `add`, so disjoint old/new positions stay
    /// separate instead of forcing a large redraw between them.
    pub fn add_transition(&mut self, previous: Option<Rect>, current: Option<Rect>) {
        if let Some(previous) = previous {
            self.add(previous);
        }
        if let Some(current) = current {
            self.add(current);
        }
    }

    /// Adds a rectangle, clipping it to the configured framebuffer bounds.
    ///
    /// Overlapping coverage is split into disjoint rectangles before insertion.
    /// Only rectangles sharing a complete edge are coalesced, so the stored
    /// region remains an exact representation of damaged pixels.
    ///
    /// This convenience method may panic if allocation fails. Use try_add when
    /// the caller needs to handle memory pressure explicitly.
    pub fn add(&mut self, rectangle: Rect) {
        self.try_add(rectangle)
            .expect("failed to allocate damage-region storage");
    }

    /// Fallibly adds a rectangle while preserving the region on allocation failure.
    ///
    /// All temporary and destination capacity is reserved before the stored
    /// rectangles are changed. This makes it suitable for applications that
    /// need to handle memory pressure without unwinding.
    pub fn try_add(&mut self, rectangle: Rect) -> Result<(), DamageRegionError> {
        let bounds = Rect::new(0, 0, self.width, self.height);
        let Some(clipped) = rectangle.intersection(bounds) else {
            return Ok(());
        };

        let mut pending = Vec::new();
        pending
            .try_reserve_exact(1)
            .map_err(|_| DamageRegionError::AllocationFailed)?;
        pending.push(clipped);

        for existing in &self.rectangles {
            let capacity = pending
                .len()
                .checked_mul(4)
                .ok_or(DamageRegionError::AllocationFailed)?;
            let mut remaining = Vec::new();
            remaining
                .try_reserve_exact(capacity)
                .map_err(|_| DamageRegionError::AllocationFailed)?;
            for candidate in pending {
                subtract_rect(candidate, *existing, &mut remaining);
            }
            pending = remaining;
            if pending.is_empty() {
                return Ok(());
            }
        }

        self.rectangles
            .try_reserve(pending.len())
            .map_err(|_| DamageRegionError::AllocationFailed)?;
        self.rectangles.extend(pending);

        // Coalesce only exact full-edge neighbors. This cannot fill an
        // undamaged gap in an L-shaped or diagonal region.
        let mut index = 0;
        while index < self.rectangles.len() {
            let mut other = index + 1;
            let mut merged = false;
            while other < self.rectangles.len() {
                let first = self.rectangles[index];
                let second = self.rectangles[other];
                let horizontal = first.y == second.y
                    && first.height == second.height
                    && (u64::from(first.x) + u64::from(first.width) == u64::from(second.x)
                        || u64::from(second.x) + u64::from(second.width) == u64::from(first.x));
                let vertical = first.x == second.x
                    && first.width == second.width
                    && (u64::from(first.y) + u64::from(first.height) == u64::from(second.y)
                        || u64::from(second.y) + u64::from(second.height) == u64::from(first.y));
                if horizontal || vertical {
                    let left = first.x.min(second.x);
                    let top = first.y.min(second.y);
                    let right = (u64::from(first.x) + u64::from(first.width))
                        .max(u64::from(second.x) + u64::from(second.width));
                    let bottom = (u64::from(first.y) + u64::from(first.height))
                        .max(u64::from(second.y) + u64::from(second.height));
                    self.rectangles[index] = Rect::new(
                        left,
                        top,
                        (right - u64::from(left)) as u32,
                        (bottom - u64::from(top)) as u32,
                    );
                    self.rectangles.swap_remove(other);
                    merged = true;
                    break;
                }
                other += 1;
            }
            if !merged {
                index += 1;
            }
        }
        Ok(())
    }

    /// Returns the disjoint rectangles currently marked for recomposition.
    #[must_use]
    pub fn rectangles(&self) -> &[Rect] {
        &self.rectangles
    }

    /// Returns true when no area is marked for recomposition.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rectangles.is_empty()
    }

    /// Returns the smallest rectangle enclosing all damaged areas.
    ///
    /// The returned rectangle may include undamaged pixels between disjoint
    /// regions; use rectangles() when exact damage coverage is required.
    #[must_use]
    pub fn bounding_rect(&self) -> Option<Rect> {
        let first = *self.rectangles.first()?;
        let mut left = first.x;
        let mut top = first.y;
        let mut right = u64::from(first.x) + u64::from(first.width);
        let mut bottom = u64::from(first.y) + u64::from(first.height);

        for rectangle in self.rectangles.iter().skip(1) {
            left = left.min(rectangle.x);
            top = top.min(rectangle.y);
            right = right.max(u64::from(rectangle.x) + u64::from(rectangle.width));
            bottom = bottom.max(u64::from(rectangle.y) + u64::from(rectangle.height));
        }

        Some(Rect::new(
            left,
            top,
            (right - u64::from(left)) as u32,
            (bottom - u64::from(top)) as u32,
        ))
    }

    /// Returns the total number of damaged pixels.
    ///
    /// Damage rectangles are disjoint, so each pixel is counted once.
    #[must_use]
    pub fn area(&self) -> u64 {
        self.rectangles
            .iter()
            .map(|rectangle| u64::from(rectangle.width) * u64::from(rectangle.height))
            .sum()
    }

    /// Returns true when the accumulated damage covers the entire framebuffer.
    #[must_use]
    pub fn covers_entire_target(&self) -> bool {
        self.width != 0
            && self.height != 0
            && self.area() == u64::from(self.width) * u64::from(self.height)
    }

    /// Removes all accumulated damage rectangles.
    pub fn clear(&mut self) {
        self.rectangles.clear();
    }

    /// Resets the tracked framebuffer bounds and clears pending damage.
    ///
    /// The rectangle allocation is retained, making this suitable for reusing
    /// one damage tracker across frames and framebuffer resizes.
    pub fn reset(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.rectangles.clear();
    }
}

/// Pixel blend operation used when a layer is composited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Standard source-over composition.
    #[default]
    SourceOver,
    /// Multiplies source and destination color channels.
    Multiply,
    /// Lightens using `source + destination - source * destination`.
    Screen,
    /// Adds source and destination channels, clamping at full intensity.
    Add,
    /// Selects the darker channel value.
    Darken,
    /// Selects the lighter channel value.
    Lighten,
    /// Applies the standard contrast-enhancing overlay formula.
    Overlay,
    /// Applies the inverse overlay formula.
    HardLight,
    /// Absolute difference between source and destination channels.
    Difference,
    /// Exclusion blend, a softer difference operation.
    Exclusion,
}

/// Stable application-defined identity for a compositing group.
pub type LayerGroupId = u64;

/// Tracks the last known bounds of groups by stable identity.
///
/// Keep this tracker beside a scene and call `update` once per group each
/// frame. Reordering groups does not affect their identity: only a changed,
/// added, or removed bound contributes damage. Bounds are expressed in target
/// framebuffer coordinates (including any group transform).
#[derive(Debug, Default)]
pub struct LayerGroupDamageTracker {
    bounds: BTreeMap<LayerGroupId, Rect>,
}

impl LayerGroupDamageTracker {
    /// Records a group's current bounds and adds its previous/current bounds
    /// to `damage` when they differ.
    ///
    /// Use `None` when a group has disappeared or is currently invisible.
    /// Use `Some(bounds)` when it is visible. The first appearance damages
    /// only the new bounds; removal damages only the old bounds.
    pub fn update(&mut self, id: LayerGroupId, current: Option<Rect>, damage: &mut DamageRegion) {
        let previous = self.bounds.get(&id).copied();
        if previous != current {
            damage.add_transition(previous, current);
        }
        match current {
            Some(bounds) => {
                // Update the existing tree node in place. Removing first would
                // free the node and force a fresh allocation on every frame.
                self.bounds.insert(id, bounds);
            }
            None => {
                self.bounds.remove(&id);
            }
        }
    }

    /// Removes every tracked group and invalidates all of their previous bounds.
    pub fn clear(&mut self, damage: &mut DamageRegion) {
        for (_, previous) in std::mem::take(&mut self.bounds) {
            damage.add(previous);
        }
    }

    /// Returns the number of groups currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    /// Returns whether no group bounds are currently tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    /// Returns the last known bounds for a stable group identity.
    #[must_use]
    pub fn bounds(&self, id: LayerGroupId) -> Option<Rect> {
        self.bounds.get(&id).copied()
    }
}

/// A framebuffer-backed layer positioned in the destination coordinate space.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    framebuffer: Framebuffer,
    x: i32,
    y: i32,
    opacity: u8,
    visible: bool,
    blend_mode: BlendMode,
    transform: Transform2D,
    filter: TextureFilter,
}

impl Layer {
    /// Creates a layer with its own framebuffer and an opaque initial visibility.
    pub fn new(width: u32, height: u32, clear_color: Color) -> Result<Self, FramebufferError> {
        Ok(Self::from_framebuffer(Framebuffer::new(
            width,
            height,
            clear_color,
        )?))
    }

    /// Creates a layer by taking ownership of an existing framebuffer.
    ///
    /// This avoids allocating and copying pixels when an application already
    /// renders into a GhostEngine-compatible framebuffer.
    #[must_use]
    pub fn from_framebuffer(framebuffer: Framebuffer) -> Self {
        Self {
            framebuffer,
            x: 0,
            y: 0,
            opacity: 255,
            visible: true,
            blend_mode: BlendMode::SourceOver,
            transform: Transform2D::IDENTITY,
            filter: TextureFilter::Nearest,
        }
    }

    /// Consumes the layer and returns its framebuffer without copying pixels.
    #[must_use]
    pub fn into_framebuffer(self) -> Framebuffer {
        self.framebuffer
    }

    #[must_use]
    pub const fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }

    pub fn framebuffer_mut(&mut self) -> &mut Framebuffer {
        &mut self.framebuffer
    }

    #[must_use]
    pub const fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    #[must_use]
    pub const fn opacity(&self) -> u8 {
        self.opacity
    }

    pub fn set_opacity(&mut self, opacity: u8) {
        self.opacity = opacity;
    }

    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    #[must_use]
    pub const fn blend_mode(&self) -> BlendMode {
        self.blend_mode
    }

    pub fn set_blend_mode(&mut self, blend_mode: BlendMode) {
        self.blend_mode = blend_mode;
    }

    /// Returns the affine transform applied to this layer's pixels.
    #[must_use]
    pub const fn transform(&self) -> Transform2D {
        self.transform
    }

    /// Sets the affine transform applied before the layer's position offset.
    pub fn set_transform(&mut self, transform: Transform2D) {
        self.transform = transform;
    }

    /// Returns the sampling filter used when this layer is transformed.
    #[must_use]
    pub const fn filter(&self) -> TextureFilter {
        self.filter
    }

    /// Sets the sampling filter used when this layer is transformed.
    pub fn set_filter(&mut self, filter: TextureFilter) {
        self.filter = filter;
    }
}

/// Composites layers in slice order, from back to front.
///
/// Each layer is clipped to the destination framebuffer. Layer opacity multiplies
/// the source pixel alpha before source-over composition. Hidden and fully
/// transparent layers are skipped.
pub fn compose_layers(target: &mut Framebuffer, layers: &[Layer]) {
    compose_layers_clipped(target, layers, None);
}

/// Composites layers using an automatic CPU strategy.
///
/// Small targets and scenes without transformed layers stay on the sequential
/// path. Larger scenes with transformed layers use the available logical CPU
/// workers, while preserving layer order and the same pixel-composition rules.
pub fn compose_layers_auto(target: &mut Framebuffer, layers: &[Layer]) {
    const PARALLEL_PIXEL_THRESHOLD: u64 = 131_072;

    // Base the decision on the pixels a transformed layer can actually touch,
    // not the full target size. A small sprite on a large framebuffer should
    // not pay the cost of spawning scoped worker threads.
    let has_large_transformed_layer = layers.iter().any(|layer| {
        if !layer.visible || layer.opacity == 0 || layer.transform == Transform2D::IDENTITY {
            return false;
        }
        let Some(prepared) = prepare_transformed_layer(target, layer) else {
            return false;
        };
        u64::from(prepared.bounds.width) * u64::from(prepared.bounds.height)
            >= PARALLEL_PIXEL_THRESHOLD
    });
    if !has_large_transformed_layer {
        compose_layers(target, layers);
        return;
    }

    let threads = {
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
    };
    compose_layers_parallel_with_threads(target, layers, threads);
}

/// Composites layers in order while processing transformed layers in parallel row bands.
///
/// Each layer completes before the next begins, preserving back-to-front blending.
/// Identity-transformed layers use the regular compositor; transformed layers
/// split destination rows between workers without overlapping framebuffer writes.
pub fn compose_layers_parallel_with_threads(
    target: &mut Framebuffer,
    layers: &[Layer],
    threads: usize,
) {
    if threads <= 1 || target.width() == 0 || target.height() == 0 {
        compose_layers(target, layers);
        return;
    }
    let target_bounds = Rect::new(0, 0, target.width(), target.height());
    let width = target.width() as usize;
    let height = target.height() as usize;
    let worker_count = threads.min(height).max(1);
    let rows_per_worker = height.div_ceil(worker_count);
    let chunk_len = rows_per_worker * width;

    for layer in layers {
        if !layer.visible || layer.opacity == 0 {
            continue;
        }
        if layer.transform == Transform2D::IDENTITY {
            compose_layer_clipped(target, layer, target_bounds, None);
            continue;
        }
        let Some(prepared) = prepare_transformed_layer(target, layer) else {
            continue;
        };
        let Some(clip) = prepared.bounds.intersection(target_bounds) else {
            continue;
        };
        std::thread::scope(|scope| {
            for (worker_index, pixels) in target.pixels_mut().chunks_mut(chunk_len).enumerate() {
                let row_origin = (worker_index * rows_per_worker) as u32;
                let row_end = (row_origin as usize + pixels.len() / width) as u32;
                let first_row = clip.y.max(row_origin);
                let last_row = (clip.y + clip.height).min(row_end);
                if first_row >= last_row {
                    continue;
                }
                let first_pixel = (first_row - row_origin) as usize * width;
                let last_pixel = (last_row - row_origin) as usize * width;
                let target_rows = &mut pixels[first_pixel..last_pixel];
                scope.spawn(move || {
                    compose_transformed_layer_rows(
                        target_rows,
                        width,
                        layer,
                        clip,
                        prepared,
                        first_row,
                        last_row,
                    );
                });
            }
        });
    }
}

/// Composites layers in slice order, limited to the accumulated damage rectangles.
///
/// Callers should redraw or clear the damaged target areas before recomposition
/// when the previous contents in those areas are no longer valid.
pub fn compose_layers_in_damage_regions(
    target: &mut Framebuffer,
    layers: &[Layer],
    damage: &DamageRegion,
) {
    if damage.is_empty() {
        return;
    }

    // A fragmented damage set can still cover every target pixel. In that
    // case, composing once avoids repeating layer traversal and clipping for
    // each rectangle while producing the same full-frame result.
    if damage.covers_entire_target() {
        compose_layers(target, layers);
        return;
    }

    let target_bounds = Rect::new(0, 0, target.width(), target.height());

    // Process one layer at a time. In particular, prepare transformed geometry
    // once per layer instead of recalculating its inverse and transformed
    // bounds for every disjoint damage rectangle. Damage rectangles do not
    // overlap, so changing the loop nesting preserves per-pixel layer order.
    for layer in layers {
        if !layer.visible || layer.opacity == 0 {
            continue;
        }
        let transformed = if layer.transform == Transform2D::IDENTITY {
            None
        } else {
            prepare_transformed_layer(target, layer)
        };
        if layer.transform != Transform2D::IDENTITY && transformed.is_none() {
            continue;
        }

        for rectangle in damage.rectangles() {
            if let Some(clip) = rectangle.intersection(target_bounds) {
                compose_layer_clipped(target, layer, clip, transformed.as_ref());
            }
        }
    }
}

fn compose_layers_clipped(target: &mut Framebuffer, layers: &[Layer], clip: Option<Rect>) {
    let target_bounds = Rect::new(0, 0, target.width(), target.height());
    let clip = match clip {
        Some(clip) => {
            let Some(clip) = clip.intersection(target_bounds) else {
                return;
            };
            clip
        }
        None => target_bounds,
    };

    for layer in layers {
        if !layer.visible || layer.opacity == 0 {
            continue;
        }
        let transformed = if layer.transform == Transform2D::IDENTITY {
            None
        } else {
            prepare_transformed_layer(target, layer)
        };
        if layer.transform != Transform2D::IDENTITY && transformed.is_none() {
            continue;
        }
        compose_layer_clipped(target, layer, clip, transformed.as_ref());
    }
}

#[derive(Clone, Copy)]
struct PreparedTransformedLayer {
    inverse: Transform2D,
    bounds: Rect,
}

fn prepare_transformed_layer(
    target: &Framebuffer,
    layer: &Layer,
) -> Option<PreparedTransformedLayer> {
    if target.width() == 0 || target.height() == 0 {
        return None;
    }
    let inverse = layer.transform.inverse()?;
    let width = layer.framebuffer.width() as f32;
    let height = layer.framebuffer.height() as f32;
    let corners = [
        Vector::new(0.0, 0.0),
        Vector::new(width, 0.0),
        Vector::new(width, height),
        Vector::new(0.0, height),
    ]
    .map(|point| {
        let point = layer.transform.transform_point(point);
        Vector::new(point.x + layer.x as f32, point.y + layer.y as f32)
    });
    if corners
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return None;
    }

    let min_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);

    let target_right = target.width() as f32;
    let target_bottom = target.height() as f32;
    let left = min_x.floor().clamp(0.0, target_right) as u32;
    let right = max_x.ceil().clamp(0.0, target_right) as u32;
    let top = min_y.floor().clamp(0.0, target_bottom) as u32;
    let bottom = max_y.ceil().clamp(0.0, target_bottom) as u32;
    if left >= right || top >= bottom {
        return None;
    }

    Some(PreparedTransformedLayer {
        inverse,
        bounds: Rect::new(left, top, right - left, bottom - top),
    })
}

fn compose_layer_clipped(
    target: &mut Framebuffer,
    layer: &Layer,
    clip: Rect,
    transformed: Option<&PreparedTransformedLayer>,
) {
    if layer.transform != Transform2D::IDENTITY {
        if let Some(transformed) = transformed {
            compose_transformed_layer(target, layer, clip, *transformed);
        }
        return;
    }

    let destination_left = i64::from(layer.x).max(i64::from(clip.x));
    let destination_right = (i64::from(layer.x) + i64::from(layer.framebuffer.width()))
        .min(i64::from(clip.x) + i64::from(clip.width));
    if destination_left >= destination_right {
        return;
    }
    let source_left = (destination_left - i64::from(layer.x)) as usize;
    let source_right = source_left + (destination_right - destination_left) as usize;
    let source_width = layer.framebuffer.width() as usize;

    let destination_top = i64::from(layer.y).max(i64::from(clip.y));
    let destination_bottom = (i64::from(layer.y) + i64::from(layer.framebuffer.height()))
        .min(i64::from(clip.y) + i64::from(clip.height));
    if destination_top >= destination_bottom {
        return;
    }
    let source_top = (destination_top - i64::from(layer.y)) as usize;
    let source_bottom = (destination_bottom - i64::from(layer.y)) as usize;

    // These layer-wide properties do not change between scanlines. Hoist
    // them out of both the row classifier and the per-pixel composition loop.
    let source_over = layer.blend_mode == BlendMode::SourceOver;
    let full_opacity = layer.opacity == u8::MAX;
    let can_copy_opaque = source_over && full_opacity;

    for source_y in source_top..source_bottom {
        let destination_y = (i64::from(layer.y) + source_y as i64) as u32;
        let source_start = source_y * source_width + source_left;
        let source_row =
            &layer.framebuffer.pixels()[source_start..source_y * source_width + source_right];
        let Some(destination_row) = target.row_mut(destination_y) else {
            continue;
        };
        let destination =
            &mut destination_row[destination_left as usize..destination_right as usize];

        // Classify the row in one pass. This avoids scanning every pixel
        // twice for transparent and opaque fast paths.
        let mut has_visible_pixel = false;
        let mut fully_opaque = can_copy_opaque;
        for pixel in source_row {
            has_visible_pixel |= pixel.a != 0;
            if can_copy_opaque {
                fully_opaque &= pixel.a == u8::MAX;
            }
            if has_visible_pixel && (!can_copy_opaque || !fully_opaque) {
                break;
            }
        }
        if !has_visible_pixel {
            continue;
        }
        if fully_opaque {
            destination.copy_from_slice(source_row);
            continue;
        }

        if source_over {
            if full_opacity {
                for (source, destination) in source_row.iter().zip(destination) {
                    if source.a == 0 {
                        continue;
                    }
                    if source.a == u8::MAX {
                        *destination = *source;
                    } else {
                        *destination = source.blend_over(*destination);
                    }
                }
            } else {
                for (source, destination) in source_row.iter().zip(destination) {
                    if source.a == 0 {
                        continue;
                    }
                    let alpha =
                        ((u16::from(source.a) * u16::from(layer.opacity) + 127) / 255) as u8;
                    if alpha != 0 {
                        let source = Color::rgba(source.r, source.g, source.b, alpha);
                        *destination = source.blend_over(*destination);
                    }
                }
            }
        } else {
            for (source, destination) in source_row.iter().zip(destination) {
                if source.a != 0 {
                    *destination =
                        blend_with_mode(*source, *destination, layer.opacity, layer.blend_mode);
                }
            }
        }
    }
}

fn compose_transformed_layer(
    target: &mut Framebuffer,
    layer: &Layer,
    clip: Rect,
    prepared: PreparedTransformedLayer,
) {
    let Some(clip) = prepared.bounds.intersection(clip) else {
        return;
    };
    let width = target.width() as usize;
    let end_y = clip.y + clip.height;
    let first_pixel = clip.y as usize * width;
    let last_pixel = end_y as usize * width;
    let target_rows = &mut target.pixels_mut()[first_pixel..last_pixel];
    compose_transformed_layer_rows(target_rows, width, layer, clip, prepared, clip.y, end_y);
}

fn compose_transformed_layer_rows(
    target_pixels: &mut [Color],
    target_width: usize,
    layer: &Layer,
    clip: Rect,
    prepared: PreparedTransformedLayer,
    row_start: u32,
    row_end: u32,
) {
    let inverse = prepared.inverse;
    let left = clip.x;
    let right = clip.x + clip.width;
    let top = clip.y;
    let bottom = clip.y + clip.height;

    let source_width_u32 = layer.framebuffer.width();
    let source_height_u32 = layer.framebuffer.height();
    let source_width = source_width_u32 as f32;
    let source_height = source_height_u32 as f32;
    let source_width_usize = source_width_u32 as usize;
    let source_pixels = layer.framebuffer.pixels();
    // These properties are constant for the entire draw. Keep them out of
    // the destination-pixel loop rather than repeatedly loading layer state.
    let filter = layer.filter;
    let blend_mode = layer.blend_mode;
    let source_over = blend_mode == BlendMode::SourceOver;
    let opacity = layer.opacity;
    // An affine inverse advances by a constant source-space delta for every
    // destination pixel in a row. Calculate the first sample once per row,
    // then use additions instead of rebuilding a Vector and multiplying the
    // inverse matrix for every pixel.
    let source_step_x = inverse.m11;
    let source_step_y = inverse.m21;
    let destination_offset_x = layer.x as f32;
    let destination_offset_y = layer.y as f32;
    for y in row_start.max(top)..row_end.min(bottom) {
        let destination_y = y as f32 + 0.5 - destination_offset_y;
        let mut source_x = inverse.m11 * (left as f32 + 0.5 - destination_offset_x)
            + inverse.m12 * destination_y
            + inverse.tx;
        let mut source_y = inverse.m21 * (left as f32 + 0.5 - destination_offset_x)
            + inverse.m22 * destination_y
            + inverse.ty;
        // With no vertical contribution from destination X, every sample
        // in this scanline has the same source Y. Reject an out-of-range row
        // before borrowing the destination or entering the pixel loop.
        if source_step_y == 0.0 && (source_y < 0.0 || source_y >= source_height) {
            continue;
        }
        // Bilinear sampling on an axis-aligned scanline uses the same two
        // source rows for every destination pixel. Resolve their slices and
        // the vertical interpolation factor once instead of recalculating
        // row indices and offsets for every pixel.
        let bilinear_rows = if filter == TextureFilter::Bilinear
            && source_over
            && source_step_y == 0.0
            && source_width_u32 != 0
            && source_height_u32 != 0
        {
            let sample_y = (source_y - 0.5).max(0.0);
            let y0 = sample_y.floor() as usize;
            let y1 = (y0 + 1).min(source_height_u32 as usize - 1);
            let row_start = y0 * source_width_usize;
            let next_row_start = y1 * source_width_usize;
            Some((
                &source_pixels[row_start..row_start + source_width_usize],
                &source_pixels[next_row_start..next_row_start + source_width_usize],
                sample_y - y0 as f32,
            ))
        } else {
            None
        };
        let row_offset = (y - row_start) as usize * target_width;
        let Some(destination_row) = target_pixels.get_mut(row_offset..row_offset + target_width)
        else {
            continue;
        };

        // For an axis-aligned horizontal sampling step, nearest-neighbour
        // sampling stays on one source row. Resolve that row once and use a
        // dedicated loop that does not calculate or validate source Y per pixel.
        if filter == TextureFilter::Nearest && source_step_y == 0.0 {
            let source_row_offset = source_y.floor() as usize * source_width_usize;
            if source_over && opacity == u8::MAX {
                for x in left..right {
                    if source_x >= 0.0 && source_x < source_width {
                        let source_column = source_x.floor() as usize;
                        let source = source_pixels[source_row_offset + source_column];
                        if source.a == u8::MAX {
                            destination_row[x as usize] = source;
                        } else if source.a != 0 {
                            let destination = &mut destination_row[x as usize];
                            *destination = source.blend_over(*destination);
                        }
                    }
                    source_x += source_step_x;
                }
            } else if source_over {
                for x in left..right {
                    if source_x >= 0.0 && source_x < source_width {
                        let source_column = source_x.floor() as usize;
                        let source = source_pixels[source_row_offset + source_column];
                        if source.a != 0 {
                            let alpha =
                                ((u16::from(source.a) * u16::from(opacity) + 127) / 255) as u8;
                            if alpha != 0 {
                                let destination = &mut destination_row[x as usize];
                                *destination = Color::rgba(source.r, source.g, source.b, alpha)
                                    .blend_over(*destination);
                            }
                        }
                    }
                    source_x += source_step_x;
                }
            } else {
                for x in left..right {
                    if source_x >= 0.0 && source_x < source_width {
                        let source_column = source_x.floor() as usize;
                        let source = source_pixels[source_row_offset + source_column];
                        if source.a != 0 {
                            let destination = &mut destination_row[x as usize];
                            *destination =
                                blend_with_mode(source, *destination, opacity, blend_mode);
                        }
                    }
                    source_x += source_step_x;
                }
            }
            continue;
        }

        for x in left..right {
            if source_x >= 0.0
                && source_y >= 0.0
                && source_x < source_width
                && source_y < source_height
            {
                let destination = &mut destination_row[x as usize];
                if filter == TextureFilter::Bilinear && source_over && destination.a == u8::MAX {
                    if let Some((top_row, bottom_row, ty)) = bilinear_rows {
                        blend_bilinear_over_opaque_rows(
                            top_row,
                            bottom_row,
                            source_x - 0.5,
                            ty,
                            opacity,
                            destination,
                        );
                    } else {
                        blend_bilinear_over_opaque(
                            source_pixels,
                            source_width_u32,
                            source_height_u32,
                            source_x - 0.5,
                            source_y - 0.5,
                            opacity,
                            destination,
                        );
                    }
                } else {
                    let source = match filter {
                        TextureFilter::Nearest => {
                            let sample_x = source_x.floor().clamp(0.0, source_width - 1.0) as usize;
                            let sample_y =
                                source_y.floor().clamp(0.0, source_height - 1.0) as usize;
                            source_pixels[sample_y * source_width_usize + sample_x]
                        }
                        TextureFilter::Bilinear => sample_pixels_bilinear(
                            source_pixels,
                            source_width_u32,
                            source_height_u32,
                            source_x - 0.5,
                            source_y - 0.5,
                        ),
                    };
                    if source.a != 0 {
                        if source_over {
                            let alpha =
                                ((u16::from(source.a) * u16::from(opacity) + 127) / 255) as u8;
                            if alpha > 0 {
                                *destination = Color::rgba(source.r, source.g, source.b, alpha)
                                    .blend_over(*destination);
                            }
                        } else {
                            *destination =
                                blend_with_mode(source, *destination, opacity, blend_mode);
                        }
                    }
                }
            }
            source_x += source_step_x;
            source_y += source_step_y;
        }
    }
}

// Bilinear source-over over an opaque destination can be performed directly
// from interpolated premultiplied channels. This avoids unpremultiplying into an
// intermediate 8-bit Color and blending it again.
#[inline]
fn blend_bilinear_over_opaque_rows(
    top_row: &[Color],
    bottom_row: &[Color],
    x: f32,
    ty: f32,
    opacity: u8,
    destination: &mut Color,
) {
    if top_row.is_empty() || opacity == 0 {
        return;
    }
    let x = x.max(0.0);
    let x0 = x.floor() as usize;
    let x1 = (x0 + 1).min(top_row.len() - 1);
    let tx = x - x0 as f32;
    let top_left = top_row[x0];
    let top_right = top_row[x1];
    let bottom_left = bottom_row[x0];
    let bottom_right = bottom_row[x1];

    let inverse_x = 1.0 - tx;
    let inverse_y = 1.0 - ty;
    let weight_top_left = inverse_x * inverse_y;
    let weight_top_right = tx * inverse_y;
    let weight_bottom_left = inverse_x * ty;
    let weight_bottom_right = tx * ty;
    // Fold source alpha and layer opacity into the four bilinear weights once.
    // Their sum is the final source alpha, so no separate interpolation pass
    // is needed before blending the three color channels.
    // Full-opacity layers are common in transformed UI and sprite draws.
    // Keep that hot path free of four redundant opacity multiplications.
    let (alpha_top_left, alpha_top_right, alpha_bottom_left, alpha_bottom_right) =
        if opacity == u8::MAX {
            (
                weight_top_left * f32::from(top_left.a),
                weight_top_right * f32::from(top_right.a),
                weight_bottom_left * f32::from(bottom_left.a),
                weight_bottom_right * f32::from(bottom_right.a),
            )
        } else {
            let opacity_factor = f32::from(opacity) / 255.0;
            (
                weight_top_left * f32::from(top_left.a) * opacity_factor,
                weight_top_right * f32::from(top_right.a) * opacity_factor,
                weight_bottom_left * f32::from(bottom_left.a) * opacity_factor,
                weight_bottom_right * f32::from(bottom_right.a) * opacity_factor,
            )
        };
    let alpha = alpha_top_left + alpha_top_right + alpha_bottom_left + alpha_bottom_right;
    if alpha <= f32::EPSILON {
        return;
    }
    let inverse_alpha = 255.0 - alpha;
    let channel = |tl: u8, tr: u8, bl: u8, br: u8, background: u8| {
        let premultiplied = f32::from(tl).mul_add(
            alpha_top_left,
            f32::from(tr).mul_add(
                alpha_top_right,
                f32::from(bl).mul_add(alpha_bottom_left, f32::from(br) * alpha_bottom_right),
            ),
        );
        ((premultiplied + f32::from(background) * inverse_alpha) / 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    destination.r = channel(
        top_left.r,
        top_right.r,
        bottom_left.r,
        bottom_right.r,
        destination.r,
    );
    destination.g = channel(
        top_left.g,
        top_right.g,
        bottom_left.g,
        bottom_right.g,
        destination.g,
    );
    destination.b = channel(
        top_left.b,
        top_right.b,
        bottom_left.b,
        bottom_right.b,
        destination.b,
    );
}

#[inline]
fn blend_bilinear_over_opaque(
    pixels: &[Color],
    width: u32,
    height: u32,
    x: f32,
    y: f32,
    opacity: u8,
    destination: &mut Color,
) {
    if width == 0 || height == 0 || opacity == 0 {
        return;
    }
    // The caller has already verified that the unshifted source point is
    // inside the texture. After the half-texel offset, only the lower edge
    // can be crossed; the upper edge is therefore already bounded.
    let x = x.max(0.0);
    let y = y.max(0.0);
    let row_width = width as usize;
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(row_width - 1);
    let y1 = (y0 + 1).min(height as usize - 1);
    let top_offset = y0 * row_width;
    let bottom_offset = y1 * row_width;
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let top_left = pixels[top_offset + x0];
    let top_right = pixels[top_offset + x1];
    let bottom_left = pixels[bottom_offset + x0];
    let bottom_right = pixels[bottom_offset + x1];

    // When all four taps resolve to the same texel color, interpolation cannot
    // change the source color or alpha. Skip the four weights and fused channel
    // interpolation entirely; this is common for scaled sprites and flat areas.
    if top_left == top_right && top_left == bottom_left && top_left == bottom_right {
        let alpha = f32::from(top_left.a)
            * (if opacity == u8::MAX {
                1.0
            } else {
                f32::from(opacity) / 255.0
            });
        if alpha <= f32::EPSILON {
            return;
        }
        let inverse_alpha = 255.0 - alpha;
        let blend_channel = |source: u8, background: u8| {
            ((f32::from(source).mul_add(alpha, f32::from(background) * inverse_alpha) / 255.0)
                .round()
                .clamp(0.0, 255.0)) as u8
        };
        destination.r = blend_channel(top_left.r, destination.r);
        destination.g = blend_channel(top_left.g, destination.g);
        destination.b = blend_channel(top_left.b, destination.b);
        return;
    }

    // A bilinear footprint that is constant across either row or column only
    // needs a single interpolation axis. This is common along sprite tile edges
    // and avoids calculating four weights for those boundary pixels.
    let (first, second, amount) = if top_left == top_right && bottom_left == bottom_right {
        (top_left, bottom_left, ty)
    } else if top_left == bottom_left && top_right == bottom_right {
        (top_left, top_right, tx)
    } else {
        // The general four-sample path below handles diagonal and fully mixed taps.
        (Color::TRANSPARENT, Color::TRANSPARENT, -1.0)
    };
    if amount >= 0.0 {
        let inverse_amount = 1.0 - amount;
        let opacity_factor = if opacity == u8::MAX {
            1.0
        } else {
            f32::from(opacity) / 255.0
        };
        let first_alpha = inverse_amount * f32::from(first.a) * opacity_factor;
        let second_alpha = amount * f32::from(second.a) * opacity_factor;
        let alpha = first_alpha + second_alpha;
        if alpha <= f32::EPSILON {
            return;
        }
        let inverse_alpha = 255.0 - alpha;
        let blend_channel = |a: u8, b: u8, background: u8| {
            let premultiplied = f32::from(a).mul_add(first_alpha, f32::from(b) * second_alpha);
            ((premultiplied + f32::from(background) * inverse_alpha) / 255.0)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        destination.r = blend_channel(first.r, second.r, destination.r);
        destination.g = blend_channel(first.g, second.g, destination.g);
        destination.b = blend_channel(first.b, second.b, destination.b);
        return;
    }

    // Share the four bilinear weights across alpha and all color channels.
    // The separable form repeats horizontal/vertical arithmetic for each
    // channel; these weights let each channel use only four multiply-adds.
    let inverse_x = 1.0 - tx;
    let inverse_y = 1.0 - ty;
    let weight_top_left = inverse_x * inverse_y;
    let weight_top_right = tx * inverse_y;
    let weight_bottom_left = inverse_x * ty;
    let weight_bottom_right = tx * ty;
    // Fold source alpha and layer opacity into the four bilinear weights once.
    // Their sum is the final source alpha, so no separate interpolation pass
    // is needed before blending the three color channels.
    // Full-opacity layers are common in transformed UI and sprite draws.
    // Keep that hot path free of four redundant opacity multiplications.
    let (alpha_top_left, alpha_top_right, alpha_bottom_left, alpha_bottom_right) =
        if opacity == u8::MAX {
            (
                weight_top_left * f32::from(top_left.a),
                weight_top_right * f32::from(top_right.a),
                weight_bottom_left * f32::from(bottom_left.a),
                weight_bottom_right * f32::from(bottom_right.a),
            )
        } else {
            let opacity_factor = f32::from(opacity) / 255.0;
            (
                weight_top_left * f32::from(top_left.a) * opacity_factor,
                weight_top_right * f32::from(top_right.a) * opacity_factor,
                weight_bottom_left * f32::from(bottom_left.a) * opacity_factor,
                weight_bottom_right * f32::from(bottom_right.a) * opacity_factor,
            )
        };
    let alpha = alpha_top_left + alpha_top_right + alpha_bottom_left + alpha_bottom_right;
    if alpha <= f32::EPSILON {
        return;
    }
    let inverse_alpha = 255.0 - alpha;
    let channel = |tl: u8, tr: u8, bl: u8, br: u8, background: u8| {
        let premultiplied = f32::from(tl).mul_add(
            alpha_top_left,
            f32::from(tr).mul_add(
                alpha_top_right,
                f32::from(bl).mul_add(alpha_bottom_left, f32::from(br) * alpha_bottom_right),
            ),
        );
        ((premultiplied + f32::from(background) * inverse_alpha) / 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    destination.r = channel(
        top_left.r,
        top_right.r,
        bottom_left.r,
        bottom_right.r,
        destination.r,
    );
    destination.g = channel(
        top_left.g,
        top_right.g,
        bottom_left.g,
        bottom_right.g,
        destination.g,
    );
    destination.b = channel(
        top_left.b,
        top_right.b,
        bottom_left.b,
        bottom_right.b,
        destination.b,
    );
}

fn blend_with_mode(source: Color, destination: Color, opacity: u8, mode: BlendMode) -> Color {
    let source_alpha = f32::from(source.a) / 255.0 * (f32::from(opacity) / 255.0);
    let destination_alpha = f32::from(destination.a) / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    if output_alpha <= 0.0 {
        return Color::TRANSPARENT;
    }

    let channel = |source: u8, destination: u8| {
        let source = f32::from(source) / 255.0;
        let destination = f32::from(destination) / 255.0;
        let blended = match mode {
            BlendMode::SourceOver => source,
            BlendMode::Multiply => source * destination,
            BlendMode::Screen => source + destination - source * destination,
            BlendMode::Add => (source + destination).min(1.0),
            BlendMode::Darken => source.min(destination),
            BlendMode::Lighten => source.max(destination),
            BlendMode::Overlay => {
                if destination <= 0.5 {
                    2.0 * source * destination
                } else {
                    1.0 - 2.0 * (1.0 - source) * (1.0 - destination)
                }
            }
            BlendMode::HardLight => {
                if source <= 0.5 {
                    2.0 * source * destination
                } else {
                    1.0 - 2.0 * (1.0 - source) * (1.0 - destination)
                }
            }
            BlendMode::Difference => (source - destination).abs(),
            BlendMode::Exclusion => source + destination - 2.0 * source * destination,
        };
        let premultiplied = source * source_alpha * (1.0 - destination_alpha)
            + blended * source_alpha * destination_alpha
            + destination * destination_alpha * (1.0 - source_alpha);
        (premultiplied / output_alpha * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    };

    Color::rgba(
        channel(source.r, destination.r),
        channel(source.g, destination.g),
        channel(source.b, destination.b),
        (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// A compositing group that renders its children into an isolated off-screen surface.
///
/// Group opacity and blend mode are applied once to the completed group, rather
/// than independently to each child. Groups can contain other groups, allowing
/// applications to build reusable scene hierarchies.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerGroup {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    opacity: u8,
    visible: bool,
    blend_mode: BlendMode,
    transform: Transform2D,
    filter: TextureFilter,
    children: Vec<LayerGroupChild>,
}

/// A child in a compositing group.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerGroupChild {
    Layer(Layer),
    Group(LayerGroup),
}

impl LayerGroup {
    /// Creates an isolated group surface with transparent initial contents.
    pub fn new(width: u32, height: u32) -> Result<Self, FramebufferError> {
        if width == 0 || height == 0 {
            return Err(FramebufferError::ZeroDimension);
        }
        (width as usize)
            .checked_mul(height as usize)
            .ok_or(FramebufferError::SizeOverflow)?;
        Ok(Self {
            width,
            height,
            x: 0,
            y: 0,
            opacity: 255,
            visible: true,
            blend_mode: BlendMode::SourceOver,
            transform: Transform2D::IDENTITY,
            filter: TextureFilter::Bilinear,
            children: Vec::new(),
        })
    }

    /// Creates an isolated group surface with capacity reserved for its children.
    pub fn with_capacity(
        width: u32,
        height: u32,
        child_capacity: usize,
    ) -> Result<Self, FramebufferError> {
        let mut group = Self::new(width, height)?;
        group.children.reserve(child_capacity);
        Ok(group)
    }

    /// Reserves space for additional child layers or nested groups.
    pub fn reserve_children(&mut self, additional: usize) {
        self.children.reserve(additional);
    }

    /// Returns the number of children currently stored in this group.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Returns the number of children that fit without reallocating.
    #[must_use]
    pub fn child_capacity(&self) -> usize {
        self.children.capacity()
    }

    /// Adds a framebuffer-backed child layer, preserving insertion order.
    pub fn add_layer(&mut self, layer: Layer) {
        self.children.push(LayerGroupChild::Layer(layer));
    }

    /// Adds a nested group, preserving insertion order.
    pub fn add_group(&mut self, group: LayerGroup) {
        self.children.push(LayerGroupChild::Group(group));
    }

    /// Returns the children in back-to-front composition order.
    #[must_use]
    pub fn children(&self) -> &[LayerGroupChild] {
        &self.children
    }

    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    #[must_use]
    pub const fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    #[must_use]
    pub const fn opacity(&self) -> u8 {
        self.opacity
    }

    pub fn set_opacity(&mut self, opacity: u8) {
        self.opacity = opacity;
    }

    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    #[must_use]
    pub const fn blend_mode(&self) -> BlendMode {
        self.blend_mode
    }

    pub fn set_blend_mode(&mut self, mode: BlendMode) {
        self.blend_mode = mode;
    }

    #[must_use]
    pub const fn transform(&self) -> Transform2D {
        self.transform
    }

    pub fn set_transform(&mut self, transform: Transform2D) {
        self.transform = transform;
    }

    #[must_use]
    pub const fn filter(&self) -> TextureFilter {
        self.filter
    }

    pub fn set_filter(&mut self, filter: TextureFilter) {
        self.filter = filter;
    }
}

/// Reusable off-screen surfaces for hierarchical group composition.
///
/// Keep one scratch value alongside a scene and pass it to
/// `compose_layer_groups_with_scratch` on every frame to retain allocations.
#[derive(Debug, Default)]
pub struct LayerGroupScratch {
    layers: Vec<Layer>,
}

impl LayerGroupScratch {
    /// Creates reusable group surfaces with room for the requested nesting depth.
    #[must_use]
    pub fn with_capacity(surface_capacity: usize) -> Self {
        Self {
            layers: Vec::with_capacity(surface_capacity),
        }
    }

    /// Reserves reusable surfaces before entering a real-time render loop.
    pub fn reserve_surfaces(&mut self, additional: usize) {
        self.layers.reserve(additional);
    }

    /// Returns the number of reusable surface slots that fit without reallocating.
    #[must_use]
    pub fn surface_capacity(&self) -> usize {
        self.layers.capacity()
    }

    /// Returns the pixel-buffer bytes currently owned by reusable group surfaces.
    ///
    /// This excludes allocator and vector metadata overhead.
    #[must_use]
    pub fn allocated_pixel_bytes(&self) -> usize {
        self.layers.iter().fold(0usize, |total, layer| {
            let bytes = layer
                .framebuffer()
                .pixels()
                .len()
                .saturating_mul(std::mem::size_of::<Color>());
            total.saturating_add(bytes)
        })
    }

    /// Returns the number of reusable group surfaces currently allocated.
    #[must_use]
    pub fn surface_count(&self) -> usize {
        self.layers.len()
    }
}

/** Persistent off-screen surfaces indexed by application-defined group identity.
 *
 * Unlike temporary nesting-depth scratch surfaces, this cache keeps each
 * group's pixels across frames and traversal reordering. Call mark_dirty after
 * changing local group content. Position, opacity, blend mode, and transform
 * are refreshed automatically and do not require rerendering local pixels.
 */
#[derive(Debug, Default)]
pub struct LayerGroupCache {
    entries: BTreeMap<LayerGroupId, LayerGroupCacheEntry>,
}

#[derive(Debug)]
struct LayerGroupCacheEntry {
    layer: Layer,
    damage: DamageRegion,
    initialized: bool,
    render_count: u64,
}

impl LayerGroupCache {
    /// Marks a local-space rectangle dirty for the next cached composition.
    /// Returns false when the identity has not been cached yet.
    pub fn mark_dirty(&mut self, id: LayerGroupId, rectangle: Rect) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.damage.add(rectangle);
        true
    }

    /// Invalidates the complete cached surface for a group.
    pub fn invalidate(&mut self, id: LayerGroupId) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.damage.add(Rect::new(
            0,
            0,
            entry.layer.framebuffer().width(),
            entry.layer.framebuffer().height(),
        ));
        true
    }

    /// Invalidates every cached group without releasing its pixel allocations.
    pub fn invalidate_all(&mut self) {
        for entry in self.entries.values_mut() {
            entry.damage.add(Rect::new(
                0,
                0,
                entry.layer.framebuffer().width(),
                entry.layer.framebuffer().height(),
            ));
        }
    }

    /// Removes a cached group and releases its off-screen surface.
    pub fn remove(&mut self, id: LayerGroupId) -> bool {
        self.entries.remove(&id).is_some()
    }

    /// Removes all cached surfaces.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Retains only the group identities present in the supplied frame.
    ///
    /// Call this when a scene is replaced or groups are permanently removed to
    /// release surfaces that can no longer be reused. Hidden groups remain
    /// cached as long as their identities are supplied.
    pub fn retain_groups(&mut self, groups: &[(LayerGroupId, &LayerGroup)]) {
        let live_ids: BTreeSet<_> = groups.iter().map(|(id, _)| *id).collect();
        self.entries.retain(|id, _| live_ids.contains(id));
    }

    /// Returns the number of persistent group surfaces.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no group surfaces are cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the cached pixels for a group, if it has been composed.
    #[must_use]
    pub fn framebuffer(&self, id: LayerGroupId) -> Option<&Framebuffer> {
        self.entries.get(&id).map(|entry| entry.layer.framebuffer())
    }

    /// Returns how many times a group's local surface has been regenerated.
    #[must_use]
    pub fn render_count(&self, id: LayerGroupId) -> Option<u64> {
        self.entries.get(&id).map(|entry| entry.render_count)
    }

    /// Returns the pixel-buffer memory retained by all cached groups.
    #[must_use]
    pub fn allocated_pixel_bytes(&self) -> usize {
        self.entries.values().fold(0usize, |total, entry| {
            total.saturating_add(
                entry
                    .layer
                    .framebuffer()
                    .pixels()
                    .len()
                    .saturating_mul(std::mem::size_of::<Color>()),
            )
        })
    }
}

/// Composites groups using persistent surfaces keyed by stable identity.
///
/// Keep each ID stable across frames and explicitly invalidate local content
/// changes with mark_dirty or invalidate. Reordering preserves cached surfaces;
/// a group dimension change automatically recreates and invalidates its surface.
pub fn compose_layer_groups_cached(
    target: &mut Framebuffer,
    groups: &[(LayerGroupId, &LayerGroup)],
    cache: &mut LayerGroupCache,
    scratch: &mut LayerGroupScratch,
) -> Result<(), FramebufferError> {
    compose_layer_groups_cached_internal(target, groups, None, cache, scratch)
}

/// Composites cached groups only in the requested target-space damage regions.
pub fn compose_layer_groups_cached_in_damage_regions(
    target: &mut Framebuffer,
    groups: &[(LayerGroupId, &LayerGroup)],
    target_damage: &DamageRegion,
    cache: &mut LayerGroupCache,
    scratch: &mut LayerGroupScratch,
) -> Result<(), FramebufferError> {
    if target_damage.is_empty() {
        cache.retain_groups(groups);
        return Ok(());
    }
    compose_layer_groups_cached_internal(target, groups, Some(target_damage), cache, scratch)
}

fn compose_layer_groups_cached_internal(
    target: &mut Framebuffer,
    groups: &[(LayerGroupId, &LayerGroup)],
    target_damage: Option<&DamageRegion>,
    cache: &mut LayerGroupCache,
    scratch: &mut LayerGroupScratch,
) -> Result<(), FramebufferError> {
    cache.retain_groups(groups);
    for &(id, group) in groups {
        if !group.visible || group.opacity == 0 {
            continue;
        }
        let entry = match cache.entries.entry(id) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(slot) => {
                let layer = Layer::new(group.width, group.height, Color::TRANSPARENT)?;
                let mut damage = DamageRegion::new(group.width, group.height);
                damage.add(Rect::new(0, 0, group.width, group.height));
                slot.insert(LayerGroupCacheEntry {
                    layer,
                    damage,
                    initialized: false,
                    render_count: 0,
                })
            }
        };
        if entry.layer.framebuffer().width() != group.width
            || entry.layer.framebuffer().height() != group.height
        {
            entry.layer.framebuffer_mut().resize_and_clear(
                group.width,
                group.height,
                Color::TRANSPARENT,
            )?;
            entry.damage.reset(group.width, group.height);
            entry.damage.add(Rect::new(0, 0, group.width, group.height));
            entry.initialized = false;
        }

        if !entry.damage.is_empty() {
            let dirty = entry.damage.clone();
            if !entry.initialized || dirty.covers_entire_target() {
                render_layer_group_into(group, scratch, 0)?;
                entry
                    .layer
                    .framebuffer_mut()
                    .copy_pixels_from(scratch.layers[0].framebuffer().pixels())?;
            } else {
                if scratch.layers.is_empty() {
                    scratch
                        .layers
                        .push(Layer::from_framebuffer(entry.layer.framebuffer().clone()));
                } else {
                    let surface = scratch.layers[0].framebuffer_mut();
                    if surface.width() != group.width || surface.height() != group.height {
                        surface.resize_and_clear(group.width, group.height, Color::TRANSPARENT)?;
                    }
                    surface.copy_pixels_from(entry.layer.framebuffer().pixels())?;
                }
                render_layer_group_partial_into(group, scratch, 0, &dirty)?;
                let source = scratch.layers[0].framebuffer().pixels();
                let width = group.width as usize;
                let destination = entry.layer.framebuffer_mut().pixels_mut();
                for rectangle in dirty.rectangles() {
                    for y in rectangle.y..rectangle.y + rectangle.height {
                        let start = y as usize * width + rectangle.x as usize;
                        let end = start + rectangle.width as usize;
                        destination[start..end].copy_from_slice(&source[start..end]);
                    }
                }
            }
            entry.damage.clear();
            entry.initialized = true;
            entry.render_count = entry.render_count.saturating_add(1);
        }

        entry.layer.set_position(group.x, group.y);
        entry.layer.set_opacity(group.opacity);
        entry.layer.set_blend_mode(group.blend_mode);
        entry.layer.set_transform(group.transform);
        entry.layer.set_filter(group.filter);
        if let Some(damage) = target_damage {
            compose_layers_in_damage_regions(target, std::slice::from_ref(&entry.layer), damage);
        } else {
            compose_layers_auto(target, std::slice::from_ref(&entry.layer));
        }
    }
    Ok(())
}

/// Composites isolated groups in order, applying each group's opacity and blend
/// mode once after all of its children have been rendered.
///
/// This convenience function creates temporary scratch storage. Real-time
/// render loops should use `compose_layer_groups_with_scratch` to reuse buffers.
pub fn compose_layer_groups(
    target: &mut Framebuffer,
    groups: &[LayerGroup],
) -> Result<(), FramebufferError> {
    compose_layer_groups_with_scratch(target, groups, &mut LayerGroupScratch::default())
}

/// Composites nested groups while reusing their off-screen framebuffer allocations.
///
/// Child coordinates are local to the containing group. Group transforms,
/// opacity and blend modes are applied once to the completed child surface.
pub fn compose_layer_groups_with_scratch(
    target: &mut Framebuffer,
    groups: &[LayerGroup],
    scratch: &mut LayerGroupScratch,
) -> Result<(), FramebufferError> {
    for group in groups {
        if !group.visible || group.opacity == 0 {
            continue;
        }
        render_layer_group_into(group, scratch, 0)?;
        compose_layers_auto(target, &scratch.layers[..1]);
    }
    Ok(())
}

/// Re-composites only damaged portions of isolated groups, preserving all other
/// pixels in the reusable off-screen surfaces.
///
/// The damage rectangles are expressed in target coordinates. They are mapped
/// into each group's local surface, then propagated through nested groups.
/// Identity-transformed groups can be updated partially; a group with a
/// non-identity transform is conservatively rebuilt in full. The caller must
/// include both the old and new bounds when moving or removing scene content,
/// and must restore the target background in damaged areas before calling.
pub fn compose_layer_groups_in_damage_regions(
    target: &mut Framebuffer,
    groups: &[LayerGroup],
    damage: &DamageRegion,
    scratch: &mut LayerGroupScratch,
) -> Result<(), FramebufferError> {
    if damage.is_empty() {
        return Ok(());
    }

    for group in groups {
        if !group.visible || group.opacity == 0 {
            continue;
        }

        if let Some(local_damage) = transformed_damage(
            damage,
            group.width,
            group.height,
            group.x,
            group.y,
            group.transform,
        ) {
            if local_damage.is_empty() {
                continue;
            }
            render_layer_group_partial_into(group, scratch, 0, &local_damage)?;
        } else {
            // Singular or non-finite transforms require conservative full regeneration.
            render_layer_group_into(group, scratch, 0)?;
        }

        compose_layers_in_damage_regions(target, &scratch.layers[..1], damage);
    }
    Ok(())
}

/// Maps target-space damage back into a group's local coordinates.
///
/// The one-pixel expansion is conservative for filtered sampling at transformed
/// edges. A singular or non-finite transform returns None so callers can fall
/// back to a full group rebuild rather than risk stale pixels.
fn transformed_damage(
    source: &DamageRegion,
    width: u32,
    height: u32,
    offset_x: i32,
    offset_y: i32,
    transform: Transform2D,
) -> Option<DamageRegion> {
    if transform == Transform2D::IDENTITY {
        return Some(translated_damage(source, width, height, offset_x, offset_y));
    }

    let inverse = transform.inverse()?;
    let mut local = DamageRegion::new(width, height);

    for rectangle in source.rectangles() {
        let left = rectangle.x as f32 - offset_x as f32;
        let top = rectangle.y as f32 - offset_y as f32;
        let right = (u64::from(rectangle.x) + u64::from(rectangle.width)) as f32 - offset_x as f32;
        let bottom =
            (u64::from(rectangle.y) + u64::from(rectangle.height)) as f32 - offset_y as f32;
        let corners = [
            Vector::new(left, top),
            Vector::new(right, top),
            Vector::new(right, bottom),
            Vector::new(left, bottom),
        ]
        .map(|point| inverse.transform_point(point));

        if corners
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            return None;
        }

        let min_x = corners
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min);
        let max_x = corners
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = corners
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let max_y = corners
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);

        let local_left = (min_x.floor() - 1.0).clamp(0.0, width as f32) as u32;
        let local_top = (min_y.floor() - 1.0).clamp(0.0, height as f32) as u32;
        let local_right = (max_x.ceil() + 1.0).clamp(0.0, width as f32) as u32;
        let local_bottom = (max_y.ceil() + 1.0).clamp(0.0, height as f32) as u32;

        if local_left < local_right && local_top < local_bottom {
            local.add(Rect::new(
                local_left,
                local_top,
                local_right - local_left,
                local_bottom - local_top,
            ));
        }
    }

    Some(local)
}

fn translated_damage(
    source: &DamageRegion,
    width: u32,
    height: u32,
    offset_x: i32,
    offset_y: i32,
) -> DamageRegion {
    let mut translated = DamageRegion::new(width, height);
    for rectangle in source.rectangles() {
        let left = i64::from(rectangle.x) - i64::from(offset_x);
        let top = i64::from(rectangle.y) - i64::from(offset_y);
        let right = left + i64::from(rectangle.width);
        let bottom = top + i64::from(rectangle.height);
        let clipped_left = left.max(0).min(i64::from(width));
        let clipped_top = top.max(0).min(i64::from(height));
        let clipped_right = right.max(0).min(i64::from(width));
        let clipped_bottom = bottom.max(0).min(i64::from(height));
        if clipped_left < clipped_right && clipped_top < clipped_bottom {
            translated.add(Rect::new(
                clipped_left as u32,
                clipped_top as u32,
                (clipped_right - clipped_left) as u32,
                (clipped_bottom - clipped_top) as u32,
            ));
        }
    }
    translated
}

fn render_layer_group_partial_into(
    group: &LayerGroup,
    scratch: &mut LayerGroupScratch,
    depth: usize,
    damage: &DamageRegion,
) -> Result<(), FramebufferError> {
    let needs_full_initialization = if scratch.layers.len() <= depth {
        scratch
            .layers
            .push(Layer::new(group.width, group.height, Color::TRANSPARENT)?);
        true
    } else {
        let surface = scratch.layers[depth].framebuffer_mut();
        if surface.width() != group.width || surface.height() != group.height {
            surface.resize_and_clear(group.width, group.height, Color::TRANSPARENT)?;
            true
        } else {
            false
        }
    };

    let mut effective_damage;
    let damage = if needs_full_initialization {
        effective_damage = DamageRegion::new(group.width, group.height);
        effective_damage.add(Rect::new(0, 0, group.width, group.height));
        &effective_damage
    } else {
        damage
    };

    {
        let surface = scratch.layers[depth].framebuffer_mut();
        let stride = surface.width() as usize;
        for rectangle in damage.rectangles() {
            for y in rectangle.y..rectangle.y + rectangle.height {
                let start = y as usize * stride + rectangle.x as usize;
                let end = start + rectangle.width as usize;
                surface.pixels_mut()[start..end].fill(Color::TRANSPARENT);
            }
        }
    }

    for child in &group.children {
        match child {
            LayerGroupChild::Layer(layer) => {
                compose_layers_in_damage_regions(
                    scratch.layers[depth].framebuffer_mut(),
                    std::slice::from_ref(layer),
                    damage,
                );
            }
            LayerGroupChild::Group(nested) => {
                if !nested.visible || nested.opacity == 0 {
                    continue;
                }
                if let Some(nested_damage) = transformed_damage(
                    damage,
                    nested.width,
                    nested.height,
                    nested.x,
                    nested.y,
                    nested.transform,
                ) {
                    if nested_damage.is_empty() {
                        continue;
                    }
                    render_layer_group_partial_into(nested, scratch, depth + 1, &nested_damage)?;
                } else {
                    // Singular or non-finite transforms cannot be mapped safely.
                    render_layer_group_into(nested, scratch, depth + 1)?;
                }
                let (parents, children) = scratch.layers.split_at_mut(depth + 1);
                compose_layers_in_damage_regions(
                    parents[depth].framebuffer_mut(),
                    std::slice::from_ref(&children[0]),
                    damage,
                );
            }
        }
    }

    let layer = &mut scratch.layers[depth];
    layer.set_position(group.x, group.y);
    layer.set_opacity(group.opacity);
    layer.set_blend_mode(group.blend_mode);
    layer.set_transform(group.transform);
    layer.set_filter(group.filter);
    Ok(())
}

fn render_layer_group_into(
    group: &LayerGroup,
    scratch: &mut LayerGroupScratch,
    depth: usize,
) -> Result<(), FramebufferError> {
    if scratch.layers.len() <= depth {
        scratch
            .layers
            .push(Layer::new(group.width, group.height, Color::TRANSPARENT)?);
    } else {
        let surface = scratch.layers[depth].framebuffer_mut();
        if surface.width() != group.width || surface.height() != group.height {
            surface.resize_and_clear(group.width, group.height, Color::TRANSPARENT)?;
        } else {
            surface.clear(Color::TRANSPARENT);
        }
    }

    for child in &group.children {
        match child {
            LayerGroupChild::Layer(layer) => {
                compose_layers_auto(
                    scratch.layers[depth].framebuffer_mut(),
                    std::slice::from_ref(layer),
                );
            }
            LayerGroupChild::Group(nested) => {
                if !nested.visible || nested.opacity == 0 {
                    continue;
                }
                render_layer_group_into(nested, scratch, depth + 1)?;
                let (parents, children) = scratch.layers.split_at_mut(depth + 1);
                compose_layers_auto(
                    parents[depth].framebuffer_mut(),
                    std::slice::from_ref(&children[0]),
                );
            }
        }
    }

    let layer = &mut scratch.layers[depth];
    layer.set_position(group.x, group.y);
    layer.set_opacity(group.opacity);
    layer.set_blend_mode(group.blend_mode);
    layer.set_transform(group.transform);
    layer.set_filter(group.filter);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        blend_bilinear_over_opaque, blend_bilinear_over_opaque_rows, blend_with_mode,
        compose_layer_groups, compose_layer_groups_cached,
        compose_layer_groups_cached_in_damage_regions, compose_layer_groups_with_scratch,
        compose_layers, compose_layers_in_damage_regions, BlendMode, DamageRegion, Layer,
        LayerGroup, LayerGroupCache, LayerGroupChild, LayerGroupScratch,
    };
    use crate::{Color, Framebuffer, Rect, TextureFilter, Transform2D};

    #[test]
    fn cached_group_surfaces_survive_reordering_without_rerendering() {
        let mut first = LayerGroup::new(4, 4).unwrap();
        first.add_layer(Layer::new(4, 4, Color::RED).unwrap());
        first.set_position(1, 1);
        let mut second = LayerGroup::new(4, 4).unwrap();
        second.add_layer(Layer::new(4, 4, Color::BLUE).unwrap());
        second.set_position(8, 1);

        let mut cache = LayerGroupCache::default();
        let mut scratch = LayerGroupScratch::default();
        let mut target = Framebuffer::new(16, 8, Color::BLACK).unwrap();
        compose_layer_groups_cached(
            &mut target,
            &[(11, &first), (22, &second)],
            &mut cache,
            &mut scratch,
        )
        .unwrap();

        let first_allocation = cache.framebuffer(11).unwrap().pixels().as_ptr();
        assert_eq!(cache.render_count(11), Some(1));
        assert_eq!(cache.render_count(22), Some(1));
        target.clear(Color::BLACK);
        compose_layer_groups_cached(
            &mut target,
            &[(22, &second), (11, &first)],
            &mut cache,
            &mut scratch,
        )
        .unwrap();

        assert_eq!(cache.render_count(11), Some(1));
        assert_eq!(cache.render_count(22), Some(1));
        assert_eq!(
            cache.framebuffer(11).unwrap().pixels().as_ptr(),
            first_allocation
        );
        assert_eq!(target.get_pixel(1, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(8, 1), Some(Color::BLUE));
    }

    #[test]
    fn empty_damage_still_releases_removed_cached_groups_without_rendering() {
        let first = LayerGroup::new(2, 2).unwrap();
        let second = LayerGroup::new(2, 2).unwrap();
        let mut cache = LayerGroupCache::default();
        let mut scratch = LayerGroupScratch::default();
        let mut target = Framebuffer::new(4, 2, Color::BLACK).unwrap();
        compose_layer_groups_cached(
            &mut target,
            &[(1, &first), (2, &second)],
            &mut cache,
            &mut scratch,
        )
        .unwrap();
        let render_count = cache.render_count(2);
        let empty_damage = DamageRegion::new(4, 2);

        compose_layer_groups_cached_in_damage_regions(
            &mut target,
            &[(2, &second)],
            &empty_damage,
            &mut cache,
            &mut scratch,
        )
        .unwrap();

        assert_eq!(cache.len(), 1);
        assert!(cache.framebuffer(1).is_none());
        assert_eq!(cache.render_count(2), render_count);
    }

    #[test]
    fn cached_group_releases_surfaces_for_removed_scene_groups() {
        let first = LayerGroup::new(2, 2).unwrap();
        let second = LayerGroup::new(2, 2).unwrap();
        let mut cache = LayerGroupCache::default();
        let mut scratch = LayerGroupScratch::default();
        let mut target = Framebuffer::new(4, 2, Color::BLACK).unwrap();

        compose_layer_groups_cached(
            &mut target,
            &[(1, &first), (2, &second)],
            &mut cache,
            &mut scratch,
        )
        .unwrap();
        assert_eq!(cache.len(), 2);

        compose_layer_groups_cached(&mut target, &[(2, &second)], &mut cache, &mut scratch)
            .unwrap();
        assert_eq!(cache.len(), 1);
        assert!(cache.framebuffer(1).is_none());
        assert!(cache.framebuffer(2).is_some());
    }

    #[test]
    fn cached_group_partial_invalidation_updates_only_dirty_pixels() {
        let mut group = LayerGroup::new(4, 4).unwrap();
        group.add_layer(Layer::new(4, 4, Color::RED).unwrap());
        let mut cache = LayerGroupCache::default();
        let mut scratch = LayerGroupScratch::default();
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        compose_layer_groups_cached(&mut target, &[(7, &group)], &mut cache, &mut scratch).unwrap();

        let mut overlay = Layer::new(2, 2, Color::BLUE).unwrap();
        overlay.set_position(1, 1);
        group.add_layer(overlay);
        assert!(cache.mark_dirty(7, Rect::new(1, 1, 2, 2)));
        target.clear(Color::BLACK);
        compose_layer_groups_cached(&mut target, &[(7, &group)], &mut cache, &mut scratch).unwrap();

        assert_eq!(cache.render_count(7), Some(2));
        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(1, 1), Some(Color::BLUE));
        assert_eq!(target.get_pixel(2, 2), Some(Color::BLUE));
        assert_eq!(target.get_pixel(3, 3), Some(Color::RED));
    }

    #[test]
    fn cached_group_resizes_and_rebuilds_automatically() {
        let mut small = LayerGroup::new(2, 2).unwrap();
        small.add_layer(Layer::new(2, 2, Color::RED).unwrap());
        let mut cache = LayerGroupCache::default();
        let mut scratch = LayerGroupScratch::default();
        let mut target = Framebuffer::new(8, 4, Color::BLACK).unwrap();
        compose_layer_groups_cached(&mut target, &[(3, &small)], &mut cache, &mut scratch).unwrap();

        let mut large = LayerGroup::new(5, 2).unwrap();
        large.add_layer(Layer::new(5, 2, Color::GREEN).unwrap());
        compose_layer_groups_cached(&mut target, &[(3, &large)], &mut cache, &mut scratch).unwrap();

        let cached = cache.framebuffer(3).unwrap();
        assert_eq!((cached.width(), cached.height()), (5, 2));
        assert_eq!(cache.render_count(3), Some(2));
        assert_eq!(cached.get_pixel(4, 1), Some(Color::GREEN));
    }

    #[test]
    fn stable_group_damage_tracker_survives_reordering_and_tracks_lifecycle() {
        use super::LayerGroupDamageTracker;

        let mut tracker = LayerGroupDamageTracker::default();
        let mut damage = DamageRegion::new(32, 16);
        let a = Rect::new(1, 2, 4, 4);
        let b = Rect::new(20, 2, 5, 4);

        tracker.update(100, Some(a), &mut damage);
        tracker.update(200, Some(b), &mut damage);
        assert_eq!(tracker.len(), 2);
        assert_eq!(damage.area(), 36);

        // Same identities and bounds in a different traversal order are clean.
        let mut reordered_damage = DamageRegion::new(32, 16);
        tracker.update(200, Some(b), &mut reordered_damage);
        tracker.update(100, Some(a), &mut reordered_damage);
        assert!(reordered_damage.is_empty());

        // Moving one group damages both its former and new position.
        let moved = Rect::new(10, 9, 4, 4);
        tracker.update(100, Some(moved), &mut reordered_damage);
        assert_eq!(reordered_damage.area(), 32);
        assert_eq!(tracker.bounds(100), Some(moved));

        // Removing a group damages its last known position.
        tracker.update(200, None, &mut reordered_damage);
        assert_eq!(tracker.bounds(200), None);
        assert_eq!(tracker.len(), 1);

        tracker.clear(&mut reordered_damage);
        assert!(tracker.is_empty());
        assert_eq!(tracker.len(), 0);
    }

    #[test]
    fn damage_transition_invalidates_old_and_new_bounds_without_filling_the_gap() {
        let mut damage = DamageRegion::new(20, 12);
        damage.add_transition(Some(Rect::new(2, 3, 2, 2)), Some(Rect::new(12, 3, 3, 2)));

        assert_eq!(
            damage.rectangles(),
            &[Rect::new(2, 3, 2, 2), Rect::new(12, 3, 3, 2)]
        );
        assert_eq!(damage.area(), 10);

        let mut appearing = DamageRegion::new(8, 8);
        appearing.add_transition(None, Some(Rect::new(1, 1, 2, 2)));
        assert_eq!(appearing.rectangles(), &[Rect::new(1, 1, 2, 2)]);

        let mut disappearing = DamageRegion::new(8, 8);
        disappearing.add_transition(Some(Rect::new(4, 2, 2, 3)), None);
        assert_eq!(disappearing.rectangles(), &[Rect::new(4, 2, 2, 3)]);
    }

    #[test]
    fn nested_group_damage_recomposition_matches_full_render_and_preserves_untouched_pixels() {
        use super::compose_layer_groups_in_damage_regions;

        let mut inner = LayerGroup::new(5, 4).unwrap();
        let mut red = Layer::new(5, 4, Color::RED).unwrap();
        red.set_position(0, 0);
        inner.add_layer(red);

        let mut outer = LayerGroup::new(10, 8).unwrap();
        inner.set_position(2, 1);
        outer.add_group(inner);
        let mut target = Framebuffer::new(12, 10, Color::BLACK).unwrap();
        let mut scratch = LayerGroupScratch::with_capacity(2);

        let mut full = target.clone();
        compose_layer_groups_with_scratch(
            &mut full,
            &[outer.clone()],
            &mut LayerGroupScratch::default(),
        )
        .unwrap();
        let mut initial_damage = DamageRegion::new(12, 10);
        initial_damage.add(Rect::new(0, 0, 12, 10));
        compose_layer_groups_in_damage_regions(
            &mut target,
            &[outer.clone()],
            &initial_damage,
            &mut scratch,
        )
        .unwrap();
        assert_eq!(target.pixels(), full.pixels());

        // Update one pixel in the innermost layer and invalidate only its target area.
        let mut changed_inner = outer.clone();
        if let LayerGroupChild::Group(nested) = &mut changed_inner.children[0] {
            if let LayerGroupChild::Layer(layer) = &mut nested.children[0] {
                layer.framebuffer_mut().set_pixel(3, 2, Color::BLUE);
            }
        }
        let mut dirty = DamageRegion::new(12, 10);
        dirty.add(Rect::new(5, 3, 1, 1));
        let mut expected = target.clone();
        compose_layer_groups_with_scratch(
            &mut expected,
            &[changed_inner.clone()],
            &mut LayerGroupScratch::default(),
        )
        .unwrap();
        compose_layer_groups_in_damage_regions(&mut target, &[changed_inner], &dirty, &mut scratch)
            .unwrap();
        assert_eq!(target.get_pixel(5, 3), expected.get_pixel(5, 3));
        for y in 0..target.height() {
            for x in 0..target.width() {
                if x != 5 || y != 3 {
                    assert_eq!(target.get_pixel(x, y), Some(full.get_pixel(x, y).unwrap()));
                }
            }
        }
    }

    #[test]
    fn transformed_nested_group_recomposes_only_mapped_damage() {
        use super::compose_layer_groups_in_damage_regions;

        let mut inner = LayerGroup::new(5, 4).unwrap();
        inner.add_layer(Layer::new(5, 4, Color::RED).unwrap());
        inner.set_position(2, 1);
        inner.set_transform(Transform2D::scale(2.0, 2.0));

        let mut outer = LayerGroup::new(16, 12).unwrap();
        outer.add_group(inner);
        let mut target = Framebuffer::new(16, 12, Color::BLACK).unwrap();
        let mut scratch = LayerGroupScratch::with_capacity(2);

        let mut full = target.clone();
        compose_layer_groups_with_scratch(
            &mut full,
            &[outer.clone()],
            &mut LayerGroupScratch::default(),
        )
        .unwrap();
        let mut initial = DamageRegion::new(16, 12);
        initial.add(Rect::new(0, 0, 16, 12));
        compose_layer_groups_in_damage_regions(
            &mut target,
            &[outer.clone()],
            &initial,
            &mut scratch,
        )
        .unwrap();
        assert_eq!(target.pixels(), full.pixels());

        let mut changed = outer;
        if let super::LayerGroupChild::Group(nested) = &mut changed.children[0] {
            if let super::LayerGroupChild::Layer(layer) = &mut nested.children[0] {
                layer.framebuffer_mut().set_pixel(3, 2, Color::BLUE);
            }
        }

        // Local pixel (3, 2), scaled by two and offset by (2, 1).
        let dirty_rect = Rect::new(8, 5, 2, 2);
        let mut damage = DamageRegion::new(16, 12);
        damage.add(dirty_rect);
        let mut expected = target.clone();
        compose_layer_groups_with_scratch(
            &mut expected,
            &[changed.clone()],
            &mut LayerGroupScratch::default(),
        )
        .unwrap();
        compose_layer_groups_in_damage_regions(&mut target, &[changed], &damage, &mut scratch)
            .unwrap();

        for y in 0..target.height() {
            for x in 0..target.width() {
                if x >= dirty_rect.x
                    && x < dirty_rect.x + dirty_rect.width
                    && y >= dirty_rect.y
                    && y < dirty_rect.y + dirty_rect.height
                {
                    assert_eq!(target.get_pixel(x, y), expected.get_pixel(x, y));
                } else {
                    assert_eq!(target.get_pixel(x, y), full.get_pixel(x, y));
                }
            }
        }
    }

    #[test]
    fn group_scratch_reuses_surfaces_across_frames_and_nested_depths() {
        let mut inner = LayerGroup::new(2, 1).unwrap();
        inner.add_layer(Layer::new(2, 1, Color::GREEN).unwrap());
        let mut outer = LayerGroup::new(3, 2).unwrap();
        outer.add_group(inner);

        let mut scratch = LayerGroupScratch::default();
        let mut target = Framebuffer::new(3, 2, Color::BLACK).unwrap();
        compose_layer_groups_with_scratch(&mut target, &[outer.clone()], &mut scratch).unwrap();
        assert_eq!(scratch.surface_count(), 2);
        assert_eq!(target.get_pixel(0, 0), Some(Color::GREEN));

        target.clear(Color::BLACK);
        compose_layer_groups_with_scratch(&mut target, &[outer], &mut scratch).unwrap();
        assert_eq!(scratch.surface_count(), 2);
        assert_eq!(target.get_pixel(1, 0), Some(Color::GREEN));
    }

    #[test]
    fn group_child_capacity_can_be_reserved_before_scene_construction() {
        let mut group = LayerGroup::with_capacity(4, 3, 8).unwrap();
        let capacity = group.child_capacity();
        for _ in 0..8 {
            group.add_layer(Layer::new(1, 1, Color::RED).unwrap());
        }
        assert_eq!(group.child_count(), 8);
        assert_eq!(group.child_capacity(), capacity);
    }

    #[test]
    fn group_scratch_can_preallocate_depth_and_reports_pixel_memory() {
        let mut scratch = LayerGroupScratch::with_capacity(3);
        assert!(scratch.surface_capacity() >= 3);
        scratch.reserve_surfaces(2);
        let mut nested = LayerGroup::new(2, 2).unwrap();
        nested.add_layer(Layer::new(2, 2, Color::GREEN).unwrap());
        let mut outer = LayerGroup::new(4, 4).unwrap();
        outer.add_group(nested);
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        compose_layer_groups_with_scratch(&mut target, &[outer], &mut scratch).unwrap();
        assert_eq!(scratch.surface_count(), 2);
        assert_eq!(
            scratch.allocated_pixel_bytes(),
            20 * std::mem::size_of::<Color>()
        );
    }

    #[test]
    fn group_opacity_is_applied_once_after_child_composition() {
        let mut group = LayerGroup::new(1, 1).unwrap();
        group.add_layer(Layer::new(1, 1, Color::rgba(255, 0, 0, 128)).unwrap());
        group.add_layer(Layer::new(1, 1, Color::rgba(0, 0, 255, 128)).unwrap());
        group.set_opacity(128);

        let background = Color::rgb(20, 40, 60);
        let mut actual = Framebuffer::new(1, 1, background).unwrap();
        compose_layer_groups(&mut actual, &[group]).unwrap();

        let mut isolated = Framebuffer::new(1, 1, Color::TRANSPARENT).unwrap();
        let mut children = [
            Layer::new(1, 1, Color::rgba(255, 0, 0, 128)).unwrap(),
            Layer::new(1, 1, Color::rgba(0, 0, 255, 128)).unwrap(),
        ];
        compose_layers(&mut isolated, &children);
        let mut expected_layer = Layer::from_framebuffer(isolated);
        expected_layer.set_opacity(128);
        let mut expected = Framebuffer::new(1, 1, background).unwrap();
        compose_layers(&mut expected, &[expected_layer]);

        assert_eq!(actual.pixels(), expected.pixels());
        // Applying opacity to each child independently produces a different result.
        children[0].set_opacity(128);
        children[1].set_opacity(128);
        let mut per_child = Framebuffer::new(1, 1, Color::TRANSPARENT).unwrap();
        compose_layers(&mut per_child, &children);
        assert_ne!(
            actual.get_pixel(0, 0),
            Some(per_child.get_pixel(0, 0).unwrap().blend_over(background))
        );
    }

    #[test]
    fn nested_group_parallel_composition_matches_serial_pixel_reference() {
        let mut transformed = Layer::new(512, 256, Color::TRANSPARENT).unwrap();
        for y in 0..256 {
            for x in 0..512 {
                let checker = ((x / 16) + (y / 16)) % 2 == 0;
                let color = if checker {
                    Color::rgba(55, 145, 235, 230)
                } else {
                    Color::rgba(235, 105, 65, 190)
                };
                transformed.framebuffer_mut().set_pixel(x, y, color);
            }
        }
        transformed.set_position(24, 18);
        transformed.set_opacity(211);
        transformed.set_transform(
            Transform2D::rotation(0.08).then(Transform2D::scale(1.35, 1.35)),
        );
        transformed.set_filter(TextureFilter::Bilinear);

        let mut inner = LayerGroup::new(512, 256).unwrap();
        inner.add_layer(transformed.clone());
        inner.set_position(42, 28);
        inner.set_opacity(190);
        inner.set_blend_mode(BlendMode::Screen);

        let mut background = Layer::new(640, 360, Color::rgba(35, 90, 155, 255)).unwrap();
        background.set_position(0, 0);
        let mut outer = LayerGroup::new(640, 360).unwrap();
        outer.add_layer(background.clone());
        outer.add_group(inner.clone());
        outer.set_position(10, 8);
        outer.set_opacity(230);
        outer.set_blend_mode(BlendMode::Multiply);

        let initial = Color::rgb(18, 24, 32);
        let mut actual = Framebuffer::new(660, 380, initial).unwrap();
        let mut scratch = LayerGroupScratch::with_capacity(2);
        compose_layer_groups_with_scratch(
            &mut actual,
            std::slice::from_ref(&outer),
            &mut scratch,
        )
        .unwrap();

        let mut inner_surface =
            Framebuffer::new(inner.width, inner.height, Color::TRANSPARENT).unwrap();
        compose_layers(&mut inner_surface, &[transformed]);
        let mut inner_layer = Layer::from_framebuffer(inner_surface);
        inner_layer.set_position(inner.x, inner.y);
        inner_layer.set_opacity(inner.opacity);
        inner_layer.set_blend_mode(inner.blend_mode);
        inner_layer.set_transform(inner.transform);
        inner_layer.set_filter(inner.filter);

        let mut outer_surface =
            Framebuffer::new(outer.width, outer.height, Color::TRANSPARENT).unwrap();
        compose_layers(&mut outer_surface, &[background, inner_layer]);
        let mut outer_layer = Layer::from_framebuffer(outer_surface);
        outer_layer.set_position(outer.x, outer.y);
        outer_layer.set_opacity(outer.opacity);
        outer_layer.set_blend_mode(outer.blend_mode);
        outer_layer.set_transform(outer.transform);
        outer_layer.set_filter(outer.filter);

        let mut expected = Framebuffer::new(660, 380, initial).unwrap();
        compose_layers(&mut expected, &[outer_layer]);

        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn nested_groups_respect_visibility_position_and_blend_mode() {
        let mut inner = LayerGroup::new(1, 1).unwrap();
        inner.add_layer(Layer::new(1, 1, Color::RED).unwrap());
        inner.set_position(1, 0);
        inner.set_visible(true);

        let mut outer = LayerGroup::new(3, 1).unwrap();
        outer.add_group(inner);
        outer.set_position(1, 0);
        outer.set_blend_mode(BlendMode::Screen);
        let mut hidden = LayerGroup::new(3, 1).unwrap();
        hidden.add_layer(Layer::new(3, 1, Color::WHITE).unwrap());
        hidden.set_visible(false);

        let mut target = Framebuffer::new(5, 1, Color::BLACK).unwrap();
        compose_layer_groups(&mut target, &[outer, hidden]).unwrap();

        assert_eq!(target.get_pixel(2, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(target.get_pixel(4, 0), Some(Color::BLACK));
    }

    #[test]
    fn blend_modes_produce_expected_opaque_channel_results() {
        let source = Color::rgb(128, 64, 200);
        let destination = Color::rgb(64, 128, 100);
        let cases = [
            (BlendMode::Darken, Color::rgb(64, 64, 100)),
            (BlendMode::Lighten, Color::rgb(128, 128, 200)),
            (BlendMode::Difference, Color::rgb(64, 64, 100)),
            (BlendMode::Exclusion, Color::rgb(128, 128, 143)),
        ];
        for (mode, expected) in cases {
            assert_eq!(blend_with_mode(source, destination, 255, mode), expected);
        }
    }

    #[test]
    fn overlay_and_hard_light_use_their_respective_base_channels() {
        let source = Color::rgb(192, 64, 128);
        let destination = Color::rgb(64, 192, 128);
        assert_eq!(
            blend_with_mode(source, destination, 255, BlendMode::Overlay),
            Color::rgb(96, 161, 128)
        );
        assert_eq!(
            blend_with_mode(source, destination, 255, BlendMode::HardLight),
            Color::rgb(161, 96, 128)
        );
    }

    #[test]
    fn blend_modes_preserve_source_over_alpha_composition() {
        let source = Color::rgba(220, 80, 30, 128);
        let destination = Color::rgba(20, 100, 240, 96);
        for mode in [
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::Overlay,
            BlendMode::HardLight,
            BlendMode::Difference,
            BlendMode::Exclusion,
        ] {
            let result = blend_with_mode(source, destination, 180, mode);
            assert!(result.a > destination.a);
            assert!(result.a < 255);
        }
    }

    #[test]
    fn fallible_damage_region_updates_preserve_exact_coverage() {
        let mut damage = DamageRegion::new(8, 6);
        damage.try_add(Rect::new(0, 0, 4, 4)).unwrap();
        damage.try_add(Rect::new(2, 2, 4, 4)).unwrap();

        assert_eq!(damage.area(), 28);
        assert_eq!(damage.bounding_rect(), Some(Rect::new(0, 0, 6, 6)));
        for (index, first) in damage.rectangles().iter().enumerate() {
            for second in damage.rectangles().iter().skip(index + 1) {
                assert!(first.intersection(*second).is_none());
            }
        }
    }

    #[test]
    fn fallible_damage_region_reservation_reports_capacity_overflow() {
        let mut damage = DamageRegion::new(4, 4);
        damage.add(Rect::new(0, 0, 2, 2));
        let original = damage.clone();

        assert_eq!(
            damage.try_reserve(usize::MAX),
            Err(super::DamageRegionError::AllocationFailed)
        );
        assert_eq!(damage, original);
        assert_eq!(
            damage.try_add(Rect::new(usize::MAX as u32, 0, 1, 1)),
            Ok(())
        );
        assert_eq!(damage, original);
    }

    #[test]
    fn damage_region_reports_bounds_area_and_full_coverage() {
        let mut damage = DamageRegion::new(10, 8);
        assert_eq!(damage.bounding_rect(), None);
        assert_eq!(damage.area(), 0);
        assert!(!damage.covers_entire_target());
        assert!(!DamageRegion::new(0, 0).covers_entire_target());

        damage.add(Rect::new(1, 1, 2, 3));
        damage.add(Rect::new(6, 4, 3, 2));

        assert_eq!(damage.bounding_rect(), Some(Rect::new(1, 1, 8, 5)));
        assert_eq!(damage.area(), 12);
        assert!(!damage.covers_entire_target());

        damage.clear();
        damage.add(Rect::new(0, 0, 10, 8));
        assert_eq!(damage.area(), 80);
        assert!(damage.covers_entire_target());
    }

    #[test]
    fn damage_region_full_coverage_is_correct_after_overlapping_additions() {
        let mut damage = DamageRegion::new(4, 3);
        damage.add(Rect::new(0, 0, 3, 3));
        damage.add(Rect::new(2, 0, 2, 3));

        assert_eq!(damage.area(), 12);
        assert!(damage.covers_entire_target());
        assert_eq!(damage.bounding_rect(), Some(Rect::new(0, 0, 4, 3)));
    }

    #[test]
    fn damage_region_merges_edge_adjacent_rectangles_without_expanding_area() {
        let mut damage = DamageRegion::new(8, 6);
        damage.add(Rect::new(0, 0, 2, 2));
        damage.add(Rect::new(2, 0, 3, 2));
        damage.add(Rect::new(0, 2, 5, 1));

        assert_eq!(damage.rectangles(), &[Rect::new(0, 0, 5, 3)]);
        assert_eq!(damage.area(), 15);
    }

    #[test]
    fn damage_region_preserves_exact_coverage_for_overlapping_l_shapes() {
        let mut damage = DamageRegion::new(4, 4);
        damage.add(Rect::new(0, 0, 2, 2));
        damage.add(Rect::new(1, 1, 2, 2));

        assert_eq!(damage.area(), 7);
        assert!(!damage.covers_entire_target());
        let mut covered = [[false; 4]; 4];
        for rectangle in damage.rectangles() {
            for y in rectangle.y..rectangle.y + rectangle.height {
                for x in rectangle.x..rectangle.x + rectangle.width {
                    assert!(!covered[y as usize][x as usize]);
                    covered[y as usize][x as usize] = true;
                }
            }
        }
        assert!(covered[0][0] && covered[1][0] && covered[0][1]);
        assert!(covered[1][1] && covered[2][1] && covered[1][2] && covered[2][2]);
        assert!(!covered[0][2] && !covered[2][0] && !covered[3][3]);
    }

    #[test]
    fn damage_region_does_not_merge_corner_touching_or_partial_edges() {
        let mut damage = DamageRegion::new(8, 8);
        damage.add(Rect::new(0, 0, 2, 2));
        damage.add(Rect::new(2, 2, 2, 2));
        damage.add(Rect::new(2, 0, 2, 1));

        assert_eq!(damage.rectangles().len(), 3);
        assert_eq!(damage.area(), 10);
    }

    #[test]
    fn damage_region_reuses_reserved_storage_after_clear() {
        let mut damage = DamageRegion::with_capacity(10, 10, 4);
        assert!(damage.capacity() >= 4);
        damage.add(Rect::new(0, 0, 2, 2));
        damage.add(Rect::new(4, 4, 2, 2));
        let capacity = damage.capacity();

        damage.clear();
        damage.add(Rect::new(1, 1, 2, 2));
        damage.add(Rect::new(5, 5, 2, 2));

        assert_eq!(damage.capacity(), capacity);
        assert_eq!(damage.rectangles().len(), 2);
    }

    #[test]
    fn layer_can_take_and_return_an_existing_framebuffer_without_copying() {
        let framebuffer = Framebuffer::new(2, 1, Color::BLACK).unwrap();
        let original_pixels = framebuffer.pixels().as_ptr();
        let layer = Layer::from_framebuffer(framebuffer);

        assert_eq!(layer.framebuffer().pixels().as_ptr(), original_pixels);
        let framebuffer = layer.into_framebuffer();
        assert_eq!(framebuffer.pixels().as_ptr(), original_pixels);
        assert_eq!(framebuffer.width(), 2);
        assert_eq!(framebuffer.height(), 1);
    }

    #[test]
    fn damage_region_clips_and_coalesces_overlapping_rectangles() {
        let mut damage = DamageRegion::new(10, 10);
        damage.add(Rect::new(1, 1, 4, 4));
        damage.add(Rect::new(3, 3, 4, 4));
        damage.add(Rect::new(12, 12, 2, 2));
        damage.add(Rect::new(0, 9, 1, 1));

        for y in 0..10 {
            for x in 0..10 {
                let expected = ((1..5).contains(&x) && (1..5).contains(&y))
                    || ((3..7).contains(&x) && (3..7).contains(&y))
                    || (x == 0 && y == 9);
                let coverage = damage
                    .rectangles()
                    .iter()
                    .filter(|rect| {
                        x >= rect.x
                            && y >= rect.y
                            && x < rect.x + rect.width
                            && y < rect.y + rect.height
                    })
                    .count();
                assert_eq!(coverage, usize::from(expected), "coverage at ({x}, {y})");
            }
        }

        damage.clear();
        assert!(damage.is_empty());
    }

    #[test]
    fn damage_composition_writes_only_inside_damaged_rectangles() {
        let layer = Layer::new(4, 3, Color::RED).unwrap();
        let mut target = Framebuffer::new(4, 3, Color::BLACK).unwrap();
        let mut damage = DamageRegion::new(4, 3);
        damage.add(Rect::new(1, 1, 2, 1));

        compose_layers_in_damage_regions(&mut target, &[layer], &damage);

        for y in 0..3 {
            for x in 0..4 {
                let expected = if y == 1 && (1..3).contains(&x) {
                    Color::RED
                } else {
                    Color::BLACK
                };
                assert_eq!(target.get_pixel(x, y), Some(expected));
            }
        }
    }

    #[test]
    fn transformed_layer_composition_respects_damage_clip() {
        let mut layer = Layer::new(2, 2, Color::GREEN).unwrap();
        layer.set_transform(Transform2D::scale(2.0, 2.0));
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        let mut damage = DamageRegion::new(4, 4);
        damage.add(Rect::new(1, 2, 2, 1));

        compose_layers_in_damage_regions(&mut target, &[layer], &damage);

        for y in 0..4 {
            for x in 0..4 {
                let expected = if y == 2 && (1..3).contains(&x) {
                    Color::GREEN
                } else {
                    Color::BLACK
                };
                assert_eq!(target.get_pixel(x, y), Some(expected));
            }
        }
    }

    #[test]
    fn damage_region_reset_updates_bounds_and_retains_storage() {
        let mut damage = DamageRegion::with_capacity(8, 8, 4);
        damage.add(Rect::new(1, 1, 2, 2));
        let capacity = damage.capacity();
        assert!(!damage.is_empty());

        damage.reset(3, 2);

        assert!(damage.is_empty());
        assert_eq!(damage.capacity(), capacity);
        damage.add(Rect::new(2, 1, 4, 4));
        assert_eq!(damage.rectangles(), &[Rect::new(2, 1, 1, 1)]);
    }

    #[test]
    fn full_coverage_damage_uses_full_frame_composition_semantics() {
        let mut first = Layer::new(4, 3, Color::rgba(220, 30, 10, 180)).unwrap();
        first.set_position(-1, 0);
        let mut second = Layer::new(3, 2, Color::rgba(10, 40, 230, 160)).unwrap();
        second.set_position(1, 1);
        second.set_opacity(190);
        let layers = [first, second];

        let mut damage = DamageRegion::new(4, 3);
        // Disjoint rectangles whose union covers the complete target.
        damage.add(Rect::new(0, 0, 2, 3));
        damage.add(Rect::new(2, 0, 2, 3));
        assert!(damage.covers_entire_target());

        let mut actual = Framebuffer::new(4, 3, Color::BLACK).unwrap();
        let mut expected = actual.clone();
        compose_layers_in_damage_regions(&mut actual, &layers, &damage);
        compose_layers(&mut expected, &layers);

        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn layers_compose_in_back_to_front_order() {
        let mut background = Layer::new(1, 1, Color::RED).unwrap();
        let foreground = Layer::new(1, 1, Color::BLUE).unwrap();
        background.framebuffer_mut().set_pixel(0, 0, Color::RED);

        let mut target = Framebuffer::new(1, 1, Color::BLACK).unwrap();
        compose_layers(&mut target, &[background, foreground]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::BLUE));
    }

    #[test]
    fn layer_composition_opacity_and_blend_modes_match_exact_pixel_reference() {
        let mut layers = Vec::new();

        let mut source_over = Layer::new(1, 1, Color::RED).unwrap();
        source_over.set_position(0, 0);
        layers.push(source_over);

        let mut multiply = Layer::new(1, 1, Color::RED).unwrap();
        multiply.set_position(1, 0);
        multiply.set_blend_mode(BlendMode::Multiply);
        layers.push(multiply);

        let mut screen = Layer::new(1, 1, Color::RED).unwrap();
        screen.set_position(2, 0);
        screen.set_blend_mode(BlendMode::Screen);
        layers.push(screen);

        let mut add = Layer::new(1, 1, Color::RED).unwrap();
        add.set_position(3, 0);
        add.set_blend_mode(BlendMode::Add);
        layers.push(add);

        let mut translucent = Layer::new(1, 1, Color::RED).unwrap();
        translucent.set_position(4, 0);
        translucent.set_opacity(128);
        layers.push(translucent);

        let mut target = Framebuffer::new(5, 1, Color::BLUE).unwrap();
        compose_layers(&mut target, &layers);

        assert_eq!(
            target.pixels(),
            &[
                Color::RED,
                Color::BLACK,
                Color::rgb(255, 0, 255),
                Color::rgb(255, 0, 255),
                Color::rgb(128, 0, 127),
            ],
        );
    }

    #[test]
    fn layer_opacity_blends_with_destination() {
        let mut layer = Layer::new(1, 1, Color::RED).unwrap();
        layer.set_opacity(128);
        let mut target = Framebuffer::new(1, 1, Color::BLUE).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(128, 0, 127)));
    }

    #[test]
    fn multiply_screen_and_add_blend_opaque_layer_channels() {
        let modes = [
            (BlendMode::Multiply, Color::rgb(0, 0, 0)),
            (BlendMode::Screen, Color::rgb(255, 255, 0)),
            (BlendMode::Add, Color::rgb(255, 255, 0)),
        ];

        for (mode, expected) in modes {
            let mut layer = Layer::new(1, 1, Color::RED).unwrap();
            layer.set_blend_mode(mode);
            let mut target = Framebuffer::new(1, 1, Color::GREEN).unwrap();

            compose_layers(&mut target, &[layer]);

            assert_eq!(target.get_pixel(0, 0), Some(expected));
        }
    }

    #[test]
    fn non_source_over_blends_respect_layer_and_source_alpha() {
        let mut layer = Layer::new(1, 1, Color::rgba(255, 0, 0, 128)).unwrap();
        layer.set_blend_mode(BlendMode::Screen);
        layer.set_opacity(128);
        let mut target = Framebuffer::new(1, 1, Color::rgba(0, 0, 255, 128)).unwrap();

        compose_layers(&mut target, &[layer]);

        let pixel = target.get_pixel(0, 0).unwrap();
        assert!(pixel.r > 0);
        assert!(pixel.b > 0);
        assert!(pixel.a > 128);
    }

    #[test]
    fn layer_transform_scales_pixels_using_pixel_center_sampling() {
        let mut layer = Layer::new(1, 1, Color::RED).unwrap();
        layer.set_transform(Transform2D::scale(2.0, 2.0));
        let mut target = Framebuffer::new(2, 2, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert!(target.pixels().iter().all(|pixel| *pixel == Color::RED));
    }

    #[test]
    fn fused_bilinear_composition_matches_reference_over_opaque_destinations() {
        use crate::texture::sample_pixels_bilinear;

        let pixels = vec![
            Color::rgba(240, 30, 20, 255),
            Color::rgba(20, 220, 80, 180),
            Color::TRANSPARENT,
            Color::rgba(30, 60, 240, 90),
            Color::rgba(210, 170, 40, 210),
            Color::rgba(15, 80, 230, 140),
        ];
        let width = 3;
        let height = 2;
        for opacity in [1_u8, 64, 128, 200, 255] {
            for y in [0.0, 0.25, 0.5, 0.9] {
                for x in [0.0, 0.2, 0.75, 1.4, 1.9] {
                    let mut actual = Color::rgb(37, 91, 163);
                    let source = sample_pixels_bilinear(&pixels, width, height, x, y);
                    let alpha = ((u16::from(source.a) * u16::from(opacity) + 127) / 255) as u8;
                    let expected =
                        Color::rgba(source.r, source.g, source.b, alpha).blend_over(actual);
                    blend_bilinear_over_opaque(&pixels, width, height, x, y, opacity, &mut actual);
                    assert!(actual.r.abs_diff(expected.r) <= 1);
                    assert!(actual.g.abs_diff(expected.g) <= 1);
                    assert!(actual.b.abs_diff(expected.b) <= 1);
                    assert_eq!(actual.a, 255);
                }
            }
        }
    }

    #[test]
    fn axis_aligned_bilinear_rows_match_general_sampling_path() {
        let pixels = [
            Color::rgba(240, 30, 20, 255),
            Color::rgba(20, 220, 80, 180),
            Color::TRANSPARENT,
            Color::rgba(30, 60, 240, 90),
            Color::rgba(210, 170, 40, 210),
            Color::rgba(15, 80, 230, 140),
        ];
        let top_row = &pixels[..3];
        let bottom_row = &pixels[3..];
        for opacity in [1_u8, 64, 128, 200, 255] {
            for ty in [0.0, 0.25, 0.5, 0.9] {
                for x in [0.0, 0.2, 0.75, 1.4, 1.9] {
                    let mut expected = Color::rgb(37, 91, 163);
                    let mut actual = expected;
                    blend_bilinear_over_opaque(&pixels, 3, 2, x, ty, opacity, &mut expected);
                    blend_bilinear_over_opaque_rows(
                        top_row,
                        bottom_row,
                        x,
                        ty,
                        opacity,
                        &mut actual,
                    );
                    assert_eq!(actual, expected, "x={x}, y={ty}, opacity={opacity}");
                }
            }
        }
    }

    #[test]
    fn transformed_layer_skips_scanlines_outside_source_y() {
        let mut layer = Layer::new(1, 1, Color::RED).unwrap();
        // The transformed bounds include the destination scanline, but its
        // pixel center maps exactly to source Y = 1.0, outside the 1px source.
        layer.set_transform(Transform2D::scale(0.5, 0.5));
        let mut target = Framebuffer::new(1, 1, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn horizontal_nearest_sampling_preserves_alpha_and_blend_modes() {
        let mut layer = Layer::new(2, 2, Color::TRANSPARENT).unwrap();
        layer.framebuffer_mut().pixels_mut().copy_from_slice(&[
            Color::rgba(230, 20, 40, 255),
            Color::rgba(30, 210, 70, 160),
            Color::rgba(40, 80, 220, 96),
            Color::rgba(200, 170, 30, 210),
        ]);
        layer.set_transform(Transform2D::scale(2.0, 2.0));
        layer.set_opacity(173);
        layer.set_blend_mode(BlendMode::Screen);

        let background = Color::rgb(17, 39, 83);
        let mut actual = Framebuffer::new(4, 4, background).unwrap();
        let mut expected = actual.clone();

        for y in 0..4 {
            for x in 0..4 {
                let source_x = x / 2;
                let source_y = y / 2;
                let source = layer.framebuffer().get_pixel(source_x, source_y).unwrap();
                let destination = expected.get_pixel(x, y).unwrap();
                expected.set_pixel(
                    x,
                    y,
                    blend_with_mode(source, destination, layer.opacity, layer.blend_mode),
                );
            }
        }

        compose_layers(&mut actual, &[layer]);

        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn horizontal_nearest_source_over_copies_opaque_pixels_directly() {
        let mut layer = Layer::new(2, 1, Color::TRANSPARENT).unwrap();
        layer
            .framebuffer_mut()
            .pixels_mut()
            .copy_from_slice(&[Color::rgba(230, 20, 40, 255), Color::rgba(30, 210, 70, 128)]);
        layer.set_transform(Transform2D::scale(2.0, 1.0));

        let background = Color::rgb(17, 39, 83);
        let mut actual = Framebuffer::new(4, 1, background).unwrap();
        let mut expected = actual.clone();
        let expected_pixels = [
            Color::rgba(230, 20, 40, 255),
            Color::rgba(230, 20, 40, 255),
            Color::rgba(30, 210, 70, 128).blend_over(background),
            Color::rgba(30, 210, 70, 128).blend_over(background),
        ];
        expected.pixels_mut().copy_from_slice(&expected_pixels);

        compose_layers(&mut actual, &[layer]);

        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn horizontal_nearest_source_over_matches_reference_for_all_opacities() {
        let source_pixels = [
            Color::rgba(230, 20, 40, 0),
            Color::rgba(30, 210, 70, 1),
            Color::rgba(40, 80, 220, 128),
            Color::rgba(200, 170, 30, 255),
        ];
        let background = Color::rgba(17, 39, 83, 255);

        for opacity in [1_u8, 64, 127, 254, 255] {
            let mut layer = Layer::new(4, 1, Color::TRANSPARENT).unwrap();
            layer
                .framebuffer_mut()
                .pixels_mut()
                .copy_from_slice(&source_pixels);
            layer.set_transform(Transform2D::scale(2.0, 1.0));
            layer.set_opacity(opacity);

            let mut actual = Framebuffer::new(8, 1, background).unwrap();
            let mut expected = actual.clone();
            for (source_x, source) in source_pixels.iter().copied().enumerate() {
                let alpha = ((u16::from(source.a) * u16::from(opacity) + 127) / 255) as u8;
                let blended =
                    Color::rgba(source.r, source.g, source.b, alpha).blend_over(background);
                expected.set_pixel((source_x * 2) as u32, 0, blended);
                expected.set_pixel((source_x * 2 + 1) as u32, 0, blended);
            }

            compose_layers(&mut actual, &[layer]);

            assert_eq!(actual.pixels(), expected.pixels(), "opacity={opacity}");
        }
    }

    #[test]
    fn transformed_layer_supports_bilinear_filtering() {
        let mut layer = Layer::new(2, 1, Color::TRANSPARENT).unwrap();
        layer.framebuffer_mut().set_pixel(0, 0, Color::RED);
        layer.framebuffer_mut().set_pixel(1, 0, Color::BLUE);
        layer.set_transform(Transform2D::scale(2.0, 1.0));
        layer.set_filter(TextureFilter::Bilinear);
        let mut target = Framebuffer::new(4, 1, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(1, 0), Some(Color::rgb(191, 0, 64)));
        assert_eq!(target.get_pixel(2, 0), Some(Color::rgb(64, 0, 191)));
        assert_eq!(target.get_pixel(3, 0), Some(Color::BLUE));
    }

    #[test]
    fn layer_transform_rotates_and_applies_position_offset() {
        let mut layer = Layer::new(2, 1, Color::TRANSPARENT).unwrap();
        layer.framebuffer_mut().set_pixel(0, 0, Color::RED);
        layer.framebuffer_mut().set_pixel(1, 0, Color::GREEN);
        layer.set_transform(Transform2D::rotation(core::f32::consts::FRAC_PI_2));
        layer.set_position(1, 0);
        let mut target = Framebuffer::new(2, 2, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(0, 1), Some(Color::GREEN));
        assert_eq!(target.get_pixel(1, 0), Some(Color::BLACK));
    }

    #[test]
    fn singular_layer_transform_draws_nothing() {
        let mut layer = Layer::new(1, 1, Color::RED).unwrap();
        layer.set_transform(Transform2D::scale(0.0, 1.0));
        let mut target = Framebuffer::new(2, 2, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn layer_position_is_clipped_to_target() {
        let mut layer = Layer::new(2, 2, Color::GREEN).unwrap();
        layer.set_position(-1, 1);
        let mut target = Framebuffer::new(2, 2, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(target.get_pixel(0, 1), Some(Color::GREEN));
        assert_eq!(target.get_pixel(1, 1), Some(Color::BLACK));
    }

    #[test]
    fn invisible_layers_are_not_composited() {
        let mut layer = Layer::new(1, 1, Color::RED).unwrap();
        layer.set_visible(false);
        let mut target = Framebuffer::new(1, 1, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn row_slice_composition_matches_reference_for_clipping_and_alpha() {
        let mut layers = Vec::new();
        for (index, (x, y, opacity, visible)) in [
            (-2, -1, 255, true),
            (2, 1, 137, true),
            (5, 3, 0, true),
            (-1, 4, 211, false),
            (7, -2, 93, true),
        ]
        .into_iter()
        .enumerate()
        {
            let mut layer = Layer::new(6, 5, Color::TRANSPARENT).unwrap();
            layer.set_position(x, y);
            layer.set_opacity(opacity);
            layer.set_visible(visible);
            for py in 0..5 {
                for px in 0..6 {
                    let seed = (index as u8)
                        .wrapping_mul(47)
                        .wrapping_add(((py * 6 + px) as u8).wrapping_mul(19));
                    let alpha = match (px + py + index as u32) % 4 {
                        0 => 0,
                        1 => 96,
                        2 => 190,
                        _ => 255,
                    };
                    layer.framebuffer_mut().set_pixel(
                        px,
                        py,
                        Color::rgba(seed, seed.wrapping_add(71), seed.wrapping_add(149), alpha),
                    );
                }
            }
            layers.push(layer);
        }

        let initial = Color::rgba(17, 29, 43, 255);
        let mut optimized = Framebuffer::new(8, 7, initial).unwrap();
        let mut reference = optimized.clone();

        for layer in &layers {
            if !layer.visible || layer.opacity == 0 {
                continue;
            }
            for source_y in 0..layer.framebuffer.height() {
                for source_x in 0..layer.framebuffer.width() {
                    let destination_x = i64::from(layer.x) + i64::from(source_x);
                    let destination_y = i64::from(layer.y) + i64::from(source_y);
                    if destination_x < 0
                        || destination_y < 0
                        || destination_x >= i64::from(reference.width())
                        || destination_y >= i64::from(reference.height())
                    {
                        continue;
                    }
                    let Some(source) = layer.framebuffer.get_pixel(source_x, source_y) else {
                        continue;
                    };
                    if source.a == 0 {
                        continue;
                    }
                    let alpha =
                        ((u16::from(source.a) * u16::from(layer.opacity) + 127) / 255) as u8;
                    if alpha == 0 {
                        continue;
                    }
                    let source = Color::rgba(source.r, source.g, source.b, alpha);
                    let x = destination_x as u32;
                    let y = destination_y as u32;
                    let destination = reference.get_pixel(x, y).unwrap();
                    reference.set_pixel(x, y, source.blend_over(destination));
                }
            }
        }

        compose_layers(&mut optimized, &layers);

        assert_eq!(optimized.pixels(), reference.pixels());
    }

    #[test]
    fn identity_layer_composition_skips_rows_outside_vertical_clip() {
        let mut layer = Layer::new(2, 5, Color::TRANSPARENT).unwrap();
        for y in 0..5 {
            for x in 0..2 {
                layer.framebuffer_mut().set_pixel(
                    x,
                    y,
                    Color::rgb((y * 40) as u8, (x * 80) as u8, 10),
                );
            }
        }
        layer.set_position(1, -3);

        let mut target = Framebuffer::new(4, 3, Color::BLACK).unwrap();
        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(1, 0), Some(Color::rgb(120, 0, 10)));
        assert_eq!(target.get_pixel(2, 0), Some(Color::rgb(120, 80, 10)));
        assert_eq!(target.get_pixel(1, 1), Some(Color::rgb(160, 0, 10)));
        assert_eq!(target.get_pixel(2, 1), Some(Color::rgb(160, 80, 10)));
        assert!(target.get_pixel(1, 2).unwrap() == Color::BLACK);
    }

    #[test]
    fn layer_opacity_zero_is_a_no_op() {
        let mut layer = Layer::new(1, 1, Color::RED).unwrap();
        layer.set_opacity(0);
        let mut target = Framebuffer::new(1, 1, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn opaque_source_over_rows_preserve_mixed_alpha_semantics() {
        let mut layer = Layer::new(3, 2, Color::TRANSPARENT).unwrap();
        layer.framebuffer_mut().pixels_mut().copy_from_slice(&[
            Color::RED,
            Color::GREEN,
            Color::BLUE,
            Color::rgba(200, 10, 20, 128),
            Color::TRANSPARENT,
            Color::WHITE,
        ]);
        let background = Color::rgb(12, 34, 56);
        let mut target = Framebuffer::new(3, 2, background).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(
            &target.pixels()[..3],
            &[Color::RED, Color::GREEN, Color::BLUE]
        );
        assert_eq!(
            target.get_pixel(0, 1),
            Some(Color::rgba(200, 10, 20, 128).blend_over(background))
        );
        assert_eq!(target.get_pixel(1, 1), Some(background));
        assert_eq!(target.get_pixel(2, 1), Some(Color::WHITE));
    }

    #[test]
    fn transformed_damage_composition_matches_full_composition_inside_damage() {
        let mut background = Layer::new(6, 5, Color::rgba(20, 40, 60, 255)).unwrap();
        background.set_position(-1, 0);

        let mut transformed = Layer::new(3, 2, Color::TRANSPARENT).unwrap();
        transformed
            .framebuffer_mut()
            .pixels_mut()
            .copy_from_slice(&[
                Color::RED,
                Color::rgba(20, 220, 80, 180),
                Color::BLUE,
                Color::WHITE,
                Color::rgba(240, 30, 20, 128),
                Color::TRANSPARENT,
            ]);
        transformed.set_position(2, 1);
        transformed.set_transform(
            Transform2D::translation(0.25, 0.5)
                .then(Transform2D::rotation(0.17))
                .then(Transform2D::scale(1.4, 1.2)),
        );
        transformed.set_filter(TextureFilter::Bilinear);
        transformed.set_opacity(213);

        let mut foreground = Layer::new(2, 3, Color::rgba(40, 80, 160, 150)).unwrap();
        foreground.set_position(3, 1);
        foreground.set_blend_mode(BlendMode::Screen);
        let layers = [background, transformed, foreground];

        let mut damage = DamageRegion::new(8, 6);
        damage.add(Rect::new(0, 0, 3, 2));
        damage.add(Rect::new(5, 1, 3, 4));
        damage.add(Rect::new(2, 4, 3, 2));

        let initial = Color::rgb(9, 17, 25);
        let mut actual = Framebuffer::new(8, 6, initial).unwrap();
        let mut expected = actual.clone();
        compose_layers(&mut expected, &layers);
        compose_layers_in_damage_regions(&mut actual, &layers, &damage);

        for rectangle in damage.rectangles() {
            for y in rectangle.y..rectangle.y + rectangle.height {
                for x in rectangle.x..rectangle.x + rectangle.width {
                    assert_eq!(
                        actual.get_pixel(x, y),
                        expected.get_pixel(x, y),
                        "mismatch at ({x}, {y})"
                    );
                }
            }
        }
        for y in 0..actual.height() {
            for x in 0..actual.width() {
                if !damage
                    .rectangles()
                    .iter()
                    .any(|rect| rect.contains(crate::Point::new(x as i32, y as i32)))
                {
                    assert_eq!(actual.get_pixel(x, y), Some(initial));
                }
            }
        }
    }

    #[test]
    fn fully_transparent_source_over_rows_leave_destination_unchanged() {
        let mut layer = Layer::new(4, 2, Color::TRANSPARENT).unwrap();
        layer.framebuffer_mut().pixels_mut()[4..].copy_from_slice(&[
            Color::RED,
            Color::TRANSPARENT,
            Color::BLUE,
            Color::WHITE,
        ]);
        let background = Color::rgb(19, 37, 83);
        let mut target = Framebuffer::new(4, 2, background).unwrap();

        compose_layers(&mut target, &[layer]);

        assert!(target.pixels()[..4]
            .iter()
            .all(|pixel| *pixel == background));
        assert_eq!(target.get_pixel(0, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(1, 1), Some(background));
        assert_eq!(target.get_pixel(2, 1), Some(Color::BLUE));
        assert_eq!(target.get_pixel(3, 1), Some(Color::WHITE));
    }

    #[test]
    fn opaque_source_over_row_copy_is_disabled_for_partial_opacity() {
        let mut layer = Layer::new(2, 1, Color::WHITE).unwrap();
        layer.set_opacity(128);
        let background = Color::rgb(20, 40, 60);
        let mut target = Framebuffer::new(2, 1, background).unwrap();

        compose_layers(&mut target, &[layer]);

        let expected = Color::rgba(255, 255, 255, 128).blend_over(background);
        assert!(target.pixels().iter().all(|pixel| *pixel == expected));
    }
}
