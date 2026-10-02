//! The name codes (`docs/cheats.md`): a hero named one of the game's codes
//! is a secret character, or carries powers that never run out.
//!
//! The select screen checks a new hero's name once it has blinked: one of
//! the secret characters' codes makes the hero that character — a base
//! class with its own model, in its own colour — and readies the player
//! without the class card. Every level's start checks every hero's name
//! again ([`apply`]): a gameplay code grants its powers (held, as a
//! pickup's are, until turned on from the power menu, and then never
//! running out: their time is −1), or sets the keys, potions or gold. A
//! secret character gets none of them. Two of the developers' codes grant
//! every gameplay code, a third a random handful.
//!
//! A secret character's variant is its colour's then its model's folder
//! (`BLUGEC`): everything that goes by the hero's colour reads the first
//! three letters, and [`model_folder`] gives the model.

use bevy::prelude::*;

use crate::party::Party;
use crate::player_state::{GOLD_CAP, MAX_KEYS, MAX_POTIONS, PlayerState};
use crate::population::LevelPopulation;

pub struct CheatsPlugin;

impl Plugin for CheatsPlugin {
    fn build(&self, app: &mut App) {
        // As each level starts, before the front end keeps the heroes'
        // records for it (`frontend.rs` `level_started`).
        app.add_systems(
            Update,
            level_start.run_if(resource_exists_and_changed::<LevelPopulation>).before(crate::frontend::read_input),
        );
    }
}

/// Each hero's codes as the level starts (the game runs the check as it
/// sets each hero up for the level).
fn level_start(population: Res<LevelPopulation>, mut party: ResMut<Party>) {
    let level = population.level.clone();
    for (slot, member) in party.members_mut() {
        let (name, variant) = (member.name.clone(), member.choice.variant.clone());
        let state = &mut member.state;
        let bits = random_bits(&level, slot, state.gold, state.experience);
        let n = apply(state, &name, &variant, || bits);
        if n > 0 {
            info!("player {}: {name}'s codes: {n} granted", slot + 1);
        }
    }
}

/// The random code's bits: the game draws them from its generator; here a
/// hash of what every machine has alike (online), new each level.
fn random_bits(level: &str, slot: usize, gold: u32, experience: u32) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in level.bytes().chain([slot as u8]).chain(gold.to_le_bytes()).chain(experience.to_le_bytes()) {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h
}

/// A secret character: its code, colour (0 yellow, 1 blue, 2 red,
/// 3 green), base class (0 WAR … 7 JES) and model (a folder in the
/// class's: `PLAYERS/<class>/<model>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecretCharacter {
    pub code: &'static str,
    pub colour: usize,
    pub class: usize,
    pub model: &'static str,
    /// The game takes it only with its debug level raised: never on the
    /// retail disc.
    pub debug_only: bool,
}

const fn character(code: &'static str, colour: usize, class: usize, model: &'static str) -> SecretCharacter {
    SecretCharacter { code, colour, class, model, debug_only: false }
}

/// The secret characters, in the game's order.
pub const SECRET_CHARACTERS: [SecretCharacter; 27] = [
    character("ICE600", 2, 4, "GEI"),
    character("NUD069", 1, 4, "SNM"),
    character("STX222", 1, 7, "STK"),
    character("KJH105", 3, 7, "KJH"),
    character("PNK666", 0, 7, "PNK"),
    character("BAT900", 3, 5, "GEB"),
    character("TAK118", 2, 5, "NIN"),
    character("STG333", 3, 5, "STG"),
    character("KAO292", 2, 5, "WTR"),
    character("CSS222", 0, 5, "CSS"),
    character("RIZ721", 1, 5, "RIZ"),
    character("ARV984", 1, 5, "ARV"),
    character("DIB626", 2, 5, "DIB"),
    character("SJB964", 1, 5, "SJB"),
    SecretCharacter { code: "NAK069", colour: 1, class: 1, model: "NUD", debug_only: true },
    character("TWN300", 3, 1, "GET"),
    character("AYA555", 1, 1, "SCH"),
    character("CEL721", 3, 1, "CEL"),
    character("CAS400", 1, 0, "GEC"),
    character("MTN200", 2, 0, "GEM"),
    character("RAT333", 2, 0, "RAT"),
    character("GARM99", 0, 2, "GA2"),
    character("GARM00", 3, 2, "GAM"),
    character("DES700", 0, 2, "GED"),
    character("SKY100", 3, 2, "GEP"),
    character("SUM224", 0, 2, "SUM"),
    character("DARTHC", 3, 5, "DCY"),
];

/// What a gameplay code gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grant {
    /// A power (the pickups' subtypes: 5 weapon, 6 armour, 9 special),
    /// granted for time −1.
    Power { subtype: i32, amount: f32, bits: u32 },
    /// Gold, keys and potions set to so many.
    Gold(u32),
    Keys(u32),
    Potions(usize),
}

const fn power(subtype: i32, amount: f32, bits: u32) -> Grant {
    Grant::Power { subtype, amount, bits }
}

/// The gameplay codes, in the game's order (a code may grant twice).
pub const CHEATS: [(&str, Grant); 18] = [
    // Invulnerability.
    ("INVULN", power(6, 0.0, 0x10000)),
    // The crossbow's super shots, never spent.
    ("SSHOTS", power(5, -1.0, 0x100000)),
    // The hero becomes the Pojo.
    ("EGG911", power(9, 0.0, 0x400)),
    // Levitation and the halo.
    ("1ANGEL", power(9, 0.0, 0x1)),
    ("1ANGEL", power(6, 0.0, 0x80000)),
    // Growth, and the enemies shrunk.
    ("DELTA1", power(9, 0.0, 0x300)),
    // Invisibility.
    ("000000", power(9, 0.0, 0x4)),
    // X-ray glasses.
    ("PEEKIN", power(9, 0.0, 0x2)),
    // The turbo meter kept full.
    ("PURPLE", power(9, 0.0, 0x80000)),
    // Faster actions.
    ("XSPEED", power(9, 4.0, 0x10000)),
    // Rapid fire.
    ("QCKSHT", power(5, 0.0, 0x20000000)),
    // The three-way shot.
    ("MENAGE", power(5, 0.0, 0x80000)),
    // Reflecting shots.
    ("REFLEX", power(5, 0.0, 0x200000)),
    // Nine keys and nine potions.
    ("ALLFUL", Grant::Keys(9)),
    ("ALLFUL", Grant::Potions(9)),
    // Ten thousand gold.
    ("10000K", Grant::Gold(10_000)),
    // Time stopped.
    ("NOVATO", power(9, 0.0, 0x8)),
    // The phoenix familiar.
    ("MEBERT", power(9, 0.0, 0x80)),
];

/// The developers' codes: every gameplay code, and a random handful (a bit
/// a code, in the table's order).
pub const EVERY_CHEAT: [&str; 2] = ["MNTHRX", "ARIENT"];
pub const RANDOM_CHEATS: &str = "ADMBLY";

/// The power's time: −1, so it never runs out (`player_state.rs` scales it
/// by the class's power-up time; any negative time is for good).
const FOR_GOOD: f32 = -1.0;

/// The kind a potion added by a code has (the plain potion).
const PLAIN_POTION: i32 = 0;

/// The secret character a name makes, if any (the game compares six
/// letters).
pub fn secret_character(name: &str) -> Option<&'static SecretCharacter> {
    SECRET_CHARACTERS.iter().find(|c| !c.debug_only && c.code.eq_ignore_ascii_case(name.trim()))
}

/// The colours' folders, in the game's order.
const COLOUR_FOLDERS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];

/// A variant's model folder: a secret character's own (`BLUGEC` → `GEC`),
/// else the variant itself (`BLU`, `RED40`, `GENA`).
pub fn model_folder(variant: &str) -> &str {
    match (variant.get(..3), variant.get(3..)) {
        (Some(colour), Some(model))
            if COLOUR_FOLDERS.iter().any(|c| c.eq_ignore_ascii_case(colour))
                && !model.is_empty()
                && model.bytes().all(|b| b.is_ascii_alphabetic()) =>
        {
            model
        }
        _ => variant,
    }
}

/// A secret character's variant: its colour's three letters, then its
/// model's folder.
pub fn variant(c: &SecretCharacter) -> String {
    format!("{}{}", COLOUR_FOLDERS[c.colour % 4], c.model)
}

/// What a hero named `name` gets at a level's start, in the game's order.
/// `random` gives the random code's bits (called only for it).
pub fn grants(name: &str, variant: &str, random: impl FnOnce() -> u32) -> Vec<Grant> {
    // A secret character's record has its model: no codes for it.
    if model_folder(variant) != variant {
        return Vec::new();
    }
    let name = name.trim();
    let mut every = 0u32;
    if EVERY_CHEAT.iter().any(|c| c.eq_ignore_ascii_case(name)) {
        every = u32::MAX;
    }
    if RANDOM_CHEATS.eq_ignore_ascii_case(name) {
        every = random();
    }
    CHEATS
        .iter()
        .enumerate()
        .filter(|(i, (code, _))| code.eq_ignore_ascii_case(name) || every & (1 << i) != 0)
        .map(|(_, (_, g))| *g)
        .collect()
}

/// Gives a hero what its name's codes grant (a level's start). How many
/// grants there were.
pub fn apply(state: &mut PlayerState, name: &str, variant: &str, random: impl FnOnce() -> u32) -> usize {
    let grants = grants(name, variant, random);
    for g in &grants {
        match *g {
            Grant::Power { subtype, amount, bits } => {
                state.grant_power(subtype, bits, amount, FOR_GOOD);
            }
            Grant::Gold(n) => state.gold = n.min(GOLD_CAP),
            Grant::Keys(n) => state.keys = n.min(MAX_KEYS),
            Grant::Potions(n) => state.potions.resize(n.min(MAX_POTIONS), PLAIN_POTION),
        }
    }
    grants.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_code_names_its_class_colour_and_model() {
        let c = secret_character("darthc").expect("DARTHC");
        assert_eq!((c.class, c.colour, c.model), (5, 3, "DCY"));
        assert_eq!(variant(c), "GREDCY");
        assert_eq!(model_folder("GREDCY"), "DCY");
        assert_eq!(model_folder("BLU"), "BLU");
        assert_eq!(model_folder("RED40"), "RED40");
        assert_eq!(model_folder("GENA"), "GENA");
        // NAK069 needs the game's debug level.
        assert!(secret_character("NAK069").is_none());
        assert!(secret_character("LARRY").is_none());
    }

    #[test]
    fn gameplay_codes_grant_their_powers_for_good() {
        let mut s = PlayerState::default();
        s.powerup_time = 1.0;
        assert_eq!(apply(&mut s, "1ANGEL", "BLU", || 0), 2);
        let powers: Vec<_> = s.powers.iter().filter(|p| p.live()).map(|p| (p.subtype, p.value, p.time)).collect();
        assert_eq!(powers, vec![(9, 0x1, -1.0), (6, 0x80000, -1.0)]);
        // Again at the next level: the same slots, still for good.
        apply(&mut s, "1ANGEL", "BLU", || 0);
        assert_eq!(s.powers.iter().filter(|p| p.live()).count(), 2);
    }

    #[test]
    fn allful_and_10000k_set_the_counts() {
        let mut s = PlayerState::default();
        (s.powerup_time, s.gold, s.keys) = (1.0, 50_000, 2);
        s.potions = vec![3];
        apply(&mut s, "ALLFUL", "RED", || 0);
        assert_eq!((s.keys, s.potions.len(), s.potions[0]), (9, 9, 3));
        apply(&mut s, "10000K", "RED", || 0);
        assert_eq!(s.gold, 10_000, "set, not added");
    }

    #[test]
    fn the_developers_codes_grant_every_code_or_a_random_few() {
        assert_eq!(grants("MNTHRX", "BLU", || 0).len(), CHEATS.len());
        assert_eq!(grants("ARIENT", "BLU", || 0).len(), CHEATS.len());
        // Bits 0 and 2: INVULN and EGG911.
        assert_eq!(grants("ADMBLY", "BLU", || 0b101), vec![CHEATS[0].1, CHEATS[2].1]);
        // A secret character gets none.
        assert!(grants("INVULN", "BLUGEC", || 0).is_empty());
    }
}
