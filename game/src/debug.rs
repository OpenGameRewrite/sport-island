//! Developer helpers: `SI_CAM="x y z yaw pitch"` places the camera,
//! `SI_SHOT=file.png` saves a screenshot after a few seconds and exits.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

pub fn plugin(app: &mut App) {
    if std::env::var("SI_CONTROLS").is_ok() {
        app.add_systems(PostStartup, crate::menu::preview_controls);
    }
    if std::env::var("SI_SMOKE").is_ok() {
        app.add_systems(PreUpdate, smoke.after(crate::input::read).before(crate::menu::pause_input));
    }
    // SI_PLAY skips the title screen; SI_FLY also boards the plane.
    if std::env::var("SI_PLAY").is_ok() || std::env::var("SI_FLY").is_ok() {
        app.add_systems(Update, skip_title.before(crate::plane::board));
    }
    if std::env::var("SI_MODEL").is_ok() {
        app.add_systems(Startup, inspect_model);
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
        app.add_systems(PostStartup, place_camera);
    }
}

/// Exercises the playable loop through normal controls; requires an isolated SI_SAVE.
fn smoke(
    time: Res<Time<Real>>, state: Res<State<crate::AppState>>, mode: Res<State<crate::Mode>>,
    mut c: ResMut<crate::input::Controls>, mut phase: Local<u8>, mut paused_at: Local<Option<Vec3>>,
    plane: Query<&Transform, With<crate::plane::Plane>>, menu: Res<crate::menu::Menu>,
    markers: Query<&crate::ipoint::Marker>,
) {
    assert!(std::env::var_os("SI_SAVE").is_some(), "SI_SMOKE requires SI_SAVE to protect real progress");
    let now = time.elapsed_secs();
    match *phase {
        0 if now > 2.0 => { c.down = true; *phase = 1; }
        1 if now > 3.0 => { c.confirm = true; *phase = 2; }
        2 if *state.get() == crate::AppState::Play && now > 5.0 => { assert_eq!(markers.iter().count(), 80, "all sightseeing points must spawn"); c.board = true; *phase = 3; }
        3 if *mode.get() == crate::Mode::Fly => {
            c.shoot = true;
            if now > 7.0 { c.flare = true; *phase = 4; }
        }
        4 if now > 9.0 => { c.pause = true; *paused_at = plane.single().ok().map(|t| t.translation); *phase = 5; }
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
        8 if now > 17.0 => { info!("SI_SMOKE passed: language, play, board, gun, flare, pause/resume, eject, time change"); *phase = 9; }
        _ => {}
    }
}

fn visit_point(time: Res<Time>, points: Res<crate::ipoint::IPoints>, mut done: Local<bool>, mut player: Query<&mut Transform, With<crate::player::Player>>) {
    if *done || time.elapsed_secs() < 2.0 { return; }
    assert!(std::env::var_os("SI_SAVE").is_some(), "SI_POINT requires an isolated SI_SAVE");
    let i: usize = std::env::var("SI_POINT").unwrap().parse().unwrap();
    player.single_mut().unwrap().translation = points.0[i].pos;
    *done = true;
}

fn place_camera(mut cam: Query<&mut Transform, With<Camera3d>>) {
    let v: Vec<f32> = std::env::var("SI_CAM").unwrap().split_whitespace().filter_map(|s| s.parse().ok()).collect();
    for mut t in &mut cam {
        t.translation = Vec3::new(v[0], v[1], v[2]);
        t.rotation = Quat::from_euler(EulerRot::YXZ, v[3].to_radians(), v[4].to_radians(), 0.0);
    }
}

fn shoot(mut commands: Commands, time: Res<Time>, mut done: Local<u8>, mut exit: MessageWriter<AppExit>) {
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
    planes: Query<&Transform, With<crate::plane::Plane>>,
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
        .observe(crate::player::no_culling);
    commands.spawn((
        Camera3d::default(),
        Camera { order: 5, ..default() },
        Transform::from_translation(center + Vec3::new(dist * 0.6, dist * 0.4, dist)).looking_at(center, Vec3::Y),
    ));
}
