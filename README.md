# EQ2 Tools — Combat Parser & Overlay

Parser de logs de combat **EverQuest II** en temps réel avec overlay DPS/HPS, façon
ACT (Advanced Combat Tracker), en un seul `.exe` (~7 Mo, Rust + egui).

## Fonctionnalités

- **Overlay temps réel** : barres DPS / HPS / Power par combattant (sections au choix),
  toujours au-dessus du jeu, déplaçable (drag sur la barre de titre).
  **Clic droit sur l'overlay** pour tout régler en jeu : transparence, taille (échelle),
  largeur, nombre de barres, couleurs (fond + accent), titre détaillé
  (durée • total • DPS raid • kills), texte libre, click-through.
  Redimensionnable par le grip ↘ comme une fenêtre (les barres s'adaptent à la hauteur).
  Ton personnage est surligné avec la couleur d'accent.
- **Texte custom avec variables** : template multi-lignes rendu en temps réel,
  ex. `je tape {{dps}} ({{crit}} crit) — top {{name:1}} à {{dps:1}}`.
  Variables : `{{dps}} {{hps}} {{pps}} {{dmg}} {{heal}} {{power}} {{crit}} {{maxhit}}
  {{rank}} {{taken}} {{deaths}} {{name}} {{target}} {{time}} {{raiddps}} {{raidhps}}
  {{total}} {{kills}}` — sans argument = toi, `{{dps:Nom}}` = un joueur,
  `{{dps:1}}` = le rang 1 du classement. Menu d'insertion ➕ et aperçu live
  dans Settings.
- **Détection automatique des encounters** : démarre à la première action offensive,
  se clôt après N secondes d'inactivité (réglable, défaut 6 s).
- **Historique de session** : liste des combats avec totaux, durée, kills.
- **Breakdown par sort/CA** : dégâts, soins, % du total, hits, crit rate, max hit.
- **Wards & absorbs** : les absorptions sont créditées comme soins effectifs au poseur du ward.
- **Triggers personnalisés** : regex sur les lignes du log → son (wav/mp3/ogg ou bip)
  + toast dans l'overlay.
- **Multi-personnages** : détection automatique des logs `eq2log_*.txt` par serveur,
  résolution de YOU/YOUR vers le nom du perso.
- **Attribution des pets** : auto-détection (fenêtre de 4 s après
  `You send your pet in for the attack!`, noms de pets générés un seul mot, les joueurs
  vus dans le chat `\aPC` sont exclus rétroactivement) + assignation manuelle par
  clic droit sur un combattant. Fusion à l'affichage (tables, overlay, exports, graphe),
  sorts du pet préfixés `🐾 <pet>:` dans le breakdown.
- **Exports** : ligne compacte pour le chat du jeu (≤ 250 car.) et tableau Markdown
  via presse-papiers, CSV et JSON (avec séries temporelles) vers fichier,
  graphe en image PNG.
- **Graphe temporel** : DPS / HPS / power / dégâts subis, lissage réglable
  (moyenne glissante 1-15 s), mode cumulé, filtre par joueur, et mode
  **par sort** (aires empilées des 8 plus gros sorts du combattant sélectionné).
- **Power replenish** : suivi du mana rendu (`refreshes X for N mana points`),
  table dédiée, métrique de graphe et colonne dans le breakdown.
- **Comparaison d'encounters** : épingle un combat (📌 dans l'onglet Encounters),
  ouvre-en un autre → table A/B avec Δ % par joueur + superposition des courbes
  en pointillés sur le graphe.
- **Historique persistant** : les encounters sont sauvegardés par personnage/serveur
  (`history/*.json`, cap configurable) et rechargés au lancement, avec déduplication
  au ré-import d'un log.
- **Alliés / ennemis** : inférence de faction automatique (attaque = camps opposés,
  soin = même camp) — les mobs n'apparaissent plus dans les classements, même les
  named capitalisés. Option pour tout afficher.
- **Death report** : pour chaque mort, qui / quand / tué par qui + la table des
  coups encaissés dans les 12 dernières secondes.
- **Session & zones** : pseudo-encounter « Σ Session entière » qui cumule tous les
  combats (graphes inclus), historique groupé par zone (`You have entered…`).
- **Détail par cible** : dégâts ventilés par mob, dégâts reçus par attaquant
  (vue tank), matrice de soins (donnés par bénéficiaire / reçus par soigneur),
  évitements (parade/riposte/esquive/bloc) et précision.
- **Log brut** : les lignes sources de chaque combat de la session, filtrables.
- **Profils d'overlay** : enregistre des configurations nommées (raid compact,
  solo détaillé…) et commute en un clic depuis le menu clic droit.

Voir [ROADMAP.md](ROADMAP.md) pour les features restantes vs ACT (triggers TTS/timers,
spell timers, agrégat de session, détail par cible…).

## Prérequis côté jeu

Activer le logging dans EQ2 : taper `/log` en jeu (les fichiers vont dans
`<EverQuest 2>\logs\<Serveur>\eq2log_<Personnage>.txt`).

## Utilisation

1. Lancer `eq2-tools.exe`.
2. Onglet **⚙ Settings** : vérifier le répertoire logs, cliquer sur ton personnage
   dans la liste (trié du plus récent au plus ancien).
3. Jouer. L'overlay s'anime dès le premier combat.

La configuration est sauvegardée dans `eq2-tools.json` à côté de l'exe
(dernier log suivi réattaché automatiquement au lancement).

## Formats de log supportés

Calibré et validé à 100 % sur de vrais logs (client EN, serveurs live/TLE) :

- dégâts auto-attack et sorts, mono et **multi-types** (`109 heat, 4 magic, 3 mental and 3 divine damage`)
- critiques (`for a critical of 21,385 ...`) et **nombres abrégés** (`515.9M`, `1.2B`)
- heals, wards (avec/sans montant), absorbs avec points restants
- misses / parry / riposte / dodge / block / deflect, hits sans dégâts
- threat, kills, morts, dégâts environnementaux (chute)
- noms avec apostrophes (`Vicolin J'Viniurden`) et possessifs en s (`Andreas' Faithful Swing`)

## Build

```bash
cargo build --release          # → target/release/eq2-tools.exe
cargo test                     # tests unitaires du parser et du moteur
cargo run --release --example parse_file -- "<chemin>\eq2log_X.txt"   # valider la couverture sur un log
```

## Architecture

```
src/
├── main.rs      entrée eframe (fenêtre principale + viewport overlay)
├── parser.rs    lignes log → LogEvent (regex calibrées, ~500k lignes/s)
├── combat.rs    moteur d'encounters, séries temporelles, pets, fusion à l'affichage
├── tailer.rs    tail temps réel du fichier (thread, gère rotation/troncature)
├── triggers.rs  triggers regex → audio (rodio) + toasts
├── export.rs    exports chat / Markdown / CSV / JSON
├── config.rs    persistance JSON (réglages, triggers, assignations pets)
└── ui.rs        onglets Live/Encounters/Triggers/Settings + overlay + graphe (egui_plot)
```
