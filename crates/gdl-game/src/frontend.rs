//! The game's front end (`docs/frontend.md`): title screen and its menu,
//! character select, the in-game pause menus, what happens when the hero
//! dies, and the game-over screen.
//!
//! The flow follows the game's mode switch: the title waits for Start and
//! opens its Start / Options menu; Start loads the tower hub (`levelL1`,
//! where the game begins every new game) behind the character-select
//! screen; the player picks New, enters a name, picks a class and colour,
//! and play starts in the tower. Start in play opens the Game Menu (the
//! Tower Menu in the hub). Everything is drawn in the game's 512 × 384 2D
//! layer (`font.rs`) with its own fonts, textures, strings and layout.
//!
//! `--level` / `--character` skip straight to play.
//!
//! The game's Controls menu works as the game's does (the style with its
//! controller pictures, rumble, auto aim and attack, `docs/frontend.md`),
//! and so does Compass (the setting; its pointer isn't drawn). PC
//! Settings (not the game's) sits under Options and Settings: keys, mouse
//! and pad bindings (`controls.rs`), the window, and debugging aids, all
//! saved with the options.
//!
//! Stand-ins (see the doc): the title's attract mode (movies, credits,
//! demo play) isn't run; menu sounds aren't played; the menus' spinning
//! 3D arrow is drawn as the flat `MENU_MARKER` texture; Game Options,
//! Shop, Inventory and memory-card Save/Load open or list what the game
//! lists but change nothing.

use bevy::prelude::*;
use gdl_formats::font::{FONT8X8, FONT32, INITIALS};
use gdl_formats::pdata::PlayerStats;
use gdl_formats::text::TextRom;

use crate::character::Animator;
use crate::controls::{self, Action, Input};
use crate::exits::ChangeLevelTo;
use crate::font::{Draw2d, FontTexture, GameFonts, TextStyle, UiTextures};
use crate::level::LoadedGame;
use crate::message_box::MessageBox;
use crate::options::GameOptions;
use crate::party::{MAX_PLAYERS, Party};
use crate::player::{Player, PlayerChoice};
use crate::party::Devices;
use crate::player_state::{PartyChange, PlayerState};
use crate::population::LevelPopulation;
use crate::saves::{SavedCharacter, Saves};

/// Where a new game starts: the tower hub, realm 13 level 0.
pub const TOWER: &str = "levelL1";

pub struct FrontendPlugin {
    /// Start in play (the command line picked a level or hero).
    pub skip: bool,
}

impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        let screen = if self.skip { Screen::Playing } else { Screen::Title };
        let fe = Frontend::new(screen);
        app.insert_resource(fe)
            .init_resource::<Snapshot>()
            .add_systems(Startup, (load_text, spawn_backdrop))
            .add_systems(
                Update,
                (
                    read_input,
                    run,
                    draw,
                    level_started.run_if(resource_exists_and_changed::<LevelPopulation>),
                    death,
                    show_backdrop,
                )
                    .chain(),
            );
    }
}

// ---------------------------------------------------------------------------
// Input

/// The front end's buttons this frame (edges).
#[derive(Clone, Copy, Default, Debug)]
struct Pressed {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    accept: bool,
    back: bool,
    start: bool,
    l: bool,
    r: bool,
    /// Left / right held (the sliders move while they're down).
    hold_left: bool,
    hold_right: bool,
}

/// A key, mouse button or pad button pressed while the settings wait for
/// one (rebinding), or the wait given up (Escape, or the pad's Start).
#[derive(Clone, Copy, Debug)]
enum Captured {
    Key(Input),
    Pad(GamepadButton),
    Cancel,
}

/// `GDL_MENU="start@40,down@60,accept@70"`: presses on those frames, to
/// drive the menus for screenshots.
fn menu_script() -> Vec<(String, u64)> {
    let Ok(spec) = std::env::var("GDL_MENU") else { return Vec::new() };
    spec.split(',')
        .filter_map(|item| {
            let (name, frame) = item.trim().split_once('@')?;
            Some((name.to_ascii_lowercase(), frame.parse().ok()?))
        })
        .collect()
}

/// What a front-end press came from: the keyboard and mouse, or a pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Keyboard,
    Pad(Entity),
}

impl Device {
    /// Whether these devices include it.
    fn among(self, d: &Devices) -> bool {
        match self {
            Device::Keyboard => d.keyboard,
            Device::Pad(e) => d.pad == Some(e),
        }
    }

    /// It alone (a pad's player keeps the keyboard too when it's player 1
    /// on their own: `start_select`).
    fn only(self) -> Devices {
        match self {
            Device::Keyboard => Devices { keyboard: true, ..Devices::default() },
            Device::Pad(e) => Devices { pad: Some(e), ..Devices::default() },
        }
    }
}

/// The keyboard's presses this frame.
fn keyboard_pressed(keys: &ButtonInput<KeyCode>) -> Pressed {
    let key = |ks: &[KeyCode]| ks.iter().any(|k| keys.just_pressed(*k));
    let held = |ks: &[KeyCode]| ks.iter().any(|k| keys.pressed(*k));
    Pressed {
        up: key(&[KeyCode::ArrowUp, KeyCode::KeyW]),
        down: key(&[KeyCode::ArrowDown, KeyCode::KeyS]),
        left: key(&[KeyCode::ArrowLeft, KeyCode::KeyA]),
        right: key(&[KeyCode::ArrowRight, KeyCode::KeyD]),
        accept: key(&[KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space, KeyCode::KeyJ]),
        back: key(&[KeyCode::Escape, KeyCode::Backspace, KeyCode::KeyH]),
        start: key(&[KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Escape]),
        l: key(&[KeyCode::KeyQ, KeyCode::KeyP]),
        r: key(&[KeyCode::KeyE, KeyCode::KeyO]),
        hold_left: held(&[KeyCode::ArrowLeft, KeyCode::KeyA]),
        hold_right: held(&[KeyCode::ArrowRight, KeyCode::KeyD]),
    }
}

/// A pad's presses this frame; its stick counts as a press when it crosses
/// half way (`was`: last frame's up, down, left, right).
fn pad_pressed(pad: &Gamepad, was: &mut [bool; 4]) -> Pressed {
    let stick = pad.left_stick();
    let now = [stick.y > 0.5, stick.y < -0.5, stick.x < -0.5, stick.x > 0.5];
    let edge: Vec<bool> = now.iter().zip(was.iter()).map(|(n, o)| *n && !*o).collect();
    *was = now;
    let b = |bs: &[GamepadButton]| bs.iter().any(|b| pad.just_pressed(*b));
    Pressed {
        up: b(&[GamepadButton::DPadUp]) || edge[0],
        down: b(&[GamepadButton::DPadDown]) || edge[1],
        left: b(&[GamepadButton::DPadLeft]) || edge[2],
        right: b(&[GamepadButton::DPadRight]) || edge[3],
        accept: b(&[GamepadButton::South]),
        // The GameCube's B (west); an Xbox pad's B (east) backs out too.
        back: b(&[GamepadButton::West, GamepadButton::East]),
        start: b(&[GamepadButton::Start]),
        l: b(&[GamepadButton::LeftTrigger, GamepadButton::LeftTrigger2]),
        r: b(&[GamepadButton::RightTrigger, GamepadButton::RightTrigger2]),
        hold_left: pad.pressed(GamepadButton::DPadLeft) || now[2],
        hold_right: pad.pressed(GamepadButton::DPadRight) || now[3],
    }
}

impl Pressed {
    fn any(&self) -> bool {
        self.up || self.down || self.left || self.right || self.accept || self.back || self.start || self.l || self.r
    }

    fn or(self, o: Pressed) -> Pressed {
        Pressed {
            up: self.up || o.up,
            down: self.down || o.down,
            left: self.left || o.left,
            right: self.right || o.right,
            accept: self.accept || o.accept,
            back: self.back || o.back,
            start: self.start || o.start,
            l: self.l || o.l,
            r: self.r || o.r,
            hold_left: self.hold_left || o.hold_left,
            hold_right: self.hold_right || o.hold_right,
        }
    }
}

pub(crate) fn read_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pads: Query<(Entity, &Gamepad)>,
    mut fe: ResMut<Frontend>,
    mut sticks: Local<Vec<(Entity, [bool; 4])>>,
) {
    // Waiting for a key or button to bind: nothing else counts.
    if let Some((page, _)) = fe.capture {
        let got = if keys.just_pressed(KeyCode::Escape) || pads.iter().any(|(_, p)| p.just_pressed(GamepadButton::Start)) {
            Some(Captured::Cancel)
        } else if page == Page::Keys {
            controls::just_pressed_input(&keys, &mouse).map(Captured::Key)
        } else {
            controls::just_pressed_pad(pads.iter().map(|(_, p)| p)).map(Captured::Pad)
        };
        if got.is_some() {
            fe.captured = got;
        }
        fe.input = Pressed::default();
        fe.pressed_by.clear();
        return;
    }
    let mut keyboard = keyboard_pressed(&keys);
    fe.frame += 1;
    let frame = fe.frame;
    let script: Vec<String> = fe.script.iter().filter(|(_, f)| *f == frame).map(|(n, _)| n.clone()).collect();
    for name in script {
        let p = &mut keyboard;
        match name.as_str() {
            "up" => p.up = true,
            "down" => p.down = true,
            "left" => (p.left, p.hold_left) = (true, true),
            "right" => (p.right, p.hold_right) = (true, true),
            "accept" | "a" => p.accept = true,
            "back" | "b" => p.back = true,
            "start" => p.start = true,
            "l" => p.l = true,
            "r" => p.r = true,
            other => warn!("GDL_MENU: unknown button {other}"),
        }
    }
    let mut by = vec![(Device::Keyboard, keyboard)];
    sticks.retain(|(e, _)| pads.contains(*e));
    for (e, pad) in &pads {
        let was = match sticks.iter_mut().find(|(s, _)| *s == e) {
            Some((_, w)) => w,
            None => {
                sticks.push((e, [false; 4]));
                &mut sticks.last_mut().expect("just pushed").1
            }
        };
        by.push((Device::Pad(e), pad_pressed(pad, was)));
    }
    // The menus take any device's presses; the select screen's columns
    // each their own.
    fe.input = by.iter().fold(Pressed::default(), |all, (_, p)| all.or(*p));
    if let Some((d, _)) = by.iter().find(|(_, p)| p.start || p.accept) {
        fe.last_device = *d;
    }
    fe.pressed_by = by;
}

// ---------------------------------------------------------------------------
// Menus: the game's own menu tables (`docs/frontend.md` has where each is).

/// What choosing a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    /// Title menu.
    Start,
    Options,
    /// Pause menus.
    Settings,
    ManageCharacter,
    Shop,
    Inventory,
    QuitGame,
    QuitLevel,
    /// Confirmations: "No" is the game's back (-1).
    No,
    ConfirmQuitGame,
    ConfirmAbortLevel,
    /// Sub-menus listed by the game's Options / Settings.
    Audio,
    GameOptions,
    Compass,
    Controls,
    /// Items whose settings aren't modelled.
    Setting,
    /// The game's Controls sub-menus: the style (its one line), rumble,
    /// auto aim and attack; a choice in one of the On / Off menus.
    Style,
    StyleLine,
    Rumble,
    AutoAim,
    AutoAttack,
    Choose(Setting, bool),
    /// PC Settings (not the game's): its pages, a switch on one, a binding
    /// to change, the defaults back.
    PcSettings,
    Page(Page),
    Toggle(Setting),
    Bind(Action),
    ResetBindings,
    /// The Audio menu's sliders (left/right move them).
    Volume(Volume),
    /// Character select.
    New,
    Load,
    /// A saved character in the load list (its index in the save file).
    Character(usize),
    Save,
    Change,
    Quit,
    Done,
    Yes,
}

/// A setting the menus switch ([`GameOptions`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    Rumble,
    AutoAim,
    AutoAttack,
    Compass,
    Fullscreen,
    Vsync,
    DevKeys,
    DebugOverlay,
    FrameRate,
    Collision,
}

impl Setting {
    fn get(self, o: &GameOptions) -> bool {
        match self {
            Self::Rumble => o.rumble,
            Self::AutoAim => o.auto_aim,
            Self::AutoAttack => o.auto_attack,
            Self::Compass => o.compass,
            Self::Fullscreen => o.fullscreen,
            Self::Vsync => o.vsync,
            Self::DevKeys => o.dev_keys,
            Self::DebugOverlay => o.debug_overlay,
            Self::FrameRate => o.frame_rate,
            Self::Collision => o.collision,
        }
    }
    fn set(self, o: &mut GameOptions, v: bool) {
        match self {
            Self::Rumble => o.rumble = v,
            Self::AutoAim => o.auto_aim = v,
            Self::AutoAttack => o.auto_attack = v,
            Self::Compass => o.compass = v,
            Self::Fullscreen => o.fullscreen = v,
            Self::Vsync => o.vsync = v,
            Self::DevKeys => o.dev_keys = v,
            Self::DebugOverlay => o.debug_overlay = v,
            Self::FrameRate => o.frame_rate = v,
            Self::Collision => o.collision = v,
        }
    }
    /// The game's choice menus: their titles and their two lines (the
    /// game's order).
    fn choices(self) -> (&'static str, [(&'static str, bool); 2]) {
        match self {
            Self::Rumble => ("Rumble Feature", [("Off", false), ("On", true)]),
            Self::AutoAim => ("Auto Aim", [("On", true), ("Off", false)]),
            Self::AutoAttack => ("Auto Attack", [("On", true), ("Off", false)]),
            Self::Compass => ("Compass", [("Hide", false), ("Show", true)]),
            _ => ("", [("On", true), ("Off", false)]),
        }
    }
    /// Its line on a PC Settings page.
    fn label(self) -> &'static str {
        match self {
            Self::Fullscreen => "Full Screen",
            Self::Vsync => "VSync",
            Self::DevKeys => "Developer Keys",
            Self::DebugOverlay => "Debug Overlay",
            Self::FrameRate => "Frame Rate",
            Self::Collision => "Collision Overlay",
            other => other.choices().0,
        }
    }
}

/// The PC Settings pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Keys,
    Pad,
    Video,
    Debug,
}

/// What a menu of run-time lines shows, to make them again after a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dynamic {
    Style,
    Choice(Setting),
    Page(Page),
}

/// The game's mark on the current choice (`docs/frontend.md`, "Menus";
/// `~` is a tick in its menu font), and the PC Settings pages' break
/// between an item and its value, which is drawn in a column
/// ([`VALUE_COLUMN`]).
const CURRENT: &str = " ~";
const VALUE: &str = "\t";
const VALUE_COLUMN: f32 = 220.0;

/// The lines a menu of run-time lines shows now.
fn dynamic_lines(kind: Dynamic, o: &GameOptions, style_pick: usize, capture: Option<Action>) -> Vec<(String, Item)> {
    let on_off = |v: bool| if v { "On" } else { "Off" };
    match kind {
        Dynamic::Style => vec![(controls::SCHEME_NAMES[style_pick].to_string(), Item::StyleLine)],
        Dynamic::Choice(setting) => setting
            .choices()
            .1
            .iter()
            .map(|&(label, v)| {
                let mark = if setting.get(o) == v { CURRENT } else { "" };
                (format!("{label}{mark}"), Item::Choose(setting, v))
            })
            .collect(),
        Dynamic::Page(Page::Keys) => {
            let mut lines: Vec<(String, Item)> = Action::ALL
                .iter()
                .map(|&a| {
                    let value = if capture == Some(a) {
                        "Press a key".to_string()
                    } else {
                        let names: Vec<&str> = o.bindings.keys(a).iter().map(|&i| controls::input_name(i)).collect();
                        if names.is_empty() { "-".to_string() } else { names.join(", ") }
                    };
                    (format!("{}{VALUE}{value}", a.label()), Item::Bind(a))
                })
                .collect();
            lines.push(("Reset Defaults".to_string(), Item::ResetBindings));
            lines
        }
        Dynamic::Page(Page::Pad) => {
            let mut lines = vec![(format!("Style{VALUE}{}", controls::SCHEME_NAMES[o.scheme]), Item::Style)];
            lines.extend(Action::ON_PAD.iter().map(|&a| {
                let value = if capture == Some(a) {
                    "Press a button".to_string()
                } else {
                    let names: Vec<&str> = o.bindings.pad(a, o.scheme).iter().map(|&b| controls::pad_name(b)).collect();
                    if names.is_empty() { "-".to_string() } else { names.join(", ") }
                };
                (format!("{}{VALUE}{value}", a.label()), Item::Bind(a))
            }));
            lines.push(("Reset to Style".to_string(), Item::ResetBindings));
            lines
        }
        Dynamic::Page(page) => {
            let settings: &[Setting] = match page {
                Page::Video => &[Setting::Fullscreen, Setting::Vsync],
                _ => &[Setting::DevKeys, Setting::DebugOverlay, Setting::FrameRate, Setting::Collision],
            };
            settings.iter().map(|&s| (format!("{}{VALUE}{}", s.label(), on_off(s.get(o))), Item::Toggle(s))).collect()
        }
    }
}

/// Which volume a slider sets ([`GameOptions`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Volume {
    Master,
    Music,
    Effects,
}

impl Volume {
    fn get(self, o: &GameOptions) -> f32 {
        match self {
            Self::Master => o.master_volume,
            Self::Music => o.music_volume,
            Self::Effects => o.effects_volume,
        }
    }
    fn set(self, o: &mut GameOptions, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match self {
            Self::Master => o.master_volume = v,
            Self::Music => o.music_volume = v,
            Self::Effects => o.effects_volume = v,
        }
    }
}

/// The game's slider moves 0..255 by the fields elapsed while left or
/// right is held: about four seconds end to end.
const SLIDER_PER_FIELD: f32 = 1.0 / 255.0;

/// A menu as the game's menu records describe it.
struct MenuDef {
    title: Option<&'static str>,
    /// Items' left edge, or `-centre` for centred items.
    x: f32,
    /// First item's top; the game's -1 centres the list on y 192, other
    /// negatives centre it on `-y`.
    y: f32,
    /// The menu arrow (the game's `ICON_ARROW`) points at the selection.
    arrow: bool,
    /// Title scale and item scale.
    title_scale: f32,
    item_scale: f32,
    /// `SCROLL_A` behind it: centred horizontally, top y, width, height.
    panel: Option<(f32, f32, f32)>,
    /// Letters in `FONT32_PARCH` (flag `0x40`) and flickering through
    /// `FONT32GAR0..5` as it opens (flag `0x80`).
    parch: bool,
    gar: bool,
    /// Back / Select button hints along the bottom (flags 1 and 2).
    hints: bool,
    /// The burning Gauntlet logo (`LOGO_BURN1..5`).
    logo: bool,
    /// Fades in over 30 fields (flag `0x20`).
    fade: bool,
    /// A dark backing behind the lines (PC Settings' pages, not the game's).
    backdrop: bool,
    /// Plain, and selected (glow colour; the letters go white).
    normal: [u8; 3],
    glow: [u8; 3],
    items: &'static [Entry],
}

/// A menu line: its text, what it does, and extra space below it (the
/// volume sliders sit in it).
struct Entry {
    label: &'static str,
    item: Item,
    below: f32,
}

const fn e(label: &'static str, item: Item) -> Entry {
    Entry { label, item, below: 0.0 }
}

const PANEL: Option<(f32, f32, f32)> = Some((8.0, 480.0, 360.0));
const INK: [u8; 3] = [92, 26, 3];
const EMBER: [u8; 3] = [180, 50, 10];
const PURPLE: [u8; 3] = [130, 0, 234];

const fn game_menu(title: &'static str, logo: bool, items: &'static [Entry]) -> MenuDef {
    MenuDef {
        title: Some(title),
        x: 128.0,
        y: -1.0,
        arrow: true,
        title_scale: 1.2,
        item_scale: 1.0,
        panel: PANEL,
        parch: true,
        gar: true,
        hints: true,
        logo,
        fade: true,
        backdrop: false,
        normal: INK,
        glow: PURPLE,
        items,
    }
}

const fn confirm(title: &'static str, items: &'static [Entry]) -> MenuDef {
    MenuDef {
        title: Some(title),
        x: -256.0,
        y: -1.0,
        arrow: true,
        title_scale: 1.2,
        item_scale: 1.0,
        panel: Some((64.0, 320.0, 220.0)),
        parch: true,
        gar: false,
        hints: false,
        logo: false,
        fade: true,
        backdrop: false,
        normal: INK,
        glow: PURPLE,
        items,
    }
}

static TITLE_MENU: MenuDef = MenuDef {
    title: None,
    x: -256.0,
    y: 304.0,
    arrow: true,
    title_scale: 1.2,
    item_scale: 1.0,
    panel: None,
    parch: false,
    gar: false,
    hints: false,
    logo: false,
    fade: false,
    backdrop: false,
    normal: EMBER,
    glow: PURPLE,
    items: &[e("Start", Item::Start), e("Options", Item::Options)],
};
static OPTIONS_MENU: MenuDef = game_menu(
    "Options",
    true,
    &[
        e("Audio", Item::Audio),
        e("Game Options", Item::GameOptions),
        e("Compass", Item::Compass),
        e("Controls", Item::Controls),
        e("PC Settings", Item::PcSettings),
    ],
);
static TOWER_MENU: MenuDef = game_menu(
    "Tower Menu",
    true,
    &[
        e("Settings", Item::Settings),
        e("Manage Character", Item::ManageCharacter),
        e("Shop", Item::Shop),
        e("Inventory", Item::Inventory),
        e("Quit Game", Item::QuitGame),
    ],
);
static GAME_MENU: MenuDef = game_menu("Game Menu", true, &[e("Settings", Item::Settings), e("Quit Level", Item::QuitLevel)]);
/// Settings from the Tower Menu has a Compass entry; from the Game Menu not.
static TOWER_SETTINGS: MenuDef = game_menu(
    "Settings",
    true,
    &[e("Audio", Item::Audio), e("Compass", Item::Compass), e("Controls", Item::Controls), e("PC Settings", Item::PcSettings)],
);
static LEVEL_SETTINGS: MenuDef =
    game_menu("Settings", true, &[e("Audio", Item::Audio), e("Controls", Item::Controls), e("PC Settings", Item::PcSettings)]);
static GAME_OPTIONS: MenuDef =
    game_menu("Game Options", false, &[e("Difficulty", Item::Setting), e("Multiplayer Mode", Item::Setting)]);
/// The game's Audio menu is Music Volume and Sfx Volume sliders (52 below
/// each for the bar) and a Mono / Stereo switch. Here the switch (stereo
/// only) gives way to a Master Volume slider, which the game doesn't have,
/// and the gaps close up so three bars fit above the button hints.
static AUDIO_MENU: MenuDef = MenuDef {
    y: 108.0,
    item_scale: 0.8,
    gar: false,
    ..game_menu(
        "Audio",
        false,
        &[
            Entry { label: "Master Volume", item: Item::Volume(Volume::Master), below: SLIDER_GAP },
            Entry { label: "Music Volume", item: Item::Volume(Volume::Music), below: SLIDER_GAP },
            Entry { label: "Sfx Volume", item: Item::Volume(Volume::Effects), below: SLIDER_GAP },
        ],
    )
};
const SLIDER_GAP: f32 = 40.0;
static CONTROLS_MENU: MenuDef = game_menu(
    "Controls",
    false,
    &[
        e("Style ", Item::Style),
        e("Rumble Feature ", Item::Rumble),
        e("Auto Aim ", Item::AutoAim),
        e("Auto Attack ", Item::AutoAttack),
    ],
);
/// The game's On / Off menus (Rumble Feature, Auto Aim, Auto Attack) and
/// Compass: two lines, the current marked; choosing one sets it and closes.
static RUMBLE_MENU: MenuDef = game_menu("Rumble Feature", false, &[]);
static AUTO_AIM_MENU: MenuDef = game_menu("Auto Aim", false, &[]);
static AUTO_ATTACK_MENU: MenuDef = game_menu("Auto Attack", false, &[]);
static COMPASS_MENU: MenuDef = game_menu("Compass", true, &[]);
/// The game's Control Style menu (`docs/frontend.md`): the style's name
/// centred at y 265, left / right change it, Select keeps it; the
/// controller pictures above ([`STYLE_PICTURES`]).
static STYLE_MENU: MenuDef = MenuDef { x: -256.0, y: 265.0, logo: false, ..game_menu("Control Style", false, &[]) };
/// The pictures the Control Style menu shows: name, left, top (their own
/// size).
const STYLE_PICTURES: [(&str, f32, f32); 3] = [("CONTROLER_1", 96.0, 104.0), ("CONTROLER_2", 352.0, 104.0), ("CONTROLER_3", 128.0, 232.0)];
static PC_MENU: MenuDef = game_menu(
    "PC Settings",
    true,
    &[
        e("Keyboard & Mouse", Item::Page(Page::Keys)),
        e("Controller", Item::Page(Page::Pad)),
        e("Video", Item::Page(Page::Video)),
        e("Debug", Item::Page(Page::Debug)),
    ],
);
/// A PC Settings page: its lines smaller, from the panel's top.
const fn page_menu(title: &'static str) -> MenuDef {
    MenuDef {
        x: 48.0,
        y: 96.0,
        item_scale: 0.5,
        title_scale: 0.9,
        hints: false,
        logo: false,
        gar: false,
        parch: false,
        backdrop: true,
        normal: PAGE_INK,
        ..game_menu(title, false, &[])
    }
}
/// The pages' plain lines, light over their backing.
const PAGE_INK: [u8; 3] = [236, 222, 196];
static KEYS_PAGE: MenuDef = page_menu("Keyboard & Mouse");
static PAD_PAGE: MenuDef = page_menu("Controller");
static VIDEO_PAGE: MenuDef = MenuDef { x: 128.0, y: -1.0, item_scale: 1.0, ..page_menu("Video") };
static DEBUG_PAGE: MenuDef = MenuDef { x: 96.0, y: -1.0, item_scale: 0.8, ..page_menu("Debug") };
static QUIT_GAME: MenuDef = confirm("Quit Game?", &[e("No", Item::No), e("Yes", Item::ConfirmQuitGame)]);
static ABORT_LEVEL: MenuDef = confirm("Abort Level?", &[e("No", Item::No), e("Yes", Item::ConfirmAbortLevel)]);

/// The select screen's per-player menus: centred in the player's column,
/// on y 128, at scale 0.667.
const fn select_menu(items: &'static [Entry]) -> MenuDef {
    MenuDef {
        title: None,
        x: -64.0,
        y: -128.0,
        arrow: false,
        title_scale: 1.0,
        item_scale: 0.667,
        panel: None,
        parch: false,
        gar: false,
        hints: false,
        logo: false,
        fade: false,
        backdrop: false,
        normal: EMBER,
        glow: PURPLE,
        items,
    }
}
static NEW_LOAD: MenuDef = select_menu(&[e("New", Item::New), e("Load", Item::Load)]);
static CHARACTER_MENU: MenuDef = select_menu(&[
    e("Save", Item::Save),
    e("Change", Item::Change),
    e("Load", Item::Load),
    e("Quit", Item::Quit),
    e("Done", Item::Done),
]);
static YES_NO: MenuDef = MenuDef { y: 164.0, ..select_menu(&[e("Yes", Item::Yes), e("No", Item::No)]) };
/// The saved characters, listed smaller to fit the column (stand-in for
/// the game's memory card screens).
static LOAD_LIST: MenuDef = MenuDef { item_scale: 0.5, ..select_menu(&[]) };
/// At most this many saved characters are listed.
const LOAD_LIST_MAX: usize = 10;

/// The character menu: Load only with saved characters.
fn character_menu(has_saves: bool) -> Menu {
    Menu::new(&CHARACTER_MENU).disabling(if has_saves { &[] } else { &[Item::Load] })
}

/// New / Load: Load only with saved characters.
fn new_load_menu(has_saves: bool) -> Menu {
    Menu::new(&NEW_LOAD).disabling(if has_saves { &[] } else { &[Item::Load] })
}

/// The load list: each saved character's name and level.
fn load_list(saves: &Saves) -> Menu {
    let lines = saves.file.characters.iter().enumerate().take(LOAD_LIST_MAX).map(|(i, c)| (c.label(), Item::Character(i))).collect();
    Menu::with_lines(&LOAD_LIST, lines)
}

/// An open menu.
struct Menu {
    def: &'static MenuDef,
    /// Lines made at run time (the saved characters), in place of the
    /// definition's items; the definition still gives the look.
    lines: Option<Vec<(String, Item)>>,
    selected: usize,
    /// Fields since it opened.
    t: f32,
    /// Items that can't be chosen (drawn half transparent).
    disabled: Vec<Item>,
    /// The column the select screen's menus sit in.
    column: f32,
    /// What its run-time lines show, to make them again.
    kind: Option<Dynamic>,
}

impl Menu {
    fn new(def: &'static MenuDef) -> Self {
        Self { def, lines: None, selected: 0, t: 0.0, disabled: Vec::new(), column: 0.0, kind: None }
    }
    /// A menu of run-time lines showing `kind`.
    fn dynamic(def: &'static MenuDef, kind: Dynamic, o: &GameOptions, style_pick: usize) -> Self {
        Self { kind: Some(kind), ..Self::with_lines(def, dynamic_lines(kind, o, style_pick, None)) }
    }
    /// A menu of run-time lines in the style of `def`.
    fn with_lines(def: &'static MenuDef, lines: Vec<(String, Item)>) -> Self {
        Self { lines: Some(lines), ..Self::new(def) }
    }
    fn len(&self) -> usize {
        self.lines.as_ref().map_or(self.def.items.len(), Vec::len)
    }
    fn item(&self, i: usize) -> Item {
        match &self.lines {
            Some(lines) => lines.get(i).map_or(Item::No, |l| l.1),
            None => self.def.items.get(i).map_or(Item::No, |e| e.item),
        }
    }
    fn label(&self, i: usize) -> &str {
        match &self.lines {
            Some(lines) => lines.get(i).map_or("", |l| l.0.as_str()),
            None => self.def.items.get(i).map_or("", |e| e.label),
        }
    }
    /// Extra space below line `i` (the volume sliders sit in it).
    fn below(&self, i: usize) -> f32 {
        match &self.lines {
            Some(_) => 0.0,
            None => self.def.items.get(i).map_or(0.0, |e| e.below),
        }
    }
    fn disabling(mut self, items: &[Item]) -> Self {
        self.disabled = items.to_vec();
        self.selected = (0..self.len()).find(|&i| !self.disabled.contains(&self.item(i))).unwrap_or(0);
        self
    }
    fn selecting(mut self, item: Item) -> Self {
        self.selected = (0..self.len()).find(|&i| self.item(i) == item).unwrap_or(self.selected);
        self
    }
    fn enabled(&self, i: usize) -> bool {
        !self.disabled.contains(&self.item(i))
    }

    /// Moves the selection; returns the chosen item on accept, `Some(No)`
    /// on back.
    fn update(&mut self, p: &Pressed) -> Option<Item> {
        let n = self.len();
        if n == 0 {
            return p.back.then_some(Item::No);
        }
        let step = |from: usize, by: isize| (from as isize + by).rem_euclid(n as isize) as usize;
        for (pressed, by) in [(p.down, 1), (p.up, -1)] {
            if !pressed {
                continue;
            }
            let mut next = step(self.selected, by);
            while next != self.selected && !self.enabled(next) {
                next = step(next, by);
            }
            self.selected = next;
        }
        if p.back {
            return Some(Item::No);
        }
        (p.accept && self.enabled(self.selected)).then(|| self.item(self.selected))
    }
}

// ---------------------------------------------------------------------------
// State

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Title,
    /// "Loading..." for a frame, then the tower loads behind the select
    /// screen.
    LoadingSelect,
    Select,
    /// "Loading..." then the hero appears in the tower.
    LoadingGame,
    Playing,
    GameOver,
}

/// Where the player is on the select screen (the game's per-player select
/// state; `docs/frontend.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// New / Load.
    NewOrLoad,
    /// Save / Change / Load / Quit / Done (Manage Character).
    Character,
    /// "Character Not Saved / Quit Anyway?" Yes / No.
    ConfirmQuit,
    /// The saved characters (Load from New / Load or the character menu).
    LoadList,
    /// Six initials, picked with up/down.
    Name,
    /// The name blinks for 60 fields.
    NameShown,
    /// Class (left/right, L/R) and colour (up/down).
    Class,
}

/// A player's column on the select screen: what drives it, where it is,
/// and whether its player is ready (the game's state 3).
struct Column {
    devices: Devices,
    select: Select,
    ready: bool,
}

impl Column {
    /// A new player at New / Load.
    fn new(devices: Devices, has_saves: bool) -> Self {
        Self {
            devices,
            select: Select {
                step: Step::NewOrLoad,
                from_character_menu: false,
                name: String::new(),
                letter: b'@',
                // A new character record starts as the Sorceress in
                // yellow (the reset routine's defaults).
                class: 6,
                colour: 0,
                menu: new_load_menu(has_saves),
                t: 0.0,
            },
            ready: false,
        }
    }

    /// A player in the game at the character menu, or ready.
    fn member(devices: Devices, choice: &PlayerChoice, name: &str, has_saves: bool, ready: bool) -> Self {
        let mut c = Self::new(devices, has_saves);
        let s = &mut c.select;
        s.step = Step::Character;
        s.menu = character_menu(has_saves).selecting(Item::Done);
        s.name = name.to_string();
        s.class = CLASSES.iter().position(|k| *k == choice.class).unwrap_or(6);
        s.colour = COLOURS.iter().position(|k| choice.variant.starts_with(k)).unwrap_or(0);
        c.ready = ready;
        c
    }
}

struct Select {
    step: Step,
    /// The step class select returns to.
    from_character_menu: bool,
    name: String,
    /// The character being picked in name entry.
    letter: u8,
    class: usize,
    colour: usize,
    menu: Menu,
    t: f32,
}

/// The front end's state.
#[derive(Resource)]
pub struct Frontend {
    screen: Screen,
    /// Fields (1/60 s) since the screen began.
    t: f32,
    /// Open menus, innermost last.
    menus: Vec<Menu>,
    /// The select screen's four columns, by slot.
    columns: [Option<Column>; MAX_PLAYERS],
    /// This frame's presses, all devices' together (the menus), and each
    /// device's (the columns).
    input: Pressed,
    pressed_by: Vec<(Device, Pressed)>,
    /// The device that last pressed Start or A: the title's player 1, the
    /// pause menu's player.
    last_device: Device,
    frame: u64,
    script: Vec<(String, u64)>,
    /// By slot: seconds each hero has lain dead.
    dead_for: [f32; MAX_PLAYERS],
    /// By slot: the hero is out of the level (its death over, outside the
    /// tower): it waits for the level to end and comes back with its
    /// snapshot when the next level starts.
    out: [bool; MAX_PLAYERS],
    /// With every hero out, the level end has been asked for.
    leaving: bool,
    /// By slot: a new hero was made: its record is the snapshot from now on.
    fresh_hero: [bool; MAX_PLAYERS],
    /// PC Settings waits for a key or button for an action (on the keys or
    /// the pad page), and what came.
    capture: Option<(Page, Action)>,
    captured: Option<Captured>,
    /// The style the Control Style menu shows (kept only on Select).
    style_pick: usize,
}

impl Frontend {
    fn new(screen: Screen) -> Self {
        Self {
            screen,
            t: 0.0,
            menus: Vec::new(),
            columns: Default::default(),
            input: Pressed::default(),
            pressed_by: Vec::new(),
            last_device: Device::Keyboard,
            frame: 0,
            script: menu_script(),
            dead_for: [0.0; MAX_PLAYERS],
            out: [false; MAX_PLAYERS],
            leaving: false,
            fresh_hero: [false; MAX_PLAYERS],
            capture: None,
            captured: None,
            style_pick: 0,
        }
    }

    fn go(&mut self, screen: Screen) {
        self.screen = screen;
        self.t = 0.0;
    }

    /// Whether a full-screen 2D screen is up (title, loading, select, game
    /// over): nothing of the level shows around it.
    /// The title and loading screens cover everything; the select screen's
    /// art ends at y 320 and the tower loaded behind it shows below, and
    /// GAME OVER is drawn over the level.
    fn full_screen(&self) -> bool {
        matches!(self.screen, Screen::Title | Screen::LoadingSelect | Screen::LoadingGame)
    }

    /// Whether a level is being played (menus may be open over it).
    pub fn playing(&self) -> bool {
        self.screen == Screen::Playing
    }

    /// Whether a menu is up.
    pub fn menu_open(&self) -> bool {
        !self.menus.is_empty()
    }

    /// Whether a hero is out of the level (the game's player state `0xB`):
    /// dead outside the tower, waiting for the level to end.
    pub fn hero_out(&self, slot: usize) -> bool {
        self.out.get(slot).copied().unwrap_or(false)
    }

    /// B (back) was pressed this frame.
    pub fn back_pressed(&self) -> bool {
        self.input.back
    }

    /// Whether gameplay should be frozen (anything but plain play).
    fn frozen(&self) -> bool {
        !(self.screen == Screen::Playing && self.menus.is_empty())
    }

    /// Whether the 4:3 screen's sides are covered: any screen of the front
    /// end's but play, and a menu over play.
    fn covers_sides(&self) -> bool {
        self.screen != Screen::Playing || !self.menus.is_empty()
    }
}

/// Black behind the full-screen screens, covering the whole window (the
/// 2D layer is 4:3; a wider window would show the level at the sides) and
/// the in-play HUD.
#[derive(Component)]
struct Backdrop;

/// Black bars beside the 4:3 screen (left, right) while the front end's
/// screens or a menu are up: the game's screens are 4:3, so a wider
/// window shows nothing of the level outside them.
#[derive(Component)]
struct SideBar;

fn spawn_backdrop(mut commands: Commands) {
    commands.spawn((
        Backdrop,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(99),
        Visibility::Hidden,
    ));
    for right in [false, true] {
        let mut node = Node { position_type: PositionType::Absolute, top: Val::Px(0.0), height: Val::Percent(100.0), ..default() };
        if right {
            node.right = Val::Px(0.0);
        } else {
            node.left = Val::Px(0.0);
        }
        commands.spawn((SideBar, node, BackgroundColor(Color::BLACK), GlobalZIndex(99), Visibility::Hidden));
    }
}

fn show_backdrop(
    frontend: Res<Frontend>,
    windows: Query<&Window>,
    mut backdrop: Query<&mut Visibility, (With<Backdrop>, Without<SideBar>)>,
    mut bars: Query<(&mut Node, &mut Visibility), With<SideBar>>,
) {
    let want = if frontend.full_screen() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut backdrop {
        v.set_if_neq(want);
    }
    let width = windows.single().map_or(0.0, |w| ((w.width() - w.height() * crate::font::SCREEN_W / crate::font::SCREEN_H) / 2.0).max(0.0));
    let bars_up = frontend.covers_sides() && width > 0.0;
    for (mut node, mut v) in &mut bars {
        v.set_if_neq(if bars_up { Visibility::Inherited } else { Visibility::Hidden });
        if node.width != Val::Px(width) {
            node.width = Val::Px(width);
        }
    }
}

/// Each hero's record as it stood when the current level began (the
/// game's per-player save of the character when a level outside the tower
/// starts), by slot; a dead hero comes back with it.
#[derive(Resource, Default)]
pub struct Snapshot([Option<PlayerState>; MAX_PLAYERS]);

/// `TEXT/ENGLISH.ROM`, for the strings the front end draws.
#[derive(Resource)]
struct Strings(Option<TextRom>);

impl Strings {
    fn get(&self, group: &str, index: usize) -> Option<&str> {
        self.0.as_ref()?.get(group, index)
    }
}

/// Class codes in the game's class order, and the four colours.
const CLASSES: [&str; 17] =
    ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES", "MIN", "FAL", "JAC", "TIG", "OGR", "UNI", "MED", "HYE", "SUM"];
const COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];
/// The eight classes a new character can be; the rest are unlocked by
/// play (the character's unlock bits) — shown as shadows.
const OPEN_CLASSES: usize = 8;
/// Each player's column on the select screen, and name colour.
const COLUMN: [f32; 4] = [0.0, 128.0, 256.0, 384.0];
const PLAYER_COLOUR: [[u8; 3]; 4] = [[255, 255, 0], [135, 206, 235], [255, 0, 0], [0, 255, 0]];
/// A new hero's name when none is entered: one of the game's sixteen.
const NAMES: [&str; 16] = [
    "LARRY", "PELE", "CHUCK", "TRENT", "SPENCR", "JOFFRY", "PABLO", "JUSTIN", "MAT", "CHIP ", "FRED", "SHAWN", "JAKE",
    "CJ", "ALEX", "MARVIN",
];

fn load_text(mut commands: Commands, mut game: ResMut<LoadedGame>) {
    let rom = game.install.read("TEXT/ENGLISH.ROM").ok().and_then(|b| TextRom::parse(&b).ok());
    if rom.is_none() {
        warn!("TEXT/ENGLISH.ROM didn't load; front-end strings from it are missing");
    }
    commands.insert_resource(Strings(rom));
    commands.insert_resource(ClassStats::load(&mut game));
}

// ---------------------------------------------------------------------------
// Flow

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    mut fe: ResMut<Frontend>,
    real: Res<Time<Real>>,
    mut virt: ResMut<Time<Virtual>>,
    game: Res<LoadedGame>,
    (party, mut changes): (Res<Party>, MessageWriter<PartyChange>),
    mut to_level: MessageWriter<ChangeLevelTo>,
    mut options: ResMut<GameOptions>,
    mut saves: ResMut<Saves>,
    boxes: Res<MessageBox>,
) {
    let has_saves = !saves.file.characters.is_empty();
    let fields = real.delta_secs() * 60.0;
    fe.t += fields;
    for m in &mut fe.menus {
        m.t += fields;
    }
    for c in fe.columns.iter_mut().flatten() {
        c.select.t += fields;
        c.select.menu.t += fields;
    }
    let mut p = fe.input;
    let in_tower = game.current_name().eq_ignore_ascii_case(TOWER);

    // A volume slider under the cursor moves while left or right is held.
    if let Some(m) = fe.menus.last()
        && let Item::Volume(v) = m.item(m.selected)
        && p.hold_left != p.hold_right
    {
        let step = fields.max(1.0) * SLIDER_PER_FIELD * if p.hold_left { -1.0 } else { 1.0 };
        let now = v.get(&options);
        v.set(&mut options, now + step);
    }
    // What the settings act on isn't the menus' to act on again.
    if settings(&mut fe, &p, &mut options) {
        p.accept = false;
    }

    match fe.screen {
        Screen::Title => {
            if fe.menus.is_empty() {
                if p.start || p.accept {
                    fe.menus.push(Menu::new(&TITLE_MENU));
                }
            } else if let Some(item) = fe.menus.last_mut().and_then(|m| m.update(&p)) {
                match item {
                    Item::No => {
                        fe.menus.pop();
                    }
                    Item::Start => {
                        fe.menus.clear();
                        fe.go(Screen::LoadingSelect);
                    }
                    Item::Options => fe.menus.push(Menu::new(&OPTIONS_MENU)),
                    other => open_submenu(&mut fe.menus, other),
                }
            }
        }
        Screen::LoadingSelect => {
            // One frame shows "Loading..." before the load stalls a frame.
            if fe.t > 2.0 {
                to_level.write(ChangeLevelTo::to(TOWER));
                start_select(&mut fe, SelectFor::NewGame, has_saves, &party, &mut changes);
                fe.go(Screen::Select);
            }
        }
        Screen::Select => {
            select_tick(&mut fe);
            select(&mut fe, &party, &mut changes, &mut saves);
        }
        Screen::LoadingGame => {
            if fe.t > 2.0 {
                to_level.write(ChangeLevelTo::to(TOWER));
                fe.go(Screen::Playing);
            }
        }
        // The message box has the pads while it's up.
        Screen::Playing if boxes.is_open() => {}
        Screen::Playing => {
            if fe.menus.is_empty() {
                if p.start {
                    fe.menus.push(Menu::new(if in_tower { &TOWER_MENU } else { &GAME_MENU }));
                }
            } else if let Some(item) = fe.menus.last_mut().and_then(|m| m.update(&p)) {
                match item {
                    Item::No => {
                        fe.menus.pop();
                    }
                    Item::Settings => {
                        fe.menus.push(Menu::new(if in_tower { &TOWER_SETTINGS } else { &LEVEL_SETTINGS }))
                    }
                    Item::QuitGame => fe.menus.push(Menu::new(&QUIT_GAME)),
                    Item::QuitLevel => fe.menus.push(Menu::new(&ABORT_LEVEL)),
                    Item::ConfirmQuitGame => {
                        fe.menus.clear();
                        fe.go(Screen::GameOver);
                    }
                    Item::ConfirmAbortLevel => {
                        // The heroes leave the level; with nobody left in
                        // it the party goes back to the tower.
                        fe.menus.clear();
                        to_level.write(ChangeLevelTo::to(TOWER));
                    }
                    Item::ManageCharacter => {
                        fe.menus.clear();
                        start_select(&mut fe, SelectFor::Manage, has_saves, &party, &mut changes);
                        fe.go(Screen::Select);
                    }
                    // Stand-ins: the shop and inventory screens.
                    Item::Shop | Item::Inventory => info!("{item:?} isn't implemented"),
                    other => open_submenu(&mut fe.menus, other),
                }
            }
        }
        Screen::GameOver => {
            // 240 fields, then the game goes back to its attract loop; here,
            // to the title.
            if fe.t >= 240.0 {
                fe.menus.clear();
                fe.columns = Default::default();
                fe.go(Screen::Title);
            }
        }
    }

    let frozen = fe.frozen() || boxes.is_open();
    if frozen != virt.is_paused() {
        if frozen { virt.pause() } else { virt.unpause() }
    }
}

fn open_submenu(menus: &mut Vec<Menu>, item: Item) {
    let def = match item {
        Item::Audio => &AUDIO_MENU,
        Item::GameOptions => &GAME_OPTIONS,
        Item::Controls => &CONTROLS_MENU,
        Item::PcSettings => &PC_MENU,
        // Stand-in: the settings themselves aren't modelled.
        _ => return,
    };
    menus.push(Menu::new(def));
}

/// The settings' own work before a menu's choice is acted on: what came
/// for a binding, left / right on the style and the pages' switches, and
/// the choices that change options (which then remake the menus' lines).
/// Whether it took this frame's accept.
fn settings(fe: &mut Frontend, p: &Pressed, options: &mut GameOptions) -> bool {
    let mut changed = false;
    let mut took = false;
    if let (Some((_, action)), Some(got)) = (fe.capture, fe.captured.take()) {
        match got {
            Captured::Key(input) => options.bindings.bind_key(action, input),
            Captured::Pad(b) => options.bindings.bind_pad(action, b),
            Captured::Cancel => {}
        }
        fe.capture = None;
        changed = true;
    }
    let top = fe.menus.last().map(|m| (m.kind, m.item(m.selected)));
    match top {
        // Left / right change the style shown; Select keeps it.
        Some((Some(Dynamic::Style), _)) if p.left || p.right => {
            let n = controls::MENU_SCHEMES;
            fe.style_pick = (fe.style_pick + if p.right { 1 } else { n - 1 }) % n;
            changed = true;
        }
        // Left / right flip a page's switch too.
        Some((Some(Dynamic::Page(_)), Item::Toggle(s))) if p.left || p.right => {
            s.set(options, !s.get(options));
            changed = true;
        }
        _ => {}
    }
    if p.accept
        && let Some(m) = fe.menus.last()
        && m.enabled(m.selected)
    {
        match m.item(m.selected) {
            Item::Style => {
                fe.style_pick = options.scheme.min(controls::MENU_SCHEMES - 1);
                let menu = Menu::dynamic(&STYLE_MENU, Dynamic::Style, options, fe.style_pick);
                fe.menus.push(menu);
                took = true;
            }
            Item::StyleLine => {
                options.scheme = fe.style_pick;
                fe.menus.pop();
                changed = true;
                took = true;
            }
            Item::Rumble | Item::AutoAim | Item::AutoAttack | Item::Compass => {
                let (def, setting) = match m.item(m.selected) {
                    Item::Rumble => (&RUMBLE_MENU, Setting::Rumble),
                    Item::AutoAim => (&AUTO_AIM_MENU, Setting::AutoAim),
                    Item::AutoAttack => (&AUTO_ATTACK_MENU, Setting::AutoAttack),
                    _ => (&COMPASS_MENU, Setting::Compass),
                };
                let current = setting.get(options);
                let mut menu = Menu::dynamic(def, Dynamic::Choice(setting), options, fe.style_pick);
                menu.selected = setting.choices().1.iter().position(|&(_, v)| v == current).unwrap_or(0);
                fe.menus.push(menu);
                took = true;
            }
            Item::Choose(setting, v) => {
                setting.set(options, v);
                fe.menus.pop();
                changed = true;
                took = true;
            }
            Item::Page(page) => {
                let def = match page {
                    Page::Keys => &KEYS_PAGE,
                    Page::Pad => &PAD_PAGE,
                    Page::Video => &VIDEO_PAGE,
                    Page::Debug => &DEBUG_PAGE,
                };
                let menu = Menu::dynamic(def, Dynamic::Page(page), options, fe.style_pick);
                fe.menus.push(menu);
                took = true;
            }
            Item::Toggle(s) => {
                s.set(options, !s.get(options));
                changed = true;
                took = true;
            }
            Item::Bind(action) => {
                let page = if m.kind == Some(Dynamic::Page(Page::Pad)) { Page::Pad } else { Page::Keys };
                fe.capture = Some((page, action));
                fe.captured = None;
                changed = true;
                took = true;
            }
            Item::ResetBindings => {
                if m.kind == Some(Dynamic::Page(Page::Pad)) {
                    options.bindings.pad.clear();
                } else {
                    options.bindings.keys = controls::Bindings::default().keys;
                }
                changed = true;
                took = true;
            }
            _ => {}
        }
    }
    if changed {
        let (pick, capture) = (fe.style_pick, fe.capture.map(|(_, a)| a));
        for m in &mut fe.menus {
            if let Some(kind) = m.kind {
                m.lines = Some(dynamic_lines(kind, options, pick, capture));
            }
        }
    }
    took
}

/// How the select screen opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectFor {
    /// A new game from the title: the party starts afresh, its first
    /// player the device that started.
    NewGame,
    /// Manage Character from the tower: the players in the game, ready —
    /// but the one who asked, at their character menu.
    Manage,
}

/// Starts the select screen.
fn start_select(fe: &mut Frontend, how: SelectFor, has_saves: bool, party: &Party, changes: &mut MessageWriter<PartyChange>) {
    fe.columns = Default::default();
    match how {
        SelectFor::NewGame => {
            changes.write(PartyChange::Clear);
            // Player 1 keeps the keyboard too when they started with a pad
            // (alone, they play with both).
            let mut devices = fe.last_device.only();
            devices.keyboard = true;
            fe.columns[0] = Some(Column::new(devices, has_saves));
        }
        SelectFor::Manage => {
            let asker = fe.last_device;
            for (slot, m) in party.members() {
                let ready = !asker.among(&m.devices) && !(party.len() == 1);
                fe.columns[slot] = Some(Column::member(m.devices, &m.choice, &m.name, has_saves, ready));
            }
        }
    }
    for (slot, c) in fe.columns.iter_mut().enumerate() {
        if let Some(c) = c {
            c.select.menu.column = COLUMN[slot];
        }
    }
}

/// What a column's presses came to.
enum ColumnEvent {
    None,
    /// Its player backed out (or quit): the column empties.
    Leave,
    /// The game ends (the last player quit their character).
    Title,
}

/// The select screen: each device drives its column; a device without one
/// joins at the first free column with Start (a pad) or A. The game goes on
/// once every player in is ready; with nobody left it's back to the title.
fn select(fe: &mut Frontend, party: &Party, changes: &mut MessageWriter<PartyChange>, saves: &mut Saves) {
    let has_saves = !saves.file.characters.is_empty();
    let frame = fe.frame;
    let presses = fe.pressed_by.clone();
    for (device, p) in presses {
        if !p.any() {
            continue;
        }
        let owner = fe.columns.iter().position(|c| c.as_ref().is_some_and(|c| device.among(&c.devices)));
        match owner {
            Some(slot) => {
                let Some(column) = fe.columns[slot].as_mut() else { continue };
                match select_column(column, slot, &p, party, changes, saves, frame) {
                    ColumnEvent::None => {}
                    ColumnEvent::Leave => {
                        fe.columns[slot] = None;
                        changes.write(PartyChange::Leave(slot));
                    }
                    ColumnEvent::Title => {
                        fe.columns = Default::default();
                        changes.write(PartyChange::Clear);
                        fe.menus.clear();
                        fe.go(Screen::Title);
                        return;
                    }
                }
            }
            None if p.start || (p.accept && device != Device::Keyboard) => {
                if let Some(slot) = fe.columns.iter().position(Option::is_none) {
                    info!("player {} joins on the select screen ({device:?})", slot + 1);
                    let mut c = Column::new(device.only(), has_saves);
                    c.select.menu.column = COLUMN[slot];
                    fe.columns[slot] = Some(c);
                }
            }
            None => {}
        }
    }
    for (slot, c) in fe.columns.iter().enumerate() {
        if let Some(c) = c
            && c.ready
            && c.select.step == Step::Class
        {
            debug!("player {} ready", slot + 1);
        }
    }
    let joined: Vec<&Column> = fe.columns.iter().flatten().collect();
    if joined.is_empty() {
        // The only player backed out: back to the title.
        fe.go(Screen::Title);
    } else if joined.iter().all(|c| c.ready) {
        // Every player in is ready: into the tower.
        fe.go(Screen::LoadingGame);
    }
}

/// One column's step on its device's presses: New / Load, the character
/// menu, the load list, name entry and the class, as the game's
/// per-player select update has them (`docs/frontend.md`).
#[allow(clippy::too_many_arguments)]
fn select_column(
    column: &mut Column,
    slot: usize,
    p: &Pressed,
    party: &Party,
    changes: &mut MessageWriter<PartyChange>,
    saves: &mut Saves,
    frame: u64,
) -> ColumnEvent {
    let has_saves = !saves.file.characters.is_empty();
    let devices = column.devices;
    let member = party.get(slot);
    // The hero as it would be saved now.
    let record = member.map(|m| SavedCharacter::of(&m.name, &m.choice.class, &m.choice.variant, &m.state));
    // A ready player opens their character menu again with A (or Start).
    if column.ready {
        if p.accept || p.start {
            column.ready = false;
            let s = &mut column.select;
            if member.is_some() {
                s.step = Step::Character;
                s.menu = character_menu(has_saves).selecting(Item::Done);
            } else {
                s.step = Step::Class;
            }
            s.menu.column = COLUMN[slot];
        }
        return ColumnEvent::None;
    }
    let s = &mut column.select;
    match s.step {
        Step::NewOrLoad => match s.menu.update(p) {
            Some(Item::New) => {
                s.step = Step::Name;
                s.name.clear();
                s.letter = b'@';
            }
            Some(Item::Load) => {
                s.from_character_menu = false;
                s.step = Step::LoadList;
                s.menu = load_list(saves);
                s.menu.column = COLUMN[slot];
            }
            Some(Item::No) => return ColumnEvent::Leave,
            _ => {}
        },
        Step::Character => match s.menu.update(p) {
            Some(Item::Change) => {
                s.from_character_menu = true;
                s.step = Step::Class;
            }
            Some(Item::Save) => {
                // Stand-in for the game's memory card screens: the record
                // goes straight into the save file.
                if let Some(record) = record {
                    let name = record.name.clone();
                    match saves.save(record) {
                        Ok(()) => info!("saved {name} to {}", crate::saves::path().display()),
                        Err(e) => warn!("saving {name} failed: {e}"),
                    }
                }
                s.menu = character_menu(!saves.file.characters.is_empty()).selecting(Item::Done);
                s.menu.column = COLUMN[slot];
            }
            Some(Item::Load) => {
                s.from_character_menu = true;
                s.step = Step::LoadList;
                s.menu = load_list(saves);
                s.menu.column = COLUMN[slot];
            }
            Some(Item::Quit) => {
                // Only a hero with changes since its last save asks first.
                let saved = record.as_ref().is_some_and(|r| saves.file.characters.iter().any(|c| c == r));
                if saved {
                    return quit_event(party, slot);
                }
                s.step = Step::ConfirmQuit;
                s.menu = Menu::new(&YES_NO).selecting(Item::No);
                s.menu.column = COLUMN[slot];
            }
            // Back into the tower with the (maybe changed) hero, once
            // everyone's ready.
            Some(Item::Done | Item::No) => column.ready = true,
            _ => {}
        },
        Step::ConfirmQuit => match s.menu.update(p) {
            Some(Item::Yes) => return quit_event(party, slot),
            Some(Item::No) => {
                s.step = Step::Character;
                s.menu = character_menu(has_saves).selecting(Item::Quit);
                s.menu.column = COLUMN[slot];
            }
            _ => {}
        },
        Step::LoadList => match s.menu.update(p) {
            Some(Item::Character(i)) => {
                if let Some(c) = saves.file.characters.get(i).cloned() {
                    s.class = CLASSES.iter().position(|k| *k == c.class).unwrap_or(s.class);
                    s.colour = COLOURS.iter().position(|k| *k == c.variant).unwrap_or(s.colour);
                    s.name = c.name.clone();
                    // Even the same class and colour: a new record, the
                    // saved one laid on it (`player_state::new_member`).
                    info!("player {}: loaded {} the {} ({}), level {}", slot + 1, c.name, c.class, c.variant, c.level);
                    changes.write(PartyChange::Set {
                        slot,
                        choice: PlayerChoice { class: c.class.clone(), variant: c.variant.clone() },
                        name: c.name.clone(),
                        saved: Some(Box::new(c)),
                        fresh: true,
                        devices,
                    });
                    column.ready = true;
                }
            }
            Some(Item::No) => {
                if s.from_character_menu {
                    s.step = Step::Character;
                    s.menu = character_menu(has_saves).selecting(Item::Load);
                } else {
                    s.step = Step::NewOrLoad;
                    s.menu = new_load_menu(has_saves).selecting(Item::Load);
                }
                s.menu.column = COLUMN[slot];
            }
            _ => {}
        },
        Step::Name => {
            // Up/down step through 0-9, @ (end), A-Z, _; A takes the
            // letter (on @: done); L/R rub the last one out; B cancels.
            if p.up {
                s.letter = match s.letter {
                    b'9' => b'@',
                    b'@' => b'A',
                    b'Z' => b'_',
                    b'_' => b'0',
                    c => c + 1,
                };
            }
            if p.down {
                s.letter = match s.letter {
                    b'A' => b'@',
                    b'@' => b'9',
                    b'0' => b'_',
                    b'_' => b'Z',
                    c => c - 1,
                };
            }
            if (p.l || p.r) && !s.name.is_empty() {
                s.letter = s.name.pop().map_or(b'@', |c| c as u8);
            }
            if p.accept {
                if s.letter == b'@' {
                    s.step = Step::NameShown;
                } else {
                    if s.name.len() < 6 {
                        s.name.push(s.letter as char);
                    }
                    s.letter = b'@';
                    if s.name.len() >= 6 {
                        s.step = Step::NameShown;
                    }
                }
                if s.step == Step::NameShown {
                    if s.name.is_empty() {
                        let pick = (frame as usize + slot) % NAMES.len();
                        s.name = NAMES[pick].to_string();
                    }
                    s.t = 0.0;
                }
            } else if p.back {
                s.step = Step::NewOrLoad;
                s.menu = new_load_menu(has_saves);
                s.menu.column = COLUMN[slot];
            }
        }
        // The name blinks for its 60 fields (`select_tick`).
        Step::NameShown => {}
        Step::Class => {
            // Left/L and right/R change class (the secret seventeenth only
            // when unlocked, never here); up/down change colour.
            if p.left || p.l {
                s.class = (s.class + 15) % 16;
            }
            if p.right || p.r {
                s.class = (s.class + 1) % 16;
            }
            if p.up {
                s.colour = (s.colour + 1) % 4;
            }
            if p.down {
                s.colour = (s.colour + 3) % 4;
            }
            if p.accept && s.class < OPEN_CLASSES {
                let picked = PlayerChoice { class: CLASSES[s.class].to_string(), variant: COLOURS[s.colour].to_string() };
                // Another class or colour, or a new game even with the same
                // hero: a fresh record.
                let fresh = member.map(|m| &m.choice) != Some(&picked) || !s.from_character_menu;
                info!("player {}: {} the {} ({})", slot + 1, s.name, picked.class, picked.variant);
                changes.write(PartyChange::Set { slot, choice: picked, name: s.name.clone(), saved: None, fresh, devices });
                if s.from_character_menu {
                    s.step = Step::Character;
                    s.menu = character_menu(has_saves).selecting(Item::Done);
                    s.menu.column = COLUMN[slot];
                } else {
                    // A new hero: this player is ready.
                    column.ready = true;
                }
            } else if p.back {
                if s.from_character_menu {
                    s.step = Step::Character;
                    s.menu = character_menu(has_saves).selecting(Item::Change);
                } else {
                    s.step = Step::NewOrLoad;
                    s.menu = new_load_menu(has_saves);
                }
                s.menu.column = COLUMN[slot];
            }
        }
    }
    ColumnEvent::None
}

/// A player quitting their character: out of the game, or with nobody else
/// in, the game ends.
fn quit_event(party: &Party, slot: usize) -> ColumnEvent {
    if party.members().all(|(s, _)| s == slot) { ColumnEvent::Title } else { ColumnEvent::Leave }
}

/// The select screen's clocks: a finished name blinks for 60 fields, then
/// the class.
fn select_tick(fe: &mut Frontend) {
    for c in fe.columns.iter_mut().flatten() {
        if c.select.step == Step::NameShown && c.select.t >= 60.0 {
            c.select.step = Step::Class;
            c.select.from_character_menu = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Death

/// Stand-in for the game's dying time (its own counter on the death
/// animation isn't traced): the DEATH action, at most this long.
const DYING_SECONDS: f32 = 4.0;

/// When a level starts: outside the tower and the secret realm the game
/// saves each hero's record — except in `levelE2` and `levelF2` — and a
/// hero that died in the last level comes back with its saved record.
fn level_started(
    population: Res<LevelPopulation>,
    mut fe: ResMut<Frontend>,
    mut snapshot: ResMut<Snapshot>,
    mut party: ResMut<Party>,
) {
    let name = population.level.to_ascii_lowercase();
    let realm = name.strip_prefix("level").and_then(|r| r.chars().next());
    let saves = !matches!(realm, Some('l' | 's')) && name != "levele2" && name != "levelf2";
    fe.leaving = false;
    for (slot, state) in party.states_mut() {
        if std::mem::take(&mut fe.fresh_hero[slot]) {
            snapshot.0[slot] = None;
        }
        if std::mem::take(&mut fe.out[slot]) {
            if let Some(saved) = &snapshot.0[slot] {
                *state = saved.clone();
            }
            state.alive = true;
            state.health = state.health.max(1.0);
            info!("player {} is back in {}", slot + 1, population.level);
        }
        if saves || snapshot.0[slot].is_none() {
            snapshot.0[slot] = Some(state.clone());
        }
    }
}

/// A dead hero plays DEATH; then in the tower it stands up again with its
/// saved record, and anywhere else it is out of the level (hidden). With no
/// hero left standing the level ends from the next frame — once the voice
/// queues are empty, as every level change waits (`exits.rs`) — and the
/// party returns to the tower, where they are revived.
#[allow(clippy::too_many_arguments)]
fn death(
    time: Res<Time>,
    game: Res<LoadedGame>,
    mut fe: ResMut<Frontend>,
    snapshot: Res<Snapshot>,
    mut party: ResMut<Party>,
    mut players: Query<(Entity, &Player, &mut Animator)>,
    mut commands: Commands,
    mut to_level: MessageWriter<ChangeLevelTo>,
) {
    if fe.screen != Screen::Playing {
        fe.dead_for = [0.0; MAX_PLAYERS];
        return;
    }
    let in_play: Vec<usize> = party.members().map(|(slot, _)| slot).collect();
    if !in_play.is_empty() && in_play.iter().all(|&slot| fe.out[slot]) {
        if !fe.leaving {
            fe.leaving = true;
            to_level.write(ChangeLevelTo::to(TOWER));
            info!("no hero left standing: back to the tower");
        }
        return;
    }
    let in_tower = game.current_name().to_ascii_lowercase().starts_with("levell");
    for (entity, player, mut animator) in &mut players {
        let slot = player.slot;
        let Some(state) = party.state_mut(slot) else { continue };
        if state.alive || fe.out[slot] {
            fe.dead_for[slot] = 0.0;
            continue;
        }
        if fe.dead_for[slot] == 0.0 {
            animator.play_named("DEATH");
        }
        fe.dead_for[slot] += time.delta_secs();
        let done = animator.action_name() == "DEATH" && animator.finished();
        if !(done && fe.dead_for[slot] > 0.5) && fe.dead_for[slot] < DYING_SECONDS {
            continue;
        }
        fe.dead_for[slot] = 0.0;
        if in_tower {
            if let Some(saved) = &snapshot.0[slot] {
                *state = saved.clone();
            }
            state.alive = true;
            state.health = state.health.max(1.0);
            animator.play_named("READY");
            info!("player {} stands up again in the tower", slot + 1);
        } else {
            fe.out[slot] = true;
            commands.entity(entity).insert(Visibility::Hidden);
            info!("player {} is out of the level", slot + 1);
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing

fn rgb(c: [u8; 3]) -> Color {
    Color::srgb_u8(c[0], c[1], c[2])
}

/// The glow shimmering text gets (`0x8200EA`).
pub(crate) fn glow_colour() -> Color {
    rgb(PURPLE)
}

/// The game's pulse for glowing text: a triangle over 40 fields up and 40
/// down, then 5 at rest, from half to full opacity.
pub(crate) fn pulse(t: f32) -> f32 {
    let phase = t.rem_euclid(85.0);
    let tri = if phase > 80.0 { 0.0 } else if phase > 40.0 { 80.0 - phase } else { phase };
    0.5 + 0.5 * (tri / 40.0)
}

#[allow(clippy::too_many_arguments)]
fn draw(
    fe: Res<Frontend>,
    mut draw: ResMut<Draw2d>,
    fonts: Option<Res<GameFonts>>,
    mut tex: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    strings: Option<Res<Strings>>,
    stats: Option<Res<ClassStats>>,
    options: Res<GameOptions>,
) {
    let (Some(fonts), Some(tex), Some(strings), Some(stats)) = (fonts, tex.as_deref_mut(), strings, stats) else {
        return;
    };
    let mut d = Painter { draw: &mut draw, fonts: &fonts, tex, images: &mut images };
    match fe.screen {
        Screen::Title => {
            title(&mut d, fe.t);
            if fe.menus.is_empty() {
                d.draw.shimmer(d.fonts, FONT32, 1.0, -256.0, 320.0, "Press Start", rgb(PURPLE), pulse(fe.t));
            }
        }
        Screen::LoadingSelect | Screen::LoadingGame => {
            if fe.screen == Screen::LoadingSelect {
                title(&mut d, fe.t);
            } else if let Some(t) = d.tex.get("TRANSITION_SCREEN", d.images) {
                d.draw.image(&t, 0.0, 0.0, 512.0, 384.0, Color::WHITE);
            }
            d.draw.shimmer(d.fonts, FONT32, 1.0, -256.0, 320.0, "Loading...", rgb(PURPLE), pulse(fe.t));
        }
        Screen::Select => select_screen(&mut d, &fe, &strings, &stats),
        Screen::Playing => {}
        Screen::GameOver => {
            // The game types it out one letter per 8 fields after 60,
            // each letter already in its final place.
            let text = strings.get("GAME_OVER", 0).unwrap_or("GAME OVER");
            let shown = (((fe.t - 60.0) / 8.0).floor().clamp(0.0, 9.0)) as usize;
            let style = TextStyle::new(FONT32, 2.0, Color::WHITE);
            let width = d.fonts.width(FONT32, 2.0, text);
            let left = 256.0 - (width / 2.0).trunc();
            let part: String = text.chars().take(shown).collect();
            d.draw.text(d.fonts, &style, left, 120.0, &part);
        }
    }
    // Only the innermost menu is up: opening a sub-menu closes its parent.
    if let Some(m) = fe.menus.last() {
        draw_menu(&mut d, m, &options);
        if m.kind == Some(Dynamic::Style) {
            for (name, x, y) in STYLE_PICTURES {
                d.image(name, x, y, Color::WHITE);
            }
            style_labels(&mut d, &strings, fe.style_pick);
        }
    }
}

/// Where the Control Style screen labels the controller (the game's
/// table, `docs/frontend.md`): each label's alignment (0 left edge, 1 centred, 2 right edge,
/// at x + 256) and its middle's y.
const STYLE_LABELS: [(u8, f32, f32); 16] = [
    (2, -125.0, 108.0),
    (2, -142.0, 128.0),
    (2, -155.0, 145.0),
    (2, -155.0, 174.0),
    (2, -152.0, 200.0),
    (2, -136.0, 223.0),
    (1, -26.0, 256.0),
    (1, 30.0, 256.0),
    (2, -72.0, 245.0),
    (0, 72.0, 245.0),
    (0, 127.0, 110.0),
    (0, 140.0, 128.0),
    (0, 136.0, 148.0),
    (0, 156.0, 168.0),
    (0, 136.0, 191.0),
    (0, 160.0, 222.0),
];
/// The labels' scale (their text groups' 0.4) and line gap.
const STYLE_LABEL_SCALE: f32 = 0.4;
const STYLE_LABEL_GAP: f32 = 2.0;

/// The Control Style screen's labels: the strings of `CONTROLS1..3` for
/// the style shown, in their group's font, each line centred on the
/// label's block (`docs/frontend.md`).
fn style_labels(d: &mut Painter, strings: &Strings, style: usize) {
    let group = format!("CONTROLS{}", style + 1);
    let height = d.fonts.line_height(INITIALS, STYLE_LABEL_SCALE).trunc();
    let ink = TextStyle::new(INITIALS, STYLE_LABEL_SCALE, rgb(INK));
    for (i, &(align, x, y)) in STYLE_LABELS.iter().enumerate() {
        let Some(text) = strings.get(&group, i) else { continue };
        let lines: Vec<&str> = text.split('\n').map(str::trim_end).collect();
        let width = lines.iter().map(|l| d.fonts.width(INITIALS, STYLE_LABEL_SCALE, l)).fold(0.0, f32::max);
        let centre = match align {
            1 => x + 256.0,
            2 => x + 256.0 - (width / 2.0).trunc(),
            _ => x + 256.0 + (width / 2.0).trunc(),
        };
        let block = lines.len() as f32 * (height + STYLE_LABEL_GAP);
        let mut top = y - (block / 2.0).trunc();
        for line in lines {
            d.draw.text(d.fonts, &ink, -centre, top, line);
            top += height + STYLE_LABEL_GAP;
        }
    }
}

struct Painter<'a> {
    draw: &'a mut Draw2d,
    fonts: &'a GameFonts,
    tex: &'a mut UiTextures,
    images: &'a mut Assets<Image>,
}

impl Painter<'_> {
    fn image(&mut self, name: &str, x: f32, y: f32, color: Color) {
        if let Some(i) = self.tex.get(name, self.images) {
            self.draw.image(&i, x, y, i.size.x, i.size.y, color);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn image_sized(&mut self, name: &str, frame: u16, x: f32, y: f32, w: f32, h: f32, color: Color) {
        if let Some(i) = self.tex.frame(name, frame, self.images) {
            self.draw.image(&i, x, y, w, h, color);
        }
    }
}

/// The title: four tiles of `TITLE00..03`, and the glow flipbook
/// (`GLOWCROP_00`, ten frames, a frame per 4 fields) fading in over 60.
fn title(d: &mut Painter, t: f32) {
    d.draw.fill(0.0, 0.0, 512.0, 384.0, Color::BLACK);
    for (i, (x, y)) in [(0.0, 0.0), (256.0, 0.0), (0.0, 256.0), (256.0, 256.0)].into_iter().enumerate() {
        d.image(&format!("TITLE{i:02}"), x, y, Color::WHITE);
    }
    let frame = ((t / 4.0) as u16) % 10;
    let alpha = (t / 60.0).clamp(0.0, 1.0);
    d.image_sized("GLOWCROP_00", frame, 192.0, 0.0, 128.0, 128.0, Color::WHITE.with_alpha(alpha));
}

fn draw_menu(d: &mut Painter, m: &Menu, options: &GameOptions) {
    let def = m.def;
    let fade = if def.fade { (m.t / 30.0).clamp(0.0, 1.0) } else { 1.0 };
    let texture = if def.parch { FontTexture::Parch } else { FontTexture::Own };
    if let Some((top, w, h)) = def.panel {
        let x = 256.0 - w / 2.0;
        d.image_sized("SCROLL_A", 0, x, top, w, h, Color::WHITE.with_alpha(fade));
        if let Some(title) = def.title {
            let style = TextStyle::new(FONT32, def.title_scale, Color::WHITE.with_alpha(fade)).with_texture(texture);
            d.draw.text(d.fonts, &style, -(x + w / 2.0), top + 58.0, title);
        }
    }
    if def.logo {
        let frame = ((m.t / 8.0) as u16) % 5;
        d.image_sized("LOGO_BURN1", frame, 290.0, 142.0, 224.0, 172.0, Color::WHITE.with_alpha(fade));
    }
    let line = d.fonts.line_height(FONT32, def.item_scale).trunc();
    let total: f32 = (0..m.len()).map(|i| line + m.below(i)).sum();
    let top = match def.y {
        y if y >= 0.0 => y,
        -1.0 => 192.0 - (total / 2.0).trunc(),
        y => -y - (total / 2.0).trunc(),
    };
    let x = if def.x < 0.0 { def.x - m.column } else { def.x };
    if def.backdrop {
        let alpha = 0.6 * fade;
        d.draw.fill(x - 24.0, top - 6.0, 512.0 - 2.0 * (x - 24.0), total + 12.0, Color::srgba(0.0, 0.0, 0.0, alpha));
    }
    // Letters flicker through the GAR frames 10..22 fields after opening.
    let gar = (def.gar && (10.0..22.0).contains(&m.t)).then(|| ((m.t - 10.0) / 2.0) as u8);
    let mut y = top;
    for i in 0..m.len() {
        let (full, item) = (m.label(i), m.item(i));
        let (label, value) = full.split_once('\t').unwrap_or((full, ""));
        let alpha = fade * if m.enabled(i) { 1.0 } else { 0.5 };
        let selected = i == m.selected;
        if selected {
            // A purple glow pulsing under white letters.
            let glow = TextStyle::new(FONT32, def.item_scale, rgb(def.glow).with_alpha(alpha * pulse(m.t)))
                .with_texture(FontTexture::Glow)
                .glowing();
            let width = d.draw.text(d.fonts, &glow, x, y, label);
            d.draw.text(d.fonts, &TextStyle::new(FONT32, def.item_scale, Color::WHITE.with_alpha(alpha)), x, y, label);
            if def.arrow {
                // Stand-in for the game's spinning 3D arrow (`ICON_ARROW`),
                // which sits 16 left of the items.
                let left = if x < 0.0 { -x - (width / 2.0).trunc() } else { x };
                let size = 24.0;
                let colour = Color::WHITE.with_alpha(alpha);
                d.image_sized("MENU_MARKER", 0, left - 16.0 - size / 2.0, y + line / 2.0 - size / 2.0, size, size, colour);
            }
        } else {
            let (colour, tex) = match gar {
                Some(f) => (rgb(def.normal), FontTexture::Gar(f)),
                // Parchment letters carry their own colour.
                None if def.parch => (Color::WHITE, FontTexture::Parch),
                None => (rgb(def.normal), FontTexture::Own),
            };
            let style = TextStyle::new(FONT32, def.item_scale, colour.with_alpha(alpha)).with_texture(tex);
            d.draw.text(d.fonts, &style, x, y, label);
        }
        if !value.is_empty() {
            let colour = if selected { Color::WHITE } else { rgb(def.normal) };
            d.draw.text(d.fonts, &TextStyle::new(FONT32, def.item_scale, colour.with_alpha(alpha)), x + VALUE_COLUMN, y, value);
        }
        if let Item::Volume(v) = item {
            slider(d, x, y + line, v.get(options), if selected { fade } else { 0.6 * fade });
        }
        y += line + m.below(i);
    }
    if def.hints {
        // Two hints: Back (B) centred at 512/3, Select (A) at 2 × 512/3.
        let scale = 0.667;
        let icon = (32.0_f32 * scale).trunc();
        for (k, (label, button)) in [("Back", "BUTTON_TRI"), ("Select", "BUTTON_X")].into_iter().enumerate() {
            let centre = (512 / 3 * (k as i32 + 1)) as f32;
            let style = TextStyle::new(FONT32, scale, Color::WHITE.with_alpha(fade)).with_texture(texture);
            let w = d.draw.text(d.fonts, &style, -centre, 304.0, label);
            d.image_sized(button, 0, centre - (w / 2.0).trunc() - icon - 4.0, 304.0, icon, icon, Color::WHITE.with_alpha(fade));
        }
    }
}

/// The game's volume bar under a slider item: `EMPTY_BAR` 264 wide, the
/// `PINK_BAR` fill over it, the `slider` knob at the fill's end, and the
/// `MARKER_LEFT` / `MARKER_RIGHT` ends; items not selected are dimmer.
fn slider(d: &mut Painter, x: f32, y: f32, value: f32, alpha: f32) {
    const WIDTH: f32 = 264.0;
    let fill = (WIDTH * value).max(1.0).trunc();
    let c = Color::WHITE.with_alpha(alpha);
    d.image("MARKER_LEFT", x - 52.0, y, c);
    d.image("MARKER_RIGHT", x + WIDTH - 24.0, y, c);
    d.image_sized("EMPTY_BAR", 0, x, y + 11.0, WIDTH, 32.0, c);
    d.image_sized("PINK_BAR", 0, x, y + 15.0, fill, 32.0, c);
    d.image("SLIDER", x + fill - 20.0, y + 2.0, c);
}

/// The character-select screen: four player columns (`S1_PLYRn` over
/// `S2_PLYRn`, framed by `S1_BORDER` / `S2_BORDER`); player 1's column
/// shows its menu, name entry or class card.
/// Where each player's name starts on the select screen (the game's
/// per-player table, less 34).
const NAME_LEFT: [f32; 4] = [8.0, 138.0, 266.0, 391.0];

/// The select screen: the players' panels along the bottom (as the game
/// keeps them up there: in the player's colour once joined, dim
/// otherwise), each column's art, and in each joined column its player's
/// step — a ready player's class card.
fn select_screen(d: &mut Painter, fe: &Frontend, strings: &Strings, stats: &ClassStats) {
    for (slot, &col) in COLUMN.iter().enumerate() {
        let [r, g, b] = if fe.columns[slot].is_some() { crate::game_hud::JOINED[slot] } else { crate::game_hud::NOT_JOINED[slot] };
        d.image_sized("S3", 0, col, 304.0, 128.0, 16.0, Color::WHITE);
        d.image_sized("S4", 0, col, 320.0, 128.0, 64.0, Color::srgb_u8(r, g, b));
        d.image_sized("S4_FRAME", 0, col, 320.0, 128.0, 64.0, Color::WHITE);
    }
    for (p, &col) in COLUMN.iter().enumerate() {
        d.image(&format!("S1_PLYR{}", p + 1), col, 0.0, Color::WHITE);
        d.image(&format!("S2_PLYR{}", p + 1), col, 256.0, Color::WHITE);
    }
    for (slot, column) in fe.columns.iter().enumerate() {
        let Some(column) = column else { continue };
        if column.ready {
            class_card(d, &column.select, strings, stats, COLUMN[slot], false);
        } else {
            column_screen(d, &column.select, slot, strings, stats);
        }
    }
    for &col in &COLUMN {
        d.image("S1_BORDER", col, 0.0, Color::WHITE);
        d.image("S2_BORDER", col, 256.0, Color::WHITE);
    }
}

/// A joined player's step in their column.
fn column_screen(d: &mut Painter, s: &Select, slot: usize, strings: &Strings, stats: &ClassStats) {
    let col = COLUMN[slot];
    let small = TextStyle::new(FONT8X8, 1.2, Color::WHITE);
    let icon = 19.0;
    // The button legend: icons, then the word.
    let legend = |d: &mut Painter, icons: &[&str], y: f32, label: &str| {
        let right = col + icon + 10.0;
        let mut x = right - icon * (icons.len() as f32 - 1.0);
        for name in icons {
            d.image_sized(name, 0, x, y, icon, icon, Color::WHITE);
            x += icon;
        }
        d.draw.text(d.fonts, &small, right + icon + 8.0, y + 4.0, label);
    };
    let select_back = |d: &mut Painter, select: bool, back: bool| {
        if select {
            d.image_sized("BUTTON_X", 0, col + 20.0, 252.0, icon, icon, Color::WHITE);
            d.draw.text(d.fonts, &small, col + 20.0 + icon + 8.0, 256.0, "Select");
        }
        if back {
            d.image_sized("BUTTON_TRI", 0, col + 20.0, 272.0, icon, icon, Color::WHITE);
            d.draw.text(d.fonts, &small, col + 20.0 + icon + 8.0, 276.0, "Back");
        }
    };
    match s.step {
        Step::NewOrLoad | Step::Character | Step::ConfirmQuit | Step::LoadList => {
            if s.step == Step::ConfirmQuit {
                for (k, line) in ["Character", "Not Saved", "Quit Anyway?"].into_iter().enumerate() {
                    d.draw.text(d.fonts, &small, -(col + 64.0), 100.0 + 10.0 * k as f32, line);
                }
            }
            if s.step == Step::LoadList {
                d.draw.text(d.fonts, &small, -(col + 64.0), 60.0, "Load Character");
            }
            draw_menu(d, &s.menu, &GameOptions::default());
            select_back(d, true, true);
        }
        Step::Name | Step::NameShown => {
            let big = TextStyle::new(FONT32, 0.8, Color::WHITE);
            for (k, word) in ["Enter", "Your", "Name"].into_iter().enumerate() {
                d.draw.text(d.fonts, &big, -(col + 64.0), 64.0 + 26.0 * k as f32, word);
            }
            legend(d, &["BUTTON_U", "BUTTON_D"], 180.0, "Change");
            legend(d, &["BUTTON_L", "BUTTON_R"], 200.0, "Edit");
            legend(d, &["BUTTON_X"], 220.0, "Accept");
            legend(d, &["BUTTON_TRI"], 240.0, "Cancel");
            let colour = rgb(PLAYER_COLOUR[slot]);
            if s.step == Step::NameShown {
                // The finished name blinks, centred on the column.
                if (s.t as u32) & 0x10 != 0 {
                    d.draw.text(d.fonts, &TextStyle::new(INITIALS, 0.75, colour), -(col + 64.0), 340.0, &s.name);
                }
            } else {
                let style = TextStyle::new(INITIALS, 0.9, colour);
                let mut x = NAME_LEFT[slot];
                for c in s.name.chars() {
                    d.draw.text(d.fonts, &style, x, 340.0, &c.to_string());
                    x += 18.0;
                }
                if s.name.len() < 6 {
                    let blink = if (s.t as u32) & 0x10 == 0 { Color::srgb_u8(0x40, 0x40, 0x40) } else { Color::WHITE };
                    d.draw.text(d.fonts, &TextStyle::new(INITIALS, 0.9, blink), x, 340.0, &(s.letter as char).to_string());
                    x += 18.0;
                    for _ in s.name.len() + 1..6 {
                        d.draw.text(d.fonts, &TextStyle { color: Color::WHITE, ..style }, x, 340.0, "_");
                        x += 18.0;
                    }
                }
            }
        }
        Step::Class => class_card(d, s, strings, stats, col, true),
    }
}

/// The class card: weapon art, the class in the chosen colour (a shadow
/// and a question mark for a locked class), its name plate, the four
/// attributes with the strongest glowing, and its level.
fn class_card(d: &mut Painter, s: &Select, strings: &Strings, stats: &ClassStats, col: f32, choosing: bool) {
    let class = CLASSES[s.class];
    let open = s.class < OPEN_CLASSES;
    d.image(&format!("S12_WEAP_{}", CLASSES[s.class & 7]), col, 0.0, Color::WHITE);
    if open {
        d.image(&format!("S12_{class}_{}", COLOURS[s.colour]), col, 28.0, Color::WHITE);
        d.image(&format!("{class}_NAME"), col + 8.0, 272.0, Color::WHITE);
    } else {
        d.image(&format!("S12_{class}_SHADW"), col, 28.0, Color::WHITE);
        d.image("SELSCRN_QUESTMARK", col + 64.0 - 32.0, 160.0, Color::WHITE);
    }
    let small = TextStyle::new(FONT8X8, 1.2, Color::WHITE);
    let icon = 19.0;
    // The legend while the class is being picked (a ready player's card
    // has none).
    if choosing {
        d.image_sized("BUTTON_L", 0, col + 10.0, 232.0, icon, icon, Color::WHITE);
        d.image_sized("BUTTON_R", 0, col + 29.0, 232.0, icon, icon, Color::WHITE);
        d.draw.text(d.fonts, &small, col + 56.0, 236.0, "Change");
    }
    if open {
        if choosing {
            d.image_sized("BUTTON_X", 0, col + 20.0, 252.0, icon, icon, Color::WHITE);
            d.draw.text(d.fonts, &small, col + 20.0 + icon + 8.0, 256.0, "Select");
        }
        if let Some(values) = stats.0.get(s.class).copied().flatten() {
            let best = (0..4).max_by_key(|&i| (values[i], -(i as i32))).unwrap_or(0);
            for i in 0..4 {
                let y = 162.0 + 16.0 * i as f32;
                let label = strings.get("ATTS_DESC", i).unwrap_or(["Strength", "Speed", "Armor", "Magic"][i]);
                let w = d.fonts.width(FONT32, 0.5, label);
                if i == best {
                    d.image_sized("ATT_GLOW", 0, col + 84.0 - 6.0, y - 6.0, 68.0, 32.0, Color::WHITE);
                    d.draw.shimmer(d.fonts, FONT32, 0.5, col + 81.0 - w, y - 2.0, label, rgb(PURPLE), 1.0);
                } else {
                    d.draw.text(d.fonts, &TextStyle::new(FONT32, 0.5, Color::WHITE), col + 81.0 - w, y - 2.0, label);
                }
                d.draw.text(d.fonts, &TextStyle::new(INITIALS, 0.5, Color::WHITE), col + 84.0, y, &format!("{:03}", values[i]));
            }
        }
        d.draw.text(d.fonts, &small, -(col + 64.0), 292.0, "Level 1");
    }
}

/// Each class's attributes as a new hero has them (strength, speed,
/// armour, magic: the `PDAT` start values, capped at 999), for the class
/// card.
#[derive(Resource)]
struct ClassStats(Vec<Option<[i32; 4]>>);

impl ClassStats {
    fn load(game: &mut LoadedGame) -> Self {
        Self(
            CLASSES
                .iter()
                .map(|c| {
                    let bytes = game.install.read(&format!("PDATA/{c}.WAD")).ok()?;
                    let s = PlayerStats::parse(&bytes).ok().flatten()?;
                    let v = |x: f32| (x.round() as i32).min(999);
                    Some([v(s.strength.start), v(s.speed.start), v(s.armor.start), v(s.magic.start)])
                })
                .collect(),
        )
    }
}
