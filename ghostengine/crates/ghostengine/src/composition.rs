//! Ordered CPU layer composition with offsets, visibility, and opacity.

use crate::{Color, Framebuffer, FramebufferError};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    framebuffer: Framebuffer,
    x: i32,
    y: i32,
    opacity: u8,
    visible: bool,
    blend_mode: BlendMode,
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
}

/// Composites layers in slice order, from back to front.
///
/// Each layer is clipped to the destination framebuffer. Layer opacity multiplies
/// the source pixel alpha before source-over composition. Hidden and fully
/// transparent layers are skipped.
pub fn compose_layers(target: &mut Framebuffer, layers: &[Layer]) {
    for layer in layers {
        if !layer.visible || layer.opacity == 0 {
            continue;
        }

        let destination_left = i64::from(layer.x).max(0);
        let destination_right = (i64::from(layer.x) + i64::from(layer.framebuffer.width()))
            .min(i64::from(target.width()));
        if destination_left >= destination_right {
            continue;
        }
        let source_left = (destination_left - i64::from(layer.x)) as usize;
        let source_right = source_left + (destination_right - destination_left) as usize;
        let source_width = layer.framebuffer.width() as usize;

        for source_y in 0..layer.framebuffer.height() {
            let destination_y = i64::from(layer.y) + i64::from(source_y);
            if destination_y < 0 || destination_y >= i64::from(target.height()) {
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
    use super::{compose_layers, BlendMode, Layer};
    use crate::{Color, Framebuffer};

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
