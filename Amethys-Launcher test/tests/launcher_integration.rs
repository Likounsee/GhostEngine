use amethyst_preview::covers::{slug, CoverStore};
use std::path::PathBuf;

#[test]
fn cover_slugs_are_stable_for_game_titles() {
    let cases = [
        ("Baldur's Gate 3", "baldurs-gate-3"),
        ("Cyberpunk 2077", "cyberpunk-2077"),
        ("The Witcher 3", "the-witcher-3"),
        ("Hollow Knight", "hollow-knight"),
    ];

    for (title, expected) in cases {
        assert_eq!(slug(title), expected, "unexpected slug for {title}");
    }
}

#[test]
fn bundled_cover_library_decodes_and_builds_mipmaps() {
    let covers_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("covers");
    let store = CoverStore::load(&covers_dir);

    assert_eq!(
        store.files, 12,
        "all bundled cover images should decode; failures: {:?}",
        store.failed
    );
    assert!(
        store.failed.is_empty(),
        "cover decoding failures: {:?}",
        store.failed
    );

    for title in [
        "Alan Wake 2",
        "Balatro",
        "Baldur's Gate 3",
        "Celeste",
        "Cyberpunk 2077",
        "Disco Elysium",
        "Elden Ring",
        "Forza Horizon 5",
        "Hades",
        "Hollow Knight",
        "Stardew Valley",
        "The Witcher 3",
    ] {
        assert!(
            store.get(title).is_some(),
            "missing decoded mipmapped cover for {title}"
        );
    }

    assert!(
        store.stats().entries > 0,
        "decoded textures should be present in the public texture cache"
    );
}
