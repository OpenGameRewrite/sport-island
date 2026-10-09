//! One merged control state from keyboard/mouse and any connected gamepad.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct Controls {
    /// Walk direction / plane steering (x right, y up), unit-clamped.
    pub stick: Vec2,
    /// Camera look delta (radians this frame).
    pub look: Vec2,
    /// Plane: thrust forward (boost) and draw back (slow down), 0..1.
    pub boost: f32,
    pub slow: f32,
    pub run: bool,
    pub jump: bool,
    /// Balloon Popper (held).
    pub shoot: bool,
    /// Cut the engine (held).
    pub cut: bool,
    pub flare: bool,
    /// Board the plane / eject.
    pub board: bool,
    pub pause: bool,
    pub time: bool,
    // Menu navigation (edge-triggered).
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub confirm: bool,
    pub back: bool,
    /// Last device used was a gamepad (for on-screen button labels).
    pub pad: bool,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Controls>().add_systems(PreUpdate, read.after(bevy::input::InputSystems));
}

fn read(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<AccumulatedMouseMotion>,
    pads: Query<&Gamepad>,
    mut c: ResMut<Controls>,
    mut prev_stick_nav: Local<Vec2>,
) {
    let k = |codes: &[KeyCode]| codes.iter().any(|c| keys.pressed(*c));
    let kj = |codes: &[KeyCode]| codes.iter().any(|c| keys.just_pressed(*c));
    let axis = |neg: &[KeyCode], pos: &[KeyCode]| (k(pos) as i32 - k(neg) as i32) as f32;
    let mut stick = Vec2::new(axis(&[KeyCode::KeyA, KeyCode::ArrowLeft], &[KeyCode::KeyD, KeyCode::ArrowRight]), axis(&[KeyCode::KeyS, KeyCode::ArrowDown], &[KeyCode::KeyW, KeyCode::ArrowUp]));
    let mut look = mouse.delta * 0.003 * Vec2::new(1.0, 1.0) + Vec2::new(axis(&[KeyCode::KeyJ], &[KeyCode::KeyL]), axis(&[KeyCode::KeyK], &[KeyCode::KeyI])) * 0.04;
    let mut boost = k(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]) as u8 as f32;
    let mut slow = k(&[KeyCode::ControlLeft, KeyCode::ControlRight]) as u8 as f32;
    let mut s = Controls {
        run: k(&[KeyCode::ShiftLeft]),
        jump: kj(&[KeyCode::Space]),
        shoot: k(&[KeyCode::Space]),
        cut: k(&[KeyCode::KeyC]),
        flare: kj(&[KeyCode::KeyF]),
        board: kj(&[KeyCode::KeyE]),
        pause: kj(&[KeyCode::Escape]),
        time: kj(&[KeyCode::KeyT]),
        up: kj(&[KeyCode::ArrowUp, KeyCode::KeyW]),
        down: kj(&[KeyCode::ArrowDown, KeyCode::KeyS]),
        left: kj(&[KeyCode::ArrowLeft, KeyCode::KeyA]),
        right: kj(&[KeyCode::ArrowRight, KeyCode::KeyD]),
        confirm: kj(&[KeyCode::Enter, KeyCode::Space]),
        back: kj(&[KeyCode::Escape, KeyCode::Backspace]),
        pad: c.pad,
        ..default()
    };
    if keys.get_just_pressed().next().is_some() || mouse.delta != Vec2::ZERO {
        s.pad = false;
    }
    for p in &pads {
        let dz = |v: f32| if v.abs() < 0.15 { 0.0 } else { (v - 0.15 * v.signum()) / 0.85 };
        let ls = Vec2::new(dz(p.get(GamepadAxis::LeftStickX).unwrap_or(0.0)), dz(p.get(GamepadAxis::LeftStickY).unwrap_or(0.0)));
        let rs = Vec2::new(dz(p.get(GamepadAxis::RightStickX).unwrap_or(0.0)), dz(p.get(GamepadAxis::RightStickY).unwrap_or(0.0)));
        let dpad = Vec2::new(p.pressed(GamepadButton::DPadRight) as i32 as f32 - p.pressed(GamepadButton::DPadLeft) as i32 as f32, p.pressed(GamepadButton::DPadUp) as i32 as f32 - p.pressed(GamepadButton::DPadDown) as i32 as f32);
        let used = ls != Vec2::ZERO || rs != Vec2::ZERO || p.get_just_pressed().next().is_some();
        if used {
            s.pad = true;
        }
        stick += ls + dpad;
        look += rs * Vec2::new(0.05, 0.04);
        boost = boost.max(p.get(GamepadButton::RightTrigger2).unwrap_or(0.0));
        slow = slow.max(p.get(GamepadButton::LeftTrigger2).unwrap_or(0.0));
        s.run |= p.pressed(GamepadButton::West) || p.pressed(GamepadButton::RightTrigger2);
        s.jump |= p.just_pressed(GamepadButton::South);
        s.shoot |= p.pressed(GamepadButton::South);
        s.cut |= p.pressed(GamepadButton::East);
        s.flare |= p.just_pressed(GamepadButton::North);
        s.board |= p.just_pressed(GamepadButton::West) || p.just_pressed(GamepadButton::LeftThumb);
        s.pause |= p.just_pressed(GamepadButton::Start);
        s.time |= p.just_pressed(GamepadButton::Select);
        s.confirm |= p.just_pressed(GamepadButton::South);
        s.back |= p.just_pressed(GamepadButton::East);
        s.up |= p.just_pressed(GamepadButton::DPadUp);
        s.down |= p.just_pressed(GamepadButton::DPadDown);
        s.left |= p.just_pressed(GamepadButton::DPadLeft);
        s.right |= p.just_pressed(GamepadButton::DPadRight);
        // Stick flicks also navigate menus.
        let nav = |v: f32, prev: f32| v.abs() > 0.6 && prev.abs() <= 0.6;
        s.up |= nav(ls.y, prev_stick_nav.y) && ls.y > 0.0;
        s.down |= nav(ls.y, prev_stick_nav.y) && ls.y < 0.0;
        s.right |= nav(ls.x, prev_stick_nav.x) && ls.x > 0.0;
        s.left |= nav(ls.x, prev_stick_nav.x) && ls.x < 0.0;
        *prev_stick_nav = ls;
    }
    s.stick = stick.clamp_length_max(1.0);
    s.look = look;
    s.boost = boost.min(1.0);
    s.slow = slow.min(1.0);
    *c = s;
}
