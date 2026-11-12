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

    /// Removes all accumulated damage rectangles.
    pub fn clear(&mut self) {
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
        Ok(Self {
            framebuffer: Framebuffer::new(width, height, clear_color)?,
            x: 0,
            y: 0,
            opacity: 255,
            visible: true,
            blend_mode: BlendMode::SourceOver,
            transform: Transform2D::IDENTITY,
            filter: TextureFilter::Nearest,
        })
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

        for source_y in 0..layer.framebuffer.height() {
            let destination_y = i64::from(layer.y) + i64::from(source_y);
            if destination_y < i64::from(clip.y)
                || destination_y >= i64::from(clip.y) + i64::from(clip.height)
            {
                continue;
            }

            let source_start = source_y as usize * source_width + source_left;
            let source_row = &layer.framebuffer.pixels()
                [source_start..source_y as usize * source_width + source_right];
            let Some(destination_row) = target.row_mut(destination_y as u32) else {
                continue;
            };
            let destination =
                &mut destination_row[destination_left as usize..destination_right as usize];

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
                    let destination = &mut destination_row[x as usize];
                    if layer.blend_mode == BlendMode::SourceOver {
                        let alpha =
                            ((u16::from(source.a) * u16::from(layer.opacity) + 127) / 255) as u8;
                        if alpha > 0 {
                            *destination = Color::rgba(source.r, source.g, source.b, alpha)
                                .blend_over(*destination);
                        }
                    } else {
                        *destination =
                            blend_with_mode(source, *destination, layer.opacity, layer.blend_mode);
                    }
                }
            }
            source_x += source_step_x;
            source_y += source_step_y;
        }
    }
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
    use super::{compose_layers, compose_layers_in_damage_regions, BlendMode, DamageRegion, Layer};
    use crate::{Color, Framebuffer, Rect, TextureFilter, Transform2D};

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
    fn layer_opacity_zero_is_a_no_op() {
        let mut layer = Layer::new(1, 1, Color::RED).unwrap();
        layer.set_opacity(0);
        let mut target = Framebuffer::new(1, 1, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }
}
