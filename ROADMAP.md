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
- [ ] Import/export de packs de triggers (partage communautaire, format XML ACT ?)

### Agrégat de session / zones
- [ ] Pseudo-encounter « Session entière » qui cumule tous les combats
- [ ] Parser `You have entered <Zone>.` → grouper l'historique par zone
- [ ] Stats par zone (durée totale, DPS moyen, morts)

## 🟡 À faire — priorité moyenne

### Détail par cible
- [ ] Mes dégâts ventilés par mob (utile sur les fights multi-adds)
- [ ] Vue tank : dégâts reçus ventilés par attaquant + par type de dégâts
- [ ] Matrice de soins : qui a soigné qui

### Breakdown avoidance / défense
- [ ] Détail parry / riposte / dodge / block / deflect par combattant
      (les données sont déjà comptées, il manque l'affichage)
- [ ] Taux d'évitement global (vue tank)
- [ ] Resists par école de magie

### Confort UI
- [x] Tables triables par clic sur l'en-tête de colonne (dégâts, soins, power,
      breakdown par sort, comparaison)
- [x] Filtres texte : combattants, sorts, historique des encounters
- [ ] Vue « log brut » d'un encounter (les lignes sources, filtrables)
- [ ] Profils d'overlay commutables (raid compact / solo détaillé)

## 🟢 Plus tard / niche

- [ ] **Spell timers** : barres de durée des buffs/debuffs et recast
      (nécessite une base de données des sorts EQ2 — gros chantier)
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
