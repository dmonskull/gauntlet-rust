//! The power menu (`docs/powers.md`, "Held powers"): the powers the hero
//! carries, one at a time, named over its panel. A power picked up is held
//! until the hero turns it on here; one turned off is put away with the
//! time it has left, for later.
//!
//! The D-pad (arrows on the keyboard): Up opens the menu on the power last
//! shown (or the highest slot carrying one), and while it's open turns that
//! power on or off; Left and Right step to the previous and next slot
//! carrying a power, round and round (with none left it closes); Down
//! closes it. A power that runs out while shown gives way to the previous
//! one. The
//! name sits over player 1's panel at (24, 310) in `font32` × 0.45,
//! glowing while the power is on, plain white while it's held or off;
//! opening and closing take 32 fields each. The sounds are the menus':
//! `S_OPTMENUMOVVRT` opening or closing, `S_OPTMENUMOVHRZ` stepping,
//! `S_OPTMENUSEL` turning a power on or off.

use bevy::prelude::*;
use gdl_formats::enemy::FIELDS_PER_TICK;
use gdl_formats::font::FONT32;

use crate::audio::PlaySoundAt;
use crate::combat::button;
use crate::font::{Draw2d, Flush2d, GameFonts, TextStyle};
use crate::frontend::{self, Frontend};
use crate::message_box::DrawBox;
use crate::player::{HeroPad, PlayerTick};
use crate::player_state::{PlayerState, SlotState};

pub struct PowerMenuPlugin;

impl Plugin for PowerMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PowerMenu>()
            .add_systems(FixedUpdate, run.after(PlayerTick))
            .add_systems(PostUpdate, draw.before(DrawBox).before(Flush2d));
    }
}

/// The slots the menu steps through (the record's first ten of eleven).
const SHOWN_SLOTS: usize = 10;
/// Opening and closing count up to this, 4 a field.
const FULL: f32 = 128.0;
const STEP_PER_FIELD: f32 = 4.0;
/// Where the name sits for player 1 (the panel's centre − 52, + 12) and
/// its line once open (335 + 103 − 128).
const NAME_X: f32 = 24.0;
const NAME_Y: f32 = 310.0;
const NAME_SCALE: f32 = 0.45;
const SOUND_VOLUME: u8 = 0x7F;
const DPAD: u32 = button::DPAD_LEFT | button::DPAD_RIGHT | button::DPAD_UP | button::DPAD_DOWN;
const OPEN_CLOSE: &str = "S_OPTMENUMOVVRT";
const STEP: &str = "S_OPTMENUMOVHRZ";
const TURN: &str = "S_OPTMENUSEL";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Closed,
    Opening,
    Open,
    Closing,
}

/// The menu: its state, the slot it shows (kept between openings), how
/// far opening or closing has got, and the fields it has run (the glow's
/// pulse).
#[derive(Resource, Default)]
pub struct PowerMenu {
    state: State,
    slot: Option<usize>,
    progress: f32,
    fields: f32,
}

/// The slot before `from` carrying a power, round from the top (from the
/// last slot with none shown).
fn previous(powers: &PlayerState, from: Option<usize>) -> Option<usize> {
    let start = from.map_or(SHOWN_SLOTS - 1, |s| (s + SHOWN_SLOTS - 1) % SHOWN_SLOTS);
    (0..SHOWN_SLOTS).map(|k| (start + SHOWN_SLOTS - k) % SHOWN_SLOTS).find(|&i| powers.powers[i].live())
}

/// The slot after `from` carrying a power, round from the bottom.
fn next(powers: &PlayerState, from: Option<usize>) -> Option<usize> {
    let start = from.map_or(0, |s| (s + 1) % SHOWN_SLOTS);
    (0..SHOWN_SLOTS).map(|k| (start + k) % SHOWN_SLOTS).find(|&i| powers.powers[i].live())
}

/// One tick of the menu (the game's player update runs it every frame of
/// play).
fn run(mut menu: ResMut<PowerMenu>, mut pad: ResMut<HeroPad>, mut state: ResMut<PlayerState>, mut sounds: MessageWriter<PlaySoundAt>) {
    let pressed = std::mem::take(&mut pad.pressed);
    let menu = &mut *menu;
    if pressed & DPAD != 0 {
        debug!("power menu: D-pad {:#x} while {:?}", pressed & DPAD, menu.state);
    }
    menu.fields += FIELDS_PER_TICK;
    let mut sound = |name: &str| {
        sounds.write(PlaySoundAt::centred(name, SOUND_VOLUME));
    };
    let close = |menu: &mut PowerMenu| {
        menu.state = State::Closing;
        menu.progress = 0.0;
    };
    if menu.state == State::Open {
        let shown_gone = menu.slot.is_none_or(|s| !state.powers[s].live());
        if shown_gone || pressed & button::DPAD_LEFT != 0 {
            sound(STEP);
            match previous(&state, menu.slot) {
                Some(s) => menu.slot = Some(s),
                None => close(menu),
            }
        }
        if pressed & button::DPAD_RIGHT != 0 {
            sound(STEP);
            match next(&state, menu.slot) {
                Some(s) => menu.slot = Some(s),
                None => {
                    menu.slot = None;
                    close(menu);
                }
            }
        }
        if pressed & button::DPAD_UP != 0
            && let Some(s) = menu.slot
        {
            sound(TURN);
            state.toggle_power(s);
            debug!("power menu: slot {s} {:?}", state.powers[s].state);
        }
        if pressed & button::DPAD_DOWN != 0 && menu.state == State::Open {
            sound(OPEN_CLOSE);
            close(menu);
        }
    } else if pressed & button::DPAD_UP != 0 {
        sound(OPEN_CLOSE);
        if menu.state == State::Closed {
            if menu.slot.is_none() {
                menu.slot = previous(&state, None);
            }
            if menu.slot.is_some() {
                menu.state = State::Opening;
                menu.progress = 0.0;
                debug!("power menu: open on slot {:?}", menu.slot);
            }
        }
    }
    match menu.state {
        State::Opening | State::Closing if menu.progress < FULL => {
            menu.progress = (menu.progress + FIELDS_PER_TICK * STEP_PER_FIELD).min(FULL);
        }
        State::Opening => menu.state = State::Open,
        State::Closing => menu.state = State::Closed,
        _ => {}
    }
}

/// The game's names for the powers (`docs/powers.md`): the first entry of the
/// slot's subtype whose bits it all has names it.
const NAMES: [(i32, u32, &str); 75] = [
    (9, 0x1, "Levitate"),
    (9, 0x2, "XRay"),
    (9, 0x4, "Invisible"),
    (9, 0x8, "StopTime"),
    (9, 0x10, "FireBreath"),
    (9, 0x20, "AcidBreath"),
    (9, 0x40, "LightningBreath"),
    (9, 0x80, "Phoenix"),
    (9, 0x100, "Growth"),
    (9, 0x200, "EnemyShrink"),
    (9, 0x400, "Pojo"),
    (9, 0x1000, "BossHorns"),
    (9, 0x2000, "BossMask"),
    (9, 0x4000, "BossGauntR"),
    (9, 0x8000, "BossGauntL"),
    (9, 0x10000, "Speed"),
    (9, 0x20000, "Heath"),
    (9, 0x40000, "Dummy"),
    (9, 0x80000, "Turbo"),
    (9, 0x100000, "Mikey"),
    (9, 0x200000, "HandOfDeath"),
    (9, 0x400000, "HealthVamp"),
    (6, 0x1, "Fire Shield"),
    (6, 0x2, "Elec Shield"),
    (6, 0x4, "Resist Light"),
    (6, 0x8, "Resist Acid"),
    (6, 0x10, "Resist Magic"),
    (6, 0x100, "Immune Fire"),
    (6, 0x200, "Immune Elec"),
    (6, 0x400, "Immune Light"),
    (6, 0x800, "Immune Acid"),
    (6, 0x1000, "Immune Magic"),
    (6, 0x2000, "Immune Gas"),
    (6, 0x10000, "Invulnerable"),
    (6, 0x20000, "Reflective Armor"),
    (6, 0x40000, "Knockback Armor"),
    (6, 0x80000, "AntiDeath"),
    (6, 0x100000, "Gold Invuln"),
    (6, 0x200000, "Fire Armor"),
    (6, 0x400000, "Elec Armor"),
    (6, 0x800000, "Armor Protect"),
    (6, 0x1000000, "Armor Reflect"),
    (5, 0x10, "KnockBack"),
    (5, 0x20, "KnockDown"),
    (5, 0x40, "BlownAway"),
    (5, 0x80, "Stun"),
    (5, 0x100, "KnockOver"),
    (5, 0x200, "Magic"),
    (5, 0x400, "Explode"),
    (5, 0x800, "PoisonGas"),
    (5, 0x1000, "DeathStun"),
    (5, 0x2000, "Spike"),
    (5, 0x4000, "Grabbed"),
    (5, 0x8000, "Thrown"),
    (5, 0x10000, "Whirlwind"),
    (5, 0x20000, "Arrow"),
    (5, 0x40000, "FireBall"),
    (5, 0x80000, "3Way Shot"),
    (5, 0x100000, "Super Shot"),
    (5, 0x200000, "Weapon Reflect"),
    (5, 0x400000, "5Way Shot"),
    (5, 0x800000, "Weapon Heal"),
    (5, 0x1000000, "No Hit"),
    (5, 0x2000000, "Weapon Turbo"),
    (5, 0x4000000, "Weapon Sticky"),
    (5, 0x8000000, "Weapon Sticky"),
    (5, 0x10000000, "Hammer"),
    (5, 0x20000000, "RapidFire"),
    (5, 0x40000000, "Weapon Low"),
    (5, 0x0, "Weapon"),
    (7, 0x0, "Speed"),
    (8, 0x0, "Magic"),
    (9, 0x0, "Special"),
    (6, 0x0, "Armor"),
    (0, 0x0, "Cheat"),
];

/// The game's name for a power of `subtype` with `value` bits.
fn name(subtype: i32, value: u32) -> Option<&'static str> {
    NAMES.iter().find(|&&(s, bits, _)| s == subtype && value & bits == bits).map(|&(_, _, n)| n)
}

/// Draws the shown power's name while the menu is open, over play.
fn draw(
    menu: Res<PowerMenu>,
    state: Res<PlayerState>,
    frontend: Option<Res<Frontend>>,
    fonts: Option<Res<GameFonts>>,
    mut draw: ResMut<Draw2d>,
) {
    let (State::Open, Some(slot), Some(fonts)) = (menu.state, menu.slot, fonts) else { return };
    if frontend.is_some_and(|f| !f.playing() || f.menu_open()) {
        return;
    }
    let p = &state.powers[slot];
    let Some(text) = p.live().then(|| name(p.subtype, p.value)).flatten() else { return };
    if p.state == SlotState::On {
        draw.shimmer(&fonts, FONT32, NAME_SCALE, NAME_X, NAME_Y, text, frontend::glow_colour(), frontend::pulse(menu.fields));
    } else {
        draw.text(&fonts, &TextStyle::new(FONT32, NAME_SCALE, Color::WHITE), NAME_X, NAME_Y, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_state::power;

    #[test]
    fn names_follow_the_games_table() {
        assert_eq!(name(power::SPECIAL, power::LEVITATE), Some("Levitate"));
        assert_eq!(name(5, 0x80000), Some("3Way Shot"));
        // Gold armour (0x110000) names its first entry, plain invulnerability.
        assert_eq!(name(6, 0x11_0000), Some("Invulnerable"));
        // An element weapon has no entry of its own.
        assert_eq!(name(5, 1), Some("Weapon"));
        assert_eq!(name(7, 0), Some("Speed"));
    }

    #[test]
    fn the_menu_steps_round_the_slots_carrying_powers() {
        let mut s = PlayerState::default();
        for v in [1, 2, 4] {
            s.grant_power(power::SPECIAL, v, 0.0, 30.0);
        }
        // Slots 0–2; opening starts at the highest.
        assert_eq!(previous(&s, None), Some(2));
        assert_eq!(previous(&s, Some(2)), Some(1));
        assert_eq!(previous(&s, Some(0)), Some(2));
        assert_eq!(next(&s, Some(2)), Some(0));
        assert_eq!(next(&s, Some(0)), Some(1));
    }
}
