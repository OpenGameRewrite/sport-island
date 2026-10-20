//! Shared title/pause menus with keyboard, gamepad and pointer navigation.
use crate::input::Controls;
use crate::ipoint::IPoints;
use crate::lang::{Languages, Texts, Ui};
use crate::save::Save;
use crate::world::{MainCamera, TimeOfDay};
use crate::AppState;
use bevy::prelude::*;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy::picking::hover::Hovered;
use bevy::ui_widgets::{Activate, Button};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingGraphics {
    pub fullscreen: bool,
    pub aspect_ratio: u8,
    pub resolution_w: u32,
    pub resolution_h: u32,
    pub msaa: u8,
}

impl PendingGraphics {
    pub fn from_save(save: &Save) -> Self {
        Self {
            fullscreen: save.fullscreen,
            aspect_ratio: save.aspect_ratio,
            resolution_w: save.resolution_w,
            resolution_h: save.resolution_h,
            msaa: save.msaa,
        }
    }
}

#[derive(Resource, Default)]
pub struct Menu {
    pub open: bool,
    page: Page,
    sel: usize,
    scroll: usize,
    capturing: bool,
    direct_record: bool,
    pending_graphics: Option<PendingGraphics>,
}
#[derive(Default, PartialEq, Clone, Copy, Debug)]
enum Page { #[default] Main, Record, Controls, Settings, Graphics }
#[derive(Component)] struct MenuRoot;
#[derive(Component)] struct MenuPanel;
#[derive(Component)] struct MenuLine(usize);
#[derive(Component)] struct MenuValue(usize);
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
        .add_systems(PreUpdate, (release_for_system_ui, mouse_input, controls_input, pause_input).chain().after(crate::input::read))
        .add_systems(Update, orbit_camera.run_if(in_state(AppState::Title)))
        .add_systems(Update, (render, cursor, window_mode));
}
fn window_mode(save: Res<Save>, mut window: Single<&mut Window, With<PrimaryWindow>>, mut applied: Local<Option<(u32, u32, bool)>>) {
    use bevy::window::{MonitorSelection, WindowMode};
    let target = (save.resolution_w, save.resolution_h, save.fullscreen);
    let mode = if save.fullscreen { WindowMode::BorderlessFullscreen(MonitorSelection::Current) } else { WindowMode::Windowed };
    if window.mode != mode { window.mode = mode; }
    if applied.as_ref() != Some(&target) {
        if !save.fullscreen
            && (window.resolution.physical_width() != save.resolution_w || window.resolution.physical_height() != save.resolution_h)
        {
            window.resolution.set_physical_resolution(save.resolution_w, save.resolution_h);
        }
        *applied = Some(target);
    }
}
pub fn preview_controls(mut m: ResMut<Menu>) {
    m.open = true; m.sel = 0;
    m.page = match std::env::var("SI_MENU").as_deref() {Ok("settings")=>Page::Settings,Ok("graphics")=>Page::Graphics,Ok("record")=>Page::Record,_=>Page::Controls};
    if let Some(s) = std::env::var("SI_MENU_SCROLL").ok().and_then(|s| s.parse().ok()) { m.scroll = s; }
}
fn system_ui(window: &Window, keys: &ButtonInput<KeyCode>) -> bool {
    !window.focused || keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight, KeyCode::PrintScreen])
        || (keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]) && keys.pressed(KeyCode::Tab))
}
/// Release before Win+Shift+S/Print Screen opens its overlay, even if Windows
/// hasn't reported focus loss yet. Returning leaves the game paused until Resume.
fn release_for_system_ui(window: Single<&Window, With<PrimaryWindow>>, keys: Res<ButtonInput<KeyCode>>, state: Res<State<AppState>>, mut menu: ResMut<Menu>, mut time: ResMut<Time<Virtual>>, mut c: ResMut<Controls>) {
    // Unattended screenshot runs (SI_SHOT) are usually unfocused.
    if !system_ui(&window, &keys) || std::env::var_os("SI_SHOT").is_some() { return; }
    *c = Controls { pad: c.pad, ..default() };
    if !menu.open && *state.get() == AppState::Play {
        *menu = Menu { open: true, ..default() };
        time.pause();
    }
}
fn cursor(menu: Res<Menu>, state: Res<State<AppState>>, keys: Res<ButtonInput<KeyCode>>, mut window: Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>) {
    let active = !menu.open && *state.get() == AppState::Play && !system_ui(window.0, &keys) && std::env::var_os("SI_SHOT").is_none();
    // Avoid reapplying capture while a system overlay owns the pointer.
    if window.1.visible == active { window.1.visible = !active; }
    let grab = if active { CursorGrabMode::Confined } else { CursorGrabMode::None };
    if window.1.grab_mode != grab { window.1.grab_mode = grab; }
}
// Theme after the Wii Sports Resort menus (colours only, no original art): an
// off-white panel, a sky-blue title banner, white rounded buttons with a grey
// outline, and a thick cyan outline that grows the selected button.
pub(crate) const PANEL: Color = Color::srgba(0.95, 0.96, 0.97, 0.94);
pub(crate) const INK: Color = Color::srgb(0.24, 0.24, 0.27);
pub(crate) const ACCENT: Color = Color::srgb(0.0, 0.6, 0.95);
const BUTTON: Color = Color::WHITE;
pub(crate) const OUTLINE: Color = Color::srgb(0.78, 0.8, 0.84);
const HOVER: Color = Color::srgb(0.55, 0.8, 0.97);
const SELECTED: Color = Color::srgb(0.9, 0.96, 1.0);
const HINT: Color = Color::srgb(0.45, 0.47, 0.52);

fn spawn_ui(mut commands: Commands, font: Res<crate::lang::UiFont>) {
    let font = TextFont::from(font.0.clone());
    let overlay = Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() };
    let button = |padding: UiRect| Node { padding, border: UiRect::all(px(3)), border_radius: BorderRadius::all(px(16)), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, column_gap: px(16), ..default() };
    commands.spawn((overlay, GlobalZIndex(10), MenuRoot)).with_children(|p| {
        p.spawn((
            Node { width: px(640), max_width: percent(92), max_height: percent(94), padding: UiRect::new(px(26), px(26), px(0), px(20)), flex_direction: FlexDirection::Column, row_gap: px(8), border_radius: BorderRadius::all(px(28)), overflow: Overflow::clip(), ..default() },
            BackgroundColor(PANEL),
            BoxShadow::new(Color::srgba(0.0, 0.15, 0.3, 0.35), px(0), px(10), px(0), px(30)),
            MenuPanel,
        )).with_children(|p| {
            // Title banner across the top of the panel.
            p.spawn((Node { margin: UiRect::new(px(-26), px(-26), px(0), px(10)), padding: UiRect::axes(px(24), px(14)), justify_content: JustifyContent::Center, border_radius: BorderRadius::top(px(28)), ..default() }, BackgroundColor(ACCENT)))
                .with_child((Text::new(""), font.clone().with_font_size(FontSize::Px(34.0)), TextColor(Color::WHITE), TextLayout::justify(Justify::Center), MenuHeader));
            for i in 0..LINES {
                p.spawn((Button, Hovered::default(), Item(i), button(UiRect::axes(px(20), px(8))), BackgroundColor(BUTTON), BorderColor::all(OUTLINE), UiTransform::default()))
                    .observe(click)
                    .with_children(|p| {
                        p.spawn((Text::new(""), font.clone().with_font_size(FontSize::Px(22.0)), TextColor(INK), MenuLine(i)));
                        p.spawn((Text::new(""), font.clone().with_font_size(FontSize::Px(22.0)), TextColor(ACCENT), MenuValue(i)));
                    });
            }
            p.spawn((Text::new(""), font.clone().with_font_size(FontSize::Px(15.0)), TextColor(HINT), TextLayout::justify(Justify::Center), Node { margin: UiRect::top(px(6)), ..default() }, MenuHelp));
            // Back on the left, record paging on the right, as in Wii menus.
            p.spawn(Node { column_gap: px(10), align_items: AlignItems::Center, margin: UiRect::top(px(4)), ..default() }).with_children(|p| {
                for (step, label) in [(0, ""), (-1, "‹"), (1, "›")] {
                    let mut node = button(UiRect::axes(px(if step == 0 {28} else {18}), px(6)));
                    if step == -1 { node.margin = UiRect::left(Val::Auto); }
                    p.spawn((Button, Hovered::default(), Footer(step), node, BackgroundColor(BUTTON), BorderColor::all(OUTLINE)))
                        .observe(click)
                        .with_child((Text::new(label), font.clone().with_font_size(FontSize::Px(20.0)), TextColor(INK)));
                }
            });
        });
    });
}
fn click(event: On<Activate>, items: Query<&Item>, footers: Query<&Footer>, mut clicks: MessageWriter<Click>) {
    if let Ok(item) = items.get(event.entity) { clicks.write(Click::Row(item.0)); }
    if let Ok(footer) = footers.get(event.entity) { clicks.write(Click::Step(footer.0)); }
}
pub fn record_click(_: On<Activate>, mut clicks: MessageWriter<Click>) { clicks.write(Click::Record); }
fn mouse_input(mut m: ResMut<Menu>, mut c: ResMut<Controls>, mut clicks: MessageReader<Click>, wheel: Res<AccumulatedMouseScroll>) {
    let clicks: Vec<_> = clicks.read().collect();
    if !m.open || m.capturing { return; }
    for click in clicks {
        match *click {
            Click::Row(i) if m.page != Page::Record => { m.sel = i + if m.page == Page::Controls { m.scroll } else { 0 }; c.confirm = true; }
            Click::Step(-1) => c.left = true, Click::Step(1) => c.right = true, Click::Step(_) => c.back = true,
            Click::Record => { m.page = Page::Record; m.scroll = 0; m.direct_record = false; }
            _ => {}
        }
    }
    if wheel.delta.y != 0.0 { if wheel.delta.y > 0.0 { c.up = true; } else { c.down = true; } }
}
fn orbit_camera(time: Res<Time>, mut cam: Single<(&mut Transform, &mut Projection), With<MainCamera>>) {
    let (ref mut cam_transform, ref mut cam_proj) = *cam;
    let a = time.elapsed_secs() * 0.04;
    cam_transform.translation = Vec3::new(a.sin() * 14000.0, 4500.0, a.cos() * 14000.0);
    cam_transform.look_at(Vec3::new(0.0, 600.0, 0.0), Vec3::Y);
    if let Projection::Perspective(ref mut proj) = **cam_proj {
        proj.fov = crate::world::BASE_FOV_DEGREES.to_radians();
    }
}
fn navigate(sel: &mut usize, n: usize, c: &Controls) { if c.down { *sel = (*sel + 1) % n; } if c.up { *sel = (*sel + n - 1) % n; } }
fn record_scroll(scroll: usize, c: &Controls, total: usize) -> usize {
    (scroll as i32 + c.down as i32 - c.up as i32 + (c.right as i32 - c.left as i32) * LINES as i32).clamp(0, total.saturating_sub(LINES) as i32) as usize
}
fn close(m: &mut Menu, time: &mut Time<Virtual>, c: &mut Controls) {
    m.open = false; m.direct_record = false; m.pending_graphics = None; time.unpause();
    c.shoot = false; c.jump = false; c.board = false; c.flare = false;
}
#[allow(clippy::too_many_arguments)]
pub fn pause_input(
    mut c: ResMut<Controls>, mut m: ResMut<Menu>, state: Res<State<AppState>>, mut next: ResMut<NextState<AppState>>,
    mut vtime: ResMut<Time<Virtual>>, mut tod: ResMut<TimeOfDay>, mut save: ResMut<Save>, mut texts: ResMut<Texts>,
    langs: Res<Languages>, points: Res<IPoints>, extras: Option<Res<crate::collect::Items>>, mut exit: MessageWriter<AppExit>,
    monitors: Query<(&bevy::window::Monitor, Has<bevy::window::PrimaryMonitor>)>,
) {
    let title = *state.get() == AppState::Title;
    if !title && c.speedometer { save.speedometer = !save.speedometer; save.store(); }
    if !m.open {
        if c.pause || c.record { m.open = true; m.page = if c.record {Page::Record} else {Page::Main}; m.direct_record = c.record; m.sel = 0; vtime.pause(); }
        return;
    }
    if m.page == Page::Controls { return; }
    if m.page == Page::Record {
        let extra = extras.map_or(0, |x| x.record(&save, &texts).len());
        m.scroll = record_scroll(m.scroll, &c, points.0.len() + extra);
        if c.back || c.pause || c.record {
            if m.direct_record { close(&mut m, &mut vtime, &mut c); } else { m.page = Page::Main; m.sel = 7; }
        }
        return;
    }
    if !title && c.record { m.page = Page::Record; m.scroll = 0; m.direct_record = false; return; }
    if m.page == Page::Settings {
        if c.back || c.pause { m.page = Page::Main; m.sel = 5; return; }
        let graphics_row = if crate::pwr::available() { 6 } else { 5 };
        navigate(&mut m.sel, graphics_row + 1, &c);
        let step = c.right as i32 - c.left as i32 + c.confirm as i32;
        if step != 0 {
            match m.sel {
                0 => save.flight_sensitivity = (save.flight_sensitivity + step as f32 * 0.1).clamp(0.25, 1.5),
                1 => save.look_sensitivity = (save.look_sensitivity + step as f32 * 0.1).clamp(0.25, 1.5),
                2 => save.ground_assist = (save.ground_assist + step as f32 * 0.1).clamp(0.0, 1.0),
                3 => save.invert_pitch = !save.invert_pitch,
                4 => save.speedometer = !save.speedometer,
                5 if crate::pwr::available() => save.plane_wii_style = !save.plane_wii_style,
                _ if c.confirm => {
                    m.page = Page::Graphics;
                    m.sel = 0;
                    m.pending_graphics = Some(PendingGraphics::from_save(&save));
                }
                _ => {}
            }
            save.store();
        }
        return;
    }
    if m.page == Page::Graphics {
        let graphics_row = if crate::pwr::available() { 6 } else { 5 };
        if c.back || c.pause {
            m.page = Page::Settings;
            m.sel = graphics_row;
            m.pending_graphics = None;
            return;
        }
        navigate(&mut m.sel, 5, &c);
        let step = c.right as i32 - c.left as i32 + c.confirm as i32;
        if step != 0 {
            let mon = monitors.iter().find(|(_, is_prim)| *is_prim).or_else(|| monitors.iter().next()).map(|(m, _)| (m.physical_width, m.physical_height));
            let mut pending = m.pending_graphics.unwrap_or_else(|| PendingGraphics::from_save(&save));
            match m.sel {
                0 => pending.fullscreen = !pending.fullscreen,
                1 => {
                    pending.aspect_ratio = ((pending.aspect_ratio as i32 + step).rem_euclid(crate::save::ASPECT_RATIO_LABELS.len() as i32)) as u8;
                    let avail = crate::save::available_resolutions(pending.aspect_ratio, mon);
                    if !avail.iter().any(|r| r.w == pending.resolution_w && r.h == pending.resolution_h)
                        && let Some(best) = avail.iter().min_by_key(|r| (r.w as i32 - pending.resolution_w as i32).abs())
                    {
                        pending.resolution_w = best.w;
                        pending.resolution_h = best.h;
                    }
                }
                2 => {
                    let avail = crate::save::available_resolutions(pending.aspect_ratio, mon);
                    let cur_idx = avail.iter().position(|r| r.w == pending.resolution_w && r.h == pending.resolution_h).unwrap_or(0);
                    let next_idx = (cur_idx as i32 + step).rem_euclid(avail.len() as i32) as usize;
                    pending.resolution_w = avail[next_idx].w;
                    pending.resolution_h = avail[next_idx].h;
                }
                3 => pending.msaa = ((pending.msaa as i32 + step).rem_euclid(crate::save::MSAA_LABELS.len() as i32)) as u8,
                4 if c.confirm => {
                    save.fullscreen = pending.fullscreen;
                    save.aspect_ratio = pending.aspect_ratio;
                    save.resolution_w = pending.resolution_w;
                    save.resolution_h = pending.resolution_h;
                    save.msaa = pending.msaa;
                    save.store();
                    m.page = Page::Settings;
                    m.sel = graphics_row;
                    m.pending_graphics = None;
                    return;
                }
                _ => {}
            }
            m.pending_graphics = Some(pending);
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
        Row::Plane => save.plane = cycle(save.plane, 2 + if crate::pwr::available() { crate::vehicles::KINDS.len() as u8 } else { 0 }),
        Row::Livery if crate::vehicles::selected(&save).is_some() => save.favorite = cycle(save.favorite, 12),
        Row::Livery => save.livery = cycle(save.livery, if save.plane == 0 {3} else {2}),
        Row::Controls if c.confirm => { m.page = Page::Controls; m.sel = 0; m.scroll = 0; }
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
        // Scroll the list just enough to keep the selection on screen.
        m.scroll = m.scroll.clamp(m.sel.saturating_sub(LINES - 1), m.sel);
        if c.confirm { if m.sel == n { save.bindings.clear(); save.store(); } else { m.capturing = true; } }
    }
    c.confirm = false; c.shoot = false; c.board = false; c.flare = false; c.jump = false;
}
/// i-points with at least one of their three readings collected.
fn points_found(save: &Save, points: usize) -> usize {
    (0..points).filter(|i| (0..3).any(|t| save.collected.contains(&((i * 3 + t) as u16)))).count()
}

/// A changeable value between chevrons.
fn choice(v: impl std::fmt::Display) -> String { format!("‹  {v}  ›") }

#[allow(clippy::too_many_arguments)]
fn render(
    (m, state, texts, langs, save, tod, points, c, extras): (Res<Menu>, Res<State<AppState>>, Res<Texts>, Res<Languages>, Res<Save>, Res<TimeOfDay>, Res<IPoints>, Res<Controls>, Option<Res<crate::collect::Items>>),
    window: Single<&Window>,
    monitors: Query<(&bevy::window::Monitor, Has<bevy::window::PrimaryMonitor>)>,
    mut root: Single<&mut Visibility, With<MenuRoot>>,
    mut panel: Single<&mut Node, (With<MenuPanel>, Without<Item>, Without<Footer>)>,
    mut header: Single<(&mut Text, &mut TextFont), (With<MenuHeader>, Without<MenuLine>, Without<MenuValue>, Without<MenuHelp>)>,
    mut help: Single<&mut Text, (With<MenuHelp>, Without<MenuHeader>, Without<MenuLine>, Without<MenuValue>)>,
    mut labels: Query<(&MenuLine, &mut Text, &mut TextFont), (Without<MenuHeader>, Without<MenuHelp>, Without<MenuValue>)>,
    mut values: Query<(&MenuValue, &mut Text, &mut TextFont), (Without<MenuHeader>, Without<MenuHelp>, Without<MenuLine>)>,
    mut items: Query<(&Item, &Hovered, &mut Node, &mut BackgroundColor, &mut BorderColor, &mut UiTransform), (Without<MenuPanel>, Without<Footer>)>,
    mut footers: Query<(&Footer, &Hovered, &Children, &mut Node, &mut BorderColor), (Without<MenuPanel>, Without<Item>)>,
    mut footer_texts: Query<&mut Text, (Without<MenuHeader>, Without<MenuLine>, Without<MenuValue>, Without<MenuHelp>)>,
) {
    **root = if m.open {Visibility::Inherited} else {Visibility::Hidden};
    if !m.open { return; }
    let wide = m.page == Page::Controls || m.page == Page::Record;
    let width = px(if wide {900} else {640});
    if panel.width != width { panel.width = width; }
    let gap = px(if wide {4} else {8});
    if panel.row_gap != gap { panel.row_gap = gap; }
    let title = *state.get() == AppState::Title;
    let onoff = |b| texts.ui(if b {Ui::On} else {Ui::Off});
    let mut selected = Some(m.sel);
    let rows: Vec<(String, String)> = match m.page {
        Page::Controls => {
            header.0.set_if_neq(Text::new(texts.ui(Ui::Controls))); help.set_if_neq(Text::new(texts.ui(Ui::ControlsHint)));
            let mut rows: Vec<_> = (0..crate::input::ACTIONS.len()).map(|i| {
                let pair = crate::input::binding(&save, i);
                (texts.action(i).to_string(), if m.capturing && m.sel == i {texts.ui(Ui::Waiting).into()} else {format!("{}  /  {}", texts.binding(&pair[0]), texts.binding(&pair[1]))})
            }).collect();
            rows.push((texts.ui(Ui::ResetControls).into(), String::new()));
            // Longer than the panel: show the window that holds the selection.
            selected = m.sel.checked_sub(m.scroll);
            rows.into_iter().skip(m.scroll).take(LINES).collect()
        }
        Page::Settings => {
            header.0.set_if_neq(Text::new(texts.ui(Ui::Settings))); help.set_if_neq(Text::new(texts.ui(Ui::MenuHint)));
            let mut rows: Vec<(String, String)> = vec![
                (texts.ui(Ui::FlightSensitivity).into(), choice(format!("{:.0}%", save.flight_sensitivity * 100.0))),
                (texts.ui(Ui::LookSensitivity).into(), choice(format!("{:.0}%", save.look_sensitivity * 100.0))),
                (texts.ui(Ui::GroundAssist).into(), choice(format!("{:.0}%", save.ground_assist * 100.0))),
                (texts.ui(Ui::InvertPitch).into(), choice(onoff(save.invert_pitch))),
                (texts.ui(Ui::Speedometer).into(), choice(onoff(save.speedometer))),
            ];
            if crate::pwr::available() { rows.push((texts.ui(Ui::PlaneHandling).into(), choice(texts.ui(if save.plane_wii_style { Ui::WiiStyle } else { Ui::Original })))); }
            rows.push((texts.ui(Ui::Graphics).into(), "›".into()));
            rows
        }
        Page::Graphics => {
            header.0.set_if_neq(Text::new(texts.ui(Ui::Graphics))); help.set_if_neq(Text::new(texts.ui(Ui::GraphicsHint)));
            let pending = m.pending_graphics.unwrap_or_else(|| PendingGraphics::from_save(&save));
            let mon = monitors.iter().find(|(_, is_prim)| *is_prim).or_else(|| monitors.iter().next()).map(|(m, _)| (m.physical_width, m.physical_height));
            let aspect_label = match pending.aspect_ratio {
                0 => {
                    let detected = mon.map(|(w, h)| crate::save::detect_aspect_ratio(w, h)).unwrap_or("16:9");
                    format!("{} ({detected})", texts.ui(Ui::Auto))
                }
                5 => texts.ui(Ui::All).to_string(),
                idx => crate::save::ASPECT_RATIO_LABELS[idx as usize].to_string(),
            };
            let avail = crate::save::available_resolutions(pending.aspect_ratio, mon);
            let show_aspect = pending.aspect_ratio == 5;
            let res_label = avail.iter().find(|r| r.w == pending.resolution_w && r.h == pending.resolution_h)
                .map(|r| {
                    if r.is_native {
                        format!("{} × {} ({})", r.w, r.h, texts.ui(Ui::Native))
                    } else if show_aspect {
                        format!("{} × {} ({})", r.w, r.h, r.aspect)
                    } else {
                        format!("{} × {}", r.w, r.h)
                    }
                })
                .unwrap_or_else(|| format!("{} × {} ({})", pending.resolution_w, pending.resolution_h, texts.ui(Ui::Custom)));
            let res_val = if pending.fullscreen {
                format!("{res_label} ({})", texts.ui(Ui::Windowed))
            } else {
                res_label
            };
            let msaa_idx = (pending.msaa as usize).min(crate::save::MSAA_LABELS.len() - 1);
            vec![
                (texts.ui(Ui::Fullscreen).into(), choice(onoff(pending.fullscreen))),
                (texts.ui(Ui::AspectRatio).into(), choice(aspect_label)),
                (texts.ui(Ui::Resolution).into(), choice(res_val)),
                (texts.ui(Ui::AntiAliasing).into(), choice(crate::save::MSAA_LABELS[msaa_idx])),
                (texts.ui(Ui::Apply).into(), String::new()),
            ]
        }
        Page::Record => {
            // Each point has a day, evening and night reading.
            header.0.set_if_neq(Text::new(format!("{} — {} / {}   (● {} / {})", texts.ui(Ui::IPoints), points_found(&save, points.0.len()), points.0.len(), save.collected.len(), points.0.len() * 3)));
            let mut all: Vec<(String, String)> = (0..points.0.len()).map(|i| {
                let mark = |t| if save.collected.contains(&((i * 3) as u16 + t)) {"●"} else {"○"};
                let name = if (0..3).any(|t| save.collected.contains(&((i * 3) as u16 + t))) {points.0[i].name(&texts, c.pad)} else {"?".into()};
                (format!("{:>2}.  {}", i + 1, name), format!("{} {} {}", mark(0), mark(1), mark(2)))
            }).collect();
            if let Some(x) = &extras { all.extend(x.record(&save, &texts)); }
            help.set_if_neq(Text::new(format!("{}   ({}–{} / {})", texts.ui(Ui::RecordHint), m.scroll + 1, (m.scroll + LINES).min(all.len()), all.len())));
            selected = None;
            all.into_iter().skip(m.scroll).take(LINES).collect()
        }
        Page::Main => {
            header.0.set_if_neq(Text::new("Sport Island")); help.set_if_neq(Text::new(texts.ui(Ui::MenuHint)));
            let lang = langs.0.iter().find(|l| l.code == texts.code).map_or("", |l| l.name.as_str());
            let vehicle = crate::vehicles::selected(&save);
            let livery = if vehicle.is_some() { crate::lang::FAVORITES[save.favorite.min(11) as usize] } else if save.plane == 0 { [Ui::Red, Ui::White, Ui::Yellow][save.livery.min(2) as usize] } else { [Ui::Orange, Ui::Blue][save.livery.min(1) as usize] };
            let plane_name = match vehicle { Some(k) => texts.pwr(k.name_key()).unwrap_or_else(|| format!("{k:?}")), None => texts.ui(if save.plane == 0 {Ui::Plane1} else {Ui::Plane2}).into() };
            (if title {TITLE} else {PAUSE}).iter().map(|row| match row {
                Row::Start => (texts.ui(Ui::Start).into(), String::new()),
                Row::Resume => (texts.ui(Ui::Resume).into(), String::new()),
                Row::Time => (texts.ui(Ui::TimeOfDay).into(), choice(texts.ui([Ui::Day, Ui::Dusk, Ui::Night][tod.0 as usize]))),
                Row::Plane => (if title {texts.ui(Ui::Plane).into()} else {format!("{} ({})", texts.ui(Ui::Plane), texts.ui(Ui::NextBoarding))}, choice(&plane_name)),
                Row::Livery => (texts.ui(Ui::Livery).into(), choice(texts.ui(livery))),
                Row::Controls => (texts.ui(Ui::Controls).into(), "›".into()),
                Row::Settings => (texts.ui(Ui::Settings).into(), "›".into()),
                Row::Language => (texts.ui(Ui::Language).into(), choice(lang)),
                Row::Record => (texts.ui(Ui::IPoints).into(), format!("{} / {}  ›", points_found(&save, points.0.len()), points.0.len())),
                Row::Quit => (texts.ui(Ui::Quit).into(), String::new()),
            }).collect()
        }
    };
    let big = FontSize::Px((window.height() * (if title && m.page == Page::Main {0.06} else {0.042})).clamp(24.0, 48.0));
    if header.1.font_size != big { header.1.font_size = big; }
    let size = FontSize::Px((window.height() * if wide {0.022} else {0.026}).clamp(14.0, 22.0));
    for (l, mut t, mut font) in &mut labels {
        t.set_if_neq(Text::new(rows.get(l.0).map(|r| r.0.clone()).unwrap_or_default()));
        if font.font_size != size { font.font_size = size; }
    }
    for (v, mut t, mut font) in &mut values {
        t.set_if_neq(Text::new(rows.get(v.0).map(|r| r.1.clone()).unwrap_or_default()));
        if font.font_size != size { font.font_size = size; }
    }
    let record = m.page == Page::Record;
    // The long pages (record, controls) use compact rows.
    let pad = if record {px(2)} else if wide {px(3)} else if m.page == Page::Settings || m.page == Page::Graphics {px(6)} else {px(8)};
    for (item, hovered, mut node, mut bg, mut border, mut transform) in &mut items {
        let display = if item.0 < rows.len() {Display::Flex} else {Display::None};
        if node.display != display { node.display = display; }
        if node.padding.top != pad { node.padding = UiRect::axes(px(20), pad); }
        let chosen = selected == Some(item.0);
        let (fill, line) = if record { (Color::NONE, Color::NONE) } else if chosen { (SELECTED, ACCENT) } else if hovered.get() { (BUTTON, HOVER) } else { (BUTTON, OUTLINE) };
        bg.set_if_neq(BackgroundColor(fill));
        border.set_if_neq(BorderColor::all(line));
        let width = if chosen { px(4) } else { px(3) };
        if node.border.top != width { node.border = UiRect::all(width); }
        let scale = Vec2::splat(if chosen { 1.03 } else { 1.0 });
        if transform.scale != scale { transform.scale = scale; }
    }
    // Back everywhere but the title's main page; paging arrows only for the record.
    for (footer, hovered, children, mut node, mut border) in &mut footers {
        let shown = if footer.0 == 0 { !(title && m.page == Page::Main) } else { record };
        let display = if shown {Display::Flex} else {Display::None};
        if node.display != display { node.display = display; }
        border.set_if_neq(BorderColor::all(if hovered.get() {HOVER} else {OUTLINE}));
        if footer.0 == 0 { for e in children { if let Ok(mut t) = footer_texts.get_mut(*e) { t.set_if_neq(Text::new(texts.ui(Ui::Back))); } } }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    #[test] fn system_overlays_release_capture_and_require_explicit_resume() {
        let mut w = World::new();
        w.insert_resource(State::new(AppState::Play)); w.init_resource::<Menu>();
        w.init_resource::<Controls>(); w.init_resource::<Time<Virtual>>();
        w.init_resource::<ButtonInput<KeyCode>>();
        let window = w.spawn((Window::default(), PrimaryWindow, CursorOptions::default())).id();
        w.run_system_once(cursor).unwrap();
        assert_eq!(w.get::<CursorOptions>(window).unwrap().grab_mode, CursorGrabMode::Confined);
        for shortcut in [None, Some(KeyCode::SuperLeft), Some(KeyCode::PrintScreen), Some(KeyCode::AltLeft)] {
            w.get_mut::<Window>(window).unwrap().focused = shortcut.is_some();
            if let Some(key) = shortcut { w.resource_mut::<ButtonInput<KeyCode>>().press(key); }
            w.resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Tab);
            w.insert_resource(Controls { look: Vec2::ONE, stick: Vec2::ONE, shoot: true, record: true, ..default() });
            w.run_system_once(release_for_system_ui).unwrap(); w.run_system_once(cursor).unwrap();
            assert!(w.resource::<Menu>().open && w.resource::<Time<Virtual>>().is_paused());
            assert_eq!(w.resource::<Controls>().look, Vec2::ZERO);
            assert!(!w.resource::<Controls>().shoot && !w.resource::<Controls>().record);
            let options = w.get::<CursorOptions>(window).unwrap();
            assert!(options.visible && options.grab_mode == CursorGrabMode::None);
            // Closing the overlay must not immediately recapture the mouse.
            w.resource_mut::<ButtonInput<KeyCode>>().reset_all();
            w.get_mut::<Window>(window).unwrap().focused = true;
            w.run_system_once(release_for_system_ui).unwrap(); w.run_system_once(cursor).unwrap();
            assert_eq!(w.get::<CursorOptions>(window).unwrap().grab_mode, CursorGrabMode::None);
            w.resource_scope(|w, mut menu: Mut<Menu>| {
                w.resource_scope(|w, mut time: Mut<Time<Virtual>>| close(&mut menu, &mut time, &mut w.resource_mut::<Controls>()));
            });
            w.run_system_once(cursor).unwrap();
            assert_eq!(w.get::<CursorOptions>(window).unwrap().grab_mode, CursorGrabMode::Confined);
        }
    }
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
    #[test] fn controls_list_scrolls_to_keep_the_selection_visible() {
        let mut w = World::new();
        w.insert_resource(Menu {open:true, page:Page::Controls, ..default()}); w.init_resource::<Save>();
        w.init_resource::<ButtonInput<KeyCode>>(); w.init_resource::<ButtonInput<MouseButton>>();
        let rows = crate::input::ACTIONS.len() + 1;
        for _ in 0..rows - 1 { w.insert_resource(Controls {down:true,..default()}); w.run_system_once(controls_input).unwrap(); }
        let m = w.resource::<Menu>();
        assert_eq!(m.sel, rows - 1);
        assert!(m.scroll + LINES > m.sel && m.scroll <= m.sel, "selection {} outside rows {}..{}", m.sel, m.scroll, m.scroll + LINES);
        w.insert_resource(Controls {down:true,..default()}); w.run_system_once(controls_input).unwrap();
        assert_eq!((w.resource::<Menu>().sel, w.resource::<Menu>().scroll), (0, 0), "wraps back to the top");
    }
    #[test] fn navigation_and_record_pages_are_bounded() {
        let mut sel = 0; let c = Controls { down: true, ..default() };
        for _ in 0..3 { navigate(&mut sel, TITLE.len(), &c); }
        assert_eq!(TITLE[sel] as u8, Row::Livery as u8);
        assert_eq!(record_scroll(0, &Controls {left:true,..default()}, 80),0);
        assert_eq!(record_scroll(0, &Controls {right:true,..default()}, 80),16);
        assert_eq!(record_scroll(63, &Controls {right:true,..default()}, 80),64);
    }
    #[test] fn title_navigation_preserves_language_and_record_shortcut_pauses_then_resumes() {
        let mut w = World::new();
        w.insert_resource(State::new(AppState::Title)); w.init_resource::<NextState<AppState>>();
        w.insert_resource(Menu {open:true,..default()}); w.insert_resource(Controls {down:true,..default()});
        w.init_resource::<Time<Virtual>>(); w.insert_resource(TimeOfDay(0)); w.init_resource::<Save>();
        let mut texts=Texts::default(); texts.code="FR".into();
        w.insert_resource(texts); w.insert_resource(Languages(vec![])); w.insert_resource(IPoints(vec![])); w.init_resource::<Messages<AppExit>>();
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
    #[test] fn settings_opens_graphics_and_applies_staged_changes() {
        let mut w = World::new();
        w.insert_resource(State::new(AppState::Play)); w.init_resource::<NextState<AppState>>();
        let graphics_row = if crate::pwr::available() { 6 } else { 5 };
        w.insert_resource(Menu { open: true, page: Page::Settings, sel: graphics_row, ..default() });
        w.insert_resource(Controls { confirm: true, ..default() });
        w.init_resource::<Time<Virtual>>(); w.insert_resource(TimeOfDay(0));
        let mut save = Save::default();
        save.aspect_ratio = 1;
        save.resolution_w = 1600;
        save.resolution_h = 900;
        save.msaa = 0;
        w.insert_resource(save);
        w.insert_resource(Texts::default()); w.insert_resource(Languages(vec![])); w.insert_resource(IPoints(vec![]));
        w.init_resource::<Messages<AppExit>>();

        w.run_system_once(pause_input).unwrap();
        assert_eq!(w.resource::<Menu>().page, Page::Graphics);
        assert!(w.resource::<Menu>().pending_graphics.is_some());

        w.resource_mut::<Menu>().sel = 1;
        w.insert_resource(Controls { right: true, ..default() });
        w.run_system_once(pause_input).unwrap();
        let pending = w.resource::<Menu>().pending_graphics.unwrap();
        assert_eq!(pending.aspect_ratio, 2);
        assert_eq!(w.resource::<Save>().aspect_ratio, 1);

        w.resource_mut::<Menu>().sel = 2;
        w.insert_resource(Controls { right: true, ..default() });
        w.run_system_once(pause_input).unwrap();
        let pending = w.resource::<Menu>().pending_graphics.unwrap();
        assert_ne!((pending.resolution_w, pending.resolution_h), (1600, 900));
        assert_eq!(w.resource::<Save>().resolution_w, 1600);

        w.resource_mut::<Menu>().sel = 3;
        w.insert_resource(Controls { right: true, ..default() });
        w.run_system_once(pause_input).unwrap();
        assert_eq!(w.resource::<Menu>().pending_graphics.unwrap().msaa, 1);
        assert_eq!(w.resource::<Save>().msaa, 0);

        w.resource_mut::<Menu>().sel = 4;
        w.insert_resource(Controls { confirm: true, ..default() });
        w.run_system_once(pause_input).unwrap();
        assert_eq!(w.resource::<Menu>().page, Page::Settings);
        assert!(w.resource::<Menu>().pending_graphics.is_none());
        assert_eq!(w.resource::<Save>().aspect_ratio, 2);
        assert_eq!(w.resource::<Save>().msaa, 1);
        assert_ne!(w.resource::<Save>().resolution_w, 1600);
    }
    #[test] fn language_change_does_not_alter_windowed_resolution() {
        let mut w = World::new();
        let mut save = Save::default();
        save.fullscreen = false;
        save.resolution_w = 1280;
        save.resolution_h = 720;
        w.insert_resource(save);
        // On DPI > 1 (e.g. 1.5), Winit inflates physical resolution from initial logical (1280, 720) to 1920x1080
        let mut win = Window { resolution: (1280u32, 720u32).into(), ..default() };
        win.resolution.set_physical_resolution(1920, 1080);
        let win_entity = w.spawn((win, PrimaryWindow)).id();
        let sys = w.register_system(window_mode);
        // First run corrects inflated physical resolution to the save's requested physical resolution
        w.run_system(sys).unwrap();
        let cur = w.get::<Window>(win_entity).unwrap().resolution.clone();
        assert_eq!((cur.physical_width(), cur.physical_height()), (1280, 720));

        // Changing language triggers save change, but must NOT change resolution
        w.resource_mut::<Save>().lang = Some("ES".into());
        w.run_system(sys).unwrap();
        let cur = w.get::<Window>(win_entity).unwrap().resolution.clone();
        assert_eq!((cur.physical_width(), cur.physical_height()), (1280, 720));

        // When resolution actually changes, window_mode updates resolution
        w.resource_mut::<Save>().resolution_w = 1600;
        w.resource_mut::<Save>().resolution_h = 900;
        w.run_system(sys).unwrap();
        let cur = w.get::<Window>(win_entity).unwrap().resolution.clone();
        assert_eq!((cur.physical_width(), cur.physical_height()), (1600, 900));
    }
}
