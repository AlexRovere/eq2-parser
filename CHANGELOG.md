# Changelog

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
