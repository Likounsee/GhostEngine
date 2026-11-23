# Amethys Launcher — GhostEngine integration test

Ce dossier contient un **prototype d'interface inspiré d'Amethys Launcher**, développé pour exercer GhostEngine dans un logiciel réel plutôt que dans une simple scène de démonstration.

Le prototype est rendu par le rasterizer CPU de GhostEngine. Il utilise l'API publique du moteur pour le texte, les formes, les images, les textures, les mipmaps, le framebuffer, le défilement, les interactions et la saisie.

> Ce projet est un banc d'intégration visuel : ses prix, recommandations, bibliothèques et données de jeux sont fictifs. Il ne constitue pas le code de production du launcher Amethys.

## Lancer la fenêtre interactive

Depuis la racine du dépôt :

```sh
cargo run --manifest-path "Amethys-Launcher test/Cargo.toml" --release
```

Sous Windows, `run.cmd` fournit également un raccourci.

## Exécution headless

La session scriptée utilise exactement le même `App::frame` que la fenêtre native. Elle ne crée aucune fenêtre et ne nécessite pas de serveur d'affichage :

```sh
cargo run --manifest-path "Amethys-Launcher test/Cargo.toml" --release -- --snapshot target/amethyst-launcher-snapshots
```

Elle enregistre 18 images PNG qui couvrent notamment la boutique, le survol, le défilement, la navigation vers la bibliothèque, les filtres, la fiche produit, la recherche et le panneau de mesures/benchmark.

## Interactions de la fenêtre

- Molette : défilement avec `ScrollViewport`.
- Survol et clic : navigation, cartes, filtres et fil d'Ariane via `UiInteraction`.
- Recherche : composant texte GhostEngine avec clavier, IME et presse-papiers.
- Pastille GhostEngine : panneau de mesures et benchmark local.
- Échap : quitter.

## Ressources

- `assets/covers/` : 12 jaquettes de remplacement générées pour le prototype. Elles sont décodées par `decode_texture`, mises en cache et utilisées avec les mipmaps.
- `assets/fonts/` : polices Inter fournies avec leur licence OFL.
- `src/data.rs` : données de démonstration fictives.

Pour remplacer une jaquette, déposer un fichier PNG, JPEG ou BMP portant le slug du jeu dans `assets/covers/`.

## Validation automatisée

La CI Windows, Linux et macOS :
1. vérifie le formatage Rust de ce projet ;
2. exécute ses tests d'intégration (décodage des 12 jaquettes, mipmaps, cache et rendu d'une vraie frame) ;
3. lance la session headless complète ;
4. vérifie que les 18 captures PNG ont été produites.

Ces tests valident l'intégration logicielle et le rendu headless. Ils ne remplacent pas encore une vérification interactive de la fenêtre native sur chaque système.
