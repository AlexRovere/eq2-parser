# Onglet Optimisation : modèle, usage et évolutions

Documentation de référence de l'onglet **🎯 Optimisation** (priorité de sorts /
rotation). Sert de base pour comprendre les métriques et pour les prochaines
itérations (notamment la Piste C : rotation live).

## 1. À quoi ça sert

Classer les sorts offensifs de ton personnage par **efficacité**, pour répondre
à deux questions :

- **« Quand j'ai un créneau libre, je lance quoi ? »** : colonne **Eff/GCD**.
- **« Qu'est-ce qui pèse vraiment dans mon DPS sur la durée ? »** : colonne
  **DPS soutenu**.

C'est l'équivalent automatique du classeur Excel « EQ2 Spell Efficiency », mais
nourri par tes vrais logs plutôt que par une saisie manuelle.

## 2. Les notions clés

| Terme | Définition |
|---|---|
| **GCD** (Global Cooldown) | Temps qu'occupe un cast avant le suivant = `cast effectif + recovery`. Plancher 0,5s (le minimum EQ2). |
| **Cast effectif** | `cast_base / (1 + casting_speed%)`, borné à 0,5s. La casting speed réduit le temps d'incantation. |
| **Recovery** | Délai après le cast (0,5s par défaut, réglable). |
| **Reuse effectif** | `recast_base / (1 + reuse%)`. Le cooldown réel du sort. |
| **Intervalle utile** | Temps minimum entre deux casts *utiles* = `max(GCD, reuse, durée du DoT)`. Un DoT ne se relance qu'à sa durée ; un cooldown qu'à son reuse. |

## 3. Les deux métriques

- **Eff/GCD** = `dégâts au scénario / GCD`
  La valeur d'un cast sur un GCD libre. Sert de **priorité** : haut = bon à
  lancer maintenant. Pour un nuke spammable, c'est la métrique reine.

- **DPS soutenu** = `dégâts au scénario / intervalle utile`
  La contribution réaliste dans la durée. Borne les DoT (qui ne se relancent
  qu'à leur durée) et les cooldowns (limités par leur reuse). Pour un nuke
  spammable, Eff/GCD == DPS soutenu (pas de bridage).

Exemple : un DoT à 6442 dégâts/cast, GCD 1,5s, durée 24s →
Eff/GCD = 4295 (gros sur un GCD), DPS soutenu = 268 (ne tique qu'une fois / 24s).

## 4. Types de cible (couleurs EQ2)

| Type | Couleur | Sens | Scaling au nb de cibles |
|---|---|---|---|
| Mono | 🔴 rouge | une seule cible | ×1 |
| **AoE** | 🔵 bleu | zone au sol : touche tout dans la zone, même sans link | ×min(n, 8) toujours |
| **AE** | 🟢 vert | « area encounter » : ne touche que les mobs liés | ×n seulement si « cibles liées » coché |

Le scénario (mono / N cibles / liées) recalcule l'efficacité : le classement se
réordonne selon le combat (un AoE remonte en pack, retombe en mono).

## 5. Workflow (semi-auto)

1. **Classe** : auto-détectée par perso (à l'attache du log, depuis les sorts
   lancés) et persistée. Modifiable à la main.
2. **Sorts** : tous les dd/dot de la classe s'affichent (cast/recast/type
   pré-remplis depuis la base de 1365 sorts).
3. **Dégâts** :
   - **auto** depuis tes logs après un combat (valeur mesurée, en bleu) ;
   - **manuels** sinon : tu saisis la valeur du tooltip (planification à froid).
4. **Stats joueur** (par perso) : casting speed %, reuse %, recovery.
5. **Filtres scénario** : Mono / N cibles / cibles liées / **Combat type (s)**.

   Le réglage **Combat type (s)** escompte les **DoT** au prorata du temps de
   combat restant : un DoT de 24 s sur un combat de 6 s ne vaut que ~1/4 de ses
   dégâts (il n'a pas le temps de tiquer jusqu'au bout), donc il recule derrière
   les sorts directs. `0` = auto (médiane des durées de l'historique des
   encounters). En **rotation live**, le temps restant décroît avec l'encounter
   en cours : les DoT cessent d'être conseillés à l'approche de la mort de la
   cible. Mets une grande valeur (ou laisse l'historique la donner) pour un
   named long, une petite pour du trash qui fond.

Surcharges (cast ou dégâts édités à la main) en **orange** ; bouton
**↺ Réinit. surcharges** pour effacer. Case par sort pour **masquer** (renvoie
en bas, grisé). Tri par **clic sur les en-têtes**.

## 6. Le diagnostic

Compare ta rotation réelle à l'optimal (sur les sorts mesurés) :

- **Activité GCD** : % du combat passé à caster vs temps mort.
- **% de temps de cast à faible rendement**.
- **À mieux entretenir** : DoT / gros cooldowns sous-utilisés, avec leur
  **uptime** observé et les dégâts potentiels en plus. Restreint aux sorts qu'on
  vise à ~100 % (DoT à durée réelle, cooldowns gated) : un filler spammable ou un
  proc n'a pas de cible d'uptime, donc exclu.

## 7. Comment s'en servir en jeu (règle pratique)

L'onglet donne une **priorité**, pas un ordre à suivre aveuglément :

1. **Maintiens** tes DoT rentables (ils tiquent seuls, ne pas relancer avant la
   fin).
2. **Presse** tes gros cooldowns dès qu'ils sont prêts (⟳ orange).
3. **Comble** les GCD restants avec le plus haut **Eff/GCD** disponible.

## 8. Sources de données

- **Base de sorts** : `assets/spells.json` (1365 sorts, 294 offensifs), générée
  depuis `tools/extract_spells.py` (source wiki, cap 70 EoF). Trou connu : les
  sorts récents/TLE absents tombent en « cast inféré » (override manuel possible).
- **Mesures** : `src/optimizer.rs` (`Profiler`) reconstruit casts / cibles /
  cadence / crit depuis les logs ; `report()` produit le tableau ; `diagnose()`
  le bilan.

---

## 9. Piste C : rotation live (livrée)

Objectif : un overlay **« prochain sort »** qui dit en temps réel quoi caster,
en tenant compte de l'état réel du combat (DoT tombés, cooldowns prêts).

> **Statut : livré.** Overlay optionnel activable dans l'onglet Optimisation
> (« 🎯 Overlay rotation live »), fenêtre séparée always-on-top suivant le perso
> actif. Moteur `optimizer::next_casts` (file priorisée : gros cooldown prêt →
> DoT tombé → meilleur filler), état live `Profiler::last_casts` (détection des
> réapplications par trou de cadence). Réglages : nombre de sorts affichés,
> anticipation DoT (s). Limites V1 connues conservées (snapshot DoT, recast d'un
> DoT non tombé non détecté, granularité log à la seconde).

### Principe

Un moteur de priorité dynamique, réévalué à chaque tick :

1. **État suivi en direct** (par sort, pour le perso) :
   - DoT : reste-t-il du temps avant expiration ? (dernier cast + durée)
   - Cooldown : prêt ? (dernier cast + reuse effectif)
   - Disponibilité : hors GCD courant.
2. **Décision** à chaque GCD libre, par ordre (gros dégâts d'abord) :
   - un **gros cooldown prêt** (haute valeur) → le presser ;
   - un **DoT rentable tombé (ou sur le point)** → le rafraîchir ;
   - sinon le **filler au plus haut Eff/GCD** disponible.
3. **Sortie** : overlay dédié (réutiliser `show_mech_overlay` comme gabarit) :
   - en gros : le **sort à lancer maintenant** ;
   - en dessous : la **file** des 2-3 suivants ;
   - pastilles d'état (DoT bientôt tombé en orange, cooldown prêt en vert).

### Données nécessaires (déjà presque toutes là)

- cast/recast/durée/type : base de sorts (OK).
- casting/reuse : stats joueur (OK).
- état live des casts du perso : à ajouter dans le `Profiler` (suivre le dernier
  cast par sort en cours de combat, type `last_cast: HashMap<sort, epoch>` côté
  joueur, comme le `Learner` le fait pour les mécaniques).
- Eff/GCD par sort : déjà calculé par `report()`.

### Difficultés / limites

- **Snapshot des DoT** : EQ2 « fige » certains DoT au cast (snapshot des buffs).
  V1 peut l'ignorer (uptime simple).
- **Procs / réactifs** : non prévisibles, hors scope.
- **Granularité log à la seconde** : suffisant pour une priorité, pas pour du
  frame-perfect.
- Ce n'est pas un bot : ça **suggère**, ça ne joue pas à ta place.

### Découpage proposé

1. `Profiler` : suivi du dernier cast joueur par sort (live).
2. Moteur `next_casts(now, rows) -> Vec<Suggestion>` (priorité dynamique).
3. Overlay « prochain sort » (2e/3e viewport, gabarit `show_mech_overlay`).
4. Réglages : activer, taille, nb de suivants affichés.

---

## 10. Autres évolutions possibles

- **Enrichir la base** des sorts manquants (cast/recast) → moins de « inféré ».
- **Crit bonus** en mode manuel (comme le xlsx) pour coller au réel.
- **Scénarios nommés** (boss mono / trash AoE) commutables.
- **Groupement par rôle** dans le tableau (Entretenir / Sur cooldown / Filler).
