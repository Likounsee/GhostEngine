//! Interactive Amethyst Launcher test app built on GhostEngine's own UI pieces:
//! `ScrollViewport` (scrolling), `UiInteraction` (hover / press / click dispatch),
//! `ComponentTree` + `UiComponent::text_input` (real text field, IME and clipboard)
//! and the native window surface. Pages are pre-rendered with `pages::*` and
//! blitted every frame; hover and the scrollbar are drawn on top.

use crate::gfx::{Action, Fonts, Hit, Page, Painter, State, BG};
use crate::pages::{self, Active, PageFn, MW, MX, SIDE_W};
use ghostengine::window::InputState;
use ghostengine::{
    draw_rounded_rect_aa, fill_rounded_rect_aa, ClipboardAction, Color, ComponentId, ComponentTree,
    Framebuffer, Insets, LayoutDirection, LayoutItem, Rect, ScrollViewport, StackLayout,
    UiComponent, UiInteraction, UiTheme,
};

pub const WIN_W: u32 = 1600;
pub const WIN_H: u32 = 900;

pub struct Output {
    pub keep_running: bool,
    /// Set when the focused text field asks the host for the system clipboard.
    pub clipboard: Option<ClipboardAction>,
}

pub struct App {
    fonts: &'static Fonts,
    pub page: Page,
    pub st: State,
    side: Framebuffer,
    content: Framebuffer,
    hits: Vec<Hit>,
    pub scroll: ScrollViewport,
    inter: UiInteraction,
    trees: [ComponentTree; 2],
    search_rect: Option<(usize, Rect)>,
    theme: UiTheme,
}

fn search_tree(placeholder: &str) -> ComponentTree {
    let field = UiComponent::text_input(ComponentId(1), placeholder, "").with_layout_item(LayoutItem::flexible());
    let panel = UiComponent::panel(
        ComponentId(0),
        StackLayout::new(LayoutDirection::Vertical).with_padding(Insets::all(0)).with_gap(0),
    )
    .with_layout_item(LayoutItem::flexible())
    .with_children(vec![field]);
    ComponentTree::new(vec![panel])
}

fn dark_theme() -> UiTheme {
    use crate::gfx::*;
    UiTheme {
        panel: PANEL,
        panel_border: LINE,
        text: TXT,
        muted_text: MUT,
        accent: VIO,
        accent_hover: VIO,
        accent_pressed: ACT_TXT,
        disabled: MUT,
        control: PANEL,
        control_border: LINE,
        track: ACT,
    }
}

impl App {
    pub fn new(fonts: &'static Fonts) -> Self {
        let blank = || Framebuffer::new(WIN_W, WIN_H, BG).expect("framebuffer");
        let mut app = Self {
            fonts,
            page: Page::Store,
            st: State::default(),
            side: blank(),
            content: blank(),
            hits: Vec::new(),
            scroll: ScrollViewport::new(Rect::new(SIDE_W as u32, 0, WIN_W - SIDE_W as u32, WIN_H), LayoutDirection::Vertical, WIN_H),
            inter: UiInteraction::new(),
            trees: [search_tree("Rechercher un jeu, un studio, un genre"), search_tree("Filtrer ma bibliothèque")],
            search_rect: None,
            theme: dark_theme(),
        };
        app.render(true);
        app
    }

    /// Re-draws the current page (sidebar, content, hit regions).
    fn render(&mut self, reset_scroll: bool) {
        let (active, page): (Active, PageFn) = match self.page {
            Page::Store => (Active::Store, pages::store),
            Page::Library => (Active::Library, pages::library),
            Page::Product => (Active::Store, pages::product),
            Page::Settings => (Active::Settings, pages::settings),
        };
        let vh = WIN_H as i32;
        let mut p = Painter::new(WIN_W, WIN_H, self.fonts);
        pages::sidebar(&mut p, active, vh);
        let h = page(&mut p, vh, &self.st).max(vh);
        self.hits = std::mem::take(&mut p.hits);
        let side = p.fb;
        if h > vh {
            let mut q = Painter::new(WIN_W, h as u32, self.fonts);
            pages::sidebar(&mut q, active, h);
            page(&mut q, h, &self.st);
            self.content = q.fb;
        } else {
            let mut c = Framebuffer::new(WIN_W, WIN_H, BG).expect("framebuffer");
            c.copy_pixels_from(side.pixels()).expect("same size");
            self.content = c;
        }
        self.side = side;
        let old = self.scroll.offset();
        self.scroll.set_content_extent(h as u32);
        self.scroll.set_offset(if reset_scroll { 0 } else { old });
        self.inter.clear();
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Go(page) => {
                self.page = page;
                self.render(true);
            }
            Action::StoreFilter(i) => {
                self.st.store_filter = i;
                self.render(false);
            }
            Action::LibFilter(i) => {
                self.st.lib_filter = i;
                self.render(false);
            }
        }
    }

    fn blit(&self, fb: &mut Framebuffer) {
        let (w, fbw) = (WIN_W as usize, fb.width() as usize);
        if fbw < w {
            return;
        }
        let rows = (fb.height() as usize).min(WIN_H as usize);
        let off = self.scroll.offset() as usize;
        let side = SIDE_W as usize;
        for y in 0..rows {
            let dst = fb.pixels_mut();
            dst[y * fbw..y * fbw + side].copy_from_slice(&self.side.pixels()[y * w..y * w + side]);
            dst[y * fbw + side..y * fbw + w].copy_from_slice(&self.content.pixels()[(y + off) * w + side..(y + off) * w + w]);
        }
    }

    /// Screen-space, viewport-clipped bounds of every hit region.
    fn screen_bounds(&self) -> (Vec<Rect>, Vec<bool>) {
        let off = self.scroll.offset() as i32;
        let (mut bounds, mut enabled) = (Vec::new(), Vec::new());
        for h in &self.hits {
            let (x0, y0) = (h.rect.x as i32, h.rect.y as i32 - if h.fixed { 0 } else { off });
            let (x1, y1) = (x0 + h.rect.width as i32, y0 + h.rect.height as i32);
            let (cx0, cx1) = if h.fixed { (0, WIN_W as i32) } else { (SIDE_W, WIN_W as i32) };
            let (a, b, c, d) = (x0.max(cx0), y0.max(0), x1.min(cx1), y1.min(WIN_H as i32));
            if c > a && d > b {
                bounds.push(Rect::new(a as u32, b as u32, (c - a) as u32, (d - b) as u32));
                enabled.push(true);
            } else {
                bounds.push(Rect::new(0, 0, 0, 0));
                enabled.push(false);
            }
        }
        (bounds, enabled)
    }

    /// Runs one frame: input -> state -> pixels. Returns whether to keep running.
    pub fn frame(&mut self, fb: &mut Framebuffer, input: &InputState) -> Output {
        if input.escape_down {
            return Output { keep_running: false, clipboard: None };
        }
        let main = Rect::new(SIDE_W as u32, 0, WIN_W - SIDE_W as u32, WIN_H);
        let pointer = input.mouse_position;
        let in_main = pointer.is_some_and(|p| main.contains(p));
        if input.scroll_delta_y != 0 && in_main {
            self.scroll.scroll_by(input.scroll_delta_y);
        }

        // hover / press / click through GhostEngine's interaction model
        let (bounds, enabled) = self.screen_bounds();
        let events = self.inter.update(pointer, input.left_mouse_pressed, input.left_mouse_released, &enabled, &bounds);
        let hover = self.inter.hovered().map(|i| (bounds[i], self.hits[i].radius, self.inter.is_pressed(i)));
        let clicked = events.clicked.map(|i| self.hits[i].action);
        if let Some(action) = clicked {
            self.apply(action);
        }

        self.blit(fb);

        if clicked.is_none() {
            if let Some((r, radius, pressed)) = hover {
                fill_rounded_rect_aa(fb, r, radius, Color::rgba(185, 162, 255, if pressed { 56 } else { 26 }));
                draw_rounded_rect_aa(fb, r, radius, Color::rgba(185, 162, 255, 190));
            }
        }

        // overlay scrollbar, visible only while the pointer is over the content
        if self.scroll.is_scrollable() && in_main {
            let total = (self.scroll.max_offset() + WIN_H) as f32;
            let thumb = ((WIN_H as f32 * WIN_H as f32 / total) as u32).max(48);
            let travel = WIN_H - thumb - 8;
            let y = 4 + (self.scroll.offset() as f32 / self.scroll.max_offset() as f32 * travel as f32) as u32;
            fill_rounded_rect_aa(fb, Rect::new(WIN_W - 10, y, 6, thumb), 3, Color::rgba(185, 162, 255, 120));
        }

        // real engine text field for the search bar (scrolls away with the page)
        let mut clipboard = None;
        self.search_rect = None;
        let slot = match self.page {
            Page::Store => Some((0, MX)),
            Page::Library => Some((1, MX + MW - 520)),
            _ => None,
        };
        if let Some((idx, x)) = slot {
            let off = self.scroll.offset();
            if off <= 24 {
                let b = Rect::new(x as u32, 24 - off, 520, 44);
                let ev = self.trees[idx].handle_window_input(b, input);
                self.trees[idx].render(fb, b, &self.fonts.reg, 14.0, &self.theme);
                clipboard = ev.clipboard_action;
                self.search_rect = Some((idx, b));
            }
        }
        Output { keep_running: true, clipboard }
    }

    /// Delivers system clipboard text to the focused search field.
    pub fn paste(&mut self, text: String) {
        if let Some((idx, b)) = self.search_rect {
            let input = InputState { paste_pressed: true, clipboard_text: Some(text), ..Default::default() };
            self.trees[idx].handle_window_input(b, &input);
        }
    }
}
