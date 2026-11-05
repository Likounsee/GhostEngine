//! Ordered CPU layer composition with offsets, visibility, and opacity.

use crate::{Color, Framebuffer, FramebufferError};

/// A framebuffer-backed layer positioned in the destination coordinate space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    framebuffer: Framebuffer,
    x: i32,
    y: i32,
    opacity: u8,
    visible: bool,
}

impl Layer {
    /// Creates a layer with its own framebuffer and an opaque initial visibility.
    pub fn new(
        width: u32,
        height: u32,
        clear_color: Color,
    ) -> Result<Self, FramebufferError> {
        Ok(Self {
            framebuffer: Framebuffer::new(width, height, clear_color)?,
            x: 0,
            y: 0,
            opacity: 255,
            visible: true,
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

        for source_y in 0..layer.framebuffer.height() {
            let destination_y = i64::from(layer.y) + i64::from(source_y);
            if destination_y < 0 || destination_y >= i64::from(target.height()) {
                continue;
            }
            for source_x in 0..layer.framebuffer.width() {
                let destination_x = i64::from(layer.x) + i64::from(source_x);
                if destination_x < 0 || destination_x >= i64::from(target.width()) {
                    continue;
                }

                let Some(mut source) = layer.framebuffer.get_pixel(source_x, source_y) else {
                    continue;
                };
                if source.a == 0 {
                    continue;
                }
                source.a = ((u16::from(source.a) * u16::from(layer.opacity) + 127) / 255) as u8;
                if source.a == 0 {
                    continue;
                }

                let x = destination_x as u32;
                let y = destination_y as u32;
                if let Some(background) = target.get_pixel(x, y) {
                    target.set_pixel(x, y, source.blend_over(background));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{compose_layers, Layer};
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
    fn layer_opacity_zero_is_a_no_op() {
        let mut layer = Layer::new(1, 1, Color::RED).unwrap();
        layer.set_opacity(0);
        let mut target = Framebuffer::new(1, 1, Color::BLACK).unwrap();

        compose_layers(&mut target, &[layer]);

        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }
}
