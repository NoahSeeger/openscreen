# 3D effects — v2 : mouvement de caméra et curseur modélisé

Suite de `spec-3d.md` (PR 1 → 5b, toutes ouvertes). Deux demandes :

1. **plusieurs mouvements de caméra**, dont un où l'orientation suit la **position** du curseur ;
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
| **6** | `feat(zoom): add camera motion presets` | `cameraMotion` (`still`/`sway`/`follow`/`flip`), lois dans `regions.rs`, sélecteur, i18n ×14 | chantier 3D (PR 1, 2b) |
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
