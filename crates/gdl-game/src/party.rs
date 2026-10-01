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
    #[allow(dead_code)] // joining (coming next)
    pub fn leave(&mut self, slot: usize) -> Option<Member> {
        self.members.get_mut(slot)?.take()
    }

    /// The first free slot.
    #[allow(dead_code)] // joining (coming next)
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

/// One player's controls for a tick: the stick in the camera's frame (+Y
/// away from the camera), the right stick, and the logical buttons held
/// (`combat::button`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlotInput {
    pub stick: Vec2,
    pub c_stick: Vec2,
    pub held: u32,
}

/// Every slot's controls for this tick.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub slots: [SlotInput; MAX_PLAYERS],
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
