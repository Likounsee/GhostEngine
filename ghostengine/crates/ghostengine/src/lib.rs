#![forbid(unsafe_code)]

//! GhostEngine's backend-independent foundations and CPU rendering primitives.
//! The crate intentionally has no graphics API or platform dependencies.

pub mod cached_layer;
pub mod color;
pub mod components;
pub mod composition;
pub mod display_list;
pub mod effects;
pub mod focus;
pub mod framebuffer;
pub mod geometry;
#[cfg(feature = "image-decoding")]
pub mod image;
pub mod interaction;
pub mod layout;
pub mod mask;
pub mod mipmap;
pub mod path;
pub mod path_cache;
pub mod rasterizer;
mod simd;
pub mod shaping;
pub mod stroke;
pub mod text;
pub mod texture;
pub mod texture_atlas;
pub mod transform;
pub mod ui;

#[cfg(feature = "window")]
pub mod window;

#[cfg(feature = "native-input")]
pub mod native_input;

#[cfg(feature = "native-window")]
pub mod native_window;

#[cfg(feature = "native-input")]
pub use native_input::{NativeClipboard, NativeClipboardError, NativeInputAdapter};

#[cfg(feature = "native-window")]
pub use native_window::{NativeWindowError, NativeWindowSurface};

pub use cached_layer::CachedLayer;
pub use color::Color;
pub use components::{
    ClipboardAction, ComponentEvents, ComponentId, ComponentKey, ComponentKind, ComponentTree,
    ComponentTreeError, ContextMenuItem, ContextMenuShortcut, PlacedComponent, TextInputEvent,
    UiComponent,
};
pub use composition::{
    compose_layers, compose_layers_auto, compose_layers_in_damage_regions,
    compose_layers_parallel_with_threads, BlendMode, DamageRegion, Layer,
};
pub use display_list::{DisplayList, DisplayListRenderer, RenderStats};
pub use effects::{apply_box_blur, EffectScratch};
pub use focus::FocusManager;
pub use framebuffer::{Framebuffer, FramebufferError};
pub use geometry::{Point, Rect, Vector};
#[cfg(feature = "image-decoding")]
pub use image::{
    decode_texture, decode_texture_with_options, ImageDecodeError, ImageDecodeOptions,
};
pub use interaction::{InteractionEvents, UiInteraction};
pub use layout::{Insets, LayoutDirection, LayoutItem, ScrollViewport, StackLayout};
pub use mask::apply_alpha_mask_clipped;
pub use mipmap::{draw_mipmapped_texture_clipped, MipmapChain};
pub use path::{
    fill_path_even_odd, fill_path_even_odd_aa, fill_path_even_odd_clipped,
    fill_transformed_path_even_odd, fill_transformed_path_even_odd_aa,
    fill_transformed_path_even_odd_aa_clipped, fill_transformed_path_even_odd_clipped,
    FlattenedPath, FlattenedSubpath, Path, PathError,
};
pub use path_cache::PathGeometryCache;
pub use rasterizer::{
    blend_rect, blend_rect_clipped, blend_rect_parallel, blend_rect_parallel_with_threads,
    draw_arc_aa, draw_circle_aa, draw_ellipse_aa, draw_polygon_aa, draw_rect_aa, draw_rounded_rect,
    draw_rounded_rect_aa, draw_triangle_aa, fill_arc_aa, fill_circle_aa, fill_polygon_aa,
    fill_polygon_aa_with_scratch, fill_polygon_with_scratch, fill_rect_clipped,
    fill_rect_linear_gradient_parallel, fill_rect_linear_gradient_parallel_with_threads,
    fill_rect_parallel_with_threads, fill_rect_radial_gradient_parallel,
    fill_rect_radial_gradient_parallel_with_threads, fill_rects_clipped, fill_rounded_rect,
    fill_rounded_rect_aa, fill_transformed_rect_linear_gradient_clipped,
    fill_transformed_rect_radial_gradient_clipped, fill_triangle_aa, Gradient, GradientError,
    GradientStop, RasterizerScratch,
};
pub use shaping::{
    resolve_bidi_runs, BidiRun, OpenTypeShaper, ShapedGlyph, ShapedText, ShapingError,
    TextDirection,
};
pub use stroke::{
    stroke_path, stroke_path_aa, stroke_path_aa_clipped, stroke_path_aa_parallel_with_threads,
    stroke_path_clipped, StrokeCap, StrokeError, StrokeJoin, StrokeStyle,
};
pub use text::{
    draw_text, draw_text_bidi_clipped, draw_text_box_clipped, draw_text_box_transformed_clipped,
    draw_text_box_with_fallback_bidi_shaped_clipped, draw_text_box_with_fallback_shaped_clipped,
    draw_text_clipped, draw_text_shaped_clipped, draw_text_shaped_transformed_clipped,
    draw_text_transformed_clipped, draw_text_with_fallback_bidi_clipped,
    draw_text_with_fallback_clipped, draw_text_with_fallback_shaped_clipped, measure_with_fallback,
    FontFace, FontFamily, TextAlign, TextLayoutOptions, TextMetrics,
};
pub use texture::{
    draw_texture, draw_texture_auto, draw_texture_clipped, draw_texture_clipped_auto,
    draw_texture_clipped_parallel_with_threads, draw_texture_clipped_with_scratch,
    draw_texture_region_clipped, draw_texture_transformed_clipped,
    draw_texture_transformed_clipped_parallel_with_threads, Texture, TextureDrawScratch,
    TextureError, TextureFilter,
};
pub use texture_atlas::{AtlasRegion, TextureAtlasBuilder, TextureAtlasError};
pub use transform::Transform2D;
pub use ui::{
    draw_button, draw_checkbox, draw_combo_box, draw_divider, draw_label, draw_label_clipped,
    draw_label_with_fallback, draw_label_wrapped, draw_list, draw_panel, draw_progress_bar,
    draw_radio_button, draw_slider, draw_switch, draw_tabs, draw_text_input, state_for_pointer,
    ComboBoxState, ListOptions, ListState, TabsState, TextInputOptions, TextInputState, UiTheme,
    WidgetState,
};
