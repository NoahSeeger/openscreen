# 3D effects — v2 : mouvement de caméra et curseur modélisé

Suite de `spec-3d.md` (PR 1 → 5b, toutes ouvertes). Deux demandes :

1. **plusieurs caméras 3D**, dont une où l'orientation suit la **position** du curseur ;
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

### A.2 Le modèle : une seule liste « caméra 3D »

**Révisé après test produit.** La première version séparait l'attitude (`rotationPreset`) du
mouvement (`cameraMotion` : `still`, `sway`, `follow`, `flip`). Rejetée : deux sélecteurs dont
les effets ne se distinguaient pas, et des bugs (cf. A.3.1). Il n'y a plus qu'**un** champ,
`rotationPreset`, et **un** sélecteur « 3D camera » :

| groupe | valeur | libellé (EN) | ce que ça fait |
|---|---|---|---|
| — | absent | Off | écran droit |
| Angle fixe | `iso` | Angled from above | tourné vers la gauche, plongée marquée |
| Angle fixe | `left` | Turned left | tourné vers la gauche |
| Angle fixe | `right` | Turned right | tourné vers la droite |
| Caméra mobile | `follow-cursor` | Follows the cursor | se tourne du côté du curseur |
| Caméra mobile | `swing-clicks` | Turns to each click | pivote vers chaque clic, tient entre deux |
| Caméra mobile | `orbit` | Slow orbit | balaie d'un côté à l'autre pendant le zoom |

Les trois angles fixes gardent leurs valeurs (rendu d'un projet existant inchangé à l'octet),
avec la parallaxe de vitesse. Les caméras mobiles n'ont pas de parallaxe (la pose bouge déjà) ;
l'impact du clic (réglage à part) s'ajoute à tous les présets, dans le même budget. Sous la
liste, une ligne dit ce que fait l'option choisie.

« Turned right » veut dire que la face de l'écran regarde vers la droite : le bord droit
recule. C'est ce que fait `right` [−8, 16, 1] depuis toujours ; « From the right » aurait
décrit l'inverse.

### A.3 Les caméras mobiles

Les trois partagent **un** chemin de poses, `regions::camera_pose(u, v)`, `u` le côté (−1
gauche, +1 droite), `v` la hauteur (−1 haut, +1 bas) :

```
X = −3 − 2,5·v        plongée plus marquée quand le curseur est bas (l'écran se tourne vers lui)
Y = 16·u              toute l'amplitude de left/right : la caméra change vraiment de côté
Z = −(5,5 + 3·u²)     roulis anti-horaire, de signe constant, −5,5° au centre, −8,5° aux bords
```

**Pourquoi un roulis.** Tant qu'aucune arête ne franchit son axe, le signe de l'angle de chaque
arête est constant le long d'un chemin continu. À Y = 0, les deux arêtes horizontales ont le
signe de Z (une rotation X seule les laisse horizontales) ; à |Y| = 16 avec un petit Z, elles ont
des signes opposés, et opposés dans l'autre sens de l'autre côté (`left` : haut −, bas + ;
`right` : haut +, bas −). **Aucun chemin continu ne relie `left` à `right` sans qu'une arête
horizontale passe par 0°** : c'est exactement pourquoi l'ancien `flip` sautait. Recherche
numérique (toutes les familles X, Y, Z, impact ±1,9° compris) : la seule famille qui franchit
Y = 0 est celle où Z domine toutes les fuites, les quatre arêtes penchant du même côté que le
roulis. D'où |Z| ≥ ~5,5 au centre et ~8,5 aux bords, et une plongée X modeste (la fuite
verticale qu'elle crée doit rester sous le roulis).

**Échelle gelée.** L'échelle de containment d'une caméra mobile est le minimum sur toute
l'enveloppe (`moving_envelope_scale` : 5 × 2 poses × 4 coins d'impact, porte d'ease-in
comprise), constante pendant le zoom : le plan ne respire pas. Prix : il est ~18 % plus petit
qu'un angle fixe (0,674 contre 0,82–0,84 en 16:9).

Mesuré à l'échelle gelée sur la grille fine (41 × 9 poses × 25 impacts) : arête la plus proche
d'un axe à 2,45° en 16:9, 2,31° en 16:10, 2,26° en 21:9, 2,09° en 4:3
(`the_moving_sweep_keeps_every_edge_off_axis`), aucun débordement sur cinq formats
(`the_moving_sweep_stays_inside_at_one_frozen_scale`). En portrait la règle des 2° ne tient
pas, comme pour les présets fixes (elle n'est posée qu'en paysage).

Le curseur se lit dans l'**image source recadrée** (`CameraFrame::crop`), jamais dans la coupe
zoomée. `aim_uv` ramène chaque axe à [−1, 1] par un smoothstep sur 15 %..85 % : « le curseur
dans la partie droite » donne presque toute la pose de droite.

- **`follow-cursor`** : moyenne de la piste sur une fenêtre de Hann de 1,4 s (24 échantillons,
  retard ~0,7 s), bornée à la fenêtre du clip. Pure fonction de `t`, sans à-coup.
- **`swing-clicks`** : pose du curseur au début du zoom, puis à chaque clic (dans la fenêtre du
  clip et le recadrage) un pivot smoothstep de 0,7 s vers la pose du clic. Les clics sont
  rejoués depuis le début de la région à chaque frame ; un clic en plein pivot repart de la
  pose atteinte.
- **`orbit`** : de `u = ±1` (côté du curseur au début du zoom) à `u = ∓1`, smoothstep sur la
  durée de la région, `v = 0`.

Entre deux caméras mobiles chaînées, on interpole `(u, v)`, pas les angles : la pose reste sur
le chemin. Sans piste (curseur masqué : l'export ne la charge pas), `follow-cursor` et
`swing-clicks` tiennent la pose de face, `orbit` part de la gauche.

### A.3.1 Ce qui clochait dans `cameraMotion`

- **`flip` clignotait en focus auto** : le seuil était le centre de la coupe zoomée, que le focus
  auto place sur le curseur lissé. Le côté se jouait au bruit flottant : 44 bascules miroir en
  6 s sur une dérive lente (mesuré avant suppression). En focus manuel, une bascule sèche d'une
  frame, lue comme un glitch.
- **`follow` ne faisait rien en focus auto** (même cause : position mesurée dans la coupe
  zoomée), et plafonnait au budget dynamique (±1,9° / ±3°) en manuel : à peine visible.
- **`still` et `sway` indiscernables** : la parallaxe d'une dérive lente culmine à ~1,2°.
- **Saut en fin de transition chaînée** : le mouvement était pris sur la région sortante pendant
  toute la transition, puis sur l'entrante — un `flip` suivi d'un autre mouvement sautait de
  pose à la dernière frame.
- **Réglage mort sans curseur** : curseur masqué, aucune piste à l'export, donc aucun mouvement,
  sans que le sélecteur le dise. Le nouveau sélecteur l'écrit sous la liste.
- `iso` [−12, −18, −2] et `left` [−8, −16, −1] se ressemblent (tous deux tournés vers la gauche) :
  inchangés pour ne pas modifier les projets existants, mais les libellés le disent désormais.

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
(`VirtualPreview.tsx`) ne porte **aucun** tilt aujourd'hui. Pour une caméra mobile,
`getRotation3D` rend une pose représentative (`camera_pose(0, 0)` = [−3, 0, −5,5]) : le chemin
canvas n'a pas la piste curseur.

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
| **6** | `feat(zoom): add moving 3D camera presets` | `rotationPreset` étendu (`follow-cursor`/`swing-clicks`/`orbit`), `camera_pose` dans `regions.rs`, un seul sélecteur, i18n ×14 | chantier 3D (PR 1, 2b) |
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
