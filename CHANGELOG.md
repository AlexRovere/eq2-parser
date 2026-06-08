# Changelog

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
