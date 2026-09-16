# 3D effects — v2 : mouvement de caméra et curseur modélisé

Suite de `spec-3d.md` (PR 1 → 5b, toutes ouvertes). Deux demandes :

1. **plusieurs mouvements de caméra**, dont un où l'orientation suit la **position** du curseur ;
2. un **curseur modélisé** en vraie 3D — hauteur au-dessus du plan, ombre portée, contact au
   clic, orientation vers le point cliqué — en commençant par **deux** tracés compatibles
   (le curseur classique d'abord).

Ce document tranche les décisions que la v1 laissait ouvertes et découpe les PR.

---

## A. Le mouvement de caméra

### A.1 Ce qui existe

Un zoom porte **une attitude figée** (`rotationPreset` : `iso`, `left`, `right`), animée par
deux effets qui partagent un unique budget d'angle dynamique (`DYNAMIC_TILT_BUDGET`
= ±1,9° X, ±3,0° Y, 0° Z, `clamp_dynamic_tilt`) :

- **la parallaxe** (`dynamic_tilt`, PR 1) : pilotée par la **vitesse** lissée du curseur ;
- **l'impact du clic** (PR 2b) : piloté par la **position** du clic, `regions::tap`.

`rotated_quad_corners_px(w, h, base, dyn)` projette les coins ; l'échelle de containment est
calculée sur la **base seule** (« échelle gelée »), la part dynamique ne fait que reprojeter.

### A.2 Le modèle : attitude × mouvement

Une caméra, ce n'est pas un préset mais **deux** réglages indépendants :

| | champ | valeurs |
|---|---|---|
| **Attitude** (où penche le plan au repos) | `rotationPreset` (existant) | `iso`, `left`, `right` |
| **Mouvement** (comment elle bouge pendant le zoom) | `cameraMotion` (nouveau) | `still`, `sway`, `follow`, `flip` |

`cameraMotion` **absent = `sway`** : c'est exactement le rendu d'aujourd'hui (parallaxe de
vitesse). Aucun projet existant ne change. C'est le sens de « ajouter au iso » : le mouvement
se compose avec l'attitude au lieu de la remplacer, et chaque attitude existante gagne les
trois mouvements.

Pourquoi un **second champ** et pas des présets en plus dans la liste `iso/left/right` : la
liste actuelle dit *où penche le plan*, une valeur `follow` n'y dirait pas *sur quelle
attitude*. 3 attitudes × 4 mouvements = 12 rendus distincts pour **un** contrôle de plus ;
en présets, il faudrait 12 entrées dont 9 recopient une attitude. Le coût d'un champ est
d'ailleurs le même (schéma + migration + i18n) : c'est le seul endroit où v1 voyait une
économie, et elle n'existe pas.

Le contrôle : un `select` « Camera motion » **sous** le sélecteur 3D, désactivé (avec sa
raison) quand `rotationPreset` est `none` — rien à animer sur un plan droit, même règle que
`clickImpact`. Quatre libellés, pas de slider de plus.

### A.3 Les quatre mouvements

Signes : ceux déjà posés par la parallaxe et l'impact — **clic/déplacement à droite → +Y**
(le bord droit recule), **vers le bas → −X** (le bord bas recule).

**`still`** — aucune part dynamique. L'attitude tient, l'impact du clic reste actif (il a son
propre réglage). Le rendu des anciens présets, moins la parallaxe : la seule façon d'avoir un
plan parfaitement stable, aujourd'hui impossible.

**`sway`** (défaut) — la parallaxe de vitesse, inchangée, plus l'impact. Le rendu actuel.

**`follow`** — la **position** du curseur, pas sa vitesse. Dans la coupe visible (`cut`, repère
normalisé du curseur), le décalage du pointeur au centre :

```
rel = (p − c) / (demi-taille de la coupe)        ∈ [−1, 1]²
dyn = [−K_x · rel_y, +K_y · rel_x, 0]            K = DYNAMIC_TILT_BUDGET (le budget plein)
```

Borné et lissé par le même `tanh` que la parallaxe (pas de plateau sec aux bords), sommé à
l'impact du clic, puis `clamp_dynamic_tilt` et la porte d'ease-in s'appliquent comme avant.
Saturation douce **et** plancher : `tanh` seul laisse 0,0° au centre exact, où le plan
retrouve la pose d'un préset à l'arrêt — c'est voulu (c'est le même point que la pose
« au repos » de `sway`).

**Limite connue, à écrire dans l'UI** : en focus **auto**, la caméra cadre déjà le curseur, donc
le pointeur est au centre de la coupe et `rel ≈ 0` — l'effet est quasi nul. `follow` est un
mouvement de zoom **manuel** (le cas normal : on zoome sur un formulaire, le pointeur balaie
le cadre). C'est la raison pour laquelle v1 avait choisi la vitesse ; on garde les deux.

**`flip`** — l'attitude **change de côté** selon la moitié de la coupe où se trouve le pointeur :
`left` ↔ `right`, `iso` s'inverse en Y (`[-12, −18, −2] → [-12, +18, +2]`, le Z suivant pour
que la bascule reste une symétrie et non une rotation en plus).

Deux décisions, toutes deux structurelles :

- **ça agit sur la BASE, pas sur la part dynamique.** Passer de −18° à +18° demanderait +36°
  de dynamique, 12× le budget. `flip` est donc résolu dans `plan_frame` (`camera_base`), le
  seul endroit qui connaisse à la fois la pose, la piste curseur ET la coupe — `zoom_state_at`
  n'a pas la coupe, elle se calcule plus tard depuis le focus. Conséquence : le signe du champ
  de profondeur (`depth_k`) s'inverse aussi, et c'est correct — la profondeur de champ se
  déduit de la pose réellement dessinée.
- **bascule sèche, jamais d'interpolation.** Interpoler entre `left` et `right`
  traverse `Y = 0`, une pose où une paire d'arêtes est parallèle à un axe de l'image — le
  défaut qui a été rapporté trois fois comme « la troncature de l'enregistrement » (cf. v1,
  règle des 2°). Une bascule **instantanée** ne traverse rien : le plan saute dans l'autre
  pose à pleine force. Le côté est celui du pointeur **lissé** (`follow_at`) et rien d'autre :
  une bande morte demanderait de se souvenir du côté précédent, or une frame doit rester une
  pure fonction de `t` (même exigence que la parallaxe, cf. `regions.rs::camera_base`).
  L'échelle de containment ne saute pas : la projection du miroir a exactement les mêmes
  étendues, et `the_flipped_pose_is_the_mirror_of_its_preset` le vérifie au coin près — la
  règle des 2° est donc *transportée* par la symétrie, pas revérifiée à la main.

### A.4 Ce qui n'est pas livré ici, et pourquoi

- **`dolly`** (vertigo) : moduler la **distance de fuite** par frame. `PERSPECTIVE_FACTOR` est
  une constante, et c'est elle qui donne l'échelle de containment — le « gel » de v1 est
  exactement la machinerie qu'il faut pour un dolly-zoom (garder la taille projetée constante
  pendant que la perspective change). PR dédiée : le facteur devient un paramètre par région,
  et la géométrie le reçoit dans `TiltedQuad` (les trois backends sont déjà branchés dessus).
- **`roll`** : une rotation Z animée. Le budget Z vaut **0** aujourd'hui parce que c'est l'axe
  le plus étroit (~1° avant de casser la règle des 2°) — cette PR doit d'abord mesurer le
  budget Z réel, comme v1 l'avait fait pour X et Y.
- **`handheld`** : une vibration procédurale (somme de sinusoïdes, aucun curseur requis). Se lit
  comme un bug en dessous d'une certaine subtilité et casse la règle des 2° au-dessus : à
  calibrer sur un export réel avant de l'exposer.
- **`crane`** : l'inclinaison qui se pose à l'ease-in (angle qui décroît vers l'attitude). Le
  pendant de v1 PR 2b côté entrée ; même budget, même porte.

### A.5 Portée TS

Le natif porte la preview **et** l'export (`sceneDescription` → `scene.rs`). Le seul autre
consommateur de l'attitude est `getRotation3D` (`types.ts`), lu par
`computeRotation3DContainScale` via `zoomRegionUtils` — donc la preview CSS
(`VirtualPreview.tsx`) ne porte **aucun** tilt aujourd'hui. Rien à faire pour la parité :
`cameraMotion` est un champ du document, lu par le natif ; le chemin canvas garde l'attitude
seule et c'est déjà ce qu'il fait.

---

## B. Le curseur modélisé

### B.1 Ce qui existe

`CursorVolume` (PR 2) : le sprite est **extrudé** le long de la normale du plan (copies de la
silhouette dans `mb`, mode 13) et une ombre de **contact** (mode 12) est posée sous la pointe.
Réglage `cursor.volume`, 0 par défaut.

Le curseur est donc **posé sur le plan**, à plat dans le plan, et son ombre touche sa pointe.
Ce n'est pas un objet 3D : c'est une carte découpée avec de l'épaisseur.

### B.2 Ce que « modélisé » veut dire

Trois degrés de liberté de plus, **sans nouveau mode de shader** :

1. **La hauteur.** Le curseur flotte au-dessus du plan de `h` (fraction de la taille du sprite,
   `cursor.hover`, 0 par défaut). Le sprite se déplace de `n_xy · h · unité`, où `n_xy` est la
   normale projetée **déjà calculée** (`cursor_extrusion_px`) ; l'ombre, elle, reste **sur le
   plan**, à la verticale de la pointe. `h = 0` rend exactement le rendu d'aujourd'hui à
   l'octet — même contrat que le volume.
2. **Le contact.** Au clic, `h → 0` avec la courbe `tap()` de v1 (creux à 49,5 ms, retour nul à
   260 ms, même instant de contact que `bounce()`), puis remontée. Le curseur **touche**
   réellement le plan au lieu de le presser par une mise à l'échelle. L'ombre se resserre
   (rayon et flou ∝ `h`) : c'est elle qui vend le contact.
3. **L'orientation.** Le sprite est tourné (lacet/tangage autour de sa pointe) vers le point
   visé — au repos vers la direction du **mouvement** (le curseur s'incline dans son geste,
   comme un objet qu'on traîne), au clic vers le **point cliqué**. C'est un quad de quatre
   coins calculés en CPU sur le plan du sprite, projetés par la même perspective que l'écran :
   le mode 13 avale déjà quatre coins quelconques, aucun shader ne change.

L'ombre devient une vraie ombre **portée** : elle se déplace en sens inverse du décalage du
sprite (elle reste au sol), grossit et se floute avec `h`, et se confond avec la contact shadow
actuelle à `h = 0`.

### B.3 Les tracés compatibles (2 pour commencer)

Un pack = 16 tracés PNG (17 packs). Le traitement 3D suppose une silhouette **pleine et
convexe à la pointe** : les tracés troués, en trait fin ou pixelisés (la moitié du catalogue,
qui est décoratif) donneraient une extrusion en escalier et une ombre méconnaissable.

- **Livrés** : `arrow` (le curseur classique, priorité de la demande) et `pointer` (la main) —
  les deux seuls tracés présents dans **tous** les packs et les seuls qu'une démo utilise
  vraiment.
- **Repli** : tout autre tracé garde le volume v1 (extrusion + contact) et ignore hauteur,
  orientation et ombre portée. Un tracé inconnu n'est jamais un état d'erreur.

Le pack (le thème) reste orthogonal : ces deux tracés existent dans les 17 packs.

### B.4 Pièges

- **Ombre déjà peinte** : certains packs peignent leur ombre dans le PNG (v1 le documente pour
  l'extrusion) ; ici elle serait **projetée deux fois**, et à des endroits différents. À dire
  dans l'UI plutôt qu'à bloquer : la hauteur part de 0.
- **Le hotspot n'est pas la pointe** : le décalage de hauteur et la rotation se font autour de
  la pointe (`hotspot` du manifest), pas du centre du sprite — sinon le curseur « patine » sur
  sa propre image quand il tourne.
- **La traînée** : elle dessine N copies du sprite. L'ombre se dessine **une fois**, comme le
  fait déjà le volume (le code actuel le fait pour la contact shadow — garder ce chemin).
- **Repli math (Windows, mode 4)** : pas de sprite, donc pas de modèle — il reste droit, comme
  le volume aujourd'hui.
- **Coût** : la hauteur déplace le sprite, donc `cursor_bounds` (clip « Clip to canvas ») doit
  l'inclure, sinon un curseur qui flotte près du bord est coupé.

### B.5 Ce que la PR 7a a livré, et les trois écarts

Livré : `cursor.hover` (0..1, **0 par défaut** → le rendu d'avant, au bit près), la porte de
contact au clic, l'ombre portée et l'élargissement du clip « Clip to canvas ». Trois décisions
ont été prises en codant, et s'écartent de l'esquisse ci-dessus :

1. **La montée réutilise l'extrusion, inversée.** `e` est la normale *arrière* projetée — les
   flancs dépassent de ce côté-là — donc la hauteur, qui vient vers le spectateur, vaut `−ê`,
   à la longueur `0,75 × taille du sprite × échelle du plan` (`CURSOR_HOVER_LIFT_FRAC`). À
   plat, le repli bas-droite du volume fait donc monter le sprite vers le **haut-gauche** :
   l'ombre descend, le curseur s'élève, c'est le rendu d'un objet posé. L'ombre, elle, ne bouge
   pas d'un pixel : elle reste au pied de l'extrusion, c'est-à-dire au point que le curseur
   touche quand `h` s'annule — la même position que la contact shadow de v1, donc `hover = 0`
   reste identique au volume seul.
2. **La porte du contact est `1 + tap()`** — la courbe de `regions.rs`, déjà partagée avec
   l'impact du clic — et non un aller-retour symétrique : la hauteur s'annule à 49,5 ms puis
   **dépasse de 27 %** avant de retomber à 0 à 260 ms. Le dépassement est voulu (c'est le
   rebond d'un objet qu'on relâche) et il retombe à la fin de la fenêtre de `bounce` : sprite
   écrasé, plan basculé et pose du curseur lisent la même image. La condition d'activation lit
   le RÉGLAGE et non la hauteur instantanée — au creux celle-ci vaut 0, et rebasculer sur le
   chemin plat cette frame-là ferait disparaître l'ombre pile à l'instant où elle vend le
   contact.
3. **`taps = 1` quand il n'y a pas d'extrusion** : la hauteur seule n'a rien à extruder, et
   `mb.z ≤ 1` fait prendre au mode 13 sa branche plate, qui échantillonne déjà quatre coins
   quelconques. Deux copies identiques, elles, n'auraient fait qu'assombrir les bords
   antialiasés du sprite.

La hauteur est **agnostique du tracé** : elle ne suppose qu'un sprite et un `hotspot`, donc elle
vaut pour les 17 packs. C'est la rotation (7b) qui, elle, demandera une silhouette pleine et
convexe à la pointe.

Reste ouvert : l'ombre s'étale linéairement (`1 + 1,6·h`) et se dilue, il n'y a pas de projecteur
modélisé ; et un pack qui peint son ombre dans son PNG la voit toujours projetée deux fois.

---

## C. Découpage en PR

| PR | Titre | Contenu | Dépend de |
|---|---|---|---|
| **6** | `feat(zoom): add camera motion presets` | `cameraMotion` (`still`/`sway`/`follow`/`flip`), lois dans `regions.rs`, sélecteur, i18n ×14 | chantier 3D (PR 1, 2b) |
| **7a** | `feat(cursor): give the cursor height and contact` | `cursor.hover`, `tap` au clic, ombre portée, clip de bounds | PR 6 (indépendante en pratique) |
| **7b** | `feat(cursor): aim the modelled cursor` | rotation du sprite vers le geste / le point cliqué, tracés compatibles | 7a |
| **8** | `feat(zoom): dolly the camera` | distance de fuite par région, dolly-zoom | PR 6 |
| **9** | `feat(cursor): the cursor picks things up` *(plus tard)* | long press : le plan reste pressé pendant un glisser (v1 §PR 2b « Later ») | 7a |

La PR 6 est CPU seulement : 0 shader, aucune nouvelle constante de buffer. Elle touche la
géométrie partagée par les trois backends, donc elle est testable sans GPU — c'est la
première raison de son ordre.

La PR 7a l'est aussi, et pour la même raison : elle ne change **aucun** shader (le mode 12
dessine déjà un quad quelconque en ombre, le mode 13 accepte déjà quatre coins quelconques), et
elle non plus ne peut pas échouer en silence — les cinq tests unitaires Rust couvrent le sens de
la montée, le creux du contact, le clip et les copies, et le rendu Linux garde l'or de la v1.

**État** : PR 6 et 7a écrites et testées (branche du chantier 3D, non committées). 7b, 8 et 9
restent à faire.
