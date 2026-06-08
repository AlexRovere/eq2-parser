# Roadmap — parité ACT et au-delà

État des features comparé à ACT (Advanced Combat Tracker) + EQ2 plugin.
Mis à jour : 2026-06-07.

## ✅ Fait

- Parse temps réel (tail du log, ~500k lignes/s, 100 % de couverture validée sur logs réels)
- Encounters auto (timeout d'inactivité réglable) + historique de session
- DPS / HPS / Power replenish, crits, max hit, wards/absorbs crédités au poseur
- Breakdown par sort/CA, par combattant
- Attribution des pets (auto-détection + assignation manuelle clic droit, fusion à l'affichage)
- Overlay configurable : sections DPS/HPS/Power, titre détaillé, texte custom à
  variables (`{{dps}}`, `{{dps:Nom}}`, `{{dps:1}}`), couleurs, échelle, resize par grip,
  click-through, rangs numérotés, soi toujours visible
- Triggers regex → son (wav/mp3/ogg ou bip) + toast overlay
- Exports : ligne chat (≤250 car.), Markdown, CSV, JSON complet, PNG du graphe
- Graphe temporel : DPS/HPS/Power/subis, par joueur ou aires empilées par sort,
  lissage, cumulé, filtre par joueur
- Comparaison de 2 encounters (épingle + Δ% + superposition pointillée)
- **Historique persistant** par personnage/serveur (`history/*.json`, cap configurable,
  auto-save throttlé + à la fermeture, dédup au ré-import)
- **Séparation alliés/ennemis** : inférence de faction par propagation
  (attaque = camps opposés, soin = même camp ; graines : soi, joueurs vus en chat, pets) ;
  les classements/overlay/exports/graphes ne montrent que les alliés (option pour tout voir)
- **Death report** : qui est mort, quand, tué par qui, avec les coups encaissés
  dans les 12 dernières secondes (table détaillée par mort)

## 🟠 À faire — priorité haute

### Triggers avancés
- [x] **TTS** (synthèse vocale WinRT via crate `tts`)
- [x] **Groupes de capture** : `(?<who>\w+) casts` → message `{who} incante !`
      (aussi `{1}`…`{9}` et `{0}` = match complet)
- [x] **Timers déclenchés** : compte à rebours nommé dans l'overlay (barre orange),
      toast + bip à expiration
- [x] Cooldown de trigger (ne pas re-déclencher pendant N s)
- [x] Import/export de packs de triggers en JSON (boutons 📥/📤 dans l'onglet
      Triggers) — l'import du format XML ACT reste à faire si besoin

### Agrégat de session / zones
- [x] Pseudo-encounter « Σ Session entière » (combats concaténés bout à bout,
      séries temporelles remappées, cache invalidé à chaque nouveau combat)
- [x] Parser `You have entered <Zone>.` → en-têtes de zone dans l'historique,
      zone affichée dans le détail, zoner clôt le combat en cours
- [x] Stats par zone : clic sur l'en-tête de zone → agrégat de tous les combats
      de la zone (durée cumulée, classements, graphes, morts)

## 🟡 À faire — priorité moyenne

### Détail par cible
- [x] Mes dégâts ventilés par mob (mini-table « Dégâts par cible »)
- [x] Vue tank : dégâts reçus ventilés par attaquant
- [x] Matrice de soins : soins donnés par bénéficiaire + soins reçus par soigneur
- [x] Ventilation par type de dégâts : infligés et reçus (crushing/heat/… +
      « (environnement) » pour les chutes) — attribution au type principal de
      la ligne (les riders multi-types comptent dans le type dominant)

### Breakdown avoidance / défense
- [x] Détail parade / riposte / esquive / bloc par combattant (évitements +
      attaques ratées avec % de précision) — gère aussi les formes `YOU parry`
- [x] Resists par école de magie (`X tries to burn Y with Z, but Y resists.`,
      verbe → école : burn=heat, freeze=cold… ; la forme `YOU try to` n'était
      pas parsée du tout avant)

### Confort UI
- [x] Tables triables par clic sur l'en-tête de colonne (dégâts, soins, power,
      breakdown par sort, comparaison)
- [x] Filtres texte : combattants, sorts, historique des encounters, log brut
- [x] Vue « log brut » d'un encounter (lignes sources filtrables, cap 5000,
      session courante uniquement — non persisté)
- [x] Profils d'overlay commutables (enregistrer/appliquer/supprimer dans
      Settings, boutons d'application rapide dans le menu clic droit)

- [x] **Mécaniques auto-apprises** : sans base de sorts, détection des capacités
      ennemies récurrentes/impactantes (AoE, tank buster, mortelles), mesure de la
      période et prédiction du prochain cast (alerte toast/son/voix, décompte overlay).
      Base à 3 sources (communautaire embarquée + apprise + manuelle), import/export,
      outil de minage hors-ligne `mine_mechanics`.
- [x] **Import de packs ACT** (.xml) : SpellTimers → mécaniques, CustomTriggers →
      triggers, en un bouton + outil `import_act`. Pack de triggers de base embarqué.

## 🟢 Plus tard / niche

- [ ] **Spell timers joueur** : durée des buffs/debuffs et recast de SES sorts
      (différent des mécaniques ennemies : nécessiterait une DB de sorts ou une
      mesure cast→fade, que les logs ne donnent pas toujours)
- [ ] Enrichir la base communautaire embarquée au fil des logs de raid récupérés
- [ ] Partage réseau du parse au groupe (serveur WebSocket local, overlay web OBS)
- [ ] Upload/partage web des parses (gist, pastebin, site dédié)
- [ ] Overheal / effective healing (si les logs le permettent)
- [ ] Support multi-langues du client (logs FR/DE — regex à dupliquer)
- [ ] Système de plugins (scripts Lua/Rhai ?)

## Notes techniques

- Les logs EQ2 ne contiennent **pas** l'overheal ni les buffs/débuffs posés :
  les spell timers devront être basés sur une DB de sorts externe (cast → durée connue).
- Le partage réseau type ACT utilise un protocole propriétaire ; on ferait plutôt
  un WebSocket JSON simple + page web overlay pour OBS.
- Pour le TTS Windows sans dépendance lourde : `windows` crate → `ISpVoice`,
  ou spawn `powershell -c Add-Type -AssemblyName System.Speech; ...` (latence à tester).
