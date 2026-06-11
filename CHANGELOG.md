# Changelog

## v0.6.0 — Rotation live + couleur de perso

- Overlay **rotation live** (« prochain sort ») : une fenêtre séparée, toujours
  au-dessus, qui te dit quoi caster maintenant selon l'état réel du combat
  - File priorisée : 🔁 DoT tombé (ou sur le point) à rafraîchir, ⏳ gros
    cooldown prêt à presser, ▶ meilleur filler disponible
  - Optionnel, à activer dans l'onglet 🎯 Optimisation ; suit le perso actif et
    réutilise tes stats (casting/reuse) et ton scénario
  - Réglages : nombre de sorts affichés, anticipation avant qu'un DoT ne tombe
  - C'est un assistant qui *suggère* : il ne joue pas à ta place
- **Couleur de personnage** personnalisable : choisis la couleur de ta barre
  dans Settings > Overlay ; elle s'applique partout (overlay, courbes du graphe,
  jauges des tables), avec retour à la couleur automatique en un clic

## v0.5.0 — Optimisation de rotation

- Nouvel onglet 🎯 Optimisation : classe tes sorts par efficacité pour t'aider à
  prioriser ta rotation, à partir de tes vrais logs
  - Deux lectures : **Eff/GCD** (« je lance quoi sur un GCD libre ? ») et
    **DPS soutenu** (ce que le sort rapporte vraiment dans la durée, DoT et
    cooldowns inclus)
  - Colonne **Rôle** : 🔁 entretenir (DoT), ⏳ cooldown (à presser dès dispo),
    ▶ filler ; couleurs par type (mono / AoE zone / AE encounter)
  - Marche **sans combat** : choisis ta classe, ses sorts s'affichent (cast,
    recast, type pré-remplis depuis une base de 1365 sorts), tu saisis les
    dégâts du tooltip ; après un combat, ils se remplissent tout seuls
  - Filtres de scénario (mono / N cibles / cibles liées), tri par colonne,
    barres d'efficacité, sorts masquables
  - **Diagnostic de rotation** : activité GCD vs temps mort, sorts à mieux
    entretenir (uptime de tes DoT, gros cooldowns sous-utilisés)
  - Classe auto-détectée et mémorisée par personnage
- Overlay **mécaniques dédié** : fenêtre séparée listant les prochains casts de
  boss en compte à rebours (en plus du décompte dans l'overlay DPS)
- Combat : option « clore sur mon activité » (activée par défaut) — les combats
  de joueurs hors groupe ou de PNJ à côté ne polluent plus ton parse et ne
  gardent plus ton combat ouvert
- Mécaniques apprises : seulement les boss (nameds), silencieuses par défaut, et
  bouton 🧹 Nettoyer pour purger le trash déjà appris
- Overlays : fin du tremblement au déplacement

## v0.4.0 — Mécaniques de boss

- Nouvel onglet ⏱ Mécaniques : l'app apprend toute seule, depuis les logs, les
  capacités ennemies récurrentes et dangereuses (AoE multi-cibles, tank buster,
  coups mortels), sans aucune base de sorts
- Prédiction du prochain cast avec compte à rebours, et alerte avant l'impact
  (au choix : visuel, son ou voix, réglable par mécanique)
- Décompte optionnel dans l'overlay
- Base à trois sources, format unique : communautaire (embarquée), apprise
  localement, et saisie manuelle. Import / export pour la partager
- Plus tu joues, plus la base s'enrichit (et tu peux me l'envoyer pour qu'elle
  profite à tout le monde)
- Import de packs ACT (.xml) : bouton 🗡 Pack ACT — les timers de boss
  deviennent des mécaniques, les triggers atterrissent dans l'onglet Triggers
- Pack de triggers de base en un clic (ready check, bannière de ralliement,
  death prevents, debuffs de classe à recast, manastone…)
- Renommage en « EQ2 Parser » + nouvelle icône
- Synthèse vocale fiabilisée (SAPI Windows), bips intégrés sélectionnables
  (simple, grave, aigu, double, triple, montée, alarme), nouveau trigger
  ajouté en haut de la liste

## v0.3.2 — Mode clair
- Bascule ☀/🌙 en haut à droite : thème clair ou sombre, mémorisé
- Jauges des tables adaptées aux deux thèmes

## v0.3.1 — Nouveautés dans l'app
- Cette fenêtre ! Le changelog s'affiche une fois après chaque mise à jour
- Bouton 📋 Nouveautés dans Settings → Mises à jour pour la revoir

## v0.3.0 — Design & confort
- Settings réorganisés en sections repliables
- Onglet Triggers compact : badges (🗣 TTS, ⏱ timer, 🔁 cooldown) et aperçu du pattern
- Raccourcis clavier : Ctrl+1..4 pour les onglets, Échap ferme le breakdown
- Barres de l'overlay animées en douceur
- Icône de l'application (fenêtre + exe dans l'Explorateur)
- Couleur stable par joueur : la même partout (overlay, graphes, jauges des tables)
- Jauges colorées dans les tables Dégâts/Soins, chiffres alignés
- Croix rouges 💀 sur le graphe aux moments des morts
- Overlay : position mémorisée, verrou 🔒 anti-drag, fondu 👻 au survol
- Écran d'accueil avec checklist de démarrage (/log, perso, répertoire)
- Toasts visibles aussi dans la fenêtre principale
- Fenêtre Nouveautés (ce changelog !) après chaque mise à jour

## v0.2.0 — Zéro config
- Format des barres par défaut : DPS: 4691 (Total: 93.8k - 52.8%)
- Formats custom des barres et du titre de l'overlay (variables {{dps}}, {{pct}}…)
- Suivi automatique du perso actif : l'app bascule sur le log le plus récent
- Détection automatique du répertoire EQ2 (bibliothèques Steam, chemins usuels)
- Mise à jour automatique via les releases GitHub

## v0.1.0 — Première version
- Parse temps réel du log EQ2 (100 % de couverture validée sur logs réels)
- Encounters automatiques, historique persistant par personnage
- Overlay DPS/HPS/Power configurable (clic droit), redimensionnable
- Breakdown par sort, par cible, par type de dégâts, évitements, resists
- Pets fusionnés (auto-détection + assignation manuelle)
- Alliés/ennemis automatique, PNJ masqués
- Death reports (les 12 dernières secondes avant chaque mort)
- Sessions & zones (agrégats), comparaison de combats épinglés
- Triggers : regex, son, TTS, groupes de capture, timers, cooldown, packs JSON
- Graphes temporels (par joueur, par sort empilé), export PNG
- Exports : ligne chat, Markdown, CSV, JSON
- Texte custom à variables ({{dps}}, {{name:1}}, {{target}}…)
