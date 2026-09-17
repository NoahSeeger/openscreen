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

**Révisé deux fois après test produit.** La première version séparait l'attitude
(`rotationPreset`) du mouvement (`cameraMotion` : `still`, `sway`, `follow`, `flip`) ; la
deuxième fusionnait tout en un sélecteur avec trois caméras mobiles (`follow-cursor`,
`swing-clicks`, `orbit`) qui faisaient tourner **l'écran** sur un chemin de poses à roulis
permanent (5,5 à 8,5°). Rejetées : « pour chacune, le métrage est de travers ». Il reste **un**
champ, `rotationPreset`, et **un** sélecteur « 3D camera » :

| groupe | valeur | libellé (EN) | ce que ça fait |
|---|---|---|---|
| — | absent | Off | écran droit |
| Angle fixe | `iso` | Angled from above | tourné vers la gauche, plongée marquée |
| Angle fixe | `left` | Turned left | tourné vers la gauche |
| Angle fixe | `right` | Turned right | tourné vers la droite |
| Caméra mobile | `follow-cursor` | Follows the cursor | l'écran est immobile, une vraie caméra vise le curseur |

`swing-clicks` et `orbit` sont retirés (jamais livrés) : ils reviendront un par un sur le modèle
de caméra ci-dessous. Un projet qui les porte encore s'ouvre à plat (valeur inconnue).

Les trois angles fixes gardent leurs valeurs et leur rendu **à l'octet** (vérifié contre le
commit de base : présets, cadre, flou de confidentialité, profondeur de champ, parallaxe, impact
du clic, flèche modélisée), avec la parallaxe de vitesse et l'impact du clic.

« Turned right » veut dire que la face de l'écran regarde vers la droite : le bord droit
recule. C'est ce que fait `right` [−8, 16, 1] depuis toujours.

### A.3 `follow-cursor` : une caméra pan-tilt-zoom

`crates/compositor/src/camera.rs`. **L'écran ne bouge pas** : c'est le plan z = 0 du monde, en
px de sa boîte. **L'œil ne bouge pas** non plus. La caméra pivote sur lui pour viser le
pointeur, un point juste au-dessus du contenu, et zoome en resserrant son champ.

- **Orientation** : `lookAt` à haut fixe, lacet puis tangage. **Roulis nul par construction** :
  l'axe horizontal de l'image reste horizontal dans le monde, une verticale qui passe par le
  point visé reste verticale (mesuré au pixel sur le rendu D3D11 : ≤ 0,02°).
- **Objectif** : champ de 12° sur le petit côté de la boîte (~15° sur un cadre paddé).
- **Repos** : l'œil à 12° à gauche du centre, à sa hauteur (lacet 12°, tangage 0). Le bord
  gauche, plus proche, est ~9 % plus haut que le droit. Distance : celle où l'écran remplit sa
  boîte, puis la focale est réduite juste assez pour qu'il y tienne vu du repos (containment
  mesuré une fois, donc constant pendant que la caméra pivote).
- **Zoom** : la boîte grandit du facteur de zoom autour de son centre (pas de glissement 2D :
  le focus de la région est ignoré), ce qui revient à resserrer le champ. Force 0 = le rendu
  plat, par le même chemin (mode 0).
- **Pivot borné en douceur** à ±12° de lacet, ±8° de tangage autour du repos (identité jusqu'à
  75 % de la borne, puis `tanh`). Mesuré : 8,4° / 4,8° au pire au zoom 5, la borne ne s'engage
  pas sur le chemin normal.

**Pourquoi un objectif long et un repos sans tangage.** Une caméra qui tourne à la fois en lacet
et en tangage penche les horizontales de `atan(tan(lacet)·sin(tangage))`. Ce n'est pas un
roulis (les verticales restent droites), mais un écran plein de lignes de texte qui penchent
toutes du même côté se lit comme « de travers ». Premier réglage (champ 18°, repos 7° / 5°) :
2,5° de pente vers le coin haut-droit au zoom 2,2, visible. Réglage retenu, mesuré sur tout le
chemin (`the_content_stays_level_along_the_path`) : 0,17° au zoom 1,25, **0,66° au zoom 1,8
(défaut)**, 0,91° à 2,2, 1,37° à 3,5, 1,63° à 5.

**Le cadreur** (`follow_aim`), pure fonction de `t` : rejoué depuis le début de l'entrée de la
région à pas fixe (1/60 s), le dernier pas partiel prolongeant le ressort jusqu'à `t`.

- **Anticipation** : le pointeur moyen sur `[t, t + 1,2 s]` (7 échantillons, bornés à la fenêtre
  du clip), lu dans l'image source recadrée.
- **Zone morte** : 45 % de la vue autour de la cible. Sortie, la cible poursuit le pointeur
  jusqu'à le viser à moins de 5 % de la vue (hystérésis), puis se fige.
- **Vitesse** : la cible glisse vers le pointeur à 1,5 vue par seconde au plus, en ligne droite.
  Le mode 8 n'a pas de flou de mouvement : à la vitesse du geste, un pan d'un coin à l'autre
  montait à 140 px par image à 30 i/s et saccadait.
- **Ressort critique** ω = 6 rad/s (95 % en 0,8 s).
- **Portée** : la visée reste dans `0,5 ± (0,5 − 0,55/zoom)`, là où la vue reste dans l'écran
  (la marge de 10 % absorbe le containment et le rétrécissement du côté lointain). Zoom 1 : pas
  de pivot.
- Mesuré (portage Python du cadreur) sur un aller-retour d'un coin à l'autre en 0,6 s au
  zoom 2,2 : les clics restent dans la vue, le pointeur n'en sort que de 20 % de la demi-vue
  pendant le geste. Coût : ~5 ms par frame pour une région d'une minute en debug.

Sans piste (curseur masqué : l'export ne la charge pas), la caméra vise le centre, à son repos.

**Ce qui suit la caméra** : l'écran (mode 8), son ombre (mode 12), le cadre de fenêtre (mode 14),
le curseur plat (mode 13) et modélisé (mode 15), le flou de confidentialité (mode 10) et la
profondeur de champ, dont la mise au point suit le point visé. **L'impact du clic est coupé**
sous cette caméra : l'écran est immobile, le presser contredirait le modèle ; le panneau le dit.
L'ombre de l'écran tombe le long de la lumière de la flèche modélisée (haut-gauche) au lieu de
tomber droit, en glissant avec le poids de la caméra. Une lampe posée sur la caméra éclaire un peu
plus le côté proche (±4 %, `CAMERA_LIGHT_GAIN`).

Chaînée à un angle fixe, une région `follow-cursor` ne mélange jamais les deux modèles : la
transition passe par l'écran droit à mi-course.

**Rendu exact.** Le warp bilinéaire des angles fixes s'écarte de la projection de cette caméra de
39 px (zoom 1,25) à 183 px (zoom 5) au pire pixel visible. Les modes 8, 10, 13 et 14 prennent donc
un warp **projectif** exact sous cette caméra : l'homographie des quatre coins (forme de Heckbert)
résolue à l'envers dans le shader, drapeau par mode (`TiltedQuad::warp_flag` : `dst_prev.w` au
mode 8, `mb.w` au 10, `mb.x` au 13, `src.x` au 14). Mesuré sur une grille rendue par D3D11 :
0,04 px d'écart à la projection. `TiltedQuad` porte la caméra complète : rotation (tangage X,
lacet Y), distance `P` et translation de l'image du centre de l'écran (`offset`).

**Règle des 2°.** Le roulis est nul : une arête ne penche que par la perspective, de 0 à 2,1° sur
tout le chemin. Une arête sous 2° n'est visible qu'au bord du cadre, jamais à plus de 26 % de la
demi-largeur vers le centre (le bord droit, zoom 5, visée en haut à droite), avec ses coins
arrondis et son ombre : elle se lit comme le bord de l'écran, pas comme une troncature.

### A.3.1 Ce qui clochait avant

- **Les caméras mobiles sur l'écran tournant** (`camera_pose`) : le roulis que la règle des 2°
  imposait pour relier `left` à `right` rendait tout le métrage de travers.
- **`flip` clignotait en focus auto** : le seuil était le centre de la coupe zoomée, que le focus
  auto place sur le curseur lissé. Le côté se jouait au bruit flottant : 44 bascules miroir en
  6 s sur une dérive lente (mesuré avant suppression).
- **`follow` ne faisait rien en focus auto** (même cause), et plafonnait au budget dynamique
  (±1,9° / ±3°) en manuel : à peine visible.
- **`still` et `sway` indiscernables** : la parallaxe d'une dérive lente culmine à ~1,2°.
- **Réglage mort sans curseur** : curseur masqué, aucune piste à l'export. Le sélecteur l'écrit
  sous la liste.

### A.4 Ce qui n'est pas livré ici

- **`swing-clicks`, `orbit`** : à refaire sur `camera.rs` (l'œil qui se déplace pour l'orbite).
- **`dolly`** (vertigo) : la distance de l'œil par frame, que `TiltedQuad::perspective` sait
  déjà transporter.
- **Flou de mouvement sous la caméra** : le mode 8 n'en a pas, d'où la vitesse bornée.
- **Lumière du curseur modélisé** : elle reste fixée à la caméra ; sous un pivot de 8°, son ombre
  se déplace de quelques px sur l'écran. À fixer au monde avec le propriétaire du mode 15.

### A.5 Portée TS

Le natif porte la preview **et** l'export (`sceneDescription` → `scene.rs`). Le seul autre
consommateur de l'attitude est `getRotation3D` (`types.ts`), lu par
`computeRotation3DContainScale` via `zoomRegionUtils` — la preview CSS (`VirtualPreview.tsx`) ne
porte **aucun** tilt. Pour `follow-cursor`, `getRotation3D` rend l'angle de repos de la caméra
(Y = 12°) : le chemin canvas n'a ni la piste ni la caméra.

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
`P = TiltedQuad::perspective` (`min(w, h) × 1,6` sous un angle fixe), translation du plan dans le
repère caméra (`TiltedQuad::offset`, en `mb.zw`, nulle sous un angle fixe, cf. A.3), échelle de
containment. Un écran droit prend un plan identité : même caméra, même mode.

Deux décisions :

1. **Le hotspot est sur le rayon de vue** du point de contenu visé, à sa hauteur. La pointe ne
   glisse donc jamais à l'écran quand la flèche monte ou descend : seule l'ombre dit la hauteur.
2. **Ancrage** : sous un angle fixe, la vidéo est dessinée par un warp **bilinéaire** des coins
   projetés, qui s'écarte de la perspective exacte de quelques pixels. Tout le rendu est décalé
   de `point_px(plane_pt) − projection exacte`, pour que la pointe tombe sur le pixel que
   l'écran montre. Sous la caméra réelle le warp est exact et ce décalage est nul.

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
| **6** | `feat(zoom): add the follow-cursor 3D camera` | `rotationPreset` étendu (`follow-cursor`), caméra réelle dans `camera.rs`, warp projectif (modes 8, 10, 13, 14), un seul sélecteur, i18n ×14 | chantier 3D (PR 1, 2b) |
| **7** | `feat(cursor): model the default arrow in 3D` | `cursor.model3d`, mode 15, pose, ombre, retrait de `volume`/`hover` | PR 6 |
| **8** | `feat(zoom): dolly the camera` | distance de fuite par région, dolly-zoom | PR 6 |
| **9** | `feat(cursor): the cursor picks things up` *(plus tard)* | long press : le plan reste pressé pendant un glisser (v1 §PR 2b « Later ») | 7 |

La PR 6 touche surtout la géométrie partagée par les trois backends, testable sans GPU ; ses
shaders n'ajoutent qu'un warp projectif et une lampe, derrière un drapeau que les angles fixes
laissent à 0 (rendu inchangé à l'octet).

La PR 7 ajoute un mode de shader, donc elle se vérifie sur les trois backends : tests unitaires
Rust de la pose, du contact, de l'ancrage et de la boîte ; rendus D3D11 sur une frame NV12
synthétique ; le même test de rendu dans les modules Linux (lavapipe) et macOS (CI).

**État** : PR 6 et 7 écrites et testées. 8 et 9 restent à faire.
