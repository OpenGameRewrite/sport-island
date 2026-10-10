//! One merged control state from keyboard/mouse and any connected gamepad.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;

/// Actions shared by walking and flying. Menu navigation and look sticks remain
/// available while rebinding, so a bad binding cannot trap the user in a menu.
pub const ACTIONS: [(&str, &str, &str); 15] = [
    ("Left", "KeyA", "DPadLeft"), ("Right", "KeyD", "DPadRight"),
    ("Forward / nose down", "KeyW", "DPadUp"), ("Back / nose up", "KeyS", "DPadDown"),
    ("Run / boost impulse", "ShiftLeft", "RightTrigger2"), ("Slow impulse", "ControlLeft", "LeftTrigger2"),
    ("Jump / shoot", "Space", "South"), ("Cut engine", "KeyC", "East"),
    ("Flare", "KeyF", "North"), ("Board / eject", "KeyE", "West"),
    ("Pause", "Escape", "Start"), ("Time of day", "KeyT", "Select"),
    ("i-point list", "Tab", "LeftTrigger"), ("Speedometer", "KeyV", "RightTrigger"),
    ("Recenter camera", "KeyR", "RightThumb"),
];

pub fn binding(save: &crate::save::Save, i: usize) -> [String; 2] {
    let (name, key, pad) = ACTIONS[i];
    save.bindings.get(name).cloned().unwrap_or_else(|| [key.into(), pad.into()])
}

pub fn label(raw: &str) -> String {
    match raw { "South" => "A / Cross".into(), "East" => "B / Circle".into(), "West" => "X / Square".into(), "North" => "Y / Triangle".into(), "RightTrigger2" => "RT / R2".into(), "LeftTrigger2" => "LT / L2".into(), "LeftTrigger" => "LB / L1".into(), "RightTrigger" => "RB / R1".into(), "RightThumb" => "RS / R3".into(), "LeftThumb" => "LS / L3".into(), "Select" => "Back / Select".into(), "DPadLeft" => "D-pad left".into(), "DPadRight" => "D-pad right".into(), "DPadUp" => "D-pad up".into(), "DPadDown" => "D-pad down".into(), "ShiftLeft" => "Left Shift".into(), "ControlLeft" => "Left Ctrl".into(), _ => raw.strip_prefix("Key").unwrap_or(raw).into() }
}

const BUTTONS: [GamepadButton; 19] = [GamepadButton::South, GamepadButton::East, GamepadButton::North, GamepadButton::West, GamepadButton::C, GamepadButton::Z, GamepadButton::LeftTrigger, GamepadButton::LeftTrigger2, GamepadButton::RightTrigger, GamepadButton::RightTrigger2, GamepadButton::Select, GamepadButton::Start, GamepadButton::Mode, GamepadButton::LeftThumb, GamepadButton::RightThumb, GamepadButton::DPadUp, GamepadButton::DPadDown, GamepadButton::DPadLeft, GamepadButton::DPadRight];

#[derive(Resource, Default, Clone)]
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
    pub record: bool,
    pub speedometer: bool,
    /// Snap the camera back behind the player or plane.
    pub recenter: bool,
    // Menu navigation (initial press plus held repeat).
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

pub fn read(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<AccumulatedMouseMotion>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    save: Res<crate::save::Save>,
    pads: Query<&Gamepad>,
    time: Res<Time<Real>>,
    mut c: ResMut<Controls>,
    mut repeat: Local<[Repeat; 4]>,
) {
    let k = |codes: &[KeyCode]| codes.iter().any(|c| keys.pressed(*c));
    let kj = |codes: &[KeyCode]| codes.iter().any(|c| keys.just_pressed(*c));
    let axis = |neg: &[KeyCode], pos: &[KeyCode]| (k(pos) as i32 - k(neg) as i32) as f32;
    let bindings: Vec<_> = (0..ACTIONS.len()).map(|i| binding(&save, i)).collect();
    let held = |i: usize| keys.get_pressed().any(|k| format!("{k:?}") == bindings[i][0]) || mouse_buttons.get_pressed().any(|k| format!("Mouse{k:?}") == bindings[i][0]);
    let edge = |i: usize| keys.get_just_pressed().any(|k| format!("{k:?}") == bindings[i][0]) || mouse_buttons.get_just_pressed().any(|k| format!("Mouse{k:?}") == bindings[i][0]);
    let mut stick = Vec2::new((held(1) as i32 - held(0) as i32) as f32, (held(2) as i32 - held(3) as i32) as f32);
    let mut look = mouse.delta * 0.003 + Vec2::new(axis(&[KeyCode::KeyJ], &[KeyCode::KeyL]), axis(&[KeyCode::KeyK], &[KeyCode::KeyI])) * (2.4 * time.delta_secs().min(0.05));
    let mut boost = held(4) as u8 as f32;
    let mut slow = held(5) as u8 as f32;
    let mut nav = [k(&[KeyCode::ArrowUp, KeyCode::KeyW]), k(&[KeyCode::ArrowDown, KeyCode::KeyS]), k(&[KeyCode::ArrowLeft, KeyCode::KeyA]), k(&[KeyCode::ArrowRight, KeyCode::KeyD])];
    let mut s = Controls {
        run: held(4), jump: edge(6), shoot: held(6), cut: held(7),
        flare: edge(8), board: edge(9), pause: edge(10) || kj(&[KeyCode::Escape]), time: edge(11),
        up: kj(&[KeyCode::ArrowUp, KeyCode::KeyW]),
        down: kj(&[KeyCode::ArrowDown, KeyCode::KeyS]),
        left: kj(&[KeyCode::ArrowLeft, KeyCode::KeyA]),
        right: kj(&[KeyCode::ArrowRight, KeyCode::KeyD]),
        confirm: kj(&[KeyCode::Enter, KeyCode::Space]),
        back: kj(&[KeyCode::Escape, KeyCode::Backspace]),
        pad: c.pad, record: edge(12), speedometer: edge(13), recenter: edge(14),
        ..default()
    };
    if keys.get_just_pressed().next().is_some() || mouse.delta != Vec2::ZERO {
        s.pad = false;
    }
    for p in &pads {
        let dz = |v: f32| if v.abs() < 0.15 { 0.0 } else { (v - 0.15 * v.signum()) / 0.85 };
        let ls = Vec2::new(dz(p.get(GamepadAxis::LeftStickX).unwrap_or(0.0)), dz(p.get(GamepadAxis::LeftStickY).unwrap_or(0.0)));
        let rs = Vec2::new(dz(p.get(GamepadAxis::RightStickX).unwrap_or(0.0)), dz(p.get(GamepadAxis::RightStickY).unwrap_or(0.0)));
        let used = ls != Vec2::ZERO || rs != Vec2::ZERO || p.get_just_pressed().next().is_some();
        if used {
            s.pad = true;
        }
        let button = |i: usize| BUTTONS.iter().copied().find(|b| format!("{b:?}") == bindings[i][1]);
        let held = |i: usize| button(i).is_some_and(|b| p.pressed(b));
        let edge = |i: usize| button(i).is_some_and(|b| p.just_pressed(b));
        stick += ls + Vec2::new((held(1) as i32 - held(0) as i32) as f32, (held(2) as i32 - held(3) as i32) as f32);
        look += rs * Vec2::new(3.0, 2.4) * time.delta_secs().min(0.05);
        boost = boost.max(button(4).and_then(|b| p.get(b)).unwrap_or(0.0));
        slow = slow.max(button(5).and_then(|b| p.get(b)).unwrap_or(0.0));
        s.run |= held(4) || p.pressed(GamepadButton::LeftThumb);
        s.jump |= edge(6); s.shoot |= held(6); s.cut |= held(7);
        s.flare |= edge(8); s.board |= edge(9); s.pause |= edge(10) || p.just_pressed(GamepadButton::Start); s.time |= edge(11);
        s.record |= edge(12); s.speedometer |= edge(13); s.recenter |= edge(14);
        s.confirm |= p.just_pressed(GamepadButton::South);
        s.back |= p.just_pressed(GamepadButton::East);
        s.up |= p.just_pressed(GamepadButton::DPadUp);
        s.down |= p.just_pressed(GamepadButton::DPadDown);
        s.left |= p.just_pressed(GamepadButton::DPadLeft);
        s.right |= p.just_pressed(GamepadButton::DPadRight);
        for (i, button) in [GamepadButton::DPadUp, GamepadButton::DPadDown, GamepadButton::DPadLeft, GamepadButton::DPadRight].into_iter().enumerate() { nav[i] |= p.pressed(button); }
        nav[0] |= ls.y > 0.6; nav[1] |= ls.y < -0.6; nav[2] |= ls.x < -0.6; nav[3] |= ls.x > 0.6;
    }
    for i in 0..4 {
        let event = repeat[i].tick(nav[i], time.delta_secs());
        match i { 0 => s.up |= event, 1 => s.down |= event, 2 => s.left |= event, _ => s.right |= event }
    }
    s.stick = stick.clamp_length_max(1.0);
    s.look = look * save.look_sensitivity;
    s.boost = boost.min(1.0);
    s.slow = slow.min(1.0);
    // Retain this edge until a fixed step consumes it.
    s.flare |= c.flare;
    *c = s;
}

/// Initial press, then repeat after 350 ms at ten rows per second. Uses real
/// time so it continues while the simulation is paused.
#[derive(Default)]
pub struct Repeat { held: bool, remaining: f32 }
impl Repeat {
    fn tick(&mut self, held: bool, dt: f32) -> bool {
        if !held { self.held = false; return false; }
        if !self.held { self.held = true; self.remaining = 0.35; return true; }
        self.remaining -= dt;
        if self.remaining <= 0.0 { self.remaining += 0.1; true } else { false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_navigation_repeats_and_release_rearms_immediately() {
        let mut repeat = Repeat::default();
        assert!(repeat.tick(true, 0.016));
        assert!(!repeat.tick(true, 0.3));
        assert!(repeat.tick(true, 0.06));
        assert!(!repeat.tick(true, 0.05));
        assert!(repeat.tick(true, 0.05));
        assert!(!repeat.tick(false, 0.016));
        assert!(repeat.tick(true, 0.016));
    }
    #[test]
    fn keyboard_and_mouse_overrides_drive_actions_while_menu_escape_still_works() {
        let mut app = App::new();
        let mut save = crate::save::Save::default();
        save.bindings.insert(ACTIONS[9].0.into(), ["KeyQ".into(), "RightThumb".into()]);
        save.bindings.insert(ACTIONS[6].0.into(), ["MouseLeft".into(), "South".into()]);
        save.bindings.insert(ACTIONS[10].0.into(), ["KeyP".into(), "Start".into()]);
        app.insert_resource(save).init_resource::<Controls>()
            .init_resource::<ButtonInput<KeyCode>>().init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>().init_resource::<Time<Real>>()
            .add_systems(Update, read);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyE);
        app.update();
        assert!(!app.world().resource::<Controls>().board, "old binding must be replaced");
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyQ);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.update();
        let c = app.world().resource::<Controls>();
        assert!(c.board && c.shoot && c.jump && c.pause && c.back);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().clear();
        app.update();
        let c = app.world().resource::<Controls>();
        assert!(!c.board && !c.jump && c.shoot, "edges fire once; held shooting continues");
    }
}
