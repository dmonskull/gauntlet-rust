//! The players in the game: up to four, each in a slot as the game keeps
//! its four player records (`docs/coop.md`). A slot's member holds the
//! hero's class choice, name and record ([`PlayerState`]), which outlive
//! levels; the hero on a level is a `Player` entity with its slot.
//!
//! Each tick every slot's controls are gathered into [`Inputs`] — from the
//! slot's own device here (the keyboard and mouse, a pad), from the
//! network online — and the hero's tick reads only its slot's. A device
//! drives at most one slot: the keyboard and mouse player 1, each pad
//! whoever it joined as.

use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::prelude::*;

use crate::player::PlayerChoice;
use crate::player_state::PlayerState;

/// The game's four player records.
pub const MAX_PLAYERS: usize = 4;

/// What drives a slot's hero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Devices {
    /// The keyboard and mouse (player 1's).
    pub keyboard: bool,
    /// A pad (the entity Bevy gave it).
    pub pad: Option<Entity>,
    /// Played from another machine (online): its controls come over the
    /// network.
    pub remote: bool,
}

/// One player in the game.
#[derive(Clone, Debug)]
pub struct Member {
    pub choice: PlayerChoice,
    pub name: String,
    pub state: PlayerState,
    pub devices: Devices,
}

/// The players, by slot.
#[derive(Resource, Clone, Debug, Default)]
pub struct Party {
    members: [Option<Member>; MAX_PLAYERS],
}

impl Party {
    pub fn get(&self, slot: usize) -> Option<&Member> {
        self.members.get(slot)?.as_ref()
    }

    pub fn get_mut(&mut self, slot: usize) -> Option<&mut Member> {
        self.members.get_mut(slot)?.as_mut()
    }

    /// A slot's record.
    pub fn state(&self, slot: usize) -> Option<&PlayerState> {
        self.get(slot).map(|m| &m.state)
    }

    pub fn state_mut(&mut self, slot: usize) -> Option<&mut PlayerState> {
        self.get_mut(slot).map(|m| &mut m.state)
    }

    pub fn choice(&self, slot: usize) -> Option<&PlayerChoice> {
        self.get(slot).map(|m| &m.choice)
    }

    /// The players in the game, with their slots, in slot order.
    pub fn members(&self) -> impl Iterator<Item = (usize, &Member)> {
        self.members.iter().enumerate().filter_map(|(i, m)| Some((i, m.as_ref()?)))
    }

    pub fn members_mut(&mut self) -> impl Iterator<Item = (usize, &mut Member)> {
        self.members.iter_mut().enumerate().filter_map(|(i, m)| Some((i, m.as_mut()?)))
    }

    /// Their records.
    pub fn states(&self) -> impl Iterator<Item = (usize, &PlayerState)> {
        self.members().map(|(i, m)| (i, &m.state))
    }

    pub fn states_mut(&mut self) -> impl Iterator<Item = (usize, &mut PlayerState)> {
        self.members_mut().map(|(i, m)| (i, &mut m.state))
    }

    /// How many are in the game.
    pub fn len(&self) -> usize {
        self.members.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Puts a player in `slot` (replacing whoever was there).
    pub fn join(&mut self, slot: usize, member: Member) {
        if let Some(m) = self.members.get_mut(slot) {
            *m = Some(member);
        }
    }

    /// Takes a player out of the game.
    pub fn leave(&mut self, slot: usize) -> Option<Member> {
        self.members.get_mut(slot)?.take()
    }

    /// The first free slot.
    pub fn free_slot(&self) -> Option<usize> {
        self.members.iter().position(Option::is_none)
    }

    /// The slot a pad drives, if any.
    pub fn slot_of_pad(&self, pad: Entity) -> Option<usize> {
        self.members().find(|(_, m)| m.devices.pad == Some(pad)).map(|(i, _)| i)
    }

    /// The slot the keyboard drives, if any.
    #[allow(dead_code)] // joining (coming next)
    pub fn slot_of_keyboard(&self) -> Option<usize> {
        self.members().find(|(_, m)| m.devices.keyboard).map(|(i, _)| i)
    }

    /// Whether any player's record says `f`: the game opens a gate, an
    /// exit, a realm when any player's progress does.
    pub fn any(&self, f: impl Fn(&PlayerState) -> bool) -> bool {
        self.states().any(|(_, s)| f(s))
    }

    /// Whether every player in the game is out (dead).
    #[allow(dead_code)] // joining (coming next)
    pub fn all_out(&self) -> bool {
        self.states().all(|(_, s)| !s.alive)
    }
}

pub struct PartyPlugin;

impl Plugin for PartyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, attach_pads);
    }
}

/// Alone, a player plays with every device: a pad nobody holds becomes
/// theirs the first time it's played with (any button but Start, or a
/// stick), so its Start then pauses their game. A pad never played with
/// asks to join with Start (`frontend.rs`).
fn attach_pads(mut party: ResMut<Party>, pads: Query<(Entity, &Gamepad)>) {
    let locals: Vec<usize> = party.members().filter(|(_, m)| !m.devices.remote).map(|(slot, _)| slot).collect();
    let [slot] = locals[..] else { return };
    for (pad, gamepad) in &pads {
        if party.slot_of_pad(pad).is_some() {
            continue;
        }
        let played = gamepad.get_pressed().any(|&b| b != GamepadButton::Start)
            || gamepad.left_stick().length() > 0.5
            || gamepad.right_stick().length() > 0.5;
        if played && let Some(m) = party.get_mut(slot) {
            m.devices.pad = Some(pad);
            info!("player {} takes up pad {pad:?}", slot + 1);
        }
    }
}

/// Pads connected that nobody plays with.
pub fn free_pads(party: &Party, pads: impl Iterator<Item = Entity>) -> usize {
    pads.filter(|&e| party.slot_of_pad(e).is_none()).count()
}

/// One player's controls for a tick: the stick in the camera's frame (+Y
/// away from the camera), the right stick, and the logical buttons held
/// (`combat::button`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlotInput {
    pub stick: Vec2,
    pub c_stick: Vec2,
    pub held: u32,
}

impl SlotInput {
    /// A player's own settings ride with their buttons, in bits the
    /// game's leave free, so every machine plays each hero by its player's
    /// settings (online, the remote player's own).
    pub const AUTO_AIM: u32 = 0x0100_0000;
    pub const AUTO_ATTACK: u32 = 0x0200_0000;
    /// The Robotron control style (its right stick turns the hero).
    pub const ROBOTRON: u32 = 0x0400_0000;
    /// The B button, held (online, the message box's).
    pub const BACK: u32 = 0x0800_0000;
    /// Online, a player's Tower Menu opens the Shop or the Inventory for
    /// everyone: the command rides with their controls for a moment.
    pub const OPEN_SHOP: u32 = 0x40;
    pub const OPEN_INVENTORY: u32 = 0x80;
    /// Online, the host's camera choice (`GameOptions::online_cameras`):
    /// every machine switches between each player's own camera and the
    /// shared co-op one on the tick it changes.
    pub const OWN_CAMERAS: u32 = 0x20;
    /// Online, the host's call for a sync point (`resync.rs`): on the
    /// first tick whose controls carry it every machine stops and they
    /// put their games together again.
    pub const SYNC: u32 = 0x02;
    /// The settings' bits.
    pub const FIRST_PERSON: u32 = 0x10;
    /// Mouse look uses a wider angular range than a controller stick.
    pub const MOUSE_LOOK: u32 = 0x08;
    pub const SETTINGS: u32 = Self::AUTO_AIM | Self::AUTO_ATTACK | Self::ROBOTRON | Self::FIRST_PERSON;
    const EXTRAS: u32 =
        Self::SETTINGS | Self::MOUSE_LOOK | Self::BACK | Self::OPEN_SHOP | Self::OPEN_INVENTORY | Self::OWN_CAMERAS | Self::SYNC;

    /// The game's buttons held, without the extras.
    pub fn buttons(&self) -> u32 {
        self.held & !Self::EXTRAS
    }

    pub fn auto_aim(&self) -> bool {
        self.held & Self::AUTO_AIM != 0
    }

    pub fn auto_attack(&self) -> bool {
        self.held & Self::AUTO_ATTACK != 0
    }

    pub fn robotron(&self) -> bool {
        self.held & Self::ROBOTRON != 0
    }

    /// These controls with a player's settings riding along.
    pub fn with_settings(mut self, o: crate::options::PlayerOptions) -> Self {
        self.held &= !(Self::SETTINGS);
        if o.auto_aim {
            self.held |= Self::AUTO_AIM;
        }
        if o.auto_attack {
            self.held |= Self::AUTO_ATTACK;
        }
        if o.first_person {
            self.held |= Self::FIRST_PERSON;
        }
        if o.scheme == crate::controls::ROBOTRON {
            self.held |= Self::ROBOTRON;
        }
        self
    }
}

/// Every slot's controls for this tick.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub slots: [SlotInput; MAX_PLAYERS],
}

/// A menu, message box or screen over play has just closed: the buttons
/// held then (the B that closed it) don't reach the heroes until they're
/// let go (`player.rs`).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct InputGate {
    pub swallow: bool,
}

/// One slot's buttons through the gate: those in `mask` (held as a menu
/// closed) are dropped while still held; let go, they count again.
pub fn gate_buttons(held: u32, mask: &mut u32) -> u32 {
    *mask &= held;
    held & !*mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(class: &str) -> Member {
        Member {
            choice: PlayerChoice { class: class.into(), variant: "BLU".into() },
            name: "LARRY".into(),
            state: PlayerState::new(class, None),
            devices: Devices { keyboard: true, ..default() },
        }
    }

    #[test]
    fn players_take_the_free_slots_in_turn() {
        let mut party = Party::default();
        assert_eq!(party.free_slot(), Some(0));
        party.join(0, member("WAR"));
        party.join(1, member("VAL"));
        assert_eq!((party.len(), party.free_slot()), (2, Some(2)));
        assert_eq!(party.choice(1).map(|c| c.class.as_str()), Some("VAL"));
        party.leave(0);
        assert_eq!(party.free_slot(), Some(0));
        assert_eq!(party.members().map(|(i, _)| i).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn a_button_held_as_a_menu_closes_waits_to_be_let_go() {
        let b = 0x100;
        // B closed the menu: masked while held.
        let mut mask = b;
        assert_eq!(gate_buttons(b, &mut mask), 0);
        assert_eq!(gate_buttons(b | 0x200, &mut mask), 0x200);
        // Let go, then pressed again: it counts.
        assert_eq!(gate_buttons(0, &mut mask), 0);
        assert_eq!(gate_buttons(b, &mut mask), b);
    }

    #[test]
    fn the_party_is_out_when_every_hero_is() {
        let mut party = Party::default();
        party.join(0, member("WAR"));
        party.join(2, member("WIZ"));
        assert!(!party.all_out());
        party.state_mut(0).unwrap().alive = false;
        assert!(!party.all_out());
        party.state_mut(2).unwrap().alive = false;
        assert!(party.all_out());
    }
}

#[cfg(test)]
mod personal_view_tests {
    use super::*;
    #[test]
    fn personal_view_bits_do_not_replace_classic_controls() {
        let raw = SlotInput { held: crate::combat::button::QUICK | crate::combat::button::STRAFE,
            stick: Vec2::new(0.3, 0.8), c_stick: Vec2::new(-0.4, 0.6) };
        let options = crate::options::PlayerOptions { scheme: crate::controls::ROBOTRON, ..default() };
        let classic = raw.with_settings(options);
        assert_eq!(classic.buttons(), raw.held);
        assert!(classic.robotron());
        assert_eq!(classic.held & (SlotInput::FIRST_PERSON | SlotInput::MOUSE_LOOK), 0);
        assert_eq!((classic.stick, classic.c_stick), (raw.stick, raw.c_stick));
        let first = raw.with_settings(crate::options::PlayerOptions { first_person: true, ..options });
        assert_ne!(first.held & SlotInput::FIRST_PERSON, 0);
        assert_eq!(first.buttons(), classic.buttons());
        assert_eq!((first.stick, first.c_stick), (classic.stick, classic.c_stick));
    }
}
