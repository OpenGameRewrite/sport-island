//! Shared title/pause menus with keyboard, gamepad and pointer navigation.
use crate::input::Controls;
use crate::ipoint::{IPoints, READINGS, name_id};
use crate::lang::{Languages, Texts, Ui};
use crate::save::Save;
use crate::world::{MainCamera, TimeOfDay};
use crate::AppState;
use bevy::prelude::*;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::window::{CursorGrabMode, CursorOptions};
use bevy::picking::hover::Hovered;
use bevy::ui_widgets::{Activate, Button};

#[derive(Resource, Default)]
pub struct Menu { pub open: bool, page: Page, sel: usize, scroll: usize, capturing: bool, direct_record: bool }
#[derive(Default, PartialEq, Clone, Copy)]
enum Page { #[default] Main, Record, Controls, Settings }
#[derive(Component)] struct MenuRoot;
#[derive(Component)] struct MenuLine(usize);
#[derive(Component)] struct MenuHeader;
#[derive(Component)] struct MenuHelp;
#[derive(Component)] struct Item(usize);
#[derive(Component)] struct Footer(i32);
#[derive(Component)] pub struct RecordButton;
#[derive(Message)] pub enum Click { Row(usize), Step(i32), Record }
const LINES: usize = 16;
#[derive(Clone, Copy, PartialEq)]
enum Row { Start, Resume, Time, Plane, Livery, Controls, Settings, Language, Record, Quit }
const TITLE: &[Row] = &[Row::Start, Row::Language, Row::Plane, Row::Livery, Row::Controls, Row::Settings, Row::Quit];
const PAUSE: &[Row] = &[Row::Resume, Row::Time, Row::Plane, Row::Livery, Row::Controls, Row::Settings, Row::Language, Row::Record, Row::Quit];

pub fn playing(menu: Res<Menu>, state: Res<State<AppState>>) -> bool { !menu.open && *state.get() == AppState::Play }
pub fn plugin(app: &mut App) {
    app.init_resource::<Menu>()
        .add_message::<Click>()
        .add_systems(Startup, spawn_ui)
        .add_systems(OnEnter(AppState::Title), |mut m: ResMut<Menu>| { *m = Menu { open: true, ..default() }; })
        .add_systems(PreUpdate, (mouse_input, controls_input, pause_input).chain().after(crate::input::read))
        .add_systems(Update, orbit_camera.run_if(in_state(AppState::Title)))
        .add_systems(Update, (render, cursor));
}
pub fn preview_controls(mut m: ResMut<Menu>) {
    m.open = true; m.sel = 0;
    m.page = match std::env::var("SI_MENU").as_deref() {Ok("settings")=>Page::Settings,Ok("record")=>Page::Record,_=>Page::Controls};
}
fn cursor(menu: Res<Menu>, state: Res<State<AppState>>, mut cursor: Single<&mut CursorOptions>) {
    let active = !menu.open && *state.get() == AppState::Play;
    cursor.visible = !active; cursor.grab_mode = if active { CursorGrabMode::Confined } else { CursorGrabMode::None };
}
fn spawn_ui(mut commands: Commands) {
    let font = TextFont::from(FontSource::sans_serif());
    commands.spawn((Node { position_type: PositionType::Absolute, left: percent(20), right: percent(20), top: percent(5), padding: UiRect::all(px(20)), flex_direction: FlexDirection::Column, row_gap: px(3), border_radius: BorderRadius::all(px(18)), ..default() }, BackgroundColor(Color::srgba(0.02, 0.12, 0.3, 0.9)), GlobalZIndex(10), MenuRoot)).with_children(|p| {
        p.spawn((Text::new(""), font.clone().with_font_size(FontSize::Px(32.0)), TextColor(Color::WHITE), MenuHeader));
        for i in 0..LINES {
            p.spawn((Button, Hovered::default(), Item(i), Node { padding: UiRect::axes(px(10), px(3)), border_radius: BorderRadius::all(px(6)), ..default() }, BackgroundColor(Color::NONE))).observe(click).with_child((Text::new(""), font.clone().with_font_size(FontSize::Px(22.0)), TextColor(Color::WHITE), MenuLine(i)));
        }
        p.spawn((Text::new(""), font.clone().with_font_size(FontSize::Px(15.0)), TextColor(Color::srgb(0.65, 0.8, 0.95)), MenuHelp));
        p.spawn(Node { column_gap: px(14), justify_content: JustifyContent::SpaceBetween, ..default() }).with_children(|p| {
            for (step, label) in [(-1, "◀"), (0, ""), (1, "▶")] {
                p.spawn((Button, Footer(step), Node { padding: UiRect::axes(px(22), px(6)), border_radius: BorderRadius::all(px(8)), ..default() }, BackgroundColor(Color::srgb(0.1, 0.25, 0.45)))).observe(click).with_child((Text::new(label), font.clone().with_font_size(FontSize::Px(22.0)), TextColor(Color::WHITE)));
            }
        });
    });
}
fn click(event: On<Activate>, items: Query<&Item>, footers: Query<&Footer>, mut clicks: MessageWriter<Click>) {
    if let Ok(item) = items.get(event.entity) { clicks.write(Click::Row(item.0)); }
    if let Ok(footer) = footers.get(event.entity) { clicks.write(Click::Step(footer.0)); }
}
pub fn record_click(_: On<Activate>, mut clicks: MessageWriter<Click>) { clicks.write(Click::Record); }
fn mouse_input(mut m: ResMut<Menu>, mut c: ResMut<Controls>, mut items: Query<(&Hovered, &mut BackgroundColor), With<Item>>, mut clicks: MessageReader<Click>, wheel: Res<AccumulatedMouseScroll>) {
    let clicks: Vec<_> = clicks.read().collect();
    if !m.open || m.capturing { return; }
    for (hovered, mut color) in &mut items {
        color.0 = if !hovered.get() { Color::NONE } else { Color::srgba(0.2, 0.45, 0.7, 0.5) };
    }
    for click in clicks {
        match *click {
            Click::Row(i) if m.page != Page::Record => { m.sel = i; c.confirm = true; }
            Click::Step(-1) => c.left = true, Click::Step(1) => c.right = true, Click::Step(_) => c.back = true,
            Click::Record => { m.page = Page::Record; m.scroll = 0; m.direct_record = false; }
            _ => {}
        }
    }
    if wheel.delta.y != 0.0 { if wheel.delta.y > 0.0 { c.up = true; } else { c.down = true; } }
}
fn orbit_camera(time: Res<Time>, mut cam: Single<&mut Transform, With<MainCamera>>) {
    let a = time.elapsed_secs() * 0.04;
    cam.translation = Vec3::new(a.sin() * 14000.0, 4500.0, a.cos() * 14000.0);
    cam.look_at(Vec3::new(0.0, 600.0, 0.0), Vec3::Y);
}
fn navigate(sel: &mut usize, n: usize, c: &Controls) { if c.down { *sel = (*sel + 1) % n; } if c.up { *sel = (*sel + n - 1) % n; } }
fn record_scroll(scroll: usize, c: &Controls) -> usize {
    (scroll as i32 + c.down as i32 - c.up as i32 + (c.right as i32 - c.left as i32) * LINES as i32).clamp(0, (80 - LINES) as i32) as usize
}
fn close(m: &mut Menu, time: &mut Time<Virtual>, c: &mut Controls) {
    m.open = false; m.direct_record = false; time.unpause();
    c.shoot = false; c.jump = false; c.board = false; c.flare = false;
}
#[allow(clippy::too_many_arguments)]
pub fn pause_input(mut c: ResMut<Controls>, mut m: ResMut<Menu>, state: Res<State<AppState>>, mut next: ResMut<NextState<AppState>>, mut vtime: ResMut<Time<Virtual>>, mut tod: ResMut<TimeOfDay>, mut save: ResMut<Save>, mut texts: ResMut<Texts>, langs: Res<Languages>, mut exit: MessageWriter<AppExit>) {
    let title = *state.get() == AppState::Title;
    if !title && c.speedometer { save.speedometer = !save.speedometer; save.store(); }
    if !m.open {
        if c.pause || c.record { m.open = true; m.page = if c.record {Page::Record} else {Page::Main}; m.direct_record = c.record; m.sel = 0; vtime.pause(); }
        return;
    }
    if m.page == Page::Controls { return; }
    if m.page == Page::Record {
        m.scroll = record_scroll(m.scroll, &c);
        if c.back || c.pause || c.record {
            if m.direct_record { close(&mut m, &mut vtime, &mut c); } else { m.page = Page::Main; m.sel = 7; }
        }
        return;
    }
    if !title && c.record { m.page = Page::Record; m.scroll = 0; m.direct_record = false; return; }
    if m.page == Page::Settings {
        if c.back || c.pause { m.page = Page::Main; m.sel = 5; return; }
        navigate(&mut m.sel, 4, &c);
        let step = c.right as i32 - c.left as i32 + c.confirm as i32;
        if step != 0 {
            match m.sel {
                0 => save.flight_sensitivity = (save.flight_sensitivity + step as f32 * 0.1).clamp(0.25, 1.5),
                1 => save.look_sensitivity = (save.look_sensitivity + step as f32 * 0.1).clamp(0.25, 1.5),
                2 => save.invert_pitch = !save.invert_pitch,
                _ => save.speedometer = !save.speedometer,
            }
            save.store();
        }
        return;
    }
    if c.back || c.pause { if !title { close(&mut m, &mut vtime, &mut c); } return; }
    let rows = if title {TITLE} else {PAUSE};
    navigate(&mut m.sel, rows.len(), &c);
    let step = c.right as i32 - c.left as i32 + c.confirm as i32;
    if step == 0 { return; }
    let cycle = |v: u8, n: u8| ((v as i32 + step).rem_euclid(n as i32)) as u8;
    match rows[m.sel.min(rows.len()-1)] {
        Row::Start if c.confirm => { save.lang = Some(texts.code.clone()); close(&mut m, &mut vtime, &mut c); next.set(AppState::Play); }
        Row::Resume if c.confirm => close(&mut m, &mut vtime, &mut c),
        Row::Time => { tod.0 = cycle(tod.0, 3); save.time = tod.0; }
        Row::Plane => save.plane = cycle(save.plane, 2),
        Row::Livery => save.livery = cycle(save.livery, if save.plane == 0 {3} else {2}),
        Row::Controls if c.confirm => { m.page = Page::Controls; m.sel = 0; }
        Row::Settings if c.confirm => { m.page = Page::Settings; m.sel = 0; }
        Row::Language => {
            let i = langs.0.iter().position(|l| l.code == texts.code).unwrap_or(0) as u8;
            let code = langs.0[cycle(i, langs.0.len() as u8) as usize].code.clone();
            *texts = Texts::load(&code); save.lang = Some(code);
        }
        Row::Record if c.confirm => { m.page = Page::Record; m.scroll = 0; }
        Row::Quit if c.confirm => { exit.write(AppExit::Success); }
        _ => {}
    }
    save.store();
}
fn controls_input(mut m: ResMut<Menu>, mut c: ResMut<Controls>, mut save: ResMut<Save>, keys: Res<ButtonInput<KeyCode>>, mouse: Res<ButtonInput<MouseButton>>, pads: Query<&Gamepad>) {
    if m.page != Page::Controls { m.capturing = false; return; }
    let n = crate::input::ACTIONS.len();
    if (!m.capturing && (c.back || c.pause)) || (m.capturing && keys.just_pressed(KeyCode::Escape)) {
        if m.capturing { m.capturing = false; } else { m.page = Page::Main; m.sel = 4; }
        c.back = false; c.pause = false; c.confirm = false; return;
    }
    if m.capturing {
        let key = keys.get_just_pressed().next().map(|k| format!("{k:?}")).or_else(|| mouse.get_just_pressed().next().map(|b| format!("Mouse{b:?}")));
        let button = pads.iter().find_map(|p| p.get_just_pressed().next().map(|b| format!("{b:?}")));
        if key.is_some() || button.is_some() {
            let mut pair = crate::input::binding(&save, m.sel);
            if let Some(key) = key { pair[0] = key; } if let Some(button) = button { pair[1] = button; }
            save.bindings.insert(crate::input::ACTIONS[m.sel].0.into(), pair); save.store(); m.capturing = false;
        }
    } else {
        navigate(&mut m.sel, n+1, &c);
        if c.confirm { if m.sel == n { save.bindings.clear(); save.store(); } else { m.capturing = true; } }
    }
    c.confirm = false; c.shoot = false; c.board = false; c.flare = false; c.jump = false;
}
#[allow(clippy::too_many_arguments)]
fn render(m: Res<Menu>, state: Res<State<AppState>>, texts: Res<Texts>, langs: Res<Languages>, save: Res<Save>, tod: Res<TimeOfDay>, points: Res<IPoints>, c: Res<Controls>, window: Single<&Window>, mut root: Single<(&mut Visibility, &mut Node), (With<MenuRoot>, Without<Item>)>, mut header: Single<(&mut Text, &mut TextFont), (With<MenuHeader>, Without<MenuLine>, Without<MenuHelp>)>, mut help: Single<&mut Text, (With<MenuHelp>, Without<MenuHeader>, Without<MenuLine>)>, mut lines: Query<(&MenuLine, &mut Text, &mut TextColor, &mut TextFont), (Without<MenuHeader>, Without<MenuHelp>)>, mut items: Query<(&Item, &mut Node), Without<MenuRoot>>, footers: Query<(&Footer, &Children)>, mut footer_texts: Query<&mut Text, (Without<MenuHeader>, Without<MenuLine>, Without<MenuHelp>)>) {
    *root.0 = if m.open {Visibility::Inherited} else {Visibility::Hidden};
    let margin = if m.page == Page::Controls || m.page == Page::Record {10.0} else {20.0}; root.1.left = percent(margin); root.1.right = percent(margin);
    if !m.open { return; }
    let onoff = |b| texts.ui(if b {Ui::On} else {Ui::Off});
    let mut selected = Some(m.sel);
    let rows: Vec<String> = match m.page {
        Page::Controls => {
            header.0.0 = texts.ui(Ui::Controls).into(); help.0 = texts.ui(Ui::ControlsHint).into();
            let mut rows: Vec<_> = (0..crate::input::ACTIONS.len()).map(|i| {
                let pair = crate::input::binding(&save, i);
                format!("{}:  {}", texts.action(i), if m.capturing && m.sel == i {texts.ui(Ui::Waiting).into()} else {format!("{} / {}", texts.binding(&pair[0]), texts.binding(&pair[1]))})
            }).collect(); rows.push(texts.ui(Ui::ResetControls).into()); rows
        }
        Page::Settings => {
            header.0.0 = texts.ui(Ui::Settings).into(); help.0 = texts.ui(Ui::MenuHint).into();
            vec![format!("{}: ◀ {:.0}% ▶", texts.ui(Ui::FlightSensitivity), save.flight_sensitivity*100.0), format!("{}: ◀ {:.0}% ▶", texts.ui(Ui::LookSensitivity), save.look_sensitivity*100.0), format!("{}: ◀ {} ▶", texts.ui(Ui::InvertPitch), onoff(save.invert_pitch)), format!("{}: ◀ {} ▶", texts.ui(Ui::Speedometer), onoff(save.speedometer))]
        }
        Page::Record => {
            header.0.0 = format!("{} — {} / {}   ({}–{} / 80)", texts.ui(Ui::IPoints), save.collected.len(), READINGS, m.scroll+1, m.scroll+LINES);
            help.0 = texts.ui(Ui::RecordHint).into(); selected = None;
            (m.scroll..m.scroll+LINES).map(|i| {
                let mark = |t| if save.collected.contains(&((i*3) as u16+t)) {"●"} else {"○"};
                let name = if (0..3).any(|t| save.collected.contains(&((i*3) as u16+t))) {texts.msg(name_id(&points.0[i]), c.pad)} else {"?".into()};
                format!("{:>2}   {} {} {}   {}", i+1, mark(0), mark(1), mark(2), name)
            }).collect()
        }
        Page::Main => {
            let title = *state.get() == AppState::Title;
            header.0.0 = "Sport Island".into(); help.0 = texts.ui(Ui::MenuHint).into();
            let lang = langs.0.iter().find(|l| l.code == texts.code).map_or("", |l| l.name.as_str());
            let livery = if save.plane == 0 { [Ui::Red, Ui::White, Ui::Yellow][save.livery.min(2) as usize] } else { [Ui::Orange, Ui::Blue][save.livery.min(1) as usize] };
            (if title {TITLE} else {PAUSE}).iter().map(|row| match row {
                Row::Start => texts.ui(Ui::Start).into(), Row::Resume => texts.ui(Ui::Resume).into(),
                Row::Time => format!("{}: ◀ {} ▶", texts.ui(Ui::TimeOfDay), texts.ui([Ui::Day, Ui::Dusk, Ui::Night][tod.0 as usize])),
                Row::Plane => format!("{}: ◀ {} ▶{}", texts.ui(Ui::Plane), texts.ui(if save.plane == 0 {Ui::Plane1} else {Ui::Plane2}), if title {String::new()} else {format!(" ({})", texts.ui(Ui::NextBoarding))}),
                Row::Livery => format!("{}: ◀ {} ▶", texts.ui(Ui::Livery), texts.ui(livery)),
                Row::Controls => texts.ui(Ui::Controls).into(), Row::Settings => texts.ui(Ui::Settings).into(),
                Row::Language => format!("{}: ◀ {lang} ▶", texts.ui(Ui::Language)),
                Row::Record => format!("{}: {} / {}", texts.ui(Ui::IPoints), save.collected.len(), READINGS), Row::Quit => texts.ui(Ui::Quit).into(),
            }).collect()
        }
    };
    header.1.font_size = FontSize::Px((window.height()*0.038).clamp(20.0, 32.0));
    for (l, mut t, mut col, mut font) in &mut lines {
        t.0 = rows.get(l.0).cloned().unwrap_or_default();
        col.0 = if selected == Some(l.0) {Color::srgb(1.0, 0.85, 0.2)} else {Color::WHITE};
        font.font_size = FontSize::Px((window.height()*0.026).clamp(14.0, 22.0));
    }
    for (item, mut node) in &mut items { node.display = if item.0 < rows.len() {Display::Flex} else {Display::None}; }
    for (footer, children) in &footers { if footer.0 == 0 { for e in children { if let Ok(mut t) = footer_texts.get_mut(*e) { t.0 = texts.ui(Ui::Back).into(); } } } }
}
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    #[test] fn pointer_activation_selects_rows_and_opens_the_paused_counter() {
        let mut w = World::new();
        w.insert_resource(Menu {open:true,..default()}); w.init_resource::<Controls>();
        w.init_resource::<Messages<Click>>(); w.init_resource::<AccumulatedMouseScroll>();
        let row = w.spawn(Item(2)).observe(click).id();
        w.trigger(Activate {entity:row}); w.run_system_once(mouse_input).unwrap();
        assert_eq!(w.resource::<Menu>().sel,2);
        assert!(w.resource::<Controls>().confirm);
        let counter = w.spawn(RecordButton).observe(record_click).id();
        w.trigger(Activate {entity:counter}); w.run_system_once(mouse_input).unwrap();
        assert!(w.resource::<Menu>().page == Page::Record);
        w.resource_mut::<Menu>().open = false;
        w.resource_mut::<Menu>().page = Page::Main;
        w.trigger(Activate {entity:counter}); w.run_system_once(mouse_input).unwrap();
        assert!(w.resource::<Menu>().page == Page::Main, "counter opens only while paused");
    }
    #[test] fn navigation_and_record_pages_are_bounded() {
        let mut sel = 0; let c = Controls { down: true, ..default() };
        for _ in 0..3 { navigate(&mut sel, TITLE.len(), &c); }
        assert_eq!(TITLE[sel] as u8, Row::Livery as u8);
        assert_eq!(record_scroll(0, &Controls {left:true,..default()}),0);
        assert_eq!(record_scroll(0, &Controls {right:true,..default()}),16);
        assert_eq!(record_scroll(63, &Controls {right:true,..default()}),64);
    }
    #[test] fn title_navigation_preserves_language_and_record_shortcut_pauses_then_resumes() {
        let mut w = World::new();
        w.insert_resource(State::new(AppState::Title)); w.init_resource::<NextState<AppState>>();
        w.insert_resource(Menu {open:true,..default()}); w.insert_resource(Controls {down:true,..default()});
        w.init_resource::<Time<Virtual>>(); w.insert_resource(TimeOfDay(0)); w.init_resource::<Save>();
        let mut texts=Texts::default(); texts.code="FR".into();
        w.insert_resource(texts); w.insert_resource(Languages(vec![])); w.init_resource::<Messages<AppExit>>();
        for _ in 0..3 { w.run_system_once(pause_input).unwrap(); }
        assert_eq!(w.resource::<Texts>().code,"FR", "browsing options must not select a language");
        assert_eq!(w.resource::<Menu>().sel,3);
        w.insert_resource(State::new(AppState::Play)); w.insert_resource(Menu::default());
        w.insert_resource(Controls {record:true,..default()}); w.run_system_once(pause_input).unwrap();
        assert!(w.resource::<Menu>().open && w.resource::<Time<Virtual>>().is_paused());
        assert!(w.resource::<Menu>().page==Page::Record);
        w.run_system_once(pause_input).unwrap();
        assert!(!w.resource::<Menu>().open && !w.resource::<Time<Virtual>>().is_paused());
    }
}
