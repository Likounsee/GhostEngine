#![forbid(unsafe_code)]

//! GhostEngine's backend-independent foundations and CPU rendering primitives.
//! The crate intentionally has no graphics API or platform dependencies.

pub mod color;
pub mod components;
pub mod composition;
pub mod display_list;
pub mod focus;
pub mod framebuffer;
pub mod geometry;
pub mod interaction;
pub mod layout;
pub mod path;
pub mod rasterizer;
pub mod shaping;
pub mod stroke;
pub mod text;
pub mod texture;
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

pub use color::Color;
pub use components::{
    ClipboardAction, ComponentEvents, ComponentId, ComponentKey, ComponentKind, ComponentTree,
    ComponentTreeError, ContextMenuItem, ContextMenuShortcut, PlacedComponent, TextInputEvent,
    UiComponent,
};
pub use composition::{compose_layers, Layer};
pub use display_list::{DisplayList, RenderStats};
pub use focus::FocusManager;
pub use framebuffer::{Framebuffer, FramebufferError};
pub use geometry::{Point, Rect, Vector};
pub use interaction::{InteractionEvents, UiInteraction};
pub use layout::{Insets, LayoutDirection, LayoutItem, ScrollViewport, StackLayout};
pub use path::{
    fill_path_even_odd, fill_path_even_odd_aa, fill_path_even_odd_clipped,
    fill_transformed_path_even_odd, fill_transformed_path_even_odd_aa,
    fill_transformed_path_even_odd_aa_clipped, fill_transformed_path_even_odd_clipped,
    FlattenedPath, FlattenedSubpath, Path, PathError,
};
pub use rasterizer::{
    blend_rect, blend_rect_clipped, fill_circle_aa, fill_ellipse_aa, fill_polygon_aa,
    fill_rect_clipped, fill_transformed_rect_linear_gradient_clipped,
    fill_transformed_rect_radial_gradient_clipped, Gradient, GradientError, GradientStop,
};
pub use shaping::{
    resolve_bidi_runs, BidiRun, OpenTypeShaper, ShapedGlyph, ShapedText, ShapingError,
    TextDirection,
};
pub use stroke::{
    stroke_path, stroke_path_aa, stroke_path_aa_clipped, stroke_path_clipped, StrokeCap,
    StrokeError, StrokeJoin, StrokeStyle,
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
    draw_texture, draw_texture_clipped, draw_texture_transformed_clipped, Texture, TextureError,
    TextureFilter,
};
pub use transform::Transform2D;
pub use ui::{
    draw_button, draw_checkbox, draw_combo_box, draw_divider, draw_label, draw_label_clipped,
    draw_label_with_fallback, draw_label_wrapped, draw_list, draw_panel, draw_progress_bar,
    draw_radio_button, draw_slider, draw_switch, draw_tabs, draw_text_input, state_for_pointer,
    ComboBoxState, ListOptions, ListState, TabsState, TextInputOptions, TextInputState, UiTheme,
    WidgetState,
};
