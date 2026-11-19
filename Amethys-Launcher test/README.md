# Amethys-Launcher test

Test interactif d'Amethyst Launcher rendu **entièrement par GhostEngine** (moteur CPU, aucune autre bibliothèque de rendu).
Ce n'est pas un visualiseur d'images : c'est une vraie fenêtre avec de vraies images décodées par le moteur.

## Lancer

    cargo run --release          (ou double-cliquer run.cmd sous Windows)

Le dossier doit se trouver à côté du dossier `ghostengine/` du dépôt (dépendance `../ghostengine/crates/ghostengine`).
Si le moteur ne compile pas chez toi, appliquer `engine-build-fixes.patch` (corrections minimales, non appliquées au dépôt).

## Ce que tu peux faire

- Molette : défilement (`ScrollViewport`), barre fine qui n'apparaît que sur le contenu.
- Survol / clic (`UiInteraction`) : menu de gauche, cartes de jeu, « Voir la fiche », filtres de la boutique et de la bibliothèque, fil d'Ariane.
- Barre de recherche : vrai champ texte du moteur (`UiComponent::text_input` : clavier, IME, presse-papiers).
- Pastille « GhostEngine » en bas à droite : clic = panneau de **mesures réelles** (cadence, temps de rendu, framebuffer, images décodées, cache de textures, threads) et bouton **Lancer le benchmark** (mesure le rendu de page et des frames complètes sur ta machine). Rien n'est simulé.
- Échap : quitter.

## Images

`assets/covers/<titre-en-minuscules-avec-tirets>.png|jpg|bmp` est décodé par `decode_texture` (PNG, JPEG, BMP), rangé dans le `TextureCache`, puis échantillonné avec les mipmaps du moteur.
Les 12 fichiers fournis sont des **jaquettes de remplacement** générées avec le moteur (`cargo run --release --bin gen-covers`). Pour utiliser une vraie jaquette, dépose un fichier du même nom dans `assets/covers/` (ex. `cyberpunk-2077.png`).

## Polices

Inter (licence OFL, `assets/fonts/`). Space Grotesk n'est pas incluse : ajouter ses OTF/TTF et adapter `Fonts::load` si tu la veux.

## Sans écran

    cargo run --release -- --snapshot dossier-de-sortie

Rejoue une session scriptée (défilement, clics, filtres, panneau de mesures, benchmark) et enregistre des captures : utile pour vérifier sans fenêtre.
Les données (prix, pourcentages, temps de jeu, dossiers) sont des exemples.
