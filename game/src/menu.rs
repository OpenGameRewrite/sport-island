//! Title (language select) and pause menus. Plain text lists so they work the
//! same with keyboard and gamepad.

use crate::input::Controls;
use crate::ipoint::{IPoints, READINGS, name_id};
use crate::lang::{Languages, Texts, Ui};
use crate::save::Save;
use crate::world::{MainCamera, TimeOfDay};
use crate::AppState;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

#[derive(Resource, Default)]
pub struct Menu {
    pub open: bool,
    page: Page,
    sel: usize,
    scroll: usize,
    capturing: bool,
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Page {
    #[default]
    Main,
    Record,
    Controls,
}

#[derive(Component)]
struct MenuRoot;
#[derive(Component)]
struct MenuLine(usize);
#[derive(Component)]
struct MenuHeader;

const LINES: usize = 16;

/// Run condition: gameplay is active (in game and not paused).
pub fn playing(menu: Res<Menu>, state: Res<State<AppState>>) -> bool { !menu.open && *state.get() == AppState::Play }

pub fn plugin(app: &mut App) {
    app.init_resource::<Menu>()
        .add_systems(Startup, spawn_ui)
        .add_systems(OnEnter(AppState::Title), |mut m: ResMut<Menu>, langs: Res<Languages>, texts: Res<Texts>| {
            m.open = true;
            m.sel = langs.0.iter().position(|l| l.code == texts.code).unwrap_or(0);
        })
        .add_systems(Update, (title_input, orbit_camera).run_if(in_state(AppState::Title)))
        .add_systems(PreUpdate, pause_input.after(crate::input::read).run_if(in_state(AppState::Play)))
        .add_systems(PreUpdate, controls_input.after(crate::input::read).before(pause_input))
        .add_systems(Update, render);
    app.add_systems(Update, cursor);
}

pub fn preview_controls(mut m: ResMut<Menu>) {
    m.open = true; m.page = Page::Controls; m.sel = 0;
}

fn cursor(menu: Res<Menu>, state: Res<State<AppState>>, mut cursor: Single<&mut CursorOptions>) {
    let active = !menu.open && *state.get() == AppState::Play;
    cursor.visible = !active;
    cursor.grab_mode = if active { CursorGrabMode::Confined } else { CursorGrabMode::None };
}

fn spawn_ui(mut commands: Commands) {
    let font = TextFont::from(FontSource::sans_serif());
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(30),
                right: percent(30),
                top: percent(12),
                padding: UiRect::all(px(24)),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.12, 0.3, 0.82)),
            GlobalZIndex(10),
            MenuRoot,
        ))
        .with_children(|p| {
            p.spawn((Text::new(""), font.clone().with_font_size(FontSize::Px(38.0)), TextColor(Color::WHITE), MenuHeader));
            for i in 0..LINES {
                p.spawn((Text::new(""), font.clone().with_font_size(FontSize::Px(25.0)), TextColor(Color::WHITE), MenuLine(i)));
            }
        });
}

fn title_input(c: Res<Controls>, langs: Res<Languages>, mut m: ResMut<Menu>, mut texts: ResMut<Texts>, mut save: ResMut<Save>, mut next: ResMut<NextState<AppState>>) {
    if m.page != Page::Main { return; }
    let n = langs.0.len();
    if c.down {
        m.sel = (m.sel + 1) % (n + 3);
    }
    if c.up {
        m.sel = (m.sel + n + 2) % (n + 3);
    }
    if m.sel >= n {
        let step = c.right as i32 - c.left as i32 + c.confirm as i32;
        if m.sel == n && c.confirm { m.page = Page::Controls; m.sel = 0; }
        if m.sel == n + 1 && step != 0 { save.plane = (save.plane as i32 + step).rem_euclid(2) as u8; save.store(); }
        if m.sel == n + 2 && step != 0 { save.livery = (save.livery as i32 + step).rem_euclid(if save.plane == 0 {3} else {2}) as u8; save.store(); }
        return;
    }
    let code = &langs.0[m.sel].code;
    if *code != texts.code {
        *texts = Texts::load(code);
    }
    if c.confirm {
        save.lang = Some(code.clone());
        save.store();
        m.open = false;
        next.set(AppState::Play);
    }
}

fn orbit_camera(time: Res<Time>, mut cam: Single<&mut Transform, With<MainCamera>>) {
    let a = time.elapsed_secs() * 0.04;
    cam.translation = Vec3::new(a.sin() * 14000.0, 4500.0, a.cos() * 14000.0);
    cam.look_at(Vec3::new(0.0, 600.0, 0.0), Vec3::Y);
}

/// Main pause menu rows.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Resume,
    Time,
    Plane,
    Livery,
    Controls,
    Invert,
    Language,
    Record,
    Quit,
}
const ROWS: [Row; 9] = [Row::Resume, Row::Time, Row::Plane, Row::Livery, Row::Controls, Row::Invert, Row::Language, Row::Record, Row::Quit];

#[allow(clippy::too_many_arguments)]
pub fn pause_input(
    mut c: ResMut<Controls>,
    mut m: ResMut<Menu>,
    mut vtime: ResMut<Time<Virtual>>,
    mut tod: ResMut<TimeOfDay>,
    mut save: ResMut<Save>,
    mut texts: ResMut<Texts>,
    langs: Res<Languages>,
    mut exit: MessageWriter<AppExit>,
) {
    if !m.open {
        if c.pause {
            m.open = true;
            m.page = Page::Main;
            m.sel = 0;
            vtime.pause();
        }
        return;
    }
    if m.page == Page::Controls { return; }
    let close = |m: &mut Menu, vtime: &mut Time<Virtual>| {
        m.open = false;
        vtime.unpause();
    };
    if m.page == Page::Record {
        if c.down {
            m.scroll = (m.scroll + 1).min(80 - LINES);
        }
        if c.up {
            m.scroll = m.scroll.saturating_sub(1);
        }
        if c.back || c.confirm || c.pause {
            m.page = Page::Main;
        }
        return;
    }
    if c.pause || c.back {
        close(&mut m, &mut vtime);
        c.shoot = false;
        c.board = false;
        c.flare = false;
        return;
    }
    if c.down {
        m.sel = (m.sel + 1) % ROWS.len();
    }
    if c.up {
        m.sel = (m.sel + ROWS.len() - 1) % ROWS.len();
    }
    let step = c.right as i32 - c.left as i32 + c.confirm as i32;
    if step == 0 {
        return;
    }
    let cycle = |v: u8, n: u8| ((v as i32 + step).rem_euclid(n as i32)) as u8;
    match ROWS[m.sel] {
        Row::Resume if c.confirm => {
            close(&mut m, &mut vtime);
            c.shoot = false;
            c.jump = false;
            c.board = false;
            c.flare = false;
        }
        Row::Time => {
            tod.0 = cycle(tod.0, 3);
            save.time = tod.0;
        }
        // Takes effect the next time you board.
        Row::Plane => save.plane = cycle(save.plane, 2),
        Row::Livery => save.livery = cycle(save.livery, if save.plane == 0 { 3 } else { 2 }),
        Row::Controls if c.confirm => { m.page = Page::Controls; m.sel = 0; }
        Row::Invert => save.invert_pitch = !save.invert_pitch,
        Row::Language => {
            let i = langs.0.iter().position(|l| l.code == texts.code).unwrap_or(0) as u8;
            let code = langs.0[cycle(i, langs.0.len() as u8) as usize].code.clone();
            *texts = Texts::load(&code);
            save.lang = Some(code);
        }
        Row::Record if c.confirm => {
            m.page = Page::Record;
            m.scroll = 0;
        }
        Row::Quit if c.confirm => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
    save.store();
}

fn controls_input(mut m: ResMut<Menu>, mut c: ResMut<Controls>, mut save: ResMut<Save>, state: Res<State<AppState>>, langs: Res<Languages>, keys: Res<ButtonInput<KeyCode>>, mouse: Res<ButtonInput<MouseButton>>, pads: Query<&Gamepad>) {
    if m.page != Page::Controls { m.capturing = false; return; }
    let n = crate::input::ACTIONS.len();
    if (!m.capturing && (c.back || c.pause)) || (m.capturing && keys.just_pressed(KeyCode::Escape)) {
        if m.capturing { m.capturing = false; } else {
            m.page = Page::Main;
            m.sel = if *state.get() == AppState::Title { langs.0.len() } else { 4 };
        }
        c.back = false; c.pause = false;
        return;
    }
    if m.capturing {
        let key = keys.get_just_pressed().next().map(|k| format!("{k:?}")).or_else(|| mouse.get_just_pressed().next().map(|b| format!("Mouse{b:?}")));
        let button = pads.iter().find_map(|p| p.get_just_pressed().next().map(|b| format!("{b:?}")));
        if key.is_some() || button.is_some() {
            let mut pair = crate::input::binding(&save, m.sel);
            if let Some(key) = key { pair[0] = key; }
            if let Some(button) = button { pair[1] = button; }
            save.bindings.insert(crate::input::ACTIONS[m.sel].0.into(), pair);
            save.store(); m.capturing = false; c.confirm = false;
        }
    } else {
        if c.down { m.sel = (m.sel + 1) % (n + 1); }
        if c.up { m.sel = (m.sel + n) % (n + 1); }
        if c.confirm {
            if m.sel == n { save.bindings.clear(); save.store(); } else { m.capturing = true; }
        }
    }
    // Consumed menu inputs must not act on a newly resumed scene.
    c.shoot = false; c.board = false; c.flare = false; c.jump = false;
}

fn livery(save: &Save) -> &'static str {
    if save.plane == 0 { ["Red", "White", "Yellow"][save.livery.min(2) as usize] } else { ["Orange", "Blue"][save.livery.min(1) as usize] }
}

#[allow(clippy::too_many_arguments)]
fn render(
    m: Res<Menu>,
    state: Res<State<AppState>>,
    texts: Res<Texts>,
    langs: Res<Languages>,
    save: Res<Save>,
    tod: Res<TimeOfDay>,
    points: Res<IPoints>,
    c: Res<Controls>,
    mut root: Single<(&mut Visibility, &mut Node), With<MenuRoot>>,
    mut header: Single<&mut Text, (With<MenuHeader>, Without<MenuLine>)>,
    mut lines: Query<(&MenuLine, &mut Text, &mut TextColor), Without<MenuHeader>>,
) {
    *root.0 = if m.open { Visibility::Inherited } else { Visibility::Hidden };
    let margin = if m.page == Page::Controls { 14.0 } else { 25.0 };
    root.1.left = percent(margin); root.1.right = percent(margin);
    if !m.open {
        return;
    }
    let mut rows: Vec<String> = vec![];
    let mut selected = Some(m.sel);
    if m.page == Page::Controls {
        header.0 = texts.ui(Ui::Controls).into();
        for i in 0..crate::input::ACTIONS.len() {
            let pair = crate::input::binding(&save, i);
            rows.push(if m.capturing && m.sel == i { format!("{}:  press a key or gamepad button…", crate::input::ACTIONS[i].0) } else { format!("{}:  {} / {}", crate::input::ACTIONS[i].0, crate::input::label(&pair[0]), crate::input::label(&pair[1])) });
        }
        rows.push(texts.ui(Ui::ResetControls).into());
        rows.push("Enter / A: choose, then press a key or button".into());
        rows.push("Move: left stick · Look: mouse / right stick / I J K L".into());
        rows.push(format!("Esc / B: {}", texts.ui(Ui::Back)));
    } else if *state.get() == AppState::Title {
        header.0 = format!("Sport Island\n{}", texts.ui(Ui::ChooseLanguage));
        rows = langs.0.iter().map(|l| l.name.clone()).collect();
        rows.push(texts.ui(Ui::Controls).into());
        rows.push(format!("{}: ◀ {} ▶", texts.ui(Ui::Plane), texts.ui(if save.plane == 0 {Ui::Plane1} else {Ui::Plane2})));
        rows.push(format!("{}: ◀ {} ▶", texts.ui(Ui::Livery), livery(&save)));
        rows.push("Enter / A: start · ↑↓ / D-pad: select".into());
    } else if m.page == Page::Main {
        header.0 = format!("Sport Island — ⓘ {} / {}", save.collected.len(), READINGS);
        let onoff = |b: bool| texts.ui(if b { Ui::On } else { Ui::Off });
        let time = texts.ui([Ui::Day, Ui::Dusk, Ui::Night][tod.0 as usize]);
        let plane = texts.ui(if save.plane == 1 { Ui::Plane2 } else { Ui::Plane1 });
        let lang = langs.0.iter().find(|l| l.code == texts.code).map_or("", |l| l.name.as_str());
        let per_time = |t: u16| save.collected.iter().filter(|&&r| r % 3 == t).count();
        for row in ROWS {
            rows.push(match row {
                Row::Resume => texts.ui(Ui::Resume).to_string(),
                Row::Time => format!("{}:  ◀ {time} ▶", texts.ui(Ui::TimeOfDay)),
                Row::Plane => format!("{}:  ◀ {plane} ▶ (next boarding)", texts.ui(Ui::Plane)),
                Row::Livery => format!("{}:  ◀ {} ▶", texts.ui(Ui::Livery), livery(&save)),
                Row::Controls => texts.ui(Ui::Controls).into(),
                Row::Invert => format!("{}:  ◀ {} ▶", texts.ui(Ui::InvertPitch), onoff(save.invert_pitch)),
                Row::Language => format!("{}:  ◀ {lang} ▶", texts.ui(Ui::Language)),
                Row::Record => format!(
                    "{}:  {} / {}   ({} {} · {} {} · {} {})",
                    texts.ui(Ui::IPoints),
                    save.collected.len(),
                    READINGS,
                    texts.ui(Ui::Day),
                    per_time(0),
                    texts.ui(Ui::Dusk),
                    per_time(1),
                    texts.ui(Ui::Night),
                    per_time(2)
                ),
                Row::Quit => texts.ui(Ui::Quit).to_string(),
            });
        }
    } else {
        header.0 = format!("{} — {} / {}", texts.ui(Ui::IPoints), save.collected.len(), READINGS);
        selected = None;
        for i in m.scroll..(m.scroll + LINES).min(80) {
            let mark = |t: u16| if save.collected.contains(&((i * 3) as u16 + t)) { "●" } else { "○" };
            let name = if (0..3).any(|t| save.collected.contains(&((i * 3) as u16 + t))) { texts.msg(name_id(&points.0[i]), c.pad) } else { "?".to_string() };
            rows.push(format!("{:>2}  {}{}{}  {}", i + 1, mark(0), mark(1), mark(2), name));
        }
    }
    for (l, mut t, mut col) in &mut lines {
        t.0 = rows.get(l.0).cloned().unwrap_or_default();
        col.0 = if selected == Some(l.0) { Color::srgb(1.0, 0.85, 0.2) } else { Color::WHITE };
    }
}
