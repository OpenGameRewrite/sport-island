//! Developer helpers: `SI_CAM="x y z yaw pitch"` places the camera,
//! `SI_SHOT=file.png` saves a screenshot after a few seconds and exits.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

pub fn plugin(app: &mut App) {
    if std::env::var("SI_FRAMES").is_ok() { app.add_systems(Last, |time: Res<Time<Real>>| if time.delta_secs() > 0.1 { info!("SI_FRAMES slow frame {:.0} ms at {:.1} s", time.delta_secs() * 1000.0, time.elapsed_secs()); }); }
    if std::env::var("SI_MEMORY").is_ok() { app.add_systems(Update, memory); }
    if std::env::var("SI_TRAFFIC").is_ok() { app.add_systems(Update, traffic_check); }
    if std::env::var("SI_TEXT_STRESS").is_ok() { app.add_systems(PostUpdate, text_stress.before(bevy::ui::UiSystems::Layout)); }
    if std::env::var("SI_CONTROLS").is_ok() || std::env::var("SI_MENU").is_ok() {
        app.add_systems(Update, crate::menu::preview_controls);
    }
    if std::env::var("SI_SMOKE").is_ok() {
        app.add_systems(PreUpdate, smoke.after(crate::input::read).before(crate::menu::pause_input));
    }
    // SI_PLAY skips the title screen; SI_FLY also boards the plane.
    if std::env::var("SI_PLAY").is_ok() || std::env::var("SI_FLY").is_ok() {
        app.add_systems(Update, skip_title.before(crate::plane::board));
    }
    // SI_FLARE=seconds fires one signal flare at that time (with SI_FLY).
    if let Some(at) = std::env::var("SI_FLARE").ok().and_then(|s| s.parse::<f32>().ok()) {
        app.add_systems(PreUpdate, (move |time: Res<Time>, mut c: ResMut<crate::input::Controls>, mut done: Local<bool>| if !*done && time.elapsed_secs() > at { c.flare = true; *done = true; }).after(crate::input::read));
    }
    // SI_SHOOT=seconds holds the gun from that time (with SI_FLY).
    if let Some(at) = std::env::var("SI_SHOOT").ok().and_then(|s| s.parse::<f32>().ok()) {
        app.add_systems(PreUpdate, (move |time: Res<Time>, mut c: ResMut<crate::input::Controls>| if time.elapsed_secs() > at { c.shoot = true; }).after(crate::input::read));
    }
    // SI_HOLD="x y boost slow" holds the stick and triggers throughout (with SI_FLY).
    if let Ok(hold) = std::env::var("SI_HOLD") {
        let v: Vec<f32> = hold.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        let at = |i: usize| v.get(i).copied().unwrap_or(0.0);
        let (stick, boost, slow) = (Vec2::new(at(0), at(1)), at(2), at(3));
        app.add_systems(PreUpdate, (move |mut c: ResMut<crate::input::Controls>| { c.stick = stick; c.boost = boost; c.slow = slow; }).after(crate::input::read));
    }
    // SI_AT="x y z" drops the player there once play starts.
    if let Some(at) = std::env::var("SI_AT").ok().map(|s| s.split_whitespace().filter_map(|v| v.parse().ok()).collect::<Vec<f32>>()).filter(|v| v.len() == 3) {
        let at = Vec3::new(at[0], at[1], at[2]);
        app.add_systems(Update, move |mut done: Local<bool>, mut q: Query<(&mut Transform, &mut crate::player::Player)>| for (mut t, mut p) in &mut q { if !std::mem::replace(&mut *done, true) { t.translation = at; p.vel = Vec3::ZERO; } });
    }
    // SI_EFFECT=name spawns that particle effect 200 units ahead of the camera every second.
    if let Ok(name) = std::env::var("SI_EFFECT") {
        app.add_systems(FixedUpdate, move |mut commands: Commands, time: Res<Time>, lib: Res<crate::particles::ParticleLib>, cam: Single<&GlobalTransform, With<crate::world::MainCamera>>, mut last: Local<f32>| {
            if time.elapsed_secs() - *last < 1.0 { return; }
            *last = time.elapsed_secs();
            let at = bevy::math::Affine3A::from_translation((cam.translation() + cam.forward() * 200.0).into());
            crate::particles::spawn(&mut commands, &lib, &name, at);
        });
    }
    if std::env::var("SI_MODEL").is_ok() {
        app.add_systems(Startup, inspect_model).add_systems(Update, |mut cams: Query<&mut Camera, With<crate::world::MainCamera>>| for mut c in &mut cams { c.is_active = false; });
    }
    if std::env::var("SI_LOG").is_ok() {
        app.add_systems(Update, log_state);
    }
    if std::env::var("SI_POINT").is_ok() {
        app.add_systems(Update, visit_point.run_if(crate::menu::playing));
    }
    if std::env::var("SI_SHOT").is_ok() {
        app.add_systems(Update, shoot);
    }
    if std::env::var("SI_CAM").is_ok() {
        app.add_systems(PostUpdate, place_camera.before(crate::world::sky_follow));
    }
}

fn memory(time: Res<Time<Real>>, mut last: Local<f32>, images: Res<Assets<Image>>, fonts: Res<bevy::text::FontAtlasSet>, meshes: Res<Assets<Mesh>>, materials: Res<Assets<StandardMaterial>>) {
    if time.elapsed_secs()-*last<5.0 { return; }
    *last=time.elapsed_secs();
    info!("SI_MEMORY t={:.0} images={} font_keys={} font_bytes={} meshes={} materials={}", *last, images.len(), fonts.len(), fonts.total_bytes(&images), meshes.len(), materials.len());
    if std::env::var("SI_TEXT_STRESS").is_ok() { assert!(fonts.total_bytes(&images)<=64*1024*1024, "font atlas grew beyond 64 MiB under repeated text updates"); }
}

/// Native regression: SI_TEXT_STRESS=1 SI_MEMORY=1 SI_SHOT=... SI_WAIT=120.
/// Continually reshape Latin, symbols and emoji without allocating new fonts.
fn text_stress(mut frame: Local<u32>, mut texts: Query<&mut Text>) {
    *frame+=1;
    for mut text in &mut texts { text.0=format!("Font stress {}: é ö ü ñ ⓘ ◀ ▶ ○ ●", *frame); }
}

/// Native regression with extracted CHR0 data: SI_TRAFFIC=1 SI_PLAY=1 SI_WAIT=12.
/// A seconds/frame conversion error previously sent the ferry around in 25 s.
fn traffic_check(time: Res<Time<Real>>, bones: Query<(&Name, &GlobalTransform)>, menu: Res<crate::menu::Menu>, mut sample: Local<Option<(f32, Vec3)>>, mut done: Local<bool>) {
    if *done || menu.open || time.elapsed_secs()<5.0 { return; }
    let Some((_, transform)) = bones.iter().find(|(n,_)| n.as_str()=="liner") else { return; };
    let now = time.elapsed_secs();
    if let Some((start,pos)) = *sample {
        if now-start<5.0 { return; }
        let speed=pos.distance(transform.translation())/(now-start);
        assert!((5.0..250.0).contains(&speed), "ferry speed {speed} indicates frozen or accelerated animation");
        info!("SI_TRAFFIC passed: ferry {speed:.1} units/s"); *done=true;
    } else { *sample=Some((now, transform.translation())); }
}

/// Exercises the playable loop through normal controls; requires an isolated SI_SAVE.
fn smoke(
    time: Res<Time<Real>>, state: Res<State<crate::AppState>>, mode: Res<State<crate::Mode>>,
    mut c: ResMut<crate::input::Controls>, mut phase: Local<u8>, mut paused_at: Local<Option<Vec3>>, mut boarded_at: Local<f32>,
    plane: Query<&Transform, With<crate::plane::Flying>>, menu: Res<crate::menu::Menu>,
    markers: Query<&crate::ipoint::Marker>,
    names: Query<&Name>,
) {
    assert!(std::env::var_os("SI_SAVE").is_some(), "SI_SMOKE requires SI_SAVE to protect real progress");
    let now = time.elapsed_secs();
    match *phase {
        0 if now > 2.0 => { *phase = 1; }
        1 if now > 3.0 => { c.confirm = true; *phase = 2; }
        2 if *state.get() == crate::AppState::Play && now > 5.0 => { assert_eq!(markers.iter().count(), 80, "all sightseeing points must spawn"); c.board = true; *boarded_at = now; *phase = 3; }
        3 if *mode.get() == crate::Mode::Fly => {
            // Pull back briefly to take off from water through normal input.
            if now < *boarded_at + 2.0 { c.stick.y = -0.4; }
            c.shoot = true;
            if now > 7.0 { c.flare = true; *phase = 4; }
        }
        4 if now > 9.0 => {
            assert!(names.iter().any(|n| n.as_str()=="Seated pilot"), "cockpit pilot did not spawn");
            assert!(names.iter().any(|n| n.as_str()=="Signal flare"), "signal flare did not spawn");
            c.pause = true; *paused_at = plane.single().ok().map(|t| t.translation); *phase = 5;
        }
        5 => {
            if menu.open && let Some(pos) = *paused_at { assert_eq!(plane.single().unwrap().translation, pos, "plane moved while paused"); }
            if now > 12.0 { c.pause = true; *phase = 6; }
        }
        6 if now > 14.0 => { c.board = true; *phase = 7; }
        7 if now > 15.0 => {
            assert_eq!(*mode.get(), crate::Mode::Walk);
            assert!(plane.is_empty());
            c.time = true;
            *phase = 8;
        }
        8 if now > 17.0 => { info!("SI_SMOKE passed: play, board, cockpit pilot, gun, flare, pause/resume, eject, time change"); *phase = 9; }
        _ => {}
    }
}

fn visit_point(time: Res<Time>, points: Res<crate::ipoint::IPoints>, mut done: Local<bool>, mut player: Query<&mut Transform, With<crate::player::Player>>) {
    if *done || time.elapsed_secs() < 2.0 { return; }
    assert!(std::env::var_os("SI_SAVE").is_some(), "SI_POINT requires an isolated SI_SAVE");
    let i: usize = std::env::var("SI_POINT").unwrap().parse().unwrap();
    let distance = std::env::var("SI_POINT_DISTANCE").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
    player.single_mut().unwrap().translation = points.0[i].pos + Vec3::Z * distance;
    *done = true;
}

fn place_camera(mut cam: Query<&mut Transform, With<crate::world::MainCamera>>) {
    let v: Vec<f32> = std::env::var("SI_CAM").unwrap().split_whitespace().filter_map(|s| s.parse().ok()).collect();
    for mut t in &mut cam {
        t.translation = Vec3::new(v[0], v[1], v[2]);
        t.rotation = Quat::from_euler(EulerRot::YXZ, v[3].to_radians(), v[4].to_radians(), 0.0);
    }
}

fn shoot(mut commands: Commands, time: Res<Time<Real>>, mut done: Local<u8>, mut exit: MessageWriter<AppExit>) {
    let wait: f32 = std::env::var("SI_WAIT").ok().and_then(|s| s.parse().ok()).unwrap_or(8.0);
    let t = time.elapsed_secs();
    if *done == 0 && t > wait {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(std::env::var("SI_SHOT").unwrap()));
        *done = 1;
    } else if *done == 1 && t > wait + 2.0 {
        exit.write(AppExit::Success);
        *done = 2;
    }
}

fn skip_title(
    state: Res<State<crate::AppState>>,
    mut next: ResMut<NextState<crate::AppState>>,
    mut menu: ResMut<crate::menu::Menu>,
    mut controls: ResMut<crate::input::Controls>,
    mode: Res<State<crate::Mode>>,
    mut frames: Local<u32>,
) {
    if *state.get() == crate::AppState::Title {
        menu.open = false;
        next.set(crate::AppState::Play);
    } else if std::env::var("SI_FLY").is_ok() && *mode.get() == crate::Mode::Walk {
        *frames += 1;
        if *frames == 30 {
            controls.board = true;
        }
    }
}

pub fn log_state(
    time: Res<Time>,
    mode: Res<State<crate::Mode>>,
    state: Res<State<crate::AppState>>,
    player: Query<&Transform, With<crate::player::Player>>,
    planes: Query<&Transform, With<crate::plane::Flying>>,
    cam: Query<&Transform, With<crate::world::MainCamera>>,
    mut last: Local<f32>,
) {
    if time.elapsed_secs() - *last < 1.0 {
        return;
    }
    *last = time.elapsed_secs();
    info!(
        "t={:.1} state={:?} mode={:?} players={:?} planes={:?} cam={:?}",
        time.elapsed_secs(),
        state.get(),
        mode.get(),
        player.iter().map(|t| t.translation).collect::<Vec<_>>(),
        planes.iter().map(|t| t.translation).collect::<Vec<_>>(),
        cam.iter().map(|t| t.translation).collect::<Vec<_>>()
    );
}

/// `SI_MODEL=name[:scene]` shows one converted model in isolation (`SI_DIST` = camera distance).
fn inspect_model(mut commands: Commands, assets: Res<AssetServer>) {
    let spec = std::env::var("SI_MODEL").unwrap();
    let (name, scene) = spec.split_once(':').map_or((spec.as_str(), 0), |(n, s)| (n, s.parse().unwrap_or(0)));
    let dist: f32 = std::env::var("SI_DIST").ok().and_then(|s| s.parse().ok()).unwrap_or(100.0);
    let center = Vec3::new(0.0, 20000.0, 0.0);
    commands
        .spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(scene).from_asset(format!("models/{name}.glb")))), Transform::from_translation(center)))
        .observe(crate::player::no_culling)
        .observe(crate::plane::attach_propeller);
    commands.spawn((
        Camera3d::default(),
        Camera { order: 5, ..default() },
        Transform::from_translation(center + Vec3::new(dist * 0.6, dist * 0.4, dist)).looking_at(center, Vec3::Y),
    ));
}
