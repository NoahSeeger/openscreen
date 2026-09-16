# 3D effects — v2 : mouvement de caméra et curseur modélisé

Suite de `spec-3d.md` (PR 1 → 5b, toutes ouvertes). Deux demandes :

1. **plusieurs caméras 3D**, dont une où l'orientation suit la **position** du curseur ;
2. un **curseur modélisé** en vraie 3D — hauteur au-dessus du plan, ombre portée, contact au
   clic, orientation vers le point cliqué — en commençant par le curseur classique (la flèche
   du thème par défaut).

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

### B.1 Ce qui a été retiré

Deux essais précédents ne modélisaient rien, et ont été retirés sans compatibilité (ils n'ont
jamais atteint `main`) :

- `cursor.volume` (« 3D Depth ») empilait des copies du sprite 2D le long de la normale du plan ;
- `cursor.hover` (« Float Height ») décalait le sprite et posait une tache (mode 12) dessous.

Une carte découpée avec de l'épaisseur n'est pas un objet : pas de face éclairée, pas d'ombre
de sa forme, pas de contact. Un préréglage qui porte encore ces clés se lit toujours (les clés
inconnues sont ignorées).

### B.2 Le réglage

**Un seul interrupteur**, `cursor.model3d` (« 3D cursor », **éteint par défaut**) : il remplace la
flèche du thème par défaut par un modèle 3D. Tout autre état (`text`, `pointer`…) et tout autre
thème gardent leur sprite plat ; l'indice du panneau le dit (« Classic arrow only »). Curseur
masqué, l'interrupteur est grisé et son info-bulle dit pourquoi. Éteint, la frame est celle
d'avant **à l'octet** (vérifié à plat et incliné contre le commit de base).

Tuyauterie : `CursorVisualSettings.model3d`, clé legacy `cursorModel3d`, préréglages (absent →
éteint), `SceneCursor.model3d` (`serde(default)`), `LiveParams.cursor_model3d`, paramètre live
`cursorModel3d`.

### B.3 Le modèle (mode 15)

Un seul mode de shader, identique en HLSL, MSL et WGSL (`cursor_model`), lancé de rayons par
pixel dans la boîte de dessin :

- **Forme** : le contour de `cursors/default/arrow.png` en polygone de 10 sommets gonflé d'un
  arrondi (`ARROW_CORE`, `ARROW_ROUND`), ajusté sur l'alpha du PNG (1,1 % d'écart moyen de
  couverture). Unité = hauteur de la flèche (= `size_px`), origine au hotspot de la face du
  dessus. Extrudé de 0,19 unité, chanfrein arrondi de 0,045. Normales par gradient du champ.
- **Matières** : celles du PNG. Incrustation noire sur la face du dessus, filet blanc de
  0,058 unité autour, chanfrein et flancs blancs. Lumière fixée à la **caméra** (haut-gauche,
  devant), ambiante 0,36, diffuse 0,75, reflet sur les arrondis seulement : une face plane
  s'allumerait d'un bloc et le noir virerait au gris à chaque clic.
- **Ombre** : un rayon qui rate la flèche tombe sur le plan de l'écran. De là, marche vers la
  lumière (pénombre `k·d/t`, k = 6, bornée à 0,45 unité) et ombre de contact (0,12 unité autour
  du modèle). Opacité 0,5 chacune, et seulement à l'intérieur de l'écran.
- **Silhouette antialiasée** sur un pixel ; sortie prémultipliée, ombre noire.

### B.4 La caméra et l'ancrage

La caméra est reconstruite par pixel exactement comme `regions.rs` projette le plan : rotation
dessinée (`TiltedQuad::rot`, base + dynamique), Z puis Y puis X, perspective `P/(P − z)` avec
`P = min(w, h) × 1,6`, échelle de containment. Un écran droit prend un plan identité : même
caméra, même mode.

Deux décisions :

1. **Le hotspot est sur le rayon de vue** du point de contenu visé, à sa hauteur. La pointe ne
   glisse donc jamais à l'écran quand la flèche monte ou descend : seule l'ombre dit la hauteur.
2. **Ancrage** : la vidéo est dessinée par un warp **bilinéaire** des coins projetés, qui
   s'écarte de la perspective exacte de quelques pixels. Tout le rendu est décalé de
   `point_px(plane_pt) − projection exacte`, pour que la pointe tombe sur le pixel que
   l'écran montre.

### B.5 La pose (fonction pure de `t`)

`cursor_pose` :

- **Hauteur** : 0,35 unité de garde au repos. Chaque clic la pose **au contact** avec la courbe
  `tap()`, celle de l'impact du clic, dont le creux (49,5 ms) est celui de la pression de
  `bounce()`. Gain 1,25 : posée de 27 à 74 ms, donc au moins une image au contact jusqu'à
  21 i/s. Le point le plus bas du modèle affleure le plan (moins de 2 % d'unité, testé).
- **Tangage** : queue relevée, pointe vers le bas, 18° au repos, jusqu'à +10° au creux de la
  pression, fois `clickBounce / 2,5`.
- **Lacet** : vers la vitesse horizontale lissée (`follow_at`, différence centrée sur ±100 ms),
  et vers la cible d'un clic dans les 300 ms qui le précèdent. Borné en douceur à ±25°
  (`tanh`), nul au repos, continu en `t`.
- **Pas de rebond d'échelle** en 3D : le contact le remplace.

### B.6 Boîte, traînée, coût

- **Boîte de dessin** : les huit coins de la boîte du modèle, et leur projection au sol le long
  de la lumière, élargie de la pénombre (`min(t/k, 0,45) / lz`) et du contact. Un miroir CPU du
  shader vérifie qu'aucun pixel d'ombre n'en sort.
- **Traînée** : des copies du modèle sur GPU. Mesuré en 1080p sur RTX 4070 Ti : flèche nette
  dans le bruit (moins de 0,2 ms), taille 10 avec 16 copies +0,6 à +0,9 ms/frame. Sur WARP la
  même traînée coûte +68 à +98 ms : le backend logiciel ne dessine que la tête
  (`CursorPlan::for_backend`).
- **`LayerCB`** reste à 128 octets ; l'emploi des emplacements au mode 15 est documenté en tête
  de la section « Curseur modélisé » de `frame_geometry.rs` et dans les trois structs de shader.

### B.7 Limites

- Seule la flèche du thème par défaut a un modèle.
- Le repli math de Windows (mode 4, sans sprite) reste plat.
- L'ombre s'arrête au bord du rect de l'écran, pas à ses coins arrondis.
- Le MSL n'est compilé et exécuté que par la CI macOS.

---

## C. Découpage en PR

| PR | Titre | Contenu | Dépend de |
|---|---|---|---|
| **6** | `feat(zoom): add moving 3D camera presets` | `rotationPreset` étendu (`follow-cursor`/`swing-clicks`/`orbit`), `camera_pose` dans `regions.rs`, un seul sélecteur, i18n ×14 | chantier 3D (PR 1, 2b) |
| **7** | `feat(cursor): model the default arrow in 3D` | `cursor.model3d`, mode 15, pose, ombre, retrait de `volume`/`hover` | PR 6 |
| **8** | `feat(zoom): dolly the camera` | distance de fuite par région, dolly-zoom | PR 6 |
| **9** | `feat(cursor): the cursor picks things up` *(plus tard)* | long press : le plan reste pressé pendant un glisser (v1 §PR 2b « Later ») | 7 |

La PR 6 est CPU seulement : 0 shader, aucune nouvelle constante de buffer. Elle touche la
géométrie partagée par les trois backends, donc elle est testable sans GPU — c'est la
première raison de son ordre.

La PR 7 ajoute un mode de shader, donc elle se vérifie sur les trois backends : tests unitaires
Rust de la pose, du contact, de l'ancrage et de la boîte ; rendus D3D11 sur une frame NV12
synthétique ; le même test de rendu dans les modules Linux (lavapipe) et macOS (CI).

**État** : PR 6 et 7 écrites et testées. 8 et 9 restent à faire.
