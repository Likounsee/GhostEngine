# amethyst-preview

Rend les maquettes d'Amethyst Launcher (Boutique, Bibliothèque, fiche produit, Paramètres) avec le moteur CPU de GhostEngine, sans fenêtre ni GPU.

    cargo run --release -- sortie/ /usr/share/fonts/opentype/inter

Crate autonome (son propre workspace) : elle dépend de `../ghostengine/crates/ghostengine`.
Si le moteur ne compile pas chez toi, appliquer `engine-build-fixes.patch` (corrections minimales, non appliquées au dépôt).
Sortie : `<page>.png` (fenêtre 16:9, 1600x900) et `<page>-full.png` (page entière).
Police : Inter (OTF) ; Space Grotesk n'était pas disponible. Tout le dessin passe par des fonctions exportées par GhostEngine (rounded rects AA, dégradés, cercles, polygones, texte).
Les prix, pourcentages et noms de dossiers sont des données d'exemple.

## Mode fenêtré (test d'interaction)

    cargo run --release --features window --bin amethyst-window

Molette = défilement (`ScrollViewport`), survol et clics via `UiInteraction`, champ de recherche = vrai `UiComponent::text_input` (clavier, IME, presse-papiers). Cliquer : menu de gauche, cartes de jeu, « Voir la fiche », filtres de boutique (Boutique) et de bibliothèque, fil d'Ariane de la fiche. Échap quitte.
Sans écran : `--snapshot DOSSIER` rejoue une session scriptée et enregistre des PNG (`out/fenetre/`).
Non testé dans une vraie fenêtre ici (conteneur sans affichage) : seul le rejeu headless a été exécuté.
