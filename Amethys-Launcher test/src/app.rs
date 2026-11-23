//! Interactive Amethyst Launcher test app built on GhostEngine's own UI pieces:
//! `ScrollViewport` (scrolling), `UiInteraction` (hover / press / click dispatch),
//! `ComponentTree` + `UiComponent::text_input` (real text field, IME and clipboard)
//! and the native window surface. Pages are pre-rendered with `pages::*` and
//! blitted every frame; hover and the scrollbar are drawn on top.

use crate::covers::CoverStore;
use crate::gfx::{self, Action, Fonts, Hit, Page, Painter, State, BG, W};
use crate::pages::{self, Active, PageFn, MW, MX, SIDE_W};
use std::{collections::VecDeque, time::Instant};
use ghostengine::{
    InputState, draw_rounded_rect_aa, fill_rounded_rect_aa, ClipboardAction, Color, ComponentId, ComponentTree,
    Framebuffer, Insets, LayoutDirection, LayoutItem, Rect, ScrollViewport, StackLayout,
    UiComponent, UiInteraction, UiTheme,
};

const PANEL_W: i32 = 520;
const PANEL_H: i32 = 520;
/// Tallest page the pre-render buffer can hold.
const MAX_PAGE_H: u32 = 3200;

#[derive(Clone, Copy)]
struct Bench {
    page_avg: f64,
    page_min: f64,
    page_max: f64,
    page_n: u32,
    frame_avg: f64,
    frame_n: u32,
}

pub const WIN_W: u32 = 1600;
pub const WIN_H: u32 = 900;

pub struct Output {
    pub keep_running: bool,
    /// Set when the focused text field asks the host for the system clipboard.
    pub clipboard: Option<ClipboardAction>,
}

pub struct App {
    fonts: &'static Fonts,
    covers: &'static CoverStore,
    pub page: Page,
    pub st: State,
    /// Whole page pre-rendered once (sidebar occupies the first WIN_H rows).
    content: Framebuffer,
    hits: Vec<Hit>,
    pub scroll: ScrollViewport,
    inter: UiInteraction,
    trees: [ComponentTree; 2],
    search_rect: Option<(usize, Rect)>,
    theme: UiTheme,
    pub stats_open: bool,
    frame_ms: VecDeque<f64>,
    work_ms: f64,
    page_ms: f64,
    last_frame: Option<Instant>,
    bench: Option<Bench>,
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
    pub fn new(fonts: &'static Fonts, covers: &'static CoverStore) -> Self {
        let blank = || Framebuffer::new(WIN_W, WIN_H, BG).expect("framebuffer");
        let mut app = Self {
            fonts,
            covers,
            page: Page::Store,
            st: State::default(),
            content: blank(),
            hits: Vec::new(),
            scroll: ScrollViewport::new(Rect::new(SIDE_W as u32, 0, WIN_W - SIDE_W as u32, WIN_H), LayoutDirection::Vertical, WIN_H),
            inter: UiInteraction::new(),
            trees: [search_tree("Rechercher un jeu, un studio, un genre"), search_tree("Filtrer ma bibliothèque")],
            search_rect: None,
            theme: dark_theme(),
            stats_open: false,
            frame_ms: VecDeque::new(),
            work_ms: 0.0,
            page_ms: 0.0,
            last_frame: None,
            bench: None,
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
        let t0 = Instant::now();
        let vh = WIN_H as i32;
        let mut p = Painter::new(WIN_W, MAX_PAGE_H, self.fonts).with_covers(self.covers);
        pages::sidebar(&mut p, active, vh);
        let h = page(&mut p, vh, &self.st).clamp(vh, MAX_PAGE_H as i32);
        self.hits = std::mem::take(&mut p.hits);
        self.content = p.fb;
        let old = self.scroll.offset();
        self.scroll.set_content_extent(h as u32);
        self.scroll.set_offset(if reset_scroll { 0 } else { old });
        self.inter.clear();
        self.page_ms = t0.elapsed().as_secs_f64() * 1000.0;
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
        let (logical_w, target_w) = (WIN_W as usize, fb.width() as usize);
        if target_w < logical_w {
            return;
        }

        let rows = (fb.height() as usize).min(WIN_H as usize);
        let offset = self.scroll.offset() as usize;
        let sidebar_w = SIDE_W as usize;
        let source = self.content.pixels();
        let target = fb.pixels_mut();

        // Borrow the destination pixel buffer only once per frame. The previous
        // implementation repeatedly requested mutable access inside the row loop.
        for y in 0..rows {
            let dst_start = y * target_w;
            let src_start = y * logical_w;
            target[dst_start..dst_start + sidebar_w]
                .copy_from_slice(&source[src_start..src_start + sidebar_w]);

            let content_src_start = (y + offset) * logical_w + sidebar_w;
            let content_dst_start = dst_start + sidebar_w;
            target[content_dst_start..dst_start + logical_w]
                .copy_from_slice(&source[content_src_start..(y + offset + 1) * logical_w]);
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

    /// Pages, hover, scrollbar and search field. `mask` hides the pointer from the page
    /// (used while the pointer is over the stats overlay).
    fn draw_scene(&mut self, fb: &mut Framebuffer, real: &InputState, mask: bool) -> Option<ClipboardAction> {
        let blank = InputState::default();
        let input = if mask { &blank } else { real };
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
        clipboard
    }


    // ------------------------------------------------------------ stats overlay
    fn chip_rect(&self) -> Rect {
        Rect::new(WIN_W - 12 - 250, WIN_H - 12 - 34, 250, 34)
    }

    fn panel_rect(&self) -> Rect {
        Rect::new(WIN_W - 12 - PANEL_W as u32, WIN_H - 12 - 34 - 8 - PANEL_H as u32, PANEL_W as u32, PANEL_H as u32)
    }

    fn bench_button(&self) -> Rect {
        let p = self.panel_rect();
        Rect::new(p.x + 20, p.y + PANEL_H as u32 - 20 - 44 - 44, 200, 44)
    }

    fn avg_frame_ms(&self) -> f64 {
        if self.frame_ms.is_empty() {
            0.0
        } else {
            self.frame_ms.iter().sum::<f64>() / self.frame_ms.len() as f64
        }
    }

    /// Measures page rendering and full frames on this machine (blocks for ~1-3 s).
    fn run_bench(&mut self, fb: &mut Framebuffer) {
        let n = 10;
        let mut times = Vec::new();
        for _ in 0..n {
            let t = Instant::now();
            self.render(false);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let idle = InputState::default();
        let frames = 200;
        let t = Instant::now();
        for _ in 0..frames {
            self.draw_scene(fb, &idle, false);
        }
        let frame_avg = t.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        self.bench = Some(Bench {
            page_avg: times.iter().sum::<f64>() / n as f64,
            page_min: times.iter().cloned().fold(f64::MAX, f64::min),
            page_max: times.iter().cloned().fold(0.0, f64::max),
            page_n: n,
            frame_avg,
            frame_n: frames,
        });
    }

    fn draw_overlay(&mut self, fb: &mut Framebuffer) {
        let avg = self.avg_frame_ms();
        let fps = if avg > 0.0 { 1000.0 / avg } else { 0.0 };
        let mut c = Painter::new(250, 34, self.fonts);
        c.rrect(0, 0, 250, 34, 17, gfx::ACT);
        c.rrect_line(0, 0, 250, 34, 17, gfx::VIO);
        c.circle(18, 17, 5, gfx::GREEN);
        c.text(32, 0, 34, &format!("GhostEngine · {:.1} ms · {:.0} img/s", self.work_ms, fps), 13.0, W::Semi, gfx::ACT_TXT);
        self.copy_to(fb, &c.fb, self.chip_rect(), 17);
        if !self.stats_open {
            return;
        }
        let mut p = Painter::new(PANEL_W as u32, PANEL_H as u32, self.fonts);
        p.fill(0, 0, PANEL_W, PANEL_H, BG);
        p.rrect(0, 0, PANEL_W, PANEL_H, 18, gfx::PANEL);
        p.rrect_line(0, 0, PANEL_W, PANEL_H, 18, gfx::VIO);
        p.text(20, 16, 28, "GhostEngine — mesures en direct", 17.0, W::Disp, gfx::TXT);
        let stats = self.covers.stats();
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let rows = [
            ("Cadence (moy. 60 frames)", format!("{avg:.2} ms · {fps:.0} img/s")),
            ("Travail de la dernière frame", format!("{:.2} ms", self.work_ms)),
            ("Dernier rendu de page", format!("{:.0} ms", self.page_ms)),
            ("Framebuffer", format!("{WIN_W}×{WIN_H} RGBA · {:.1} Mo", (WIN_W * WIN_H * 4) as f64 / 1_048_576.0)),
            ("Images décodées", format!("{} fichiers · {:.0} ms", self.covers.files, self.covers.decode_ms)),
            ("Cache de textures", format!("{} entrées · {:.1} Mo", stats.entries, stats.used_bytes as f64 / 1_048_576.0)),
            ("Threads CPU disponibles", format!("{threads}")),
        ];
        let mut y = 56;
        for (k, v) in rows.iter() {
            p.fill(20, y, PANEL_W - 40, 1, gfx::LINE);
            p.text(20, y + 1, 26, k, 13.0, W::Reg, gfx::MUT);
            p.text_right(PANEL_W - 20, y + 1, 26, v, 13.0, W::Med, gfx::TXT);
            y += 28;
        }
        y += 10;
        p.text(20, y, 18, "FONCTIONS DU MOTEUR UTILISÉES ICI", 11.0, W::Semi, gfx::MUT);
        y += 22;
        for line in [
            "Rectangles arrondis anti-aliasés, cercles, polygones",
            "Dégradés linéaires et fonds à alpha",
            "Décodage PNG/JPEG + TextureCache + mipmaps",
            "Texte OpenType (shaping) en 5 graisses",
            "ScrollViewport, UiInteraction, UiComponent::text_input",
        ] {
            p.text(20, y, 18, line, 12.0, W::Reg, gfx::SOFT);
            y += 19;
        }
        let b = self.bench_button();
        let by = (b.y - self.panel_rect().y) as i32;
        p.rrect(20, by, 200, 44, 12, gfx::VIO);
        let label = "Lancer le benchmark";
        let tw = p.width(label, 14.0, W::Semi).round() as i32;
        p.text(20 + (200 - tw) / 2, by, 44, label, 14.0, W::Semi, gfx::VIO_DARK);
        let (l1, l2) = match self.bench {
            Some(b) => (
                format!("Rendu de page : {:.0} ms (min {:.0}, max {:.0}, {} mesures)", b.page_avg, b.page_min, b.page_max, b.page_n),
                format!("Frame complète : {:.2} ms soit {:.0} img/s ({} frames)", b.frame_avg, 1000.0 / b.frame_avg, b.frame_n),
            ),
            None => ("Mesure le rendu sur cette machine (≈ 1 à 3 s).".to_string(), String::new()),
        };
        p.text(20, by + 52, 18, &l1, 12.0, W::Reg, gfx::SOFT);
        p.text(20, by + 70, 18, &l2, 12.0, W::Reg, gfx::SOFT);
        self.copy_to(fb, &p.fb, self.panel_rect(), 18);
    }

    /// Copies `src` onto the framebuffer with rounded corners (pixels outside the arc are skipped).
    fn copy_to(&self, fb: &mut Framebuffer, src: &Framebuffer, at: Rect, radius: usize) {
        let (sw, sh, fw) = (src.width() as usize, src.height() as usize, fb.width() as usize);
        for row in 0..sh {
            let (dy, dx) = (at.y as usize + row, at.x as usize);
            if dy >= fb.height() as usize || dx + sw > fw {
                break;
            }
            let dist = if row < radius { radius - row } else if row + radius >= sh { row + radius + 1 - sh } else { 0 };
            let inset = if dist == 0 { 0 } else { radius - ((radius * radius - dist * dist.min(radius)) as f32).sqrt() as usize };
            let (l, r) = (inset.min(sw / 2), sw - inset.min(sw / 2));
            fb.pixels_mut()[dy * fw + dx + l..dy * fw + dx + r].copy_from_slice(&src.pixels()[row * sw + l..row * sw + r]);
        }
    }

    /// Runs one frame: input -> state -> pixels. Returns whether to keep running.
    pub fn frame(&mut self, fb: &mut Framebuffer, input: &InputState) -> Output {
        let start = Instant::now();
        if let Some(last) = self.last_frame {
            self.frame_ms.push_back(start.duration_since(last).as_secs_f64() * 1000.0);
            if self.frame_ms.len() > 60 {
                self.frame_ms.pop_front();
            }
        }
        self.last_frame = Some(start);
        if input.escape_down {
            return Output { keep_running: false, clipboard: None };
        }
        let ptr = input.mouse_position;
        let over_chip = ptr.is_some_and(|p| self.chip_rect().contains(p));
        let over_panel = self.stats_open && ptr.is_some_and(|p| self.panel_rect().contains(p));
        let mut bench_now = false;
        if input.left_mouse_pressed {
            if over_chip {
                self.stats_open = !self.stats_open;
            } else if over_panel && ptr.is_some_and(|p| self.bench_button().contains(p)) {
                bench_now = true;
            }
        }
        let clipboard = self.draw_scene(fb, input, over_chip || over_panel);
        if bench_now {
            self.run_bench(fb);
            self.draw_scene(fb, &InputState::default(), false);
        }
        self.draw_overlay(fb);
        if !bench_now {
            self.work_ms = start.elapsed().as_secs_f64() * 1000.0;
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
