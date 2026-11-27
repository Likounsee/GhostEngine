//! The four Amethyst Launcher pages, drawn with GhostEngine primitives only.
//! Coordinates are absolute pixels of a 1600x900 (16:9) launcher window.

use crate::data::*;
use crate::gfx::*;

pub const SIDE_W: i32 = 232;
pub const MX: i32 = SIDE_W + 40; // main column x
pub const MW: i32 = 1600 - MX - 40; // main column width (1288)
const COL: i32 = 196; // 6-column card grid

pub type PageFn = fn(&mut Painter, i32, &State) -> i32;

#[derive(Clone, Copy, PartialEq)]
pub enum Active {
    Store,
    Library,
    Settings,
}

// ---------------------------------------------------------------- icons
#[derive(Clone, Copy)]
enum Icon {
    Gem,
    Books,
    Heart,
    Download,
    Gear,
    Search,
}

fn icon(p: &mut Painter, ic: Icon, x: i32, y: i32, c: ghostengine::Color, bg: ghostengine::Color) {
    let (xf, yf) = (x as f32, y as f32);
    match ic {
        Icon::Gem => p.poly(&[(xf + 5.0, yf + 2.5), (xf + 15.0, yf + 2.5), (xf + 17.5, yf + 7.5), (xf + 10.0, yf + 17.5), (xf + 2.5, yf + 7.5)], c),
        Icon::Books => {
            p.rrect(x + 2, y + 3, 4, 14, 1, c);
            p.rrect(x + 8, y + 3, 4, 14, 1, c);
            p.poly(&[(xf + 14.0, yf + 4.0), (xf + 18.0, yf + 5.0), (xf + 15.5, yf + 17.5), (xf + 11.5, yf + 16.5)], c);
        }
        Icon::Heart => {
            p.circle(x + 6, y + 7, 4, c);
            p.circle(x + 14, y + 7, 4, c);
            p.poly(&[(xf + 2.2, yf + 8.5), (xf + 17.8, yf + 8.5), (xf + 10.0, yf + 17.5)], c);
        }
        Icon::Download => {
            p.rrect(x + 9, y + 2, 2, 9, 0, c);
            p.poly(&[(xf + 4.0, yf + 9.0), (xf + 16.0, yf + 9.0), (xf + 10.0, yf + 15.0)], c);
            p.rrect(x + 3, y + 17, 14, 2, 1, c);
        }
        Icon::Gear => {
            p.rrect(x + 8, y, 4, 3, 1, c);
            p.rrect(x + 8, y + 17, 4, 3, 1, c);
            p.rrect(x, y + 8, 3, 4, 1, c);
            p.rrect(x + 17, y + 8, 3, 4, 1, c);
            p.circle(x + 10, y + 10, 7, c);
            p.circle(x + 10, y + 10, 3, bg);
        }
        Icon::Search => {
            p.circle(x + 8, y + 8, 7, c);
            p.circle(x + 8, y + 8, 5, bg);
            p.poly(&[(xf + 12.5, yf + 14.0), (xf + 14.0, yf + 12.5), (xf + 19.0, yf + 17.5), (xf + 17.5, yf + 19.0)], c);
        }
    }
}

// ------------------------------------------------------------ components
fn nav_item(p: &mut Painter, y: i32, label: &str, ic: Icon, active: bool, badge: Option<&str>, go: Option<Page>) {
    let x = 16;
    if let Some(pg) = go {
        p.hit(x, y, 200, 44, 10, Action::Go(pg), true);
    }
    let (fg, bg) = if active { (ACT_TXT, ACT) } else { (MUT, SIDE) };
    if active {
        p.rrect(x, y, 200, 44, 10, ACT);
    }
    icon(p, ic, x + 12, y + 12, fg, bg);
    p.text(x + 44, y, 44, label, 14.0, W::Med, fg);
    if let Some(b) = badge {
        let tw = p.width(b, 11.0, W::Bold).round() as i32;
        let bw = tw + 16;
        p.rrect(x + 200 - 12 - bw, y + 12, bw, 20, 10, ACT);
        p.text(x + 200 - 12 - bw + 8, y + 12, 20, b, 11.0, W::Bold, ACT_TXT);
    }
}

fn gradient_cover(p: &mut Painter, x: i32, y: i32, w: i32, h: i32, r: i32, title: &str, angle: f32, bg: ghostengine::Color) {
    if p.draw_cover(title, x, y, w, h) {
        if h >= 100 {
            p.scrim(x, y, w, h);
        }
    } else {
        let (a, b) = cover(title);
        p.gradient(x, y, w, h, angle, &[(0.0, hex(a)), (1.0, hex(b))]);
    }
    p.round_corners(x, y, w, h, r, bg);
    p.rrect_line(x, y, w, h, r, ghostengine::Color::rgba(255, 255, 255, 22));
}

pub fn sidebar(p: &mut Painter, active: Active, vh: i32) {
    p.fill(0, 0, SIDE_W, vh, SIDE);
    p.fill(SIDE_W - 1, 0, 1, vh, LINE);
    // logo
    let (lx, ly) = (24.0_f32, 24.0_f32);
    p.poly(&[(lx + 9.0, ly + 2.0), (lx + 19.0, ly + 10.0), (lx + 9.0, ly + 26.0), (lx - 1.0, ly + 10.0)], VIO);
    p.poly(&[(lx + 9.0, ly + 2.0), (lx + 13.0, ly + 10.0), (lx + 5.0, ly + 10.0)], hex("#E4D9FF"));
    p.text(62, 24, 28, "Amethyst", 20.0, W::Disp, TXT);
    // main navigation
    nav_item(p, 72, "Boutique", Icon::Gem, active == Active::Store, None, Some(Page::Store));
    nav_item(p, 118, "Bibliothèque", Icon::Books, active == Active::Library, None, Some(Page::Library));
    nav_item(p, 164, "Liste d'envies", Icon::Heart, false, Some("8"), None);
    nav_item(p, 210, "Téléchargements", Icon::Download, false, Some("2"), None);
    // recently played
    p.text(28, 274, 16, "RÉCEMMENT JOUÉS", 11.0, W::Semi, MUT);
    let recent = [("Cyberpunk 2077", "Continuer · 42 h"), ("The Witcher 3", "120 h"), ("Elden Ring", "Mise à jour disponible")];
    for (i, (title, sub)) in recent.iter().enumerate() {
        let y = 298 + 48 * i as i32;
        gradient_cover(p, 28, y + 4, 32, 40, 6, title, 160.0, SIDE);
        p.text(70, y + 6, 18, title, 13.0, W::Med, TXT);
        p.text(70, y + 24, 16, sub, 12.0, W::Reg, MUT);
    }
    // bottom: settings + profile
    nav_item(p, vh - 132, "Paramètres", Icon::Gear, active == Active::Settings, None, Some(Page::Settings));
    p.fill(16, vh - 84, 200, 1, LINE);
    p.circle(46, vh - 54, 18, VIO);
    let jw = p.width("J", 15.0, W::Bold);
    p.text(46 - (jw / 2.0).round() as i32, vh - 72, 36, "J", 15.0, W::Bold, VIO_DARK);
    p.text(70, vh - 72, 18, "Joueur", 13.0, W::Semi, TXT);
    p.text(70, vh - 54, 16, "Synchronisé", 12.0, W::Reg, MUT);
}

fn search_bar(p: &mut Painter, y: i32, placeholder: &str, right_note: Option<&str>) {
    p.rrect(MX, y, 520, 44, 12, PANEL);
    p.rrect_line(MX, y, 520, 44, 12, LINE);
    icon(p, Icon::Search, MX + 14, y + 13, MUT, PANEL);
    p.text(MX + 42, y, 44, placeholder, 14.0, W::Reg, MUT);
    if let Some(n) = right_note {
        p.text_right(MX + MW, y, 44, n, 13.0, W::Reg, MUT);
    }
}

/// Returns the pill's width.
fn pill(p: &mut Painter, x: i32, y: i32, label: &str, on: bool, act: Option<Action>) -> i32 {
    let w = p.width(label, 13.0, if on { W::Semi } else { W::Med }).round() as i32 + 32;
    if let Some(a) = act {
        p.hit(x, y, w, 36, 18, a, false);
    }
    if on {
        p.rrect(x, y, w, 36, 18, VIO);
        p.text(x + 16, y, 36, label, 13.0, W::Semi, VIO_DARK);
    } else {
        p.rrect(x, y, w, 36, 18, PANEL);
        p.rrect_line(x, y, w, 36, 18, LINE);
        p.text(x + 16, y, 36, label, 13.0, W::Med, SOFT);
    }
    w
}

fn pills_width(p: &mut Painter, label: &str, items: &[(&str, bool)]) -> i32 {
    let mut w = p.width(label, 13.0, W::Reg).round() as i32 + 14;
    for (i, (s, on)) in items.iter().enumerate() {
        w += p.width(s, 13.0, if *on { W::Semi } else { W::Med }).round() as i32 + 32;
        if i + 1 < items.len() {
            w += 10;
        }
    }
    w
}

fn pills_row(p: &mut Painter, x: i32, y: i32, label: &str, items: &[(&str, bool)], act: Option<fn(usize) -> Action>) {
    let lw = p.text(x, y, 36, label, 13.0, W::Reg, MUT).round() as i32;
    let mut cx = x + lw + 14;
    for (i, (s, on)) in items.iter().enumerate() {
        cx += pill(p, cx, y, s, *on, act.map(|f| f(i))) + 10;
    }
}

fn button(p: &mut Painter, x: i32, y: i32, w: Option<i32>, label: &str, primary: bool) -> i32 {
    let tw = p.width(label, 14.0, W::Semi).round() as i32;
    let w = w.unwrap_or(tw + 40);
    if primary {
        p.rrect(x, y, w, 44, 12, VIO);
    } else {
        p.rrect(x, y, w, 44, 12, ghostengine::Color::rgba(14, 12, 19, 140));
        p.rrect_line(x, y, w, 44, 12, BTN_LINE);
    }
    let c = if primary { VIO_DARK } else { TXT };
    p.text(x + (w - tw) / 2, y, 44, label, 14.0, W::Semi, c);
    w
}

fn section_header(p: &mut Painter, x: i32, w: i32, y: i32, title: &str, sub: &str) -> i32 {
    p.text(x, y, 32, title, 24.0, W::Disp, TXT);
    p.text_right(x + w, y, 32, sub, 13.0, W::Reg, MUT);
    y + 48
}

fn chip(p: &mut Painter, x: i32, y: i32, label: &str) {
    let w = p.width(label, 11.0, W::Semi).round() as i32 + 16;
    p.rrect(x, y, w, 22, 11, ghostengine::Color::rgba(10, 8, 16, 200));
    p.text(x + 8, y, 22, label, 11.0, W::Semi, ghostengine::Color::WHITE);
}

fn badge_right(p: &mut Painter, right: i32, y: i32, label: &str, bg: ghostengine::Color, fg: ghostengine::Color) {
    let w = p.width(label, 11.0, W::Bold).round() as i32 + 16;
    p.rrect(right - w, y, w, 22, 11, bg);
    p.text(right - w + 8, y, 22, label, 11.0, W::Bold, fg);
}

fn cover_title(p: &mut Painter, x: i32, y: i32, w: i32, h: i32, title: &str, size: f32) {
    let lh = (size * 1.1).round() as i32;
    let lines = p.wrap(title, size, W::Disp, w - 28);
    let n = lines.len().min(3) as i32;
    let top = y + h - 14 - n * lh;
    for (i, l) in lines.iter().take(3).enumerate() {
        p.text(x + 14, top + i as i32 * lh, lh, l, size, W::Disp, ghostengine::Color::WHITE);
    }
}

fn price_row(p: &mut Painter, x: i32, y: i32, g: &Game) {
    let mut cx = x;
    if !g.off.is_empty() {
        let w = p.width(g.off, 12.0, W::Bold).round() as i32 + 12;
        p.rrect(cx, y, w, 20, 6, GREEN);
        p.text(cx + 6, y, 20, g.off, 12.0, W::Bold, GREEN_DARK);
        cx += w + 8;
    }
    let pw = p.text(cx, y, 20, g.price, 14.0, W::Semi, TXT).round() as i32;
    cx += pw + 8;
    if !g.old.is_empty() {
        let ow = p.text(cx, y, 20, g.old, 12.0, W::Reg, MUT).round() as i32;
        p.fill(cx, y + 10, ow, 1, MUT);
    } else if !g.when.is_empty() {
        p.text(cx, y, 20, g.when, 13.0, W::Reg, MUT);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Pick,
    Deal,
    Fresh,
    Explore,
    Simple,
}

/// Draws one store card at (x, y) with width `w`; returns its height.
fn card(p: &mut Painter, x: i32, y: i32, w: i32, g: &Game, kind: Kind) -> i32 {
    let h = w * 4 / 3;
    gradient_cover(p, x, y, w, h, 14, g.title, 160.0, BG);
    chip(p, x + 10, y + 10, g.store);
    if matches!(kind, Kind::Pick | Kind::Deal | Kind::Fresh) {
        badge_right(p, x + w - 10, y + 10, g.mat, VIO, VIO_DARK);
    }
    cover_title(p, x, y, w, h, g.title, 20.0);
    price_row(p, x, y + h + 10, g);
    let mut total = h + 10 + 20;
    if matches!(kind, Kind::Pick | Kind::Explore) {
        total += 10 + p.paragraph(x, y + total + 10, w, 17, g.why, 12.0, W::Reg, MUT, 3);
    }
    p.hit(x, y, w, total, 14, Action::Go(Page::Product), false);
    total
}

fn card_row(p: &mut Painter, y: i32, games: &[Game], kind: Kind) -> i32 {
    let mut max_h = 0;
    for (i, g) in games.iter().enumerate() {
        let x = MX + i as i32 * (COL + 22);
        max_h = max_h.max(card(p, x, y, COL, g, kind));
    }
    max_h
}

fn kv_rows(p: &mut Painter, x: i32, y: i32, w: i32, rows: &[(&str, &str)]) -> i32 {
    for (i, (k, v)) in rows.iter().enumerate() {
        let ry = y + i as i32 * 41;
        p.fill(x, ry, w, 1, LINE);
        p.text(x, ry + 1, 40, k, 14.0, W::Reg, MUT);
        p.text_right(x + w, ry + 1, 40, v, 14.0, W::Reg, TXT);
    }
    rows.len() as i32 * 41
}

// ------------------------------------------------------------------ pages
/// Store: the launcher's home page, personalised to the player's tastes.
pub fn store(p: &mut Painter, vh: i32, st: &State) -> i32 {
    search_bar(p, 24, "Rechercher un jeu, un studio, un genre", Some("Boutique mise à jour à chaque synchro"));

    // Hero
    let (hy, hh) = (104, 380);
    p.gradient(MX, hy, MW, hh, 120.0, &[(0.0, hex("#12091f")), (0.55, hex("#3b1a5e")), (1.0, hex("#0b2a4a"))]);
    p.round_corners(MX, hy, MW, hh, 20, BG);
    p.rrect_line(MX, hy, MW, hh, 20, LINE);
    let hx = MX + 40;
    let by = hy + hh - 40 - 44;
    let para = "Vous jouez surtout des RPG de science-fiction à récit fort : celui-ci coche tout, et il est moins cher chez GOG que sur vos autres boutiques.";
    let lines = p.wrap(para, 16.0, W::Reg, 600).len() as i32;
    let py = by - 16 - lines * 24;
    p.paragraph(hx, py, 600, 24, para, 16.0, W::Reg, PALE, 4);
    p.text(hx, py - 16 - 56, 56, "Cyberpunk 2077", 56.0, W::Disp, ghostengine::Color::WHITE);
    p.text(hx, py - 16 - 56 - 16 - 16, 16, "Notre meilleure suggestion pour vous · 96 % de compatibilité", 13.0, W::Semi, ACT_TXT);
    let mut bx = hx;
    let wv = button(p, bx, by, None, "Voir la fiche", true);
    p.hit(bx, by, wv, 44, 12, Action::Go(Page::Product), false);
    bx += wv + 12;
    bx += button(p, bx, by, None, "Ajouter à la liste d'envies", false) + 12;
    button(p, bx, by, None, "Pas intéressé", false);

    // Tastes + store filter
    let fy = hy + hh + 36;
    let tastes: Vec<(&str, bool)> = TASTES.iter().map(|s| (*s, true)).chain([("+ Modifier", false)]).collect();
    let stores: Vec<(&str, bool)> = STORES.iter().enumerate().map(|(i, s)| (*s, i == st.store_filter)).collect();
    pills_row(p, MX, fy, "Vos goûts", &tastes, None);
    let rw = pills_width(p, "Boutique", &stores);
    pills_row(p, MX + MW - rw, fy, "Boutique", &stores, Some(Action::StoreFilter));

    // Sections
    let mut y = fy + 36 + 36;
    for (title, sub, games, kind) in [
        ("Choisis pour vous", "Selon vos goûts et votre historique", picks(), Kind::Pick),
        ("En promotion dans vos genres", "Meilleur prix parmi vos boutiques", deals(), Kind::Deal),
        ("Nouveautés", "Triées selon vos goûts", fresh(), Kind::Fresh),
        ("Pour sortir de vos habitudes", "Hors de vos genres habituels", explore(), Kind::Explore),
    ] {
        let games: Vec<Game> = games.into_iter().filter(|g| st.store_filter == 0 || g.store == STORES[st.store_filter]).collect();
        if games.is_empty() {
            continue;
        }
        y = section_header(p, MX, MW, y, title, sub);
        y += card_row(p, y, &games, kind) + 36;
    }
    let _ = vh;
    y - 36 + 56
}

/// Library: detected games, status, and the selected game's detail panel.
pub fn library(p: &mut Painter, vh: i32, st: &State) -> i32 {
    let _ = vh;
    p.text(MX, 25, 42, "Bibliothèque", 40.0, W::Disp, TXT);
    let sx = MX + MW - 520;
    p.rrect(sx, 24, 520, 44, 12, PANEL);
    p.rrect_line(sx, 24, 520, 44, 12, LINE);
    icon(p, Icon::Search, sx + 14, 37, MUT, PANEL);
    p.text(sx + 42, 24, 44, "Filtrer ma bibliothèque", 14.0, W::Reg, MUT);

    // stats
    let sw = (MW - 48) / 4;
    for (i, (n, label)) in [("12", "jeux détectés"), ("4", "boutiques liées"), ("7", "installés"), ("2", "mises à jour")].iter().enumerate() {
        let x = MX + i as i32 * (sw + 16);
        p.rrect(x, 96, sw, 86, 14, PANEL);
        p.rrect_line(x, 96, sw, 86, 14, LINE);
        p.text(x + 18, 112, 34, n, 28.0, W::Disp, TXT);
        p.text(x + 18, 150, 16, label, 13.0, W::Reg, MUT);
    }

    // filters
    let fy = 210;
    let mut x = MX;
    for (i, f) in ["Tous", "Installés", "Favoris", "Steam", "Epic Games", "GOG", "Xbox"].iter().enumerate() {
        x += pill(p, x, fy, f, i == st.lib_filter, Some(Action::LibFilter(i))) + 10;
    }
    p.text_right(MX + MW, fy, 36, "Trier par : dernière session", 13.0, W::Reg, MUT);

    // grid (5 columns) + detail panel
    let gy = 274;
    let cw = 172;
    let items: Vec<&(&str, &str, u8, &str)> = LIBRARY.iter().filter(|e| lib_matches(st.lib_filter, e)).collect();
    for (i, (title, store, state, played)) in items.iter().map(|e| **e).enumerate() {
        let (col, row) = (i as i32 % 5, i as i32 / 5);
        let (x, y) = (MX + col * (cw + 20), gy + row * (254 + 20));
        let h = cw * 4 / 3;
        gradient_cover(p, x, y, cw, h, 14, title, 160.0, BG);
        chip(p, x + 10, y + 10, if store == "Epic Games" { "Epic" } else { store });
        let (label, bg, fg) = match state {
            0 => ("Installé", GREEN, GREEN_DARK),
            1 => ("Mise à jour", AMBER, AMBER_DARK),
            _ => ("Non installé", ghostengine::Color::rgba(10, 8, 16, 200), ghostengine::Color::WHITE),
        };
        badge_right(p, x + cw - 10, y + 10, label, bg, fg);
        cover_title(p, x, y, cw, h, title, 19.0);
        if i == 0 {
            p.rrect_line(x, y, cw, h, 14, VIO);
            p.rrect_line(x + 1, y + 1, cw - 2, h - 2, 13, VIO);
        }
        p.text(x, y + h + 8, 17, played, 13.0, W::Reg, MUT);
        p.hit(x, y, cw, h + 25, 14, Action::Go(Page::Product), false);
    }
    let rows = ((items.len() as i32 + 4) / 5).max(1);
    let grid_bottom = gy + rows * (254 + 20) - 20;

    // detail panel: shows the first game of the current filter
    let (title, store, _, played) = *items[0];
    let (hours, last) = match played.split_once(" · ") {
        Some((h, l)) => {
            let mut c = l.chars();
            (h.to_string(), c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default())
        }
        None => ("0 h".to_string(), "Jamais".to_string()),
    };
    let folder = format!("D:\\Jeux\\{title}");
    let (px, pw) = (MX + 940 + 28, 320);
    p.rrect(px, gy, pw, 496, 18, PANEL);
    p.rrect_line(px, gy, pw, 496, 18, LINE);
    gradient_cover(p, px + 20, gy + 20, 280, 175, 14, title, 160.0, PANEL);
    chip(p, px + 30, gy + 30, store);
    p.text(px + 34, gy + 20 + 175 - 14 - 26, 26, title, 24.0, W::Disp, ghostengine::Color::WHITE);
    let fy2 = gy + 20 + 175 + 16;
    let fiche = p.width("Fiche", 14.0, W::Semi).round() as i32 + 40;
    button(p, px + 20, fy2, Some(280 - 10 - fiche), "Jouer", true);
    button(p, px + 20 + 280 - fiche, fy2, Some(fiche), "Fiche", false);
    p.hit(px + 20 + 280 - fiche, fy2, fiche, 44, 12, Action::Go(Page::Product), false);
    kv_rows(
        p,
        px + 20,
        fy2 + 44 + 16,
        280,
        &[("Temps de jeu", hours.as_str()), ("Dernière session", last.as_str()), ("Boutique", store), ("Dossier", folder.as_str()), ("Espace disque", "—")],
    );
    grid_bottom.max(gy + 496) + 56
}

fn lib_matches(filter: usize, e: &(&str, &str, u8, &str)) -> bool {
    match filter {
        0 => true,
        1 => e.2 < 2,
        2 => ["Cyberpunk 2077", "The Witcher 3", "Hades", "Celeste"].contains(&e.0),
        3 => e.1 == "Steam",
        4 => e.1 == "Epic Games",
        5 => e.1 == "GOG",
        _ => e.1 == "Xbox",
    }
}

/// Product page for Cyberpunk 2077: price comparison across linked stores.
pub fn product(p: &mut Painter, vh: i32, _st: &State) -> i32 {
    let _ = vh;
    let bw = p.text(MX, 24, 17, "Boutique", 13.0, W::Reg, SOFT).round() as i32;
    p.hit(MX, 22, bw, 21, 4, Action::Go(Page::Store), false);
    p.text(MX + bw, 24, 17, "  /  RPG  /  Cyberpunk 2077", 13.0, W::Reg, MUT);

    // hero
    let hy = 77;
    gradient_cover(p, MX, hy, 300, 400, 14, "Cyberpunk 2077", 160.0, BG);
    chip(p, MX + 10, hy + 10, "Steam · GOG · Epic Games");
    cover_title(p, MX, hy, 300, 400, "Cyberpunk 2077", 30.0);
    let rx = MX + 300 + 36;
    p.text(rx, hy, 17, "CD PROJEKT RED · Sorti le 10 décembre 2020", 13.0, W::Semi, ACT_TXT);
    p.text(rx, hy + 25, 53, "Cyberpunk 2077", 52.0, W::Disp, ghostengine::Color::WHITE);
    let mut tx = rx;
    for t in ["RPG", "Monde ouvert", "Science-fiction", "Solo", "Narratif"] {
        let w = p.width(t, 13.0, W::Med).round() as i32 + 28;
        p.rrect(tx, hy + 96, w, 32, 16, PANEL);
        p.rrect_line(tx, hy + 96, w, 32, 16, LINE);
        p.text(tx + 14, hy + 96, 32, t, 13.0, W::Med, SOFT);
        tx += w + 8;
    }
    let desc = "Dans Night City, mégalopole saturée de technologie et de pouvoir, vous incarnez V, mercenaire en quête d'un implant qui détient la clé de l'immortalité. Monde ouvert, combats au choix (armes, piratage, mêlée) et un récit qui réagit à vos décisions.";
    let dh = p.paragraph(rx, hy + 146, 640, 26, desc, 16.0, W::Reg, PALE, 6);
    let by = hy + 146 + dh + 18;
    let w1 = button(p, rx, by, None, "Acheter chez GOG · 29,99 €", true);
    button(p, rx + w1 + 12, by, None, "Ajouter à la liste d'envies", false);
    let oy = by + 44 + 18;
    p.rrect(rx, oy, 56, 20, 6, GREEN);
    p.text(rx + 6, oy, 20, "-50 %", 12.0, W::Bold, GREEN_DARK);
    p.text(rx + 68, oy, 20, "Meilleur prix actuel parmi les boutiques comparées", 13.0, W::Reg, MUT);

    // price comparison
    let cy = hy + 400 + 36;
    p.rrect(MX, cy, MW, 312, 18, PANEL);
    p.rrect_line(MX, cy, MW, 312, 18, LINE);
    section_header(p, MX + 24, MW - 48, cy + 24, "Comparer les prix", "Prix d'exemple, à brancher sur les API des boutiques");
    let offers = [
        ("GOG", "Sans DRM · meilleur prix", "29,99 €", "59,99 €", "-50 %", true),
        ("Steam", "Clé liée à votre compte Steam", "35,99 €", "59,99 €", "-40 %", false),
        ("Epic Games", "Boutique Epic", "41,99 €", "59,99 €", "-30 %", false),
    ];
    for (i, (store, note, price, old, off, best)) in offers.iter().enumerate() {
        let ry = cy + 24 + 32 + 16 + i as i32 * 72;
        if i > 0 {
            p.fill(MX + 24, ry, MW - 48, 1, LINE);
        }
        let y = ry + 14;
        p.text(MX + 24, y + 4, 18, store, 15.0, W::Bold, TXT);
        p.text(MX + 24, y + 24, 16, note, 13.0, W::Reg, MUT);
        let btw = p.width("Acheter", 14.0, W::Semi).round() as i32 + 40;
        let bx = MX + MW - 24 - btw;
        button(p, bx, y, Some(btw), "Acheter", *best);
        let px = bx - 16 - 190;
        p.rrect(px, y + 12, 56, 20, 6, GREEN);
        p.text(px + 6, y + 12, 20, off, 12.0, W::Bold, GREEN_DARK);
        let pw = p.text(px + 64, y + 10, 24, price, 18.0, W::Semi, TXT).round() as i32;
        let ow = p.text(px + 64 + pw + 8, y + 12, 20, old, 12.0, W::Reg, MUT).round() as i32;
        p.fill(px + 64 + pw + 8, y + 22, ow, 1, MUT);
    }

    // screenshots + info
    let sy = cy + 312 + 36;
    section_header(p, MX, 960, sy, "Aperçus", "Emplacements pour les captures officielles");
    for i in 0..4 {
        let x = MX + i * (228 + 16);
        let y = sy + 48;
        let shades = [("#3b0f4a", "#0a1f3d"), ("#0a1f3d", "#6b1d7a"), ("#1d2a5a", "#3b0f4a"), ("#4a1d6b", "#0b1b3a")];
        p.gradient(x, y, 228, 128, 135.0, &[(0.0, hex(shades[i as usize].0)), (1.0, hex(shades[i as usize].1))]);
        p.round_corners(x, y, 228, 128, 14, BG);
        p.rrect_line(x, y, 228, 128, 14, ghostengine::Color::rgba(255, 255, 255, 22));
        p.text(x + 12, y + 128 - 12 - 16, 16, &format!("Capture {}", i + 1), 12.0, W::Reg, hex("#E6DEF5"));
    }
    let ax = MX + 960 + 28;
    p.rrect(ax, sy, 300, 285, 18, PANEL);
    p.rrect_line(ax, sy, 300, 285, 18, LINE);
    p.text(ax + 24, sy + 24, 24, "Informations", 20.0, W::Disp, TXT);
    kv_rows(
        p,
        ax + 24,
        sy + 24 + 24 + 8,
        252,
        &[("Développeur", "CD PROJEKT RED"), ("Genre", "RPG d'action"), ("Plateformes", "PC"), ("Langues", "Interface en français"), ("Extension", "Phantom Liberty")],
    );

    // similar
    let my = sy + 285 + 36;
    let y = section_header(p, MX, MW, my, "Dans le même esprit", "Selon vos goûts");
    let h = card_row(p, y, &similar(), Kind::Simple);
    y + h + 56
}

/// Settings: linked stores moved here from the sidebar, plus local detection.
pub fn settings(p: &mut Painter, vh: i32, _st: &State) -> i32 {
    let _ = vh;
    p.text(MX, 32, 42, "Paramètres", 40.0, W::Disp, TXT);
    // sub navigation
    for (i, s) in ["Général", "Boutiques liées", "Détection des jeux", "Goûts et recommandations", "Apparence", "À propos"].iter().enumerate() {
        let y = 102 + i as i32 * 46;
        let on = i == 1;
        if on {
            p.rrect(MX, y, 220, 44, 10, ACT);
        }
        p.text(MX + 14, y, 44, s, 14.0, W::Med, if on { ACT_TXT } else { MUT });
    }
    let cx = MX + 220 + 36;
    let cw = MW - 256;

    // linked stores
    let y = section_header(p, cx, cw, 102, "Boutiques liées", "Leurs jeux alimentent la boutique et la bibliothèque");
    p.rrect(cx, y, cw, 400, 18, PANEL);
    p.rrect_line(cx, y, cw, 400, 18, LINE);
    let stores = [
        ("Steam", "S", "Dernière synchro il y a 4 min · 5 jeux"),
        ("Epic Games", "E", "Dernière synchro il y a 4 min · 3 jeux"),
        ("GOG", "G", "Dernière synchro il y a 4 min · 3 jeux"),
        ("Xbox", "X", "Dernière synchro il y a 4 min · 2 jeux"),
    ];
    for (i, (name, letter, sync)) in stores.iter().enumerate() {
        let ry = y + 8 + i as i32 * 96;
        if i > 0 {
            p.fill(cx + 24, ry, cw - 48, 1, LINE);
        }
        let mid = ry + 48;
        p.rrect(cx + 24, mid - 22, 44, 44, 12, ACT);
        let lw = p.width(letter, 18.0, W::Disp).round() as i32;
        p.text(cx + 24 + (44 - lw) / 2, mid - 22, 44, letter, 18.0, W::Disp, ACT_TXT);
        let tx = cx + 24 + 44 + 16;
        p.text(tx, mid - 34, 20, name, 16.0, W::Semi, TXT);
        p.circle(tx + 4, mid - 4, 4, GREEN);
        p.text(tx + 14, mid - 12, 16, "Connecté", 12.0, W::Semi, GREEN);
        p.text(tx, mid + 8, 17, sync, 13.0, W::Reg, MUT);
        let dw = p.width("Déconnecter", 14.0, W::Semi).round() as i32 + 40;
        let rw = p.width("Resynchroniser", 14.0, W::Semi).round() as i32 + 40;
        let dx = cx + cw - 24 - dw;
        button(p, dx, mid - 22, Some(dw), "Déconnecter", false);
        button(p, dx - 16 - rw, mid - 22, Some(rw), "Resynchroniser", false);
    }
    button(p, cx, y + 400 + 16, None, "Lier une boutique", true);

    // local detection
    let y2 = y + 400 + 16 + 44 + 32;
    let y2b = section_header(p, cx, cw, y2, "Détection locale", "Dossiers analysés pour trouver les jeux installés");
    p.rrect(cx, y2b, cw, 256, 18, PANEL);
    p.rrect_line(cx, y2b, cw, 256, 18, LINE);
    let folders = [
        ("D:\\Jeux", "Analysé automatiquement au démarrage"),
        ("C:\\Program Files (x86)\\Steam\\steamapps", "Dossier détecté via la boutique liée"),
        ("C:\\Games\\GOG Galaxy", "Dossier détecté via la boutique liée"),
    ];
    for (i, (path, note)) in folders.iter().enumerate() {
        let ry = y2b + 8 + i as i32 * 80;
        if i > 0 {
            p.fill(cx + 24, ry, cw - 48, 1, LINE);
        }
        let mid = ry + 40;
        p.text(cx + 24, mid - 20, 18, path, 15.0, W::Semi, TXT);
        p.text(cx + 24, mid + 2, 17, note, 13.0, W::Reg, MUT);
        let rw = p.width("Retirer", 14.0, W::Semi).round() as i32 + 40;
        button(p, cx + cw - 24 - rw, mid - 22, Some(rw), "Retirer", false);
    }
    button(p, cx, y2b + 256 + 16, None, "Ajouter un dossier", false);
    y2b + 256 + 16 + 44 + 56
}
