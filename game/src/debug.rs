//! Developer helpers: `SI_CAM="x y z yaw pitch"` places the camera,
//! `SI_SHOT=file.png` saves a screenshot after a few seconds and exits.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

pub fn plugin(app: &mut App) {
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
