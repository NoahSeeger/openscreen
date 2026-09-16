//! La flèche modélisée (`cursor.model3d`, mode 15) rendue par le vrai compositeur D3D11.
//!
//! La source est une frame NV12 SYNTHÉTIQUE (une texture remplie ici, présentée comme une frame
//! D3D11VA) : rien à décoder, donc le test tourne sur toute machine qui a un GPU, sans variable
//! d'environnement. Sans adaptateur matériel, il se saute (« test saute »).
//!
//! ```powershell
//! $env:OPENSCREEN_CURSOR3D_OUT = "...\renders"   # facultatif : un PNG par cas
//! cargo test -p openscreen-compositor --test cursor_model_render -- --nocapture
//! # coût d'une frame 1080p, flèche 3D allumée contre éteinte (opt-in) :
//! $env:OPENSCREEN_CURSOR3D_BENCH = "1"
//! cargo test -p openscreen-compositor --release --test cursor_model_render bench -- --nocapture
//! ```

#![cfg(windows)]

use openscreen_compositor::compositor::Compositor;
use openscreen_compositor::config::Cfg;
use openscreen_compositor::cursor::CursorTrack;
use openscreen_compositor::d3d::Gpu;
use openscreen_compositor::ffi::AVFrame;
use openscreen_compositor::frame_geometry::{
    live_params_from_scene, plan_cursor, plan_frame, CursorPlacement, CursorPlanInput,
    FrameGeometryInput,
};
use openscreen_compositor::scene::Scene;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_WRITE,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DYNAMIC,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};

const SRC: (u32, u32) = (640, 360);
/// Instant rendu : en plein palier de la région de zoom (0..10 s).
const T: f32 = 2.0;
/// Creux de `regions::tap` : un clic à `T - CONTACT_S` montre la flèche posée à `T`.
const CONTACT_S: f32 = 0.0495;

fn gpu() -> Option<Gpu> {
    match Gpu::create(false) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("pas de device D3D11 matériel ({e:#}) — test saute");
            None
        }
    }
}

/// Une frame NV12 synthétique : `data[0]` = la texture, `data[1]` = tranche 0, comme une frame
/// D3D11VA (le seul contrat que `Compositor::nv12_srvs` lit). Un bleu moyen strié de barres
/// plus sombres : rien d'aussi clair ni d'aussi sombre que la flèche, et surtout rien de NEUTRE,
/// si bien qu'un pixel gris est la flèche et un pixel bleu assombri, son ombre.
struct FakeFrame {
    frame: Box<AVFrame>,
    _tex: ID3D11Texture2D,
}

impl FakeFrame {
    fn new(gpu: &Gpu) -> FakeFrame {
        let (w, h) = SRC;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
        };
        unsafe {
            let mut tex: Option<ID3D11Texture2D> = None;
            gpu.device.CreateTexture2D(&desc, None, Some(&mut tex)).expect("texture NV12");
            let tex = tex.expect("texture NV12");
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            gpu.context.Map(&tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m)).expect("Map");
            let pitch = m.RowPitch as usize;
            let dst = m.pData as *mut u8;
            for row in 0..h as usize {
                for col in 0..w as usize {
                    let bar = row % 24 >= 8 && row % 24 < 12 && (col / 40) % 3 != 2;
                    *dst.add(row * pitch + col) = if bar { 90 } else { 150 };
                }
            }
            for row in 0..(h / 2) as usize {
                for col in 0..w as usize {
                    let uv = (h as usize + row) * pitch + col;
                    *dst.add(uv) = if col % 2 == 0 { 150 } else { 120 };
                }
            }
            gpu.context.Unmap(&tex, 0);
            let mut frame: Box<AVFrame> = Box::new(std::mem::zeroed());
            frame.data[0] = tex.as_raw() as *mut u8;
            frame.data[1] = std::ptr::null_mut();
            frame.width = w as i32;
            frame.height = h as i32;
            FakeFrame { frame, _tex: tex }
        }
    }

    fn as_ptr(&self) -> *const AVFrame {
        &*self.frame as *const AVFrame
    }
}

fn arrow_path() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/cursors/default/arrow.png")
        .to_string_lossy()
        .replace('\\', "/")
}

/// `model3d` : `None` = clé absente (payload d'avant le réglage).
fn scene_json(rotation: &str, model3d: Option<bool>, theme: &str, motion_blur: f32, size: f32) -> String {
    let model3d = model3d.map(|m| format!(r#","model3d":{m}"#)).unwrap_or_default();
    let arrow = arrow_path();
    format!(
        r##"{{"clips":[{{"screenPath":"/s.mp4","webcamPath":"","sourceStartSec":0,"sourceEndSec":10,"webcamOffsetSec":0,"hasAudio":false}}],
            "layout":{{"preset":"no-webcam","webcamSize":1,"webcamShape":"rounded","webcamMirror":false,"webcamPosition":null,"webcamReactiveZoom":false,
                       "screenRect":{{"x":0.1,"y":0.1,"width":0.8,"height":0.8}}}},
            "effects":{{"padding":0.2,"blur":false,"shadow":0,"roundnessFrac":0.03,"motionBlur":0}},
            "background":{{"kind":"gradient","angleDeg":135,"stops":["#5b6ee1","#e8a0bf"]}},
            "zoomRegions":[{{"clipIndex":0,"startSec":0,"endSec":10,"scale":1,"focusX":0.5,"focusY":0.5,"focusMode":"manual","rotation":{rotation}}}],
            "annotations":[],
            "cursor":{{"show":true,"size":{size},"smoothing":0,"motionBlur":{motion_blur},"clickBounce":2.5{model3d},"clipToBounds":false,"theme":"{theme}",
                       "cursorSprites":{{"arrow":{{"path":"{arrow}","hotspotX":0.119,"hotspotY":0.0874}}}}}},
            "cropByClip":[null],
            "output":{{"width":1280,"height":720,"fps":30}}}}"##
    )
}

/// Piste curseur écrite dans un sidecar temporaire : `(t, x, y, clic)`.
fn track(name: &str, samples: &[(f32, f32, f32, bool)]) -> CursorTrack {
    let body: Vec<String> = samples
        .iter()
        .map(|&(t, x, y, click)| {
            let kind = if click { r#","interactionType":"click""# } else { "" };
            format!(r#"{{"timeMs":{},"cx":{x},"cy":{y}{kind}}}"#, t * 1000.0)
        })
        .collect();
    let path = std::env::temp_dir().join(format!("os_cursor_model_{name}.json"));
    std::fs::write(&path, format!(r#"{{"samples":[{}]}}"#, body.join(","))).expect("sidecar");
    CursorTrack::load(path.to_str().unwrap(), 0.0, 10.0).expect("piste curseur")
}

/// Une piste immobile en (0.45, 0.45), avec ou sans clic au creux du contact à `T`.
fn resting(name: &str, click: bool) -> CursorTrack {
    let mut s = vec![(0.0, 0.45, 0.45, false), (T - CONTACT_S - 0.2, 0.45, 0.45, false)];
    if click {
        s.push((T - CONTACT_S, 0.45, 0.45, true));
    }
    s.push((9.0, 0.45, 0.45, false));
    track(name, &s)
}

fn cfg() -> Cfg {
    let mut cfg = Cfg::c8();
    cfg.bg_blur = false;
    cfg.zoom = false;
    cfg.layout_anim = false;
    cfg.cursor = true;
    cfg.mblur_n = 1;
    cfg.shadow = false;
    cfg
}

/// Ce que la frame dit de la flèche : le pixel du contenu qu'elle vise et sa hauteur en px.
struct Probe {
    tip: [f32; 2],
    unit: f32,
}

fn render(comp: &Compositor, screen: &FakeFrame, json: &str, track: &CursorTrack) -> (Vec<u8>, Probe) {
    let (rgba, probe) = render_any(comp, screen, json, track);
    (rgba, probe.expect("un curseur à dessiner"))
}

/// `render`, sans exiger de curseur (scène au curseur masqué).
fn render_any(
    comp: &Compositor,
    screen: &FakeFrame,
    json: &str,
    track: &CursorTrack,
) -> (Vec<u8>, Option<Probe>) {
    let scene = Scene::from_json(json).expect("scène valide");
    let mut live = live_params_from_scene(&scene);
    live.has_webcam = false;
    comp.set_live_params(live);
    comp.set_has_webcam(false);
    comp.set_scene(Some(scene.clone()));
    comp.set_cursor(track.clone());
    comp.set_cursor_time(Some(T));
    comp.set_timeline_time(Some(T));
    let cfg = cfg();
    let (w, h, rgba) = unsafe {
        comp.compose_frame(screen.as_ptr(), screen.as_ptr(), 0.0, &cfg).expect("compose_frame");
        comp.readback_direct().expect("readback")
    };
    let render_px = [w as f32, h as f32];
    let src = [SRC.0 as f32, SRC.1 as f32];
    let g = plan_frame(&FrameGeometryInput {
        render_px,
        screen_tex_px: src,
        screen_visible_px: src,
        webcam_visible_px: src,
        u_max: 1.0,
        v_max: 1.0,
        frame: 0.0,
        cfg: &cfg,
        live,
        scene: Some(&scene),
        cursor: Some(track),
        timeline_t_override: Some(T),
        programme_time: None,
    });
    let Some(plan) = plan_cursor(
        &g,
        &CursorPlanInput {
            render_px,
            u_max: 1.0,
            v_max: 1.0,
            cfg: &cfg,
            live,
            scene: Some(&scene),
            track,
            t: T,
        },
    ) else {
        return (rgba, None);
    };
    let (tip, scale) = match plan.placement {
        CursorPlacement::Tilted { plane_pt, quad, center_px, .. } => {
            let (x, y) = quad.point_px(plane_pt[0], plane_pt[1]);
            ([center_px[0] + x, center_px[1] + y], quad.scale)
        }
        CursorPlacement::Upright { center } => ([center[0] * render_px[0], center[1] * render_px[1]], 1.0),
    };
    (rgba, Some(Probe { tip, unit: plan.size_px * scale }))
}

fn px(rgba: &[u8], x: i32, y: i32) -> [u8; 4] {
    let i = ((y * 1280 + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn luma(p: [u8; 4]) -> f32 {
    0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
}

fn is_neutral(p: [u8; 4]) -> bool {
    p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) < 12
}

fn is_white(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] > 200 && p[2] > 200
}

fn is_black(p: [u8; 4]) -> bool {
    p[0] < 45 && p[1] < 45 && p[2] < 45
}

/// Ce qui distingue une frame avec flèche de la même sans : les pixels de la flèche (neutres,
/// quand le contenu est bleu) et ceux d'ombre (le bleu du contenu, assombri), avec leurs
/// centroïdes et la distance de l'ombre la plus proche de la pointe.
struct Split {
    body: usize,
    white: usize,
    black: usize,
    shadow: usize,
    body_c: [f32; 2],
    shadow_c: [f32; 2],
    /// Distance (px) du pixel d'ombre le plus proche de l'apex de la silhouette.
    shadow_near_apex: f32,
}

fn split(with: &[u8], without: &[u8], apex: [f32; 2]) -> Split {
    let mut s = Split {
        body: 0,
        white: 0,
        black: 0,
        shadow: 0,
        body_c: [0.0; 2],
        shadow_c: [0.0; 2],
        shadow_near_apex: f32::MAX,
    };
    let body = |x: i32, y: i32| {
        (0..1280).contains(&x)
            && (0..720).contains(&y)
            && px(with, x, y) != px(without, x, y)
            && is_neutral(px(with, x, y))
    };
    for y in 0..720 {
        for x in 0..1280 {
            let (a, b) = (px(with, x, y), px(without, x, y));
            if a == b {
                continue;
            }
            // La frange antialiasée de la silhouette mêle la flèche au contenu : ni corps ni
            // ombre, on l'écarte (deux pixels autour du corps).
            let fringe = (-2..=2).any(|dy| (-2..=2).any(|dx| body(x + dx, y + dy)));
            if is_neutral(a) {
                s.body += 1;
                s.white += is_white(a) as usize;
                s.black += is_black(a) as usize;
                s.body_c[0] += x as f32;
                s.body_c[1] += y as f32;
            } else if !fringe && luma(a) < luma(b) - 6.0 {
                s.shadow += 1;
                s.shadow_c[0] += x as f32;
                s.shadow_c[1] += y as f32;
                let d = (x as f32 - apex[0]).hypot(y as f32 - apex[1]);
                s.shadow_near_apex = s.shadow_near_apex.min(d);
            }
        }
    }
    for (c, n) in [(&mut s.body_c, s.body), (&mut s.shadow_c, s.shadow)] {
        *c = [c[0] / n.max(1) as f32, c[1] / n.max(1) as f32];
    }
    s
}

fn save(name: &str, rgba: &[u8]) {
    let Ok(dir) = std::env::var("OPENSCREEN_CURSOR3D_OUT") else { return };
    std::fs::create_dir_all(&dir).expect("dossier de sortie");
    let path = format!("{dir}/{name}.png");
    image::RgbaImage::from_raw(1280, 720, rgba.to_vec())
        .expect("dimensions du readback")
        .save(&path)
        .unwrap_or_else(|e| panic!("écriture {path} : {e}"));
    println!("écrit {path}");
}

#[test]
fn the_modelled_arrow_stands_on_the_screen_and_casts_its_shadow() {
    let Some(gpu) = gpu() else { return };
    let comp = Compositor::new_sized(&gpu, 1280, 720).expect("compositor");
    let screen = FakeFrame::new(&gpu);
    let still = resting("still", false);
    let clicked = resting("clicked", true);
    // Même scène, curseur masqué : la référence « sans flèche ».
    let hidden = |rotation: &str| {
        let json = scene_json(rotation, Some(true), "default", 0.0, 3.0)
            .replace(r#""show":true"#, r#""show":false"#);
        render_any(&comp, &screen, &json, &still).0
    };

    for (name, rotation) in [("flat", "null"), ("iso", r#""iso""#)] {
        let bare = hidden(rotation);
        let (hover, p) = render(&comp, &screen, &scene_json(rotation, Some(true), "default", 0.0, 3.0), &still);
        let (touch, q) =
            render(&comp, &screen, &scene_json(rotation, Some(true), "default", 0.0, 3.0), &clicked);
        save(&format!("{name}-hover"), &hover);
        save(&format!("{name}-touch"), &touch);
        // L'apex de la silhouette : le filet blanc dépasse le hotspot vers le haut-gauche.
        let apex = |p: &Probe| [p.tip[0] - 0.03 * p.unit, p.tip[1] - 0.07 * p.unit];
        let (a, b) = (split(&hover, &bare, apex(&p)), split(&touch, &bare, apex(&q)));
        println!(
            "{name} : unité {:.1} px, pointe {:?} | en l'air : corps {} (blanc {}, noir {}), ombre {} \
             à {:.1} px de l'apex, centroïdes {:?} / {:?} | posée : ombre à {:.1} px, centroïdes {:?} / {:?}",
            p.unit, p.tip, a.body, a.white, a.black, a.shadow, a.shadow_near_apex, a.body_c, a.shadow_c,
            b.shadow_near_apex, b.body_c, b.shadow_c
        );

        // La pointe : le hotspot est la pointe de l'incrustation noire. Juste sous elle, dans le
        // corps, c'est noir ; juste au-dessus-gauche, le filet blanc ; loin au-dessus-gauche, le
        // contenu, intact (l'ombre part vers le bas-droite).
        let u = p.unit;
        let at = |dx: f32, dy: f32| px(&hover, (p.tip[0] + dx * u) as i32, (p.tip[1] + dy * u) as i32);
        assert!(is_black(at(0.07, 0.3)), "{name}: pas d'incrustation noire sous la pointe : {:?}", at(0.07, 0.3));
        assert!(is_white(at(-0.035, -0.02)) || is_white(at(-0.04, 0.05)), "{name}: pas de filet blanc à la pointe");
        let far = [(p.tip[0] - 0.3 * u) as i32, (p.tip[1] - 0.3 * u) as i32];
        assert_eq!(px(&hover, far[0], far[1]), px(&bare, far[0], far[1]), "{name}: le haut-gauche a bougé");

        // Corps blanc ET noir, de la taille d'une flèche : 0,35 u² de silhouette dans le PNG.
        let area = 0.35 * u * u;
        assert!(a.white as f32 > 0.15 * area && a.black as f32 > 0.25 * area, "{name}: matières absentes");
        assert!((a.body as f32) < 1.5 * area && (a.body as f32) > 0.7 * area, "{name}: taille {} pour {area}", a.body);

        // L'ombre : présente, du côté opposé à la lumière (bas-droite de la flèche).
        assert!(a.shadow as f32 > 0.1 * area, "{name}: pas d'ombre en l'air ({})", a.shadow);
        assert!(
            a.shadow_c[0] > a.body_c[0] && a.shadow_c[1] > a.body_c[1],
            "{name}: l'ombre n'est pas en bas à droite ({:?} / {:?})",
            a.shadow_c,
            a.body_c
        );
        // Posée, l'ombre rejoint la pointe et se resserre sous la flèche.
        assert!(b.shadow_near_apex < 0.1 * u, "{name}: posée, l'ombre est à {:.1} px de l'apex", b.shadow_near_apex);
        assert!(a.shadow_near_apex > 0.2 * u, "{name}: en l'air, l'ombre touche l'apex ({:.1} px)", a.shadow_near_apex);
        let gap = |s: &Split| (s.shadow_c[0] - s.body_c[0]).hypot(s.shadow_c[1] - s.body_c[1]);
        // (La queue, relevée par la pression, garde son ombre au loin : le centroïde ne se
        // rapproche que d'une fraction de la hauteur.)
        assert!(gap(&b) < gap(&a) - 0.1 * u, "{name}: l'ombre ne se rapproche pas au contact ({} / {})", gap(&b), gap(&a));
        // La pointe ne bouge pas quand la flèche descend (elle est sur le rayon de vue).
        assert!((p.tip[0] - q.tip[0]).abs() < 0.01 && (p.tip[1] - q.tip[1]).abs() < 0.01);
    }
}

/// Incliné et à plat, la flèche n'a pas la même silhouette : le plan l'emporte avec lui.
#[test]
fn the_tilt_turns_the_modelled_arrow_with_the_screen() {
    let Some(gpu) = gpu() else { return };
    let comp = Compositor::new_sized(&gpu, 1280, 720).expect("compositor");
    let screen = FakeFrame::new(&gpu);
    let still = resting("tilt", false);
    let mask = |rotation: &str| -> (Vec<bool>, Probe) {
        let (rgba, p) = render(&comp, &screen, &scene_json(rotation, Some(true), "default", 0.0, 3.0), &still);
        // Silhouette relative à la pointe, sur une fenêtre de 1,2 unité.
        let r = (1.2 * p.unit) as i32;
        let mut m = Vec::new();
        for dy in -r / 4..r {
            for dx in -r / 4..r {
                let q = px(&rgba, p.tip[0] as i32 + dx, p.tip[1] as i32 + dy);
                m.push(is_neutral(q));
            }
        }
        (m, p)
    };
    let (flat, pf) = mask("null");
    let (iso, pi) = mask(r#""iso""#);
    let n = flat.len().min(iso.len());
    let differ = (0..n).filter(|&k| flat[k] != iso[k]).count();
    println!("silhouettes : {differ} px diffèrent (unités {:.1} / {:.1})", pf.unit, pi.unit);
    assert!(differ as f32 > 0.05 * pf.unit * pf.unit, "la flèche ne suit pas l'inclinaison ({differ})");
}

/// Réglage éteint : la frame est celle du sprite plat, quelle que soit la façon de le dire (clé
/// absente, `false`, ou allumé sur un thème qui n'a pas de modèle).
#[test]
fn without_the_model_the_cursor_renders_the_flat_sprite() {
    let Some(gpu) = gpu() else { return };
    let comp = Compositor::new_sized(&gpu, 1280, 720).expect("compositor");
    let screen = FakeFrame::new(&gpu);
    let still = resting("flat-sprite", false);
    for rotation in ["null", r#""iso""#] {
        let absent = render(&comp, &screen, &scene_json(rotation, None, "default", 0.0, 3.0), &still).0;
        if let Ok(dir) = std::env::var("OPENSCREEN_CURSOR3D_FLAT_REF") {
            // Comparaison à une frame rendue AVANT le mode 15 : le même test, lancé sur le commit
            // de base, écrit la référence ; lancé ici, il la relit.
            let name = format!("{dir}/flat-{}.rgba", if rotation == "null" { "flat" } else { "iso" });
            match std::fs::read(&name) {
                Ok(before) => assert!(before == absent, "{rotation}: la frame plate diffère d'avant le mode 15"),
                Err(_) => std::fs::write(&name, &absent).expect("écriture de la référence"),
            }
        }
        let off = render(&comp, &screen, &scene_json(rotation, Some(false), "default", 0.0, 3.0), &still).0;
        let other = render(&comp, &screen, &scene_json(rotation, Some(true), "other", 0.0, 3.0), &still).0;
        let on = render(&comp, &screen, &scene_json(rotation, Some(true), "default", 0.0, 3.0), &still).0;
        assert!(absent == off, "{rotation}: model3d=false a changé la frame");
        assert!(absent == other, "{rotation}: un thème sans modèle a changé la frame");
        assert!(absent != on, "{rotation}: le réglage allumé ne change rien");
    }
}

/// Le lacet : en mouvement vers la droite, la flèche tourne sa pointe vers la droite.
#[test]
fn a_moving_arrow_turns_towards_its_motion() {
    let Some(gpu) = gpu() else { return };
    let comp = Compositor::new_sized(&gpu, 1280, 720).expect("compositor");
    let screen = FakeFrame::new(&gpu);
    let moving = track("moving", &[(0.0, 0.2, 0.45, false), (T + 1.0, 0.8, 0.45, false)]);
    let json = scene_json(r#""iso""#, Some(true), "default", 0.0, 3.0);
    let (rgba, p) = render(&comp, &screen, &json, &moving);
    save("iso-moving", &rgba);
    // Au repos la queue descend à droite de la pointe ; tournée vers la droite, elle part vers
    // la gauche : l'extrémité de la queue (bas du corps) est donc moins à droite qu'au repos.
    let still = resting("still-yaw", false);
    let (rest, r) = render(&comp, &screen, &json, &still);
    let tail_x = |rgba: &[u8], p: &Probe| {
        let y = (p.tip[1] + 0.8 * p.unit) as i32;
        let xs: Vec<i32> = (0..1280).filter(|&x| is_neutral(px(rgba, x, y))).collect();
        xs.iter().sum::<i32>() as f32 / xs.len().max(1) as f32 - p.tip[0]
    };
    let (moving_x, rest_x) = (tail_x(&rgba, &p), tail_x(&rest, &r));
    println!("queue : {moving_x:.1} px en mouvement, {rest_x:.1} px au repos");
    assert!(moving_x < rest_x - 0.05 * p.unit, "la flèche ne tourne pas vers son mouvement");
}

/// La traînée de flou de mouvement : des copies du modèle, pas un sprite plat.
#[test]
fn the_motion_blur_trail_draws_modelled_copies() {
    let Some(gpu) = gpu() else { return };
    let comp = Compositor::new_sized(&gpu, 1280, 720).expect("compositor");
    let screen = FakeFrame::new(&gpu);
    let moving = track("trail", &[(0.0, 0.2, 0.45, false), (T + 1.0, 0.8, 0.45, false)]);
    let (rgba, _) = render(&comp, &screen, &scene_json("null", Some(true), "default", 1.0, 3.0), &moving);
    let (flat, _) = render(&comp, &screen, &scene_json("null", Some(false), "default", 1.0, 3.0), &moving);
    save("flat-trail", &rgba);
    assert!(rgba != flat, "la traînée 3D est celle du sprite plat");
}

/// Coût d'une frame 1080p, flèche allumée contre éteinte (opt-in, à lancer en release). Le
/// readback synchronise chaque frame (sans lui on ne mesurerait que la mise en file) et pèse
/// autant des deux côtés : on alterne les deux réglages et on garde le meilleur de cinq passes.
/// `OPENSCREEN_CURSOR3D_BENCH=warp` mesure le backend logiciel (WARP) au lieu du GPU.
#[test]
fn bench_the_modelled_arrow_at_1080p() {
    let Ok(which) = std::env::var("OPENSCREEN_CURSOR3D_BENCH") else {
        eprintln!("OPENSCREEN_CURSOR3D_BENCH absent — saute");
        return;
    };
    let gpu = if which == "warp" {
        Gpu::create_backend(openscreen_compositor::d3d::Backend::Cpu, false).expect("WARP")
    } else {
        let Some(gpu) = gpu() else { return };
        gpu
    };
    let comp = Compositor::new_sized(&gpu, 1920, 1080).expect("compositor");
    let screen = FakeFrame::new(&gpu);
    let still = resting("bench", false);
    // Un geste rapide : la traînée prend ses 16 copies.
    let fast = track("bench-fast", &[(0.0, 0.2, 0.45, false), (T - 0.15, 0.2, 0.45, false), (T + 0.15, 0.8, 0.45, false), (9.0, 0.8, 0.45, false)]);
    let time = |json: &str, track: &CursorTrack| {
        let scene = Scene::from_json(json).expect("scène");
        comp.set_live_params(live_params_from_scene(&scene));
        comp.set_has_webcam(false);
        comp.set_scene(Some(scene));
        comp.set_cursor(track.clone());
        comp.set_cursor_time(Some(T));
        comp.set_timeline_time(Some(T));
        let cfg = cfg();
        let t0 = std::time::Instant::now();
        for _ in 0..100 {
            unsafe {
                comp.compose_frame(screen.as_ptr(), screen.as_ptr(), 0.0, &cfg).expect("compose");
                comp.readback_direct().expect("readback");
            }
        }
        t0.elapsed().as_secs_f64() * 10.0
    };
    for rotation in ["null", r#""iso""#] {
        for (label, blur, size, tr) in
            [("taille 3, net", 0.0, 3.0, &still), ("taille 10, net", 0.0, 10.0, &still), ("taille 10, traînée 16", 1.0, 10.0, &fast)]
        {
            let (off_json, on_json) = (
                scene_json(rotation, Some(false), "default", blur, size),
                scene_json(rotation, Some(true), "default", blur, size),
            );
            time(&off_json, tr);
            time(&on_json, tr);
            let (mut off, mut on) = (f64::MAX, f64::MAX);
            for _ in 0..5 {
                off = off.min(time(&off_json, tr));
                on = on.min(time(&on_json, tr));
            }
            println!("{which} 1080p {rotation} {label} : sprite {off:.2} ms, flèche 3D {on:.2} ms ({:+.2} ms)", on - off);
        }
    }
}
