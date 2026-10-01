//! The controls in play (`docs/combat.md`, "Logical buttons" and "The
//! four schemes"): what the keyboard, the mouse and the pads press.
//!
//! The pads follow the game's control styles. Each logical button is the
//! GameCube buttons its scheme names ([`SCHEMES`], the game's table), and
//! a GameCube button is the pad button in its place: A south, B west, X
//! east, Y north (an Xbox pad's A, X, B and Y), L the left trigger (and
//! bumper), R the right trigger, Z the right bumper, Start the menu
//! button. The D-pad is the power menu (`power_menu.rs`). Any action can
//! be put on another pad button in the settings; the keyboard and mouse
//! are bound per action. The bindings are saved with the options
//! (`options.rs`).

use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::prelude::*;

use crate::combat::button;
use crate::options::GameOptions;

/// Something the player can do in play, bound to keys, mouse buttons and a
/// pad button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Walk,
    Attack,
    Power,
    Turbo,
    Magic,
    Charge,
    Strafe,
    Combo,
    MenuUp,
    MenuDown,
    MenuLeft,
    MenuRight,
}

impl Action {
    pub const ALL: [Action; 16] = [
        Action::MoveUp,
        Action::MoveDown,
        Action::MoveLeft,
        Action::MoveRight,
        Action::Walk,
        Action::Attack,
        Action::Power,
        Action::Turbo,
        Action::Magic,
        Action::Charge,
        Action::Strafe,
        Action::Combo,
        Action::MenuUp,
        Action::MenuDown,
        Action::MenuLeft,
        Action::MenuRight,
    ];

    /// The pad's actions: the buttons (the left stick moves, walking is
    /// how far it's pushed).
    pub const ON_PAD: [Action; 11] = [
        Action::Attack,
        Action::Power,
        Action::Turbo,
        Action::Magic,
        Action::Charge,
        Action::Strafe,
        Action::Combo,
        Action::MenuUp,
        Action::MenuDown,
        Action::MenuLeft,
        Action::MenuRight,
    ];

    /// Its name in the settings.
    pub fn label(self) -> &'static str {
        match self {
            Action::MoveUp => "Move Up",
            Action::MoveDown => "Move Down",
            Action::MoveLeft => "Move Left",
            Action::MoveRight => "Move Right",
            Action::Walk => "Walk",
            Action::Attack => "Attack",
            Action::Power => "Power Attack",
            Action::Turbo => "Turbo / Defend",
            Action::Magic => "Magic",
            Action::Charge => "Charge",
            Action::Strafe => "Strafe",
            Action::Combo => "Combo Move",
            Action::MenuUp => "Powers Menu Up",
            Action::MenuDown => "Powers Menu Down",
            Action::MenuLeft => "Powers Menu Left",
            Action::MenuRight => "Powers Menu Right",
        }
    }

    /// Its name in the options file.
    pub fn key(self) -> &'static str {
        match self {
            Action::MoveUp => "move_up",
            Action::MoveDown => "move_down",
            Action::MoveLeft => "move_left",
            Action::MoveRight => "move_right",
            Action::Walk => "walk",
            Action::Attack => "attack",
            Action::Power => "power",
            Action::Turbo => "turbo",
            Action::Magic => "magic",
            Action::Charge => "charge",
            Action::Strafe => "strafe",
            Action::Combo => "combo",
            Action::MenuUp => "menu_up",
            Action::MenuDown => "menu_down",
            Action::MenuLeft => "menu_left",
            Action::MenuRight => "menu_right",
        }
    }

    /// The logical buttons it presses (none for moving).
    pub fn bits(self) -> u32 {
        match self {
            Action::Attack => button::QUICK,
            Action::Power => button::POWER,
            Action::Turbo => button::TURBO | button::DEFEND,
            Action::Magic => button::MAGIC,
            Action::Charge => button::CHARGE,
            Action::Strafe => button::STRAFE,
            Action::Combo => button::COMBO_MOVE,
            Action::MenuUp => button::DPAD_UP,
            Action::MenuDown => button::DPAD_DOWN,
            Action::MenuLeft => button::DPAD_LEFT,
            Action::MenuRight => button::DPAD_RIGHT,
            _ => 0,
        }
    }
}

/// A key or mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    Key(KeyCode),
    Mouse(MouseButton),
}

/// The keyboard and mouse out of the box: WASD moves (Shift walks), J, L,
/// H, U, P, O and G are the game's buttons (the mouse's left and right
/// attack too), the arrows the D-pad.
pub fn default_keys(action: Action) -> Vec<Input> {
    use Input::{Key, Mouse};
    match action {
        Action::MoveUp => vec![Key(KeyCode::KeyW)],
        Action::MoveDown => vec![Key(KeyCode::KeyS)],
        Action::MoveLeft => vec![Key(KeyCode::KeyA)],
        Action::MoveRight => vec![Key(KeyCode::KeyD)],
        Action::Walk => vec![Key(KeyCode::ShiftLeft), Key(KeyCode::ShiftRight)],
        Action::Attack => vec![Key(KeyCode::KeyJ), Mouse(MouseButton::Left)],
        Action::Power => vec![Key(KeyCode::KeyL), Mouse(MouseButton::Right)],
        Action::Turbo => vec![Key(KeyCode::KeyH)],
        Action::Magic => vec![Key(KeyCode::KeyU)],
        Action::Charge => vec![Key(KeyCode::KeyP)],
        Action::Strafe => vec![Key(KeyCode::KeyO)],
        Action::Combo => vec![Key(KeyCode::KeyG)],
        Action::MenuUp => vec![Key(KeyCode::ArrowUp)],
        Action::MenuDown => vec![Key(KeyCode::ArrowDown)],
        Action::MenuLeft => vec![Key(KeyCode::ArrowLeft)],
        Action::MenuRight => vec![Key(KeyCode::ArrowRight)],
    }
}

/// The GameCube's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gc {
    A,
    B,
    X,
    Y,
    L,
    R,
    Z,
}

/// The game's control styles' names (`docs/combat.md`); the Controls
/// menu cycles the first three.
pub const SCHEME_NAMES: [&str; 4] = ["Default", "Arcade", "Robotron", "One Handed"];
/// The styles the menu offers.
pub const MENU_SCHEMES: usize = 3;
/// The style that attacks with the right stick (the GameCube's C-stick).
pub const ROBOTRON: usize = 2;

/// Per scheme, the GameCube buttons of magic, attack, power, turbo and
/// defend, charge, strafe and the combo move (the game's table).
const SCHEMES: [[&[Gc]; 7]; 4] = [
    [&[Gc::X], &[Gc::A], &[Gc::Y], &[Gc::B], &[Gc::L], &[Gc::R], &[Gc::Z]],
    [&[Gc::X], &[Gc::A], &[Gc::B], &[Gc::Y], &[Gc::L], &[Gc::R], &[Gc::Z]],
    [&[Gc::B], &[Gc::A], &[Gc::L], &[Gc::R], &[Gc::Y, Gc::X], &[], &[Gc::Z]],
    [&[Gc::B], &[Gc::A], &[Gc::L], &[Gc::R], &[], &[], &[Gc::Z]],
];

/// Where a GameCube button sits on a modern pad.
fn gc_pad(gc: Gc) -> &'static [GamepadButton] {
    match gc {
        Gc::A => &[GamepadButton::South],
        Gc::B => &[GamepadButton::West],
        Gc::X => &[GamepadButton::East],
        Gc::Y => &[GamepadButton::North],
        Gc::L => &[GamepadButton::LeftTrigger2, GamepadButton::LeftTrigger],
        Gc::R => &[GamepadButton::RightTrigger2],
        Gc::Z => &[GamepadButton::RightTrigger],
    }
}

/// The pad buttons `action` is on under `scheme`.
pub fn scheme_pad(action: Action, scheme: usize) -> Vec<GamepadButton> {
    let row = &SCHEMES[scheme.min(SCHEMES.len() - 1)];
    let gc: &[Gc] = match action {
        Action::Magic => row[0],
        Action::Attack => row[1],
        Action::Power => row[2],
        Action::Turbo => row[3],
        Action::Charge => row[4],
        Action::Strafe => row[5],
        Action::Combo => row[6],
        Action::MenuUp => return vec![GamepadButton::DPadUp],
        Action::MenuDown => return vec![GamepadButton::DPadDown],
        Action::MenuLeft => return vec![GamepadButton::DPadLeft],
        Action::MenuRight => return vec![GamepadButton::DPadRight],
        _ => return Vec::new(),
    };
    gc.iter().flat_map(|&g| gc_pad(g).iter().copied()).collect()
}

/// The player's bindings: the keys and mouse buttons of every action, and
/// the pad buttons of those moved off their style's.
#[derive(Clone, Debug, PartialEq)]
pub struct Bindings {
    pub keys: Vec<(Action, Vec<Input>)>,
    pub pad: Vec<(Action, GamepadButton)>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self { keys: Action::ALL.iter().map(|&a| (a, default_keys(a))).collect(), pad: Vec::new() }
    }
}

impl Bindings {
    pub fn keys(&self, action: Action) -> &[Input] {
        self.keys.iter().find(|(a, _)| *a == action).map_or(&[], |(_, k)| k.as_slice())
    }

    /// Puts `input` on `action` alone (and off any other action).
    pub fn bind_key(&mut self, action: Action, input: Input) {
        for (a, keys) in &mut self.keys {
            keys.retain(|k| *k != input);
            if *a == action {
                *keys = vec![input];
            }
        }
    }

    /// The pad buttons `action` is on: the player's own, else the style's
    /// (less any the player has put on another action).
    pub fn pad(&self, action: Action, scheme: usize) -> Vec<GamepadButton> {
        match self.pad.iter().find(|(a, _)| *a == action) {
            Some(&(_, b)) => vec![b],
            None => scheme_pad(action, scheme)
                .into_iter()
                .filter(|b| !self.pad.iter().any(|(a, x)| *a != action && x == b))
                .collect(),
        }
    }

    /// Puts `action` on pad button `b` (taking it off any other action the
    /// player had put there).
    pub fn bind_pad(&mut self, action: Action, b: GamepadButton) {
        self.pad.retain(|(a, x)| *a != action && *x != b);
        self.pad.push((action, b));
    }
}

/// The logical buttons the keyboard and mouse hold, as bound.
pub fn held_keys(keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>, options: &GameOptions) -> u32 {
    let b = &options.bindings;
    Action::ALL
        .into_iter()
        .filter(|a| {
            b.keys(*a).iter().any(|i| match *i {
                Input::Key(k) => keys.pressed(k),
                Input::Mouse(m) => mouse.pressed(m),
            })
        })
        .fold(0, |held, a| held | a.bits())
}

/// The logical buttons a pad holds, by the style `scheme` and the pad
/// bindings.
pub fn held_pad(pad: &Gamepad, scheme: usize, options: &GameOptions) -> u32 {
    let b = &options.bindings;
    Action::ALL
        .into_iter()
        .filter(|a| a.bits() != 0 && b.pad(*a, scheme).iter().any(|&btn| pad.pressed(btn)))
        .fold(0, |held, a| held | a.bits())
}

/// The movement keys as a stick (running; Walk halves it).
pub fn stick_keys(keys: &ButtonInput<KeyCode>, options: &GameOptions) -> Vec2 {
    let b = &options.bindings;
    let down = |a: Action| {
        b.keys(a).iter().any(|i| match *i {
            Input::Key(k) => keys.pressed(k),
            Input::Mouse(_) => false,
        })
    };
    let axis = |pos: Action, neg: Action| (down(pos) as i32 - down(neg) as i32) as f32;
    let v = Vec2::new(axis(Action::MoveRight, Action::MoveLeft), axis(Action::MoveUp, Action::MoveDown)).normalize_or_zero();
    if down(Action::Walk) { v * 0.5 } else { v }
}

/// A pad's left and right sticks (the right one the GameCube's C-stick,
/// which the Robotron style attacks with), at most 1 long.
pub fn pad_sticks(pad: &Gamepad) -> (Vec2, Vec2) {
    (pad.left_stick().clamp_length_max(1.0), pad.right_stick().clamp_length_max(1.0))
}

/// Keys the settings name and save (`KeyJ` ↔ "J").
const KEY_NAMES: &[(KeyCode, &str)] = &[
    (KeyCode::KeyA, "A"),
    (KeyCode::KeyB, "B"),
    (KeyCode::KeyC, "C"),
    (KeyCode::KeyD, "D"),
    (KeyCode::KeyE, "E"),
    (KeyCode::KeyF, "F"),
    (KeyCode::KeyG, "G"),
    (KeyCode::KeyH, "H"),
    (KeyCode::KeyI, "I"),
    (KeyCode::KeyJ, "J"),
    (KeyCode::KeyK, "K"),
    (KeyCode::KeyL, "L"),
    (KeyCode::KeyM, "M"),
    (KeyCode::KeyN, "N"),
    (KeyCode::KeyO, "O"),
    (KeyCode::KeyP, "P"),
    (KeyCode::KeyQ, "Q"),
    (KeyCode::KeyR, "R"),
    (KeyCode::KeyS, "S"),
    (KeyCode::KeyT, "T"),
    (KeyCode::KeyU, "U"),
    (KeyCode::KeyV, "V"),
    (KeyCode::KeyW, "W"),
    (KeyCode::KeyX, "X"),
    (KeyCode::KeyY, "Y"),
    (KeyCode::KeyZ, "Z"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit1, "1"),
    (KeyCode::Digit2, "2"),
    (KeyCode::Digit3, "3"),
    (KeyCode::Digit4, "4"),
    (KeyCode::Digit5, "5"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::ArrowUp, "Up Arrow"),
    (KeyCode::ArrowDown, "Down Arrow"),
    (KeyCode::ArrowLeft, "Left Arrow"),
    (KeyCode::ArrowRight, "Right Arrow"),
    (KeyCode::Space, "Space"),
    (KeyCode::Tab, "Tab"),
    (KeyCode::ShiftLeft, "Left Shift"),
    (KeyCode::ShiftRight, "Right Shift"),
    (KeyCode::ControlLeft, "Left Ctrl"),
    (KeyCode::ControlRight, "Right Ctrl"),
    (KeyCode::AltLeft, "Left Alt"),
    (KeyCode::AltRight, "Right Alt"),
    (KeyCode::SuperLeft, "Left Cmd"),
    (KeyCode::SuperRight, "Right Cmd"),
    (KeyCode::CapsLock, "Caps Lock"),
    (KeyCode::Backquote, "`"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::BracketLeft, "["),
    (KeyCode::BracketRight, "]"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::Semicolon, ";"),
    (KeyCode::Quote, "'"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
    (KeyCode::Numpad0, "Num 0"),
    (KeyCode::Numpad1, "Num 1"),
    (KeyCode::Numpad2, "Num 2"),
    (KeyCode::Numpad3, "Num 3"),
    (KeyCode::Numpad4, "Num 4"),
    (KeyCode::Numpad5, "Num 5"),
    (KeyCode::Numpad6, "Num 6"),
    (KeyCode::Numpad7, "Num 7"),
    (KeyCode::Numpad8, "Num 8"),
    (KeyCode::Numpad9, "Num 9"),
    (KeyCode::F1, "F1"),
    (KeyCode::F2, "F2"),
    (KeyCode::F3, "F3"),
    (KeyCode::F4, "F4"),
    (KeyCode::F5, "F5"),
    (KeyCode::F6, "F6"),
    (KeyCode::F7, "F7"),
    (KeyCode::F8, "F8"),
    (KeyCode::F9, "F9"),
    (KeyCode::F10, "F10"),
    (KeyCode::F11, "F11"),
    (KeyCode::F12, "F12"),
];

const MOUSE_NAMES: &[(MouseButton, &str)] = &[
    (MouseButton::Left, "Mouse Left"),
    (MouseButton::Right, "Mouse Right"),
    (MouseButton::Middle, "Mouse Middle"),
    (MouseButton::Back, "Mouse Back"),
    (MouseButton::Forward, "Mouse Forward"),
];

/// Pad buttons by their Xbox names.
const PAD_NAMES: &[(GamepadButton, &str)] = &[
    (GamepadButton::South, "A"),
    (GamepadButton::East, "B"),
    (GamepadButton::West, "X"),
    (GamepadButton::North, "Y"),
    (GamepadButton::LeftTrigger, "LB"),
    (GamepadButton::RightTrigger, "RB"),
    (GamepadButton::LeftTrigger2, "LT"),
    (GamepadButton::RightTrigger2, "RT"),
    (GamepadButton::Select, "View"),
    (GamepadButton::LeftThumb, "LS"),
    (GamepadButton::RightThumb, "RS"),
    (GamepadButton::DPadUp, "DPad Up"),
    (GamepadButton::DPadDown, "DPad Down"),
    (GamepadButton::DPadLeft, "DPad Left"),
    (GamepadButton::DPadRight, "DPad Right"),
];

/// A key's or mouse button's name.
pub fn input_name(input: Input) -> &'static str {
    match input {
        Input::Key(k) => KEY_NAMES.iter().find(|(c, _)| *c == k).map_or("?", |(_, n)| n),
        Input::Mouse(m) => MOUSE_NAMES.iter().find(|(b, _)| *b == m).map_or("?", |(_, n)| n),
    }
}

/// The key or mouse button a name stands for.
pub fn input_from_name(name: &str) -> Option<Input> {
    KEY_NAMES
        .iter()
        .find(|(_, n)| *n == name)
        .map(|&(k, _)| Input::Key(k))
        .or_else(|| MOUSE_NAMES.iter().find(|(_, n)| *n == name).map(|&(m, _)| Input::Mouse(m)))
}

/// A pad button's name (Xbox), and back.
pub fn pad_name(b: GamepadButton) -> &'static str {
    PAD_NAMES.iter().find(|(x, _)| *x == b).map_or("?", |(_, n)| n)
}

pub fn pad_from_name(name: &str) -> Option<GamepadButton> {
    PAD_NAMES.iter().find(|(_, n)| *n == name).map(|&(b, _)| b)
}

/// A key or mouse button just pressed that can be bound (the settings'
/// "press a key"); `Escape` isn't one.
pub fn just_pressed_input(keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> Option<Input> {
    KEY_NAMES
        .iter()
        .find(|(k, _)| keys.just_pressed(*k))
        .map(|&(k, _)| Input::Key(k))
        .or_else(|| MOUSE_NAMES.iter().find(|(m, _)| mouse.just_pressed(*m)).map(|&(m, _)| Input::Mouse(m)))
}

/// A pad button just pressed that can be bound (not Start, which backs
/// out).
pub fn just_pressed_pad(pads: &Query<&Gamepad>) -> Option<GamepadButton> {
    pads.iter().find_map(|p| PAD_NAMES.iter().find(|(b, _)| p.just_pressed(*b)).map(|&(b, _)| b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_styles_follow_the_games_table() {
        // Default: attack A, power Y, turbo B (an Xbox pad's X), magic X
        // (its B), charge L, strafe R, combo Z.
        assert_eq!(scheme_pad(Action::Attack, 0), vec![GamepadButton::South]);
        assert_eq!(scheme_pad(Action::Turbo, 0), vec![GamepadButton::West]);
        assert_eq!(scheme_pad(Action::Magic, 0), vec![GamepadButton::East]);
        assert_eq!(scheme_pad(Action::Power, 0), vec![GamepadButton::North]);
        assert_eq!(scheme_pad(Action::Charge, 0), vec![GamepadButton::LeftTrigger2, GamepadButton::LeftTrigger]);
        // Arcade swaps power and turbo; Robotron has no strafe and charges
        // on Y or X.
        assert_eq!(scheme_pad(Action::Power, 1), vec![GamepadButton::West]);
        assert!(scheme_pad(Action::Strafe, ROBOTRON).is_empty());
        assert_eq!(scheme_pad(Action::Charge, ROBOTRON), vec![GamepadButton::North, GamepadButton::East]);
    }

    #[test]
    fn rebinding_moves_a_key_or_button_off_its_old_action() {
        let mut b = Bindings::default();
        b.bind_key(Action::Magic, Input::Key(KeyCode::KeyJ));
        assert_eq!(b.keys(Action::Magic), &[Input::Key(KeyCode::KeyJ)]);
        assert_eq!(b.keys(Action::Attack), &[Input::Mouse(MouseButton::Left)]);
        b.bind_pad(Action::Combo, GamepadButton::LeftTrigger);
        assert_eq!(b.pad(Action::Combo, 0), vec![GamepadButton::LeftTrigger]);
        assert_eq!(b.pad(Action::Attack, 0), vec![GamepadButton::South]);
        // The bumper leaves charge (which the style puts on L and the bumper).
        assert_eq!(b.pad(Action::Charge, 0), vec![GamepadButton::LeftTrigger2]);
        // Names go both ways.
        for (k, _) in KEY_NAMES {
            assert_eq!(input_from_name(input_name(Input::Key(*k))), Some(Input::Key(*k)));
        }
        assert_eq!(pad_from_name(pad_name(GamepadButton::RightTrigger2)), Some(GamepadButton::RightTrigger2));
    }
}
