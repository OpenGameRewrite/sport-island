use bevy::asset::AssetPlugin;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use std::path::PathBuf;

mod debug;

/// Extracted assets live in the working directory, next to the executable, or at the repo root.
fn asset_dir() -> PathBuf {
    let candidates = [
        std::env::current_dir().ok().map(|d| d.join("assets")),
        std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("assets"))),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("VERSION").exists()).unwrap_or_else(|| {
        eprintln!("No extracted assets found. Run sport-island-extract with your Wii Sports Resort disc first.");
        std::process::exit(1);
    })
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin { file_path: asset_dir().to_string_lossy().into_owned(), ..default() })
                .set(WindowPlugin { primary_window: Some(Window { title: "Sport Island".into(), resolution: (1280, 720).into(), ..default() }), ..default() }),
        )
        .add_plugins(debug::plugin)
        .add_systems(Startup, setup)
        .add_systems(Update, fly)
        .run();
}

#[derive(Component)]
struct Fly;

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn(WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/island.glb"))));
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { far: 200_000.0, near: 5.0, ..default() }),
        Transform::from_xyz(0.0, 3000.0, 8000.0).looking_at(Vec3::ZERO, Vec3::Y),
        Fly,
    ));
    commands.spawn((DirectionalLight { illuminance: 8000.0, ..default() }, Transform::from_xyz(1.0, 2.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y)));
}

fn fly(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    time: Res<Time>,
    mut cam: Single<&mut Transform, With<Fly>>,
) {
    if mouse.pressed(MouseButton::Right) {
        let (mut yaw, mut pitch, _) = cam.rotation.to_euler(EulerRot::YXZ);
        yaw -= motion.delta.x * 0.003;
        pitch = (pitch - motion.delta.y * 0.003).clamp(-1.5, 1.5);
        cam.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    }
    let mut v = Vec3::ZERO;
    for (k, d) in [(KeyCode::KeyW, cam.forward().as_vec3()), (KeyCode::KeyS, cam.back().as_vec3()), (KeyCode::KeyA, cam.left().as_vec3()), (KeyCode::KeyD, cam.right().as_vec3()), (KeyCode::KeyE, Vec3::Y), (KeyCode::KeyQ, Vec3::NEG_Y)] {
        if keys.pressed(k) {
            v += d;
        }
    }
    let speed = if keys.pressed(KeyCode::ShiftLeft) { 8000.0 } else { 1500.0 };
    cam.translation += v * speed * time.delta_secs();
}
