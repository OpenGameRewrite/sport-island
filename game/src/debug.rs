//! Developer helpers: `SI_CAM="x y z yaw pitch"` places the camera,
//! `SI_SHOT=file.png` saves a screenshot after a few seconds and exits.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

pub fn plugin(app: &mut App) {
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
    if std::env::var("SI_SHOT").is_ok() {
        app.add_systems(Update, shoot);
    }
    if std::env::var("SI_CAM").is_ok() {
        app.add_systems(PostStartup, place_camera);
    }
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
