//! Ordered CPU layer composition with offsets, visibility, and opacity.

use crate::texture::{sample_pixels_bilinear, sample_pixels_nearest};
use crate::{Color, Framebuffer, FramebufferError, Rect, TextureFilter, Transform2D, Vector};

/// A bounded collection of framebuffer regions that need recomposition.
///
/// Added rectangles are clipped to the dimensions supplied at construction.
/// Overlapping rectangles are coalesced into conservative bounding rectangles.
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

    /// Reserves additional storage for at least the requested number of more rectangles.
    pub fn reserve(&mut self, additional: usize) {
        self.rectangles.reserve(additional);
    }

    /// Returns the number of rectangles that can be stored without reallocating.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.rectangles.capacity()
    }

    /// Adds a rectangle, clipping it to the configured framebuffer bounds.
    ///
    /// Overlapping entries are merged. The resulting rectangles remain disjoint,
    /// so a damaged pixel is never composited twice.
    pub fn add(&mut self, rectangle: Rect) {
        let bounds = Rect::new(0, 0, self.width, self.height);
        let Some(mut merged) = rectangle.intersection(bounds) else {
            return;
        };

        let mut index = 0;
        while index < self.rectangles.len() {
            if merged.intersection(self.rectangles[index]).is_some() {
                let current = self.rectangles.swap_remove(index);
                let left = merged.x.min(current.x);
                let top = merged.y.min(current.y);
                let right = (u64::from(merged.x) + u64::from(merged.width))
                    .max(u64::from(current.x) + u64::from(current.width));
                let bottom = (u64::from(merged.y) + u64::from(merged.height))
                    .max(u64::from(current.y) + u64::from(current.height));
                merged = Rect::new(
                    left,
                    top,
                    (right - u64::from(left)) as u32,
                    (bottom - u64::from(top)) as u32,
                );
                index = 0;
            } else {
                index += 1;
            }
        }
        self.rectangles.push(merged);
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
        Self::from_framebuffer(Framebuffer::new(width, height, clear_color)?)
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

    for rectangle in damage.rectangles() {
        compose_layers_clipped(target, layers, Some(*rectangle));
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
        if layer.transform != Transform2D::IDENTITY {
            compose_transformed_layer(target, layer, clip);
            continue;
        }

        let destination_left = i64::from(layer.x).max(i64::from(clip.x));
        let destination_right = (i64::from(layer.x) + i64::from(layer.framebuffer.width()))
            .min(i64::from(clip.x) + i64::from(clip.width));
        if destination_left >= destination_right {
            continue;
        }
        let source_left = (destination_left - i64::from(layer.x)) as usize;
        let source_right = source_left + (destination_right - destination_left) as usize;
        let source_width = layer.framebuffer.width() as usize;

        // Intersect vertically before entering the scanline loop. This avoids
        // visiting every off-screen source row when a large layer is only
        // partially visible (or almost entirely outside the target).
        let destination_top = i64::from(layer.y).max(i64::from(clip.y));
        let destination_bottom = (i64::from(layer.y)
            + i64::from(layer.framebuffer.height()))
        .min(i64::from(clip.y) + i64::from(clip.height));
        if destination_top >= destination_bottom {
            continue;
        }
        let source_top = (destination_top - i64::from(layer.y)) as usize;
        let source_bottom = (destination_bottom - i64::from(layer.y)) as usize;

        for source_y in source_top..source_bottom {
            let destination_y = (i64::from(layer.y) + source_y as i64) as u32;
            let source_start = source_y * source_width + source_left;
            let source_row = &layer.framebuffer.pixels()
                [source_start..source_y as usize * source_width + source_right];
            let Some(destination_row) = target.row_mut(destination_y as u32) else {
                continue;
            };
            let destination =
                &mut destination_row[destination_left as usize..destination_right as usize];

            // Fully opaque source-over scanlines need no per-pixel blending.
            // Check once per row, then copy the contiguous span in one operation.
            if layer.blend_mode == BlendMode::SourceOver
                && layer.opacity == u8::MAX
                && source_row.iter().all(|pixel| pixel.a == u8::MAX)
            {
                destination.copy_from_slice(source_row);
                continue;
            }

            for (source, destination) in source_row.iter().zip(destination) {
                if source.a == 0 {
                    continue;
                }
                if layer.blend_mode == BlendMode::SourceOver {
                    if layer.opacity == u8::MAX && source.a == u8::MAX {
                        *destination = *source;
                        continue;
                    }
                    let alpha = if layer.opacity == u8::MAX {
                        source.a
                    } else {
                        ((u16::from(source.a) * u16::from(layer.opacity) + 127) / 255) as u8
                    };
                    if alpha == 0 {
                        continue;
                    }
                    let source = Color::rgba(source.r, source.g, source.b, alpha);
                    *destination = source.blend_over(*destination);
                } else {
                    *destination =
                        blend_with_mode(*source, *destination, layer.opacity, layer.blend_mode);
                }
            }
        }
    }
}

fn compose_transformed_layer(target: &mut Framebuffer, layer: &Layer, clip: Rect) {
    let Some(inverse) = layer.transform.inverse() else {
        return;
    };
    if target.width() == 0 || target.height() == 0 {
        return;
    }

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
        return;
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

    let clip_left = clip.x as f32;
    let clip_top = clip.y as f32;
    let clip_right = (u64::from(clip.x) + u64::from(clip.width)) as f32;
    let clip_bottom = (u64::from(clip.y) + u64::from(clip.height)) as f32;
    let left = min_x.floor().clamp(clip_left, clip_right) as u32;
    let right = max_x.ceil().clamp(clip_left, clip_right) as u32;
    let top = min_y.floor().clamp(clip_top, clip_bottom) as u32;
    let bottom = max_y.ceil().clamp(clip_top, clip_bottom) as u32;
    if left >= right || top >= bottom {
        return;
    }

    let source_width = layer.framebuffer.width() as f32;
    let source_height = layer.framebuffer.height() as f32;
    // An affine inverse advances by a constant source-space delta for every
    // destination pixel in a row. Calculate the first sample once per row,
    // then use additions instead of rebuilding a Vector and multiplying the
    // inverse matrix for every pixel.
    let source_step_x = inverse.m11;
    let source_step_y = inverse.m21;
    let destination_offset_x = layer.x as f32;
    let destination_offset_y = layer.y as f32;
    let opacity_factor = f32::from(layer.opacity) / 255.0;
    for y in top..bottom {
        let destination_y = y as f32 + 0.5 - destination_offset_y;
        let mut source_x = inverse.m11 * (left as f32 + 0.5 - destination_offset_x)
            + inverse.m12 * destination_y
            + inverse.tx;
        let mut source_y = inverse.m21 * (left as f32 + 0.5 - destination_offset_x)
            + inverse.m22 * destination_y
            + inverse.ty;
        let Some(destination_row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            if source_x >= 0.0
                && source_y >= 0.0
                && source_x < source_width
                && source_y < source_height
            {
                let destination = &mut destination_row[x as usize];
                if layer.filter == TextureFilter::Bilinear
                    && layer.blend_mode == BlendMode::SourceOver
                    && destination.a == u8::MAX
                {
                    blend_bilinear_over_opaque(
                        layer.framebuffer.pixels(),
                        layer.framebuffer.width(),
                        layer.framebuffer.height(),
                        source_x - 0.5,
                        source_y - 0.5,
                        opacity_factor,
                        destination,
                    );
                } else {
                    let source = match layer.filter {
                        TextureFilter::Nearest => sample_pixels_nearest(
                            layer.framebuffer.pixels(),
                            layer.framebuffer.width(),
                            layer.framebuffer.height(),
                            source_x,
                            source_y,
                        ),
                        TextureFilter::Bilinear => sample_pixels_bilinear(
                            layer.framebuffer.pixels(),
                            layer.framebuffer.width(),
                            layer.framebuffer.height(),
                            source_x - 0.5,
                            source_y - 0.5,
                        ),
                    };
                    if source.a != 0 {
                        if layer.blend_mode == BlendMode::SourceOver {
                            let alpha = ((u16::from(source.a) * u16::from(layer.opacity) + 127)
                                / 255) as u8;
                            if alpha > 0 {
                                *destination = Color::rgba(source.r, source.g, source.b, alpha)
                                    .blend_over(*destination);
                            }
                        } else {
                            *destination = blend_with_mode(
                                source,
                                *destination,
                                layer.opacity,
                                layer.blend_mode,
                            );
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
fn blend_bilinear_over_opaque(
    pixels: &[Color],
    width: u32,
    height: u32,
    x: f32,
    y: f32,
    opacity_factor: f32,
    destination: &mut Color,
) {
    if width == 0 || height == 0 || opacity_factor <= 0.0 {
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

    // Share the four bilinear weights across alpha and all color channels.
    // The separable form repeats horizontal/vertical arithmetic for each
    // channel; these weights let each channel use only four multiply-adds.
    let inverse_x = 1.0 - tx;
    let inverse_y = 1.0 - ty;
    let weight_top_left = inverse_x * inverse_y;
    let weight_top_right = tx * inverse_y;
    let weight_bottom_left = inverse_x * ty;
    let weight_bottom_right = tx * ty;
    let alpha = (f32::from(top_left.a) * weight_top_left
        + f32::from(top_right.a) * weight_top_right
        + f32::from(bottom_left.a) * weight_bottom_left
        + f32::from(bottom_right.a) * weight_bottom_right)
        * opacity_factor;
    if alpha <= f32::EPSILON {
        return;
    }

    let inverse_alpha = 255.0 - alpha;
    let channel = |tl: u8, tr: u8, bl: u8, br: u8, background: u8| {
        let premultiplied = (f32::from(tl) * f32::from(top_left.a) * weight_top_left
            + f32::from(tr) * f32::from(top_right.a) * weight_top_right
            + f32::from(bl) * f32::from(bottom_left.a) * weight_bottom_left
            + f32::from(br) * f32::from(bottom_right.a) * weight_bottom_right)
            * opacity_factor;
        ((premultiplied + f32::from(background) * inverse_alpha) / 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    destination.r = channel(top_left.r, top_right.r, bottom_left.r, bottom_right.r, destination.r);
    destination.g = channel(top_left.g, top_right.g, bottom_left.g, bottom_right.g, destination.g);
    destination.b = channel(top_left.b, top_right.b, bottom_left.b, bottom_right.b, destination.b);
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

#[cfg(test)]
mod tests {
    use super::{
        blend_bilinear_over_opaque, compose_layers, compose_layers_in_damage_regions, BlendMode,
        DamageRegion, Layer,
    };
    use crate::{Color, Framebuffer, Rect, TextureFilter, Transform2D};

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

        assert_eq!(damage.rectangles().len(), 2);
        assert!(damage.rectangles().contains(&Rect::new(1, 1, 6, 6)));
        assert!(damage.rectangles().contains(&Rect::new(0, 9, 1, 1)));

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
                    let alpha =
                        ((u16::from(source.a) * u16::from(opacity) + 127) / 255) as u8;
                    let expected = Color::rgba(source.r, source.g, source.b, alpha)
                        .blend_over(actual);
                    blend_bilinear_over_opaque(
                        &pixels,
                        width,
                        height,
                        x,
                        y,
                        f32::from(opacity) / 255.0,
                        &mut actual,
                    );
                    assert!(actual.r.abs_diff(expected.r) <= 1);
                    assert!(actual.g.abs_diff(expected.g) <= 1);
                    assert!(actual.b.abs_diff(expected.b) <= 1);
                    assert_eq!(actual.a, 255);
                }
            }
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

        assert_eq!(&target.pixels()[..3], &[Color::RED, Color::GREEN, Color::BLUE]);
        assert_eq!(
            target.get_pixel(0, 1),
            Some(Color::rgba(200, 10, 20, 128).blend_over(background))
        );
        assert_eq!(target.get_pixel(1, 1), Some(background));
        assert_eq!(target.get_pixel(2, 1), Some(Color::WHITE));
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
