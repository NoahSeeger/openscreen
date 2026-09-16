//! Les caméras 3D mobiles (`follow-cursor`, `swing-clicks`, `orbit`) rendues par le vrai
//! compositeur D3D11, à côté des angles fixes : le côté change vraiment, aucune arête ne longe
//! un axe de l'image, et le plan ne sort jamais du cadre.
//!
//! Piloté par l'environnement, comme `tilt_parallax_render.rs` (même source quadrillée) :
//!
//! ```powershell
//! ffmpeg -f lavfi -i "color=c=0x707070:s=1920x1080:r=30:d=6,drawgrid=w=96:h=96:t=3:c=white" -c:v h264_mf -b:v 6M -pix_fmt nv12 grid.mp4
//! $env:OPENSCREEN_CAMERA_SOURCE = "...\grid.mp4"
//! $env:OPENSCREEN_CAMERA_OUT = "...\renders"   # facultatif : un PNG par cas
//! cargo test -p openscreen-compositor --test camera_presets_render -- --nocapture
//! ```

// Windows seulement : le readback et le décodage D3D11VA de ce harnais n'existent que là.
#![cfg(windows)]

use openscreen_compositor::compositor::Compositor;
use openscreen_compositor::config;
use openscreen_compositor::cursor::CursorTrack;
use openscreen_compositor::d3d::Gpu;
use openscreen_compositor::frame_geometry::live_params_from_scene;
use openscreen_compositor::live::Player;
use openscreen_compositor::scene::Scene;

const W: u32 = 1920;
const H: u32 = 1080;

fn write_sidecar(path: &std::path::Path, at: &dyn Fn(f32) -> (f32, f32), clicks: &[f32]) {
    let samples: Vec<String> = (0..=180)
        .map(|i| {
            let t = i as f32 / 30.0;
            let (cx, cy) = at(t);
            let click = if clicks.iter().any(|c| (c - t).abs() < 1e-3) {
                r#","interactionType":"click""#
            } else {
                ""
            };
            format!(r#"{{"timeMs":{},"cx":{cx},"cy":{cy}{click}}}"#, (t * 1000.0).round())
        })
        .collect();
    std::fs::write(path, format!(r#"{{"samples":[{}]}}"#, samples.join(","))).expect("sidecar");
}

fn scene_json(source: &str, rotation: &str, start: f32, end: f32) -> String {
    let s = source.replace('\\', "/");
    format!(
        r##"{{
        "clips": [{{"screenPath":"{s}","webcamPath":"","sourceStartSec":0,"sourceEndSec":6,"webcamOffsetSec":0,"hasAudio":false}}],
        "layout": {{"preset":"no-webcam","webcamSize":1.0,"webcamShape":"rounded","webcamMirror":false,"webcamPosition":null,"webcamReactiveZoom":false,
                    "screenRect":{{"x":0.15,"y":0.15,"width":0.7,"height":0.7}}}},
        "effects": {{"padding":0.1,"blur":false,"shadow":0.5,"roundnessFrac":0.02,"motionBlur":0.0}},
        "background": {{"kind":"color","color":"#2060d0"}},
        "zoomRegions": [{{"id":"z","startSec":{start},"endSec":{end},"scale":1.15,"focusX":0.5,"focusY":0.5,"focusMode":"manual","rotation":"{rotation}"}}],
        "annotations": [],
        "speedRegions": [],
        "cursor": {{"show":true,"size":1,"smoothing":0,"motionBlur":0,"clickBounce":0,"clipToBounds":false,"theme":"default"}},
        "cropByClip": [null],
        "output": {{"width":{W},"height":{H},"fps":null}}
    }}"##
    )
}

/// Un pixel du plan : tout ce qui n'est pas le fond bleu (ombre comprise, qui reste bleue).
fn is_plane(rgba: &[u8], x: u32, y: u32) -> bool {
    let p = &rgba[((y * W + x) * 4) as usize..];
    !(p[2] as i32 > p[0] as i32 + 60)
}

fn silhouette_diff(a: &[u8], b: &[u8]) -> usize {
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| is_plane(a, x, y) != is_plane(b, x, y))
        .count()
}

/// La géométrie lue sur les pixels : boîte englobante, hauteur du plan vers sa gauche et vers
/// sa droite, et l'angle (degrés, contre la verticale) de ces deux bords, ajusté par moindres
/// carrés sur le tiers central de leur course.
struct Measured {
    bbox: [u32; 4],
    left_h: u32,
    right_h: u32,
    left_deg: f32,
    right_deg: f32,
}

fn measure(rgba: &[u8]) -> Measured {
    let cols: Vec<u32> = (0..W).filter(|&x| (0..H).any(|y| is_plane(rgba, x, y))).collect();
    let rows: Vec<u32> = (0..H).filter(|&y| (0..W).any(|x| is_plane(rgba, x, y))).collect();
    let bbox = [cols[0], rows[0], *cols.last().unwrap(), *rows.last().unwrap()];
    let height_at = |x: u32| (0..H).filter(|&y| is_plane(rgba, x, y)).count() as u32;
    let fit = |edge: &dyn Fn(u32) -> Option<u32>| {
        let (y0, y1) = (bbox[1] + (bbox[3] - bbox[1]) / 3, bbox[3] - (bbox[3] - bbox[1]) / 3);
        let pts: Vec<(f32, f32)> =
            (y0..y1).filter_map(|y| edge(y).map(|x| (y as f32, x as f32))).collect();
        let n = pts.len() as f32;
        let (my, mx) = (pts.iter().map(|p| p.0).sum::<f32>() / n, pts.iter().map(|p| p.1).sum::<f32>() / n);
        let slope = pts.iter().map(|p| (p.0 - my) * (p.1 - mx)).sum::<f32>()
            / pts.iter().map(|p| (p.0 - my).powi(2)).sum::<f32>();
        slope.atan().to_degrees()
    };
    let left_deg = fit(&|y| (0..W).find(|&x| is_plane(rgba, x, y)));
    let right_deg = fit(&|y| (0..W).rev().find(|&x| is_plane(rgba, x, y)));
    // Aux cinquièmes de la largeur : assez loin des coins pour que le roulis ne compte plus
    // (deux arêtes parallèles gardent un écart vertical constant), donc seule la fuite parle.
    let (w5, x0) = ((bbox[2] - bbox[0]) / 5, bbox[0]);
    Measured {
        bbox,
        left_h: height_at(x0 + w5),
        right_h: height_at(x0 + 4 * w5),
        left_deg,
        right_deg,
    }
}

struct Case {
    name: &'static str,
    rotation: &'static str,
    region: (f32, f32),
    at: fn(f32) -> (f32, f32),
    clicks: &'static [f32],
    t: f64,
}

fn left(_: f32) -> (f32, f32) {
    (0.15, 0.5)
}
fn right(_: f32) -> (f32, f32) {
    (0.85, 0.5)
}
fn high(_: f32) -> (f32, f32) {
    (0.5, 0.15)
}
fn low(_: f32) -> (f32, f32) {
    (0.5, 0.85)
}
/// À gauche jusqu'à 2,5 s, à droite ensuite (clic à 3 s).
fn moves_right(t: f32) -> (f32, f32) {
    (if t < 2.5 { 0.15 } else { 0.85 }, 0.5)
}

#[test]
fn the_moving_cameras_change_side_visibly_and_safely() {
    let Ok(source) = std::env::var("OPENSCREEN_CAMERA_SOURCE") else {
        println!("SKIP: definir OPENSCREEN_CAMERA_SOURCE (voir l'en-tete du fichier).");
        return;
    };
    let out_dir = std::env::var("OPENSCREEN_CAMERA_OUT").ok().map(std::path::PathBuf::from);
    let sidecar_dir = out_dir.clone().unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&sidecar_dir).expect("creer le dossier de sortie");

    let cases = [
        Case { name: "fixed-iso", rotation: "iso", region: (0.0, 6.0), at: left, clicks: &[], t: 3.0 },
        Case { name: "fixed-left", rotation: "left", region: (0.0, 6.0), at: left, clicks: &[], t: 3.0 },
        Case { name: "fixed-right", rotation: "right", region: (0.0, 6.0), at: left, clicks: &[], t: 3.0 },
        Case { name: "follow-left", rotation: "follow-cursor", region: (0.0, 6.0), at: left, clicks: &[], t: 3.0 },
        Case { name: "follow-right", rotation: "follow-cursor", region: (0.0, 6.0), at: right, clicks: &[], t: 3.0 },
        Case { name: "follow-high", rotation: "follow-cursor", region: (0.0, 6.0), at: high, clicks: &[], t: 3.0 },
        Case { name: "follow-low", rotation: "follow-cursor", region: (0.0, 6.0), at: low, clicks: &[], t: 3.0 },
        Case { name: "swing-before-click", rotation: "swing-clicks", region: (0.0, 6.0), at: moves_right, clicks: &[3.0], t: 2.9 },
        Case { name: "swing-mid-click", rotation: "swing-clicks", region: (0.0, 6.0), at: moves_right, clicks: &[3.0], t: 3.35 },
        Case { name: "swing-after-click", rotation: "swing-clicks", region: (0.0, 6.0), at: moves_right, clicks: &[3.0], t: 3.9 },
        Case { name: "orbit-start", rotation: "orbit", region: (0.5, 5.5), at: right, clicks: &[], t: 1.0 },
        Case { name: "orbit-middle", rotation: "orbit", region: (0.5, 5.5), at: right, clicks: &[], t: 3.0 },
        Case { name: "orbit-end", rotation: "orbit", region: (0.5, 5.5), at: right, clicks: &[], t: 5.45 },
    ];

    let gpu = Gpu::create(false).expect("device d3d11");
    let mut cfg = config::all().pop().expect("au moins une config");
    cfg.zoom = false;
    cfg.layout_anim = false;
    let mut frames = std::collections::HashMap::new();
    for case in &cases {
        let sidecar = sidecar_dir.join(format!("camera-{}-{}.cursor.json", case.name, std::process::id()));
        write_sidecar(&sidecar, &case.at, case.clicks);
        let track = CursorTrack::load(sidecar.to_str().expect("chemin utf-8"), 0.0, 6.0).expect("piste");
        let _ = std::fs::remove_file(&sidecar);
        let comp = Compositor::new_sized(&gpu, W, H).expect("compositor");
        let scene = Scene::from_json(&scene_json(&source, case.rotation, case.region.0, case.region.1))
            .expect("scene valide");
        comp.set_live_params(live_params_from_scene(&scene));
        comp.set_scene(Some(scene));
        comp.set_cursor(track.smoothed(0.0));
        let rgba = unsafe {
            let mut player = Player::open(&source, "", &gpu).expect("ouvrir la source");
            player.present_frame(&comp, &cfg, case.t).expect("composer la frame");
            comp.readback_resized(W, H).expect("readback")
        };
        if let Some(dir) = &out_dir {
            image::save_buffer(dir.join(format!("{}.png", case.name)), &rgba, W, H, image::ColorType::Rgba8)
                .expect("ecrire le png");
        }
        let m = measure(&rgba);
        println!(
            "{:<19} bbox {:?}  hauteur G {:>4} D {:>4}  bord G {:>6.2}° D {:>6.2}°",
            case.name, m.bbox, m.left_h, m.right_h, m.left_deg, m.right_deg
        );
        // Jamais coupé par le cadre, jamais un bord vertical (règle des 2°, marge de mesure).
        assert!(m.bbox[0] > 0 && m.bbox[1] > 0 && m.bbox[2] < W - 1 && m.bbox[3] < H - 1, "{}", case.name);
        assert!(m.left_deg.abs() > 1.8 && m.right_deg.abs() > 1.8, "{} : bord quasi vertical", case.name);
        frames.insert(case.name, (rgba, m));
    }

    let px = |a: &str, b: &str| silhouette_diff(&frames[a].0, &frames[b].0);
    let side = |name: &str| {
        let m = &frames[name].1;
        m.left_h as f32 / m.right_h as f32
    };
    // Le côté change : à gauche le bord droit est le proche (plus haut), à droite l'inverse.
    assert!(side("follow-left") < 0.9 && side("follow-right") > 1.1, "{} {}", side("follow-left"), side("follow-right"));
    assert!(px("follow-left", "follow-right") > 40_000, "{}", px("follow-left", "follow-right"));
    // Même sens que les angles fixes.
    assert!(side("fixed-left") < 1.0 && side("fixed-right") > 1.0);
    // La hauteur se voit aussi, en moins fort.
    assert!(px("follow-high", "follow-low") > 3_000, "{}", px("follow-high", "follow-low"));
    // `swing-clicks` tient la pose jusqu'au clic, puis prend celle du clic.
    assert!(px("swing-before-click", "follow-left") < 200, "{}", px("swing-before-click", "follow-left"));
    assert!(px("swing-after-click", "follow-right") < 200, "{}", px("swing-after-click", "follow-right"));
    assert!(px("swing-mid-click", "swing-before-click") > 5_000);
    // `orbit` part du côté du pointeur et finit de l'autre.
    assert!(side("orbit-start") > 1.1 && side("orbit-end") < 0.9, "{} {}", side("orbit-start"), side("orbit-end"));
}
