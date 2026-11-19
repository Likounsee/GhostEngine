//! Sample content for the mockups. Every value here is invented placeholder data.

#[derive(Clone, Copy)]
pub struct Game {
    pub title: &'static str,
    pub store: &'static str,
    pub price: &'static str,
    pub old: &'static str,
    pub off: &'static str,
    pub mat: &'static str,
    pub why: &'static str,
    pub when: &'static str,
}

const fn g(title: &'static str, store: &'static str, price: &'static str, old: &'static str, off: &'static str, mat: &'static str, why: &'static str, when: &'static str) -> Game {
    Game { title, store, price, old, off, mat, why, when }
}

/// Cover gradient colours (top, bottom) per game title.
pub fn cover(title: &str) -> (&'static str, &'static str) {
    match title {
        "Cyberpunk 2077" => ("#6b1d7a", "#0a1f3d"),
        "The Witcher 3" => ("#2a3b2f", "#0f1a14"),
        "Elden Ring" => ("#4a3a14", "#14110a"),
        "Alan Wake 2" => ("#1f3a4a", "#0a0f14"),
        "Forza Horizon 5" => ("#1f5a7a", "#0c1a2a"),
        "Hades" => ("#7a1f2b", "#1d0a12"),
        "Baldur's Gate 3" => ("#5a2a1a", "#1a0f1f"),
        "Balatro" => ("#5a1f6b", "#1a0a2e"),
        "Celeste" => ("#1f4a6b", "#14122e"),
        "Stardew Valley" => ("#2f6b3a", "#10261a"),
        "Disco Elysium" => ("#6b4a1f", "#1a1410"),
        _ => ("#7a2a3a", "#140a14"), // Hollow Knight
    }
}

pub const TASTES: [&str; 5] = ["RPG", "Monde ouvert", "Science-fiction", "Narratif", "Solo"];
pub const STORES: [&str; 5] = ["Toutes", "Steam", "Epic Games", "GOG", "Xbox"];

pub fn picks() -> Vec<Game> {
    vec![
        g("Alan Wake 2", "Epic Games", "23,99 €", "", "", "88 %", "Récit dense et ambiance soignée, idéal en solo.", ""),
        g("Disco Elysium", "GOG", "39,99 €", "", "", "85 %", "Pour l'écriture et les choix qui changent la partie.", ""),
        g("Baldur's Gate 3", "Steam", "59,99 €", "", "", "94 %", "Vous enchaînez les RPG narratifs aux longues sessions.", ""),
        g("The Witcher 3", "GOG", "9,99 €", "", "", "93 %", "Monde ouvert et quêtes à histoires, comme vos derniers jeux.", ""),
        g("Balatro", "Steam", "13,99 €", "", "", "71 %", "Une respiration entre deux longues campagnes.", ""),
        g("Elden Ring", "Steam", "41,99 €", "", "", "82 %", "Monde ouvert exigeant, en solo.", ""),
    ]
}

pub fn deals() -> Vec<Game> {
    vec![
        g("The Witcher 3", "GOG", "9,99 €", "39,99 €", "-75 %", "93 %", "", ""),
        g("Cyberpunk 2077", "GOG", "29,99 €", "59,99 €", "-50 %", "96 %", "", ""),
        g("Elden Ring", "Steam", "41,99 €", "59,99 €", "-30 %", "82 %", "", ""),
        g("Alan Wake 2", "Epic Games", "23,99 €", "39,99 €", "-40 %", "88 %", "", ""),
        g("Hades", "Steam", "12,49 €", "24,99 €", "-50 %", "74 %", "", ""),
        g("Hollow Knight", "Xbox", "14,99 €", "24,99 €", "-40 %", "69 %", "", ""),
    ]
}

pub fn fresh() -> Vec<Game> {
    vec![
        g("Baldur's Gate 3", "Steam", "59,99 €", "", "", "94 %", "", "Cette semaine"),
        g("Balatro", "Steam", "13,99 €", "", "", "71 %", "", "Cette semaine"),
        g("Celeste", "Epic Games", "19,99 €", "", "", "60 %", "", "Ce mois-ci"),
        g("Stardew Valley", "GOG", "13,99 €", "", "", "58 %", "", "Ce mois-ci"),
        g("Disco Elysium", "GOG", "39,99 €", "", "", "85 %", "", "Ce mois-ci"),
        g("Forza Horizon 5", "Xbox", "29,99 €", "", "", "47 %", "", "Ce mois-ci"),
    ]
}

pub fn explore() -> Vec<Game> {
    vec![
        g("Forza Horizon 5", "Xbox", "29,99 €", "", "", "", "Course en monde ouvert, pour changer de rythme.", ""),
        g("Stardew Valley", "GOG", "13,99 €", "", "", "", "Un jeu de ferme tranquille, loin de vos univers sombres.", ""),
        g("Celeste", "Epic Games", "19,99 €", "", "", "", "Un plateformeur exigeant et court.", ""),
    ]
}

pub fn similar() -> Vec<Game> {
    vec![
        g("The Witcher 3", "GOG", "9,99 €", "", "", "", "", ""),
        g("Alan Wake 2", "Epic Games", "23,99 €", "", "", "", "", ""),
        g("Disco Elysium", "GOG", "39,99 €", "", "", "", "", ""),
        g("Baldur's Gate 3", "Steam", "59,99 €", "", "", "", "", ""),
        g("Elden Ring", "Steam", "41,99 €", "", "", "", "", ""),
    ]
}

/// Library entries: (title, store, state 0=installed 1=update 2=not installed, played)
pub const LIBRARY: [(&str, &str, u8, &str); 12] = [
    ("Cyberpunk 2077", "Steam", 0, "42 h · hier"),
    ("The Witcher 3", "GOG", 0, "120 h · il y a 3 jours"),
    ("Elden Ring", "Steam", 1, "64 h · il y a 1 semaine"),
    ("Stardew Valley", "GOG", 0, "18 h · il y a 2 semaines"),
    ("Forza Horizon 5", "Xbox", 0, "11 h · il y a 3 semaines"),
    ("Alan Wake 2", "Epic Games", 1, "9 h · il y a 1 mois"),
    ("Celeste", "Epic Games", 0, "6 h · il y a 1 mois"),
    ("Hades", "Steam", 0, "33 h · il y a 2 mois"),
    ("Disco Elysium", "GOG", 2, "Jamais lancé"),
    ("Balatro", "Steam", 2, "Jamais lancé"),
    ("Hollow Knight", "Xbox", 2, "Jamais lancé"),
    ("Baldur's Gate 3", "Steam", 2, "Jamais lancé"),
];
