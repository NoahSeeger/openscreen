//! Caméra 3D réelle, pan-tilt-zoom, pour `follow-cursor`.
//!
//! L'ÉCRAN NE BOUGE PAS. L'œil est posé une fois pour toutes, un peu à gauche et sous le centre de
//! l'écran ; la caméra pivote sur lui (lacet autour de la verticale du monde, puis tangage autour
//! de son propre axe horizontal, le `lookAt` classique à haut fixe) pour viser un point, et zoome
//! en resserrant son champ. Le roulis est donc nul PAR CONSTRUCTION : l'axe horizontal de l'image
//! reste horizontal dans le monde, et une verticale qui passe par le point visé reste verticale à
//! l'image. Ce qui penche encore, c'est la perspective, et elle seule (règle des 2° : cf. tests).
//!
//! Repère du monde : celui de l'écran, en px de sa boîte (`s_dst`, zoom compris), origine au
//! centre de l'écran, x à droite, y vers le bas, z vers l'œil ; l'écran est le plan z = 0. Prendre
//! la boîte zoomée comme unité revient à multiplier tout le monde ET la focale par le zoom : la
//! perspective (angles, fuite) ne change pas, l'image grossit d'autant autour du point principal.
//! C'est un zoom optique, sans aucun glissement 2D : le cadrage vient du seul pivot de la caméra.
//!
//! Le rendu passe par `TiltedQuad`, que les modes 8, 12, 13, 14 et 15 savent déjà dessiner : la
//! rotation de la caméra y est une rotation X (tangage) puis Y (lacet) du plan, exactement celle de
//! `regions::rotate_point`, plus une translation de l'image du centre de l'écran (`offset`),
//! puisque la caméra ne vise plus ce centre, et le warp devient projectif (`projective`).

use crate::cursor::CursorTrack;
use crate::regions::{CameraFrame, TiltedQuad};

/// Champ de l'objectif à zoom 1, mesuré sur le petit côté de la boîte écran (degrés), soit ~15° sur
/// un cadre paddé (la boîte en occupe ~80 %). Un objectif LONG, et c'est ce qui garde le texte
/// droit : une caméra qui tourne à la fois en lacet et en tangage penche les horizontales de
/// `atan(tan(lacet)·sin(tangage))` (ce n'est pas un roulis, les verticales restent droites), et
/// plus l'objectif est long, plus les pivots qu'il faut pour suivre le pointeur sont petits.
/// Mesuré : à 18° avec un repos de (7°, 5°), les lignes de texte penchaient de 2,5° vers le coin
/// haut-droit à zoom 2,2 ; à 12° avec le repos ci-dessous, 0,9° (cf.
/// `the_content_stays_level_along_the_path`).
pub const FOV_DEG: f32 = 12.0;
/// Pose de repos, (lacet, tangage) en degrés : l'œil à gauche du centre de l'écran, à sa hauteur ;
/// la caméra regarde donc vers la droite. Le côté gauche, plus proche, est ~9 % plus haut que le
/// droit : c'est le relief. Pas de tangage au repos : il s'ajouterait à celui du suivi et
/// pencherait tout le texte (cf. `FOV_DEG`).
pub const REST_DEG: [f32; 2] = [12.0, 0.0];
/// Débattement maximal du pivot autour du repos, (lacet, tangage) en degrés, borné en douceur.
pub const TURN_LIMIT_DEG: [f32; 2] = [12.0, 8.0];
/// Hauteur du point visé au-dessus de l'écran, en fraction du petit côté de la boîte : le pointeur
/// flotte juste au-dessus du contenu.
const AIM_LIFT: f32 = 0.01;

/// Ce que la caméra reçoit d'une frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    /// Poids de la caméra (0..1) : la force de la région de zoom. Les angles de repos en sont
    /// multipliés ; à 0, la caméra regarde l'écran de face et la projection est l'identité.
    pub weight: f32,
    /// Point visé dans l'écran (0..1 depuis son coin haut-gauche), déjà pondéré.
    pub aim: [f32; 2],
}

/// Une caméra posée : œil, orientation et focale, dans le repère du monde.
#[derive(Clone, Copy, Debug)]
pub struct View {
    eye: [f32; 3],
    /// Lacet et tangage (rad) : positif = vers la droite, vers le haut.
    yaw: f32,
    pitch: f32,
    rest: [f32; 2],
    /// Focale en px de sortie.
    focal: f32,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Direction de visée pour un lacet et un tangage.
fn forward(yaw: f32, pitch: f32) -> [f32; 3] {
    [yaw.sin() * pitch.cos(), -pitch.sin(), -yaw.cos() * pitch.cos()]
}

/// Lacet et tangage qui visent `target` depuis `eye`.
fn look_at(eye: [f32; 3], target: [f32; 3]) -> [f32; 2] {
    let d = sub(target, eye);
    [d[0].atan2(-d[2]), (-d[1]).atan2(d[0].hypot(d[2]))]
}

/// Borne douce : l'identité jusqu'aux trois quarts de `limit`, puis une tangente hyperbolique qui
/// n'atteint jamais `limit`. Dérivée continue au genou, donc pas d'à-coup quand elle s'engage.
fn soft_clamp(x: f32, limit: f32) -> f32 {
    let knee = 0.75 * limit;
    if x.abs() <= knee {
        return x;
    }
    let room = limit - knee;
    x.signum() * (knee + room * ((x.abs() - knee) / room).tanh())
}

impl View {
    /// La caméra qui filme une boîte écran de `box_px` px (zoom compris) sous `pose`.
    ///
    /// L'œil est à la distance où la focale de repos rend l'écran à sa taille, puis la focale est
    /// réduite juste assez pour que l'écran, vu de la pose de repos, tienne dans sa boîte : c'est
    /// le containment des angles fixes, mais mesuré une fois sur le repos, donc constant pendant
    /// que la caméra pivote (le plan ne respire pas).
    pub fn new(box_px: [f32; 2], pose: CameraPose) -> View {
        let w = pose.weight.clamp(0.0, 1.0);
        let [bw, bh] = box_px;
        let f0 = bw.min(bh) * 0.5 / (FOV_DEG.to_radians() * 0.5).tan();
        let eye = forward(REST_DEG[0].to_radians() * w, REST_DEG[1].to_radians() * w).map(|c| -f0 * c);
        let lift = AIM_LIFT * bw.min(bh);
        // Le repos vise le centre de l'écran, à la hauteur du pointeur.
        let rest = look_at(eye, [0.0, 0.0, lift]);
        let fit = View { eye, yaw: rest[0], pitch: rest[1], rest, focal: f0 }.fit(bw, bh);
        let [yaw, pitch] = look_at(eye, [(pose.aim[0] - 0.5) * bw, (pose.aim[1] - 0.5) * bh, lift]);
        let limit = TURN_LIMIT_DEG.map(f32::to_radians);
        View {
            eye,
            yaw: rest[0] + soft_clamp(yaw - rest[0], limit[0]),
            pitch: rest[1] + soft_clamp(pitch - rest[1], limit[1]),
            rest,
            focal: f0 * fit,
        }
    }

    /// Le facteur (≤ 1) qui fait tenir les quatre coins projetés dans la boîte.
    fn fit(&self, bw: f32, bh: f32) -> f32 {
        let (mut mx, mut my) = (0.0f32, 0.0f32);
        for [x, y] in corners(bw, bh) {
            if let Some([px, py]) = self.project([x, y, 0.0]) {
                (mx, my) = (mx.max(px.abs()), my.max(py.abs()));
            }
        }
        if mx > 0.0 && my > 0.0 { (bw * 0.5 / mx).min(bh * 0.5 / my).min(1.0) } else { 1.0 }
    }

    /// Axes de la caméra dans le monde : droite, bas, visée.
    fn basis(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let (sy, cy, sp, cp) = (self.yaw.sin(), self.yaw.cos(), self.pitch.sin(), self.pitch.cos());
        ([cy, 0.0, sy], [sy * sp, cp, -cy * sp], forward(self.yaw, self.pitch))
    }

    /// Un point du monde en px relatifs au point principal (le centre de la boîte) ; `None`
    /// derrière la caméra.
    pub fn project(&self, p: [f32; 3]) -> Option<[f32; 2]> {
        let (r, u, f) = self.basis();
        let q = sub(p, self.eye);
        let z = dot(f, q);
        (z > 1e-3).then(|| [self.focal * dot(r, q) / z, self.focal * dot(u, q) / z])
    }

    /// Le pivot par rapport au repos, (lacet, tangage) en degrés.
    pub fn turn_deg(&self) -> [f32; 2] {
        [(self.yaw - self.rest[0]).to_degrees(), (self.pitch - self.rest[1]).to_degrees()]
    }

    /// L'écran `box_px` vu par cette caméra, dans la convention de `TiltedQuad`.
    ///
    /// Un point `p` du plan, en px du plan (le monde multiplié par `scale`), vaut
    /// `w = R·p + (offset, 0)` dans le repère de la caméra et se projette en `P·w.xy / (P − w.z)`,
    /// avec `R = rotate_point(·, rot)` et `P = focal` : c'est la projection de `project`, le monde
    /// mis à l'échelle autour de l'œil pour que le centre de l'écran soit à la profondeur `P`.
    pub fn quad(&self, box_px: [f32; 2]) -> TiltedQuad {
        let (_, _, f) = self.basis();
        let project = |p: [f32; 3]| self.project(p).unwrap_or([p[0], p[1]]);
        let c = corners(box_px[0], box_px[1]).map(|[x, y]| {
            let [px, py] = project([x, y, 0.0]);
            (px, py)
        });
        let rot = [self.pitch.to_degrees(), self.yaw.to_degrees(), 0.0];
        TiltedQuad {
            corners: c,
            scale: self.focal / -dot(f, self.eye),
            depth_k: crate::regions::depth_coefficients(rot),
            rot,
            perspective: self.focal,
            offset: project([0.0; 3]),
            projective: true,
        }
    }
}

/// Coins TL, TR, BR, BL d'une boîte centrée.
fn corners(bw: f32, bh: f32) -> [[f32; 2]; 4] {
    let (hw, hh) = (bw * 0.5, bh * 0.5);
    [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
}

// ---- Le cadreur ----------------------------------------------------------------------------

/// Pas de la simulation, en secondes. Le ressort est intégré EXACTEMENT sur chaque pas : le pas ne
/// fixe que la cadence à laquelle la zone morte relit le pointeur.
const FOLLOW_STEP_S: f32 = 1.0 / 60.0;
/// Pulsation du ressort critique (rad/s) : 95 % du chemin en 0,8 s, sans dépassement.
const FOLLOW_OMEGA: f32 = 6.0;
/// Vitesse maximale de la cible, en vues par seconde. Le mode 8 n'a pas de flou de mouvement :
/// un pan d'un coin à l'autre à la vitesse du geste (140 px par image mesurés à 30 i/s) saccade.
/// La cible glisse donc vers le pointeur à ce rythme au plus, et le ressort en arrondit les bouts.
const FOLLOW_MAX_SPEED: f32 = 1.5;
/// Anticipation : le pointeur est lu en moyenne sur `[t, t + 1,2 s]`, soit 0,6 s devant. Un
/// enregistrement se monte après coup : la caméra part avant le geste, ce qui lui laisse le temps
/// d'arriver sans aller plus vite que `FOLLOW_MAX_SPEED`. Mesuré sur un aller-retour d'un coin à
/// l'autre de l'écran en 0,6 s au zoom 2,2 : le clic reste dans la vue, le pointeur n'en sort
/// que de 20 % de la demi-vue pendant le geste (0,4 s d'anticipation : le clic tombait dehors).
const FOLLOW_LOOKAHEAD_S: f32 = 1.2;
const FOLLOW_LOOKAHEAD_TAPS: usize = 7;
/// Zone morte, en fraction de la vue et centrée sur la cible : tant que le pointeur y reste, la
/// caméra ne bouge pas.
pub const DEAD_ZONE: f32 = 0.45;
/// Hystérésis : sortie de la zone morte, la caméra poursuit le pointeur jusqu'à le viser à moins de
/// cette fraction de la vue, puis se fige de nouveau.
const RELOCK: f32 = 0.05;

/// La vue comptée plus large que `1/zoom` de l'écran : vu de biais, l'écran est un peu plus petit
/// que sa boîte (containment du repos) et son côté lointain rétrécit encore. Sans cette marge, la
/// caméra tournée vers un bord laisse voir le fond au bord du cadre.
const VIEW_MARGIN: f32 = 1.1;

/// Un axe du cadreur : position visée (fraction de l'écran), vitesse, cible, et s'il poursuit.
#[derive(Clone, Copy)]
struct Axis {
    x: f32,
    v: f32,
    target: f32,
    tracking: bool,
}

impl Axis {
    fn at(p: f32) -> Axis {
        Axis { x: p, v: 0.0, target: p, tracking: false }
    }

    /// Où la cible veut aller : le pointeur quand l'axe le poursuit, sinon là où elle est.
    fn want(&mut self, pointer: f32, dead: f32, relock: f32) -> f32 {
        if !self.tracking && (pointer - self.target).abs() > dead {
            self.tracking = true;
        }
        if !self.tracking {
            return self.target;
        }
        if (self.x - pointer).abs() < relock {
            self.tracking = false;
        }
        pointer
    }

    /// Ressort critique vers `target`, solution exacte sur `dt` (cible constante).
    fn spring(&mut self, dt: f32) {
        let x = self.x - self.target;
        let e = (-FOLLOW_OMEGA * dt).exp();
        let k = self.v + FOLLOW_OMEGA * x;
        self.x = self.target + (x + k * dt) * e;
        self.v = (self.v - FOLLOW_OMEGA * k * dt) * e;
    }
}

/// Où `follow-cursor` vise à `t` (0..1 dans l'écran recadré), pour une région de zoom `zoom` dont
/// l'entrée commence à `anchor`. Sans piste : le centre.
///
/// Pure fonction de `t` : le cadreur est rejoué depuis `anchor` à chaque appel, à pas fixe, et le
/// dernier pas partiel prolonge le ressort jusqu'à `t` exactement, donc la visée est continue.
/// Aucune mémoire d'une frame à l'autre : lecture, seek et export donnent la même image.
///
/// - Zone morte de `DEAD_ZONE` de la vue autour de la cible, hystérésis `RELOCK`.
/// - Cible qui glisse vers le pointeur à `FOLLOW_MAX_SPEED` au plus, en ligne droite.
/// - Ressort critique (`FOLLOW_OMEGA`), anticipation `FOLLOW_LOOKAHEAD_S`.
/// - La visée reste là où la vue reste dans l'écran (`0,5 ± (0,5 − 0,5·VIEW_MARGIN/zoom)`) : la
///   caméra ne montre pas le fond au bord du cadre, comme le zoom plat.
///
/// ponytail: rejoue toute la région à chaque frame (60 pas par seconde de région) ; mémoriser le
/// dernier état par piste si des régions de plusieurs minutes pèsent sur la preview.
pub fn follow_aim(frame: &CameraFrame, anchor: f32, t: f32, zoom: f32) -> [f32; 2] {
    let Some(track) = frame.track else { return [0.5; 2] };
    let view = 1.0 / zoom.max(1.0);
    let reach = (0.5 - 0.5 * VIEW_MARGIN * view).max(0.0);
    let (dead, relock) = (0.5 * DEAD_ZONE * view, RELOCK * view);
    let glide = FOLLOW_MAX_SPEED * view * FOLLOW_STEP_S;
    let pointer = |s: f32| {
        lookahead(track, frame, s).map(|p| p.map(|c| c.clamp(0.5 - reach, 0.5 + reach)))
    };
    let Some(p0) = pointer(anchor) else { return [0.5; 2] };
    let mut axes = p0.map(Axis::at);
    for k in 0u32.. {
        let tk = anchor + k as f32 * FOLLOW_STEP_S;
        let dt = (t - tk).min(FOLLOW_STEP_S);
        if !(dt > 0.0) {
            break;
        }
        if let Some(p) = pointer(tk) {
            let want = [axes[0].want(p[0], dead, relock), axes[1].want(p[1], dead, relock)];
            let d = [want[0] - axes[0].target, want[1] - axes[1].target];
            let k = (glide / d[0].hypot(d[1]).max(1e-9)).min(1.0);
            for (axis, d) in axes.iter_mut().zip(d) {
                axis.target += d * k;
            }
        }
        for axis in &mut axes {
            axis.spring(dt);
        }
    }
    axes.map(|a| a.x)
}

/// Le pointeur moyen sur `[s, s + FOLLOW_LOOKAHEAD_S]`, borné à la fenêtre du clip, en fraction du
/// recadrage.
fn lookahead(track: &CursorTrack, frame: &CameraFrame, s: f32) -> Option<[f32; 2]> {
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for k in 0..FOLLOW_LOOKAHEAD_TAPS {
        let e = k as f32 / (FOLLOW_LOOKAHEAD_TAPS - 1) as f32;
        let at = (s + e * FOLLOW_LOOKAHEAD_S).max(frame.window[0]).min(frame.window[1]);
        let (px, py) = track.at(at)?;
        (x, y) = (x + px, y + py);
    }
    let n = FOLLOW_LOOKAHEAD_TAPS as f32;
    let [x0, y0, x1, y1] = frame.crop;
    Some([(x / n - x0) / (x1 - x0).max(1e-6), (y / n - y0) / (y1 - y0).max(1e-6)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: [f32; 2] = [1536.0, 864.0];

    fn view(aim: [f32; 2]) -> View {
        View::new(BOX, CameraPose { weight: 1.0, aim })
    }

    /// Une verticale du monde qui passe par le point regardé reste verticale à l'image, et
    /// l'horizontale de l'image reste horizontale dans le monde : roulis nul, partout, pivot borné
    /// compris.
    #[test]
    fn the_roll_is_zero_by_construction() {
        for ax in [-0.5f32, 0.2, 0.5, 0.8, 1.5] {
            for ay in [-0.5f32, 0.2, 0.5, 0.8, 1.5] {
                let v = view([ax, ay]);
                let (r, _, f) = v.basis();
                assert_eq!(r[1], 0.0, "l'axe horizontal de l'image quitte l'horizontale");
                let looked = [0, 1, 2].map(|k| v.eye[k] + 2500.0 * f[k]);
                for dy in [-400.0f32, -150.0, 150.0, 400.0] {
                    let p = v.project([looked[0], looked[1] + dy, looked[2]]).unwrap();
                    assert!(p[0].abs() < 2e-3, "{ax},{ay} dy {dy} : {p:?}");
                }
            }
        }
    }

    /// Le point visé tombe au point principal (quand le pivot n'est pas borné).
    #[test]
    fn the_camera_looks_at_its_aim() {
        let v = view([0.62, 0.41]);
        let aim = [0.12 * BOX[0], -0.09 * BOX[1], AIM_LIFT * BOX[1]];
        let p = v.project(aim).unwrap();
        assert!(p[0].abs() < 1e-2 && p[1].abs() < 1e-2, "{p:?}");
    }

    /// À poids nul, la caméra regarde l'écran de face et le rend à sa taille : le rendu plat.
    #[test]
    fn a_zero_weight_is_the_flat_layout() {
        let v = View::new(BOX, CameraPose { weight: 0.0, aim: [0.5; 2] });
        let q = v.quad(BOX);
        for (c, e) in q.corners.iter().zip(corners(BOX[0], BOX[1])) {
            assert!((c.0 - e[0]).abs() < 0.05 && (c.1 - e[1]).abs() < 0.05, "{c:?} {e:?}");
        }
        assert!((q.scale - 1.0).abs() < 1e-4 && q.rot == [0.0; 3], "{} {:?}", q.scale, q.rot);
    }

    /// Au repos, l'écran entier tient dans sa boîte, et la remplit sur un axe.
    #[test]
    fn at_rest_the_screen_fills_its_box() {
        for bx in [BOX, [864.0, 1536.0], [1000.0, 1000.0], [2560.0, 1080.0]] {
            let q = View::new(bx, CameraPose { weight: 1.0, aim: [0.5; 2] }).quad(bx);
            let (mx, my) = q.half_extents_px();
            assert!(mx <= bx[0] * 0.5 + 0.01 && my <= bx[1] * 0.5 + 0.01, "{bx:?} {mx} {my}");
            assert!(mx > bx[0] * 0.5 - 0.5 || my > bx[1] * 0.5 - 0.5, "{bx:?} {mx} {my}");
        }
    }

    /// Le pivot est borné à ±12° de lacet et ±8° de tangage autour du repos, sans à-coup.
    #[test]
    fn the_turn_is_softly_clamped() {
        let mut last = view([-1.0, 1.5]).turn_deg();
        for i in 0..=400 {
            let a = -1.0 + i as f32 / 200.0;
            let t = view([a, 0.5 - a]).turn_deg();
            assert!(t[0].abs() <= TURN_LIMIT_DEG[0] && t[1].abs() <= TURN_LIMIT_DEG[1], "{a} {t:?}");
            assert!((t[0] - last[0]).abs() < 0.3 && (t[1] - last[1]).abs() < 0.3, "{a} {t:?} {last:?}");
            last = t;
        }
        // Garde : la borne s'engage bien, loin hors de l'écran.
        assert!(view([-1.0, 1.5]).turn_deg()[0] < -0.9 * TURN_LIMIT_DEG[0]);
        assert_eq!(soft_clamp(0.1, 1.0), 0.1);
    }

    /// Le quad, sa translation, sa focale et sa rotation reproduisent la caméra : ce que le mode 15
    /// reconstruit (rayon par pixel) tombe exactement sur ce que le mode 8 dessine.
    #[test]
    fn the_quad_convention_reproduces_the_camera() {
        for aim in [[0.5, 0.5], [0.8, 0.3], [0.15, 0.9]] {
            let v = view(aim);
            let q = v.quad(BOX);
            let s = q.scale;
            for (fx, fy) in [(0.0, 0.0), (0.3, 0.7), (1.0, 1.0), (0.9, 0.1), (-0.05, 1.05)] {
                let p = [(fx - 0.5) * BOX[0], (fy - 0.5) * BOX[1], 0.0];
                let want = v.project(p).unwrap();
                let w = crate::regions::rotate_point(p.map(|c| c * s), q.rot);
                let (wx, wy) = (w[0] + q.offset[0], w[1] + q.offset[1]);
                let d = q.perspective - w[2];
                let got = [wx * q.perspective / d, wy * q.perspective / d];
                assert!(
                    (got[0] - want[0]).abs() < 0.05 && (got[1] - want[1]).abs() < 0.05,
                    "{aim:?} {got:?} {want:?}"
                );
                // Et la correspondance directe du warp (homographie des coins) est la même.
                let h = q.point_px(fx, fy);
                assert!(
                    (h.0 - want[0]).abs() < 0.05 && (h.1 - want[1]).abs() < 0.05,
                    "{aim:?} {h:?} {want:?}"
                );
            }
            // L'œil, dans le repère du plan, est celui de la caméra.
            let eye = crate::regions::rotate_point_inv(
                [-q.offset[0], -q.offset[1], q.perspective],
                q.rot,
            );
            for k in 0..3 {
                assert!((eye[k] - v.eye[k] * s).abs() < 0.1, "{aim:?} {eye:?} {:?}", v.eye);
            }
        }
    }

    /// Les poses extrêmes du cadreur : visée aux coins et aux milieux de la zone atteignable, à
    /// chaque zoom de l'app, pendant l'entrée aussi (le zoom et le poids montent ensemble, comme
    /// dans `zoom_state_in`). Boîte 16:9 à 80 % d'un cadre 1920×1080.
    fn extreme_quads() -> Vec<(f32, f32, [f32; 2], TiltedQuad)> {
        let mut out = Vec::new();
        for zoom in [1.25f32, 1.5, 1.8, 2.2, 3.5, 5.0] {
            let reach = (0.5 - 0.5 * VIEW_MARGIN / zoom).max(0.0);
            for k in 1..=10 {
                let weight = k as f32 / 10.0;
                let z = 1.0 + (zoom - 1.0) * weight;
                let bx = [BOX[0] * z, BOX[1] * z];
                for i in -2..=2 {
                    for j in -2..=2 {
                        let aim = [0.5 + i as f32 * reach / 2.0, 0.5 + j as f32 * reach / 2.0];
                        let pose = CameraPose { weight, aim: aim.map(|a| 0.5 + (a - 0.5) * weight) };
                        out.push((zoom, weight, aim, View::new(bx, pose).quad(bx)));
                    }
                }
            }
        }
        out
    }

    /// Le warp bilinéaire des angles fixes s'écarte de la projection exacte de plusieurs px sous
    /// cette caméra (mesuré sur la partie visible du cadre) : le warp projectif est nécessaire.
    #[test]
    fn the_bilinear_warp_is_too_far_from_the_camera() {
        let bilinear = |c: &[(f32, f32); 4], u: f32, v: f32| {
            let top = (c[0].0 + (c[1].0 - c[0].0) * u, c[0].1 + (c[1].1 - c[0].1) * u);
            let bottom = (c[3].0 + (c[2].0 - c[3].0) * u, c[3].1 + (c[2].1 - c[3].1) * u);
            (top.0 + (bottom.0 - top.0) * v, top.1 + (bottom.1 - top.1) * v)
        };
        let mut worst_by_zoom: Vec<(f32, f32)> = Vec::new();
        for (zoom, _, _, q) in extreme_quads() {
            for i in 0..=40 {
                for j in 0..=40 {
                    let (u, v) = (i as f32 / 40.0, j as f32 / 40.0);
                    let exact = q.point_px(u, v);
                    if exact.0.abs() > 960.0 || exact.1.abs() > 540.0 {
                        continue;
                    }
                    let b = bilinear(&q.corners, u, v);
                    let e = (b.0 - exact.0).hypot(b.1 - exact.1);
                    match worst_by_zoom.last_mut() {
                        Some((z, w)) if *z == zoom => *w = w.max(e),
                        _ => worst_by_zoom.push((zoom, e)),
                    }
                }
            }
        }
        println!("écart bilinéaire / projectif, pire px visible par zoom : {worst_by_zoom:?}");
        assert!(worst_by_zoom.iter().all(|&(_, w)| w > 0.5), "{worst_by_zoom:?}");
    }

    /// Règle des 2°. Le roulis est nul : une arête ne penche que par la perspective. Mesuré sur tout
    /// le chemin du cadreur, pour chaque arête VISIBLE dans le cadre : son écart à l'axe le plus
    /// proche, et, quand il passe sous 2°, où elle se trouve. Une arête quasi droite ne doit jamais
    /// traverser le milieu du cadre, où elle se lirait comme une troncature.
    #[test]
    fn the_edges_only_slope_through_perspective() {
        let (half_w, half_h) = (960.0f32, 540.0f32);
        let (mut min_angle, mut max_angle) = (f32::MAX, 0.0f32);
        let (mut worst_inset, mut worst_at) = (0.0f32, String::new());
        for (zoom, weight, aim, q) in extreme_quads() {
            let c = q.corners;
            for k in 0..4 {
                let (a, b) = (c[k], c[(k + 1) % 4]);
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                // Arêtes paires (haut, bas) horizontales, impaires verticales.
                let deg = if k % 2 == 0 { dy.atan2(dx.abs()) } else { dx.atan2(dy.abs()) }
                    .to_degrees()
                    .abs();
                // La partie visible : l'arête coupée au cadre (échantillonnée).
                let visible: Vec<(f32, f32)> = (0..=64)
                    .map(|i| {
                        let t = i as f32 / 64.0;
                        (a.0 + dx * t, a.1 + dy * t)
                    })
                    .filter(|p| p.0.abs() <= half_w && p.1.abs() <= half_h)
                    .collect();
                if visible.len() < 2 {
                    continue;
                }
                min_angle = min_angle.min(deg);
                max_angle = max_angle.max(deg);
                if deg < 2.0 {
                    // Distance au bord du cadre, en fraction de la demi-largeur (ou hauteur).
                    let inset = visible
                        .iter()
                        .map(|p| if k % 2 == 0 { 1.0 - p.1.abs() / half_h } else { 1.0 - p.0.abs() / half_w })
                        .fold(0.0f32, f32::max);
                    if inset > worst_inset {
                        worst_inset = inset;
                        worst_at = format!("arête {k} à {deg:.2}°, zoom {zoom}, poids {weight}, visée {aim:?}");
                    }
                    assert!(
                        inset < 0.35,
                        "arête {k} à {deg:.2}° à {:.0} % du bord (zoom {zoom}, poids {weight}, visée {aim:?})",
                        inset * 100.0
                    );
                }
            }
        }
        println!(
            "pente des arêtes visibles : {min_angle:.2}° .. {max_angle:.2}° ; sous 2°, jamais à plus de {:.0} % du bord vers le centre ({worst_at})",
            worst_inset * 100.0
        );
        assert!(max_angle < 8.0, "{max_angle}");
    }

    /// Ce qui se lit comme « penché » : l'horizontale du contenu au centre de la vue. Nulle en
    /// roulis, elle ne penche que du produit lacet × tangage, borné sur tout le chemin : moins d'un
    /// degré jusqu'au zoom 2,2 (1,8 par défaut), moins de 1,7° au zoom maximal.
    #[test]
    fn the_content_stays_level_along_the_path() {
        let mut worst: Vec<(f32, f32)> = Vec::new();
        for zoom in [1.25f32, 1.5, 1.8, 2.2, 3.5, 5.0] {
            let reach = (0.5 - 0.5 * VIEW_MARGIN / zoom).max(0.0);
            let bx = [BOX[0] * zoom, BOX[1] * zoom];
            let mut w = 0.0f32;
            for i in -4..=4 {
                for j in -4..=4 {
                    let aim = [0.5 + i as f32 * reach / 4.0, 0.5 + j as f32 * reach / 4.0];
                    let v = View::new(bx, CameraPose { weight: 1.0, aim });
                    w = w.max((v.yaw.tan() * v.pitch.sin()).atan().to_degrees().abs());
                }
            }
            worst.push((zoom, w));
        }
        println!("pente du contenu au centre de la vue, par zoom : {worst:?}");
        for (zoom, w) in worst {
            assert!(w < if zoom <= 2.2 { 1.0 } else { 1.7 }, "zoom {zoom} : {w:.2}°");
        }
    }

    fn track(at: impl Fn(f32) -> (f32, f32)) -> CursorTrack {
        CursorTrack::new(
            (0..=1800)
                .map(|i| {
                    let t = i as f32 / 30.0;
                    let (x, y) = at(t);
                    (t, x, y)
                })
                .collect(),
            vec![],
            vec![],
        )
    }

    fn whole(track: &CursorTrack) -> CameraFrame<'_> {
        CameraFrame { track: Some(track), crop: [0.0, 0.0, 1.0, 1.0], window: [0.0, 100.0] }
    }

    /// Lecture, seek arrière, sauts : même visée pour le même `t`, et aucune frame ne saute.
    #[test]
    fn the_aim_is_a_pure_continuous_function_of_time() {
        let tr = track(|t| (0.5 + 0.4 * (t * 1.3).sin(), 0.5 + 0.3 * (t * 0.7).cos()));
        let f = whole(&tr);
        let at = |i: usize| follow_aim(&f, 1.0, 1.0 + i as f32 / 60.0, 2.0);
        let forward: Vec<_> = (0..600).map(at).collect();
        for i in (0..600).rev().step_by(7) {
            assert_eq!(at(i), forward[i], "frame {i}");
        }
        // Jamais plus vite que `FOLLOW_MAX_SPEED` vues par seconde (la vue fait 1/2 de l'écran).
        let cap = FOLLOW_MAX_SPEED * 0.5 / 60.0;
        for pair in forward.windows(2) {
            let d = (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
            assert!(d <= cap * 1.02, "trop vite : {pair:?} ({d} > {cap})");
        }
    }

    /// Le pointeur qui bouge dans la zone morte ne fait rien ; sorti, la caméra le rejoint en
    /// ~1 s et le garde au centre de la vue une fois posée.
    #[test]
    fn the_dead_zone_holds_then_the_camera_settles_on_the_pointer() {
        let zoom = 2.0;
        let dead = 0.5 * DEAD_ZONE / zoom;
        // Petits gestes autour du centre (moins d'une zone morte d'écart avec la cible de départ,
        // anticipation comprise), puis un saut en bas à droite à 4 s.
        let tr = track(|t| {
            if t < 4.0 { (0.5 + 0.45 * dead * (t * 3.0).sin(), 0.5) } else { (0.72, 0.62) }
        });
        let f = whole(&tr);
        let start = follow_aim(&f, 0.0, 0.0, zoom);
        // L'anticipation lit 1,2 s devant : on s'arrête avant que le saut n'y entre.
        for i in 0..100 {
            let a = follow_aim(&f, 0.0, 1.0 + i as f32 / 60.0, zoom);
            assert_eq!(a, start, "la caméra a bougé dans la zone morte");
        }
        // Anticipation : la caméra part avant le saut.
        assert!(follow_aim(&f, 0.0, 3.95, zoom)[0] > 0.52);
        // Posée en ~1 s : une demi-vue à une vue par seconde, puis le ressort.
        let settled = follow_aim(&f, 0.0, 5.1, zoom);
        assert!((settled[0] - 0.72).abs() < 0.02 && (settled[1] - 0.62).abs() < 0.02, "{settled:?}");
        // Au repos, le pointeur est au centre de la vue, à l'hystérésis près.
        let late = follow_aim(&f, 0.0, 8.0, zoom);
        let r = RELOCK / zoom;
        assert!((late[0] - 0.72).abs() < r && (late[1] - 0.62).abs() < r, "{late:?}");
    }

    /// La visée reste là où la vue reste dans l'écran : jamais plus loin que
    /// `0,5 ± (0,5 − 0,5·VIEW_MARGIN/zoom)`, et au centre à zoom 1. Recadrage compris ; sans piste,
    /// le centre.
    #[test]
    fn the_aim_keeps_the_view_on_the_screen() {
        let corner = track(|_| (0.99, 0.01));
        for zoom in [1.0f32, 1.5, 2.0, 3.0] {
            let a = follow_aim(&whole(&corner), 0.0, 5.0, zoom);
            let reach = (0.5 - 0.5 * VIEW_MARGIN / zoom).max(0.0);
            assert!(
                (a[0] - (0.5 + reach)).abs() < 1e-3 && (a[1] - (0.5 - reach)).abs() < 1e-3,
                "{zoom} {a:?}"
            );
        }
        // Dans un recadrage [0,2 ; 0,4], x = 0,38 est tout à droite de ce qu'on voit.
        let tr = track(|_| (0.38, 0.5));
        let cropped = CameraFrame { crop: [0.2, 0.0, 0.4, 1.0], ..whole(&tr) };
        assert!(follow_aim(&cropped, 0.0, 5.0, 2.0)[0] > 0.72);
        assert!(follow_aim(&whole(&tr), 0.0, 5.0, 2.0)[0] < 0.4);
        assert_eq!(follow_aim(&CameraFrame::NONE, 0.0, 5.0, 2.0), [0.5; 2]);
    }

    /// Même sur une région d'une minute, rejouer le cadreur à chaque frame reste bon marché.
    #[test]
    fn a_long_region_stays_cheap() {
        let tr = track(|t| (0.5 + 0.4 * (t * 0.9).sin(), 0.5 + 0.3 * (t * 0.4).cos()));
        let f = whole(&tr);
        let t0 = std::time::Instant::now();
        let n = 20;
        for i in 0..n {
            std::hint::black_box(follow_aim(&f, 0.0, 60.0 - i as f32 * 0.01, 2.0));
        }
        let per_frame = t0.elapsed() / n;
        println!("cadreur, région de 60 s : {per_frame:?} par frame");
        assert!(per_frame < std::time::Duration::from_millis(20), "{per_frame:?}");
    }
}
