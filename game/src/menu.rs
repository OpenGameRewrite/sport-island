//! Title (language select) and pause menus. Plain text lists so they work the
//! same with keyboard and gamepad.

use crate::input::Controls;
use crate::ipoint::{IPoints, READINGS, name_id};
use crate::lang::{Languages, Texts, Ui};
use crate::save::Save;
use crate::world::{MainCamera, TimeOfDay};
use crate::{AppState, Mode};
use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct Menu {
    pub open: bool,
    page: Page,
    sel: usize,
    scroll: usize,
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Page {
    #[default]
    Main,
    Record,
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
        .add_systems(Update, pause_input.run_if(in_state(AppState::Play)))
        .add_systems(Update, render);
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
    let n = langs.0.len();
    if c.down {
        m.sel = (m.sel + 1) % n;
    }
    if c.up {
        m.sel = (m.sel + n - 1) % n;
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
    Invert,
    Language,
    Record,
    Quit,
}
const ROWS: [Row; 7] = [Row::Resume, Row::Time, Row::Plane, Row::Invert, Row::Language, Row::Record, Row::Quit];

#[allow(clippy::too_many_arguments)]
fn pause_input(
    c: Res<Controls>,
    mut m: ResMut<Menu>,
    mut vtime: ResMut<Time<Virtual>>,
    mut tod: ResMut<TimeOfDay>,
    mut save: ResMut<Save>,
    mut texts: ResMut<Texts>,
    langs: Res<Languages>,
    mode: Res<State<Mode>>,
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
    let close = |m: &mut Menu, vtime: &mut Time<Virtual>| {
        m.open = false;
        vtime.unpause();
    };
    if m.page == Page::Record {
        if c.down {
            m.scroll = (m.scroll + 1).min(80 - LINES + 1);
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
        Row::Resume if c.confirm => close(&mut m, &mut vtime),
        Row::Time => {
            tod.0 = cycle(tod.0, 3);
            save.time = tod.0;
        }
        // Takes effect the next time you board.
        Row::Plane if *mode.get() == Mode::Walk || c.confirm => save.plane = cycle(save.plane, 2),
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
    mut root: Single<&mut Visibility, With<MenuRoot>>,
    mut header: Single<&mut Text, (With<MenuHeader>, Without<MenuLine>)>,
    mut lines: Query<(&MenuLine, &mut Text, &mut TextColor), Without<MenuHeader>>,
) {
    **root = if m.open { Visibility::Inherited } else { Visibility::Hidden };
    if !m.open {
        return;
    }
    let mut rows: Vec<String> = vec![];
    let mut selected = Some(m.sel);
    if *state.get() == AppState::Title {
        header.0 = format!("Sport Island\n{}", texts.ui(Ui::ChooseLanguage));
        rows = langs.0.iter().map(|l| l.name.clone()).collect();
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
                Row::Plane => format!("{}:  ◀ {plane} ▶", texts.ui(Ui::Plane)),
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
