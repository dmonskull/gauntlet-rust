//! The hero's actions and how one leads to the next: the game's player
//! action table, each action's category and movement/turn factors, and its
//! action state machine (which action follows the playing one, given the one
//! the controls ask for). Pure logic, stepped at the 30 Hz tick by
//! `player.rs`. See `docs/player-movement.md` ("Action chaining") and
//! `docs/combat.md`.

/// A player action: an index into the game's action table ([`NAMES`]).
/// Several indices share a clip name; the state machine tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Action(pub u8);

/// The game's player action names, by action index. The clip an action plays
/// is the class's clip of the same name.
pub const NAMES: [&str; 149] = [
    "READY", "IDLE1", "IDLE2", "IDLE2_LOOP", "DEFENDLEFT", "DEFENDRIGHT", "DEFENDBACK", "DEFENDBACK",
    "SHOVE", "STRAFE_WLKF1", "STRAFE_WLKF2", "STRAFE_WLKB1", "STRAFE_WLKB2", "STRAFE_WLKL1", "STRAFE_WLKL2",
    "STRAFE_WLKR1", "STRAFE_WLKR2", "WALK1", "WALK2", "RUN1", "RUN2", "SHIELD_READY", "SHIELD_RUN", "PIVOTL",
    "PIVOTR", "PUSH", "PUSHED", "HITREACT", "PICK", "DEATHGRABS", "DEATHGRAB", "DEATHGRABR", "ATTSTART",
    "ATTSLOW1", "ATTSLOW1R", "ATTPWRACLOSE", "ATTPWRACLOSER", "ATTPWRAMED", "ATTPWRAMEDR", "ATTQUICK1",
    "ATTQUICK2", "ATTQUICK3", "ATTQUICK2R", "ATTQUICK3R", "ATTQ3RIGHT", "ATTQ2RIGHT", "ATTQ3RIGHTR",
    "ATTQ2RIGHTR", "ATTQ3LEFT", "ATTQ2LEFT", "ATTQ3LEFTR", "ATTQ2LEFTR", "ATTQ2180", "ATTQ3180", "ATTQ2180R",
    "ATTQ3180R", "ATTQ2180L", "ATTQ3180L", "ATTQ2180LR", "ATTQ3180LR", "ATT360", "ATT360R", "ATTSTEP1",
    "ATTSTEP2", "ATTSTEP3", "ATTSTEP2R", "ATTSTEP3R", "ATTQ3TOSTEP1", "ATTQ3TOSTEP1R", "ATTWALK2", "ATTWALK2R",
    "STRAFE_ATKF1", "STRAFE_ATKF2", "STRAFE_ATKB1", "STRAFE_ATKB2", "STRAFE_ATKL1", "STRAFE_ATKL2",
    "STRAFE_ATKR1", "STRAFE_ATKR2", "ATTLOW1", "ATTLOW2", "ATTLOWR", "ATTLOWK", "ATTLOWKR", "ATTPWRALOW",
    "ATTPWRALOWR", "ATTPWRB", "ATTPWRC", "COMBOACT1", "COMBOACT2", "COMBOACT3", "THROW1S", "THROW1S", "THROW2S",
    "THROW2S", "THROW1", "THROW2", "THROW1R", "THROW2R", "ATTPWRATHROW", "ATTPWRATHROWR", "STRAFE_ATKF1",
    "STRAFE_ATKF2", "ATTFIREL", "ATTFIRELR", "ATTFIRER", "ATTFIRERR", "SSHOT1", "SSHOT2", "SSHOTR",
    "ATTBREATHE", "ATTBREATHER", "ATTCHOP", "ATTCHOPR", "ATTACK1", "MAGICS", "MAGICR", "THROWPOTIONS",
    "THROWPOTIONR", "DEFEND1", "DEFEND2", "DEFENDR", "STUN2", "VICTORY", "START", "INIT", "DEATH", "STUN1",
    "WEBREACT", "HITREACT", "HITREACT", "FALLDOWN", "GETUP", "FALLFRNT", "GETUP2", "FLYUP", "COMBOWAR1",
    "COMBOWAR2", "COMBOWAR3", "COMBOVAL", "COMBOWIZ", "COMBOARC", "COMBODWF1", "COMBODWF2", "COMBODWF3",
    "COMBOKNI", "COMBOSOR", "COMBOJES", "GRABBED",
];

/// Named actions (not all of them are reached yet).
#[allow(dead_code)]
impl Action {
    pub const READY: Self = Self(0x00);
    pub const IDLE1: Self = Self(0x01);
    pub const IDLE2: Self = Self(0x02);
    pub const IDLE2_LOOP: Self = Self(0x03);
    pub const SHOVE: Self = Self(0x08);
    /// Standing and running behind a shield.
    pub const SHIELD_READY: Self = Self(0x15);
    pub const SHIELD_RUN: Self = Self(0x16);
    pub const STRAFE_WLKF1: Self = Self(0x09);
    pub const STRAFE_WLKF2: Self = Self(0x0A);
    pub const STRAFE_WLKB1: Self = Self(0x0B);
    pub const STRAFE_WLKB2: Self = Self(0x0C);
    pub const STRAFE_WLKL1: Self = Self(0x0D);
    pub const STRAFE_WLKL2: Self = Self(0x0E);
    pub const STRAFE_WLKR1: Self = Self(0x0F);
    pub const STRAFE_WLKR2: Self = Self(0x10);
    pub const WALK1: Self = Self(0x11);
    pub const WALK2: Self = Self(0x12);
    pub const RUN1: Self = Self(0x13);
    pub const RUN2: Self = Self(0x14);
    pub const HITREACT: Self = Self(0x1B);
    pub const ATTSTART: Self = Self(0x20);
    pub const ATTSLOW1: Self = Self(0x21);
    pub const ATTSLOW1R: Self = Self(0x22);
    pub const ATTPWRACLOSE: Self = Self(0x23);
    pub const ATTPWRACLOSER: Self = Self(0x24);
    pub const ATTPWRAMED: Self = Self(0x25);
    pub const ATTPWRAMEDR: Self = Self(0x26);
    pub const ATTQUICK1: Self = Self(0x27);
    pub const ATTQUICK2: Self = Self(0x28);
    pub const ATTQUICK3: Self = Self(0x29);
    pub const ATTQUICK2R: Self = Self(0x2A);
    pub const ATTQUICK3R: Self = Self(0x2B);
    pub const ATTQ3RIGHT: Self = Self(0x2C);
    pub const ATTQ2RIGHT: Self = Self(0x2D);
    pub const ATTQ3LEFT: Self = Self(0x30);
    pub const ATTQ2LEFT: Self = Self(0x31);
    pub const ATTQ2180: Self = Self(0x34);
    pub const ATTQ3180: Self = Self(0x35);
    pub const ATTQ2180L: Self = Self(0x38);
    pub const ATTQ3180L: Self = Self(0x39);
    pub const ATT360: Self = Self(0x3C);
    pub const ATT360R: Self = Self(0x3D);
    pub const ATTSTEP1: Self = Self(0x3E);
    pub const ATTSTEP2: Self = Self(0x3F);
    pub const ATTSTEP3: Self = Self(0x40);
    pub const ATTSTEP2R: Self = Self(0x41);
    pub const ATTSTEP3R: Self = Self(0x42);
    pub const ATTQ3TOSTEP1: Self = Self(0x43);
    pub const ATTWALK2: Self = Self(0x45);
    pub const STRAFE_ATKF1: Self = Self(0x47);
    pub const STRAFE_ATKF2: Self = Self(0x48);
    pub const STRAFE_ATKB1: Self = Self(0x49);
    pub const STRAFE_ATKB2: Self = Self(0x4A);
    pub const STRAFE_ATKL1: Self = Self(0x4B);
    pub const STRAFE_ATKL2: Self = Self(0x4C);
    pub const STRAFE_ATKR1: Self = Self(0x4D);
    pub const STRAFE_ATKR2: Self = Self(0x4E);
    pub const ATTLOW1: Self = Self(0x4F);
    pub const ATTLOW2: Self = Self(0x50);
    pub const ATTLOWR: Self = Self(0x51);
    pub const ATTLOWK: Self = Self(0x52);
    pub const ATTLOWKR: Self = Self(0x53);
    pub const ATTPWRALOW: Self = Self(0x54);
    pub const ATTPWRALOWR: Self = Self(0x55);
    /// The throw start picked by the controls (0x5B, the same clip, is
    /// never requested).
    pub const THROW1S: Self = Self(0x5C);
    pub const THROW2S_FROM_STRIDE: Self = Self(0x5D);
    pub const THROW2S: Self = Self(0x5E);
    pub const THROW1: Self = Self(0x5F);
    pub const THROW2: Self = Self(0x60);
    pub const THROW1R: Self = Self(0x61);
    pub const THROW2R: Self = Self(0x62);
    pub const ATTPWRATHROW: Self = Self(0x63);
    pub const ATTPWRATHROWR: Self = Self(0x64);
    /// Skorne's gauntlets: the left's shot and recovery are ATTFIREL and
    /// ATTFIRER, the right's ATTFIRELR and ATTFIRERR (the table's names
    /// run in that order).
    pub const ATTFIREL: Self = Self(0x67);
    pub const ATTFIRELR: Self = Self(0x68);
    pub const ATTFIRER: Self = Self(0x69);
    pub const ATTFIRERR: Self = Self(0x6A);
    /// The super crossbow: SSHOT1, SSHOT2 over and over while the attack
    /// is held, then SSHOTR.
    pub const SSHOT1: Self = Self(0x6B);
    pub const SSHOT2: Self = Self(0x6C);
    pub const SSHOTR: Self = Self(0x6D);
    /// A breath (or Skorne's horns or mask) and its recovery; the
    /// hammer's chop and its recovery.
    pub const ATTBREATHE: Self = Self(0x6E);
    pub const ATTBREATHER: Self = Self(0x6F);
    pub const ATTCHOP: Self = Self(0x70);
    pub const ATTCHOPR: Self = Self(0x71);
    /// Grabbing Death to drain it (the halo).
    pub const DEATHGRABS: Self = Self(0x1D);
    pub const MAGICS: Self = Self(0x73);
    pub const MAGICR: Self = Self(0x74);
    pub const THROWPOTIONS: Self = Self(0x75);
    pub const THROWPOTIONR: Self = Self(0x76);
    pub const DEFEND1: Self = Self(0x77);
    pub const DEFEND2: Self = Self(0x78);
    pub const DEFENDR: Self = Self(0x79);
    /// Standing while a blow's stun lasts (a looping clip).
    pub const STUN2: Self = Self(0x7A);
    /// A stunning blow (kind `0x80`: the damage tiles).
    pub const STUN1: Self = Self(0x7F);
    pub const WEBREACT: Self = Self(0x80);
    /// Held by a critter's grab (looping; `docs/critters.md`, "7 — grab").
    pub const GRABBED: Self = Self(0x94);
    /// A blow of kind `0x2000` (its clip is HITREACT, as 0x1B's and
    /// 0x82's are).
    pub const STUNREACT: Self = Self(0x81);
}

impl Action {
    pub fn name(self) -> &'static str {
        NAMES.get(self.0 as usize).copied().unwrap_or("READY")
    }

    /// The game's action category (`docs/player-movement.md`).
    pub fn category(self) -> Category {
        let a = self.0;
        Category(match a {
            0x20..=0x26 => 2,
            0x27..=0x2B => 3,
            0x2C..=0x3B => 4,
            0x3C..=0x3D => 5,
            0x3E..=0x46 => 6,
            0x47..=0x4E => 7,
            0x4F..=0x55 => 8,
            0x56..=0x57 => 11,
            0x58..=0x5A => 12,
            0x5B..=0x62 => 9,
            0x63..=0x6A => 10,
            0x6B..=0x71 => 11,
            0x88..=0x93 => 12,
            0x77..=0x79 | 4..=8 => 1,
            _ => 0,
        })
    }
}

/// 0 locomotion and misc, 1 defend/shove, 2 slow and power attacks, 3 the
/// quick combo, 4 directional quick attacks, 5 the 360, 6 lunges, 7 strafe
/// attacks, 8 low attacks, 9 throws, 10 fire/power throw, 11 other attacks,
/// 12 combos.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Category(pub u8);

/// Player class indices the game special-cases (its class order: WAR VAL
/// WIZ ARC DWF KNI SOR JES …).
pub mod class {
    pub const WIZ: usize = 2;
    pub const ARC: usize = 3;
    pub const KNI: usize = 5;
    pub const SOR: usize = 6;
    pub const JES: usize = 7;
}

/// Movement and turn factors the state machine gives the action playing
/// when it runs: the movement one scales the next tick's step, the turn one
/// this tick's turning. `class` is the game's class index.
pub fn factors(action: Action, class: usize) -> (f32, f32) {
    const Z: f32 = 0.0;
    const Q: f32 = 0.25;
    const H: f32 = 0.5;
    const S: f32 = 0.667;
    let a = action.0;
    match a {
        0x20..=0x22 => (Z, 1.0),
        0x23 | 0x24 | 0x26 => match class {
            class::KNI | class::SOR => (Z, Z),
            class::WIZ => (Q, 1.0),
            _ => (H, 1.0),
        },
        0x25 => match class {
            class::SOR | class::JES => (Z, Z),
            class::WIZ | class::ARC => (Q, 1.0),
            _ => (H, 1.0),
        },
        0x27..=0x2B => (Q, Z),
        0x2C..=0x3B => (1.0, 1.0),
        0x3C..=0x3D => (H, 1.0),
        0x3E..=0x46 => (1.0, Q),
        0x47..=0x4E => (S, 1.0),
        0x4F..=0x53 => (1.0, 1.0),
        0x54..=0x55 => (Q, 1.0),
        0x56 => (Z, 1.0),
        // ATTPWRC turns at a quarter, except the Sorceress's after frame 11.
        0x57 => (Z, Q),
        0x58..=0x5A => (Z, Z),
        0x5B..=0x62 => (Z, H),
        0x63..=0x64 => (Q, 1.0),
        0x65..=0x66 => (1.0, 1.0),
        0x67..=0x6A => (Q, 1.0),
        0x6B..=0x71 => (Z, H),
        0x80 => (0.4, H),
        0x13 | 0x14 | 0x16 => (1.3, 1.0),
        0x8F => (1.5, H),
        0x08 => (1.5, 1.0),
        0x09..=0x10 => (S, 1.0),
        _ => (1.0, 1.0),
    }
}

/// How much of the stick the action lets through: none during magic,
/// defending and the other actions from `MAGICS` on (except the victory
/// pose, web and one combo).
pub fn stick_scale(action: Action) -> f32 {
    match action.0 {
        0x7B | 0x80 | 0x8F => 1.0,
        a if a >= 0x73 && !(0x20..=0x71).contains(&a) => 0.0,
        _ => 1.0,
    }
}

/// Lunges and strafe attacks carry the hero forward at half the stick even
/// with the stick released.
pub fn drifts_forward(action: Action) -> bool {
    (0x3E..=0x4E).contains(&action.0)
}

/// When the next action takes over from the playing one (the game's
/// transition modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    /// Right away if it differs, or once the playing clip has ended (2).
    Now,
    /// Once the playing clip has ended, if it differs (0).
    AtEnd,
    /// Once the playing clip has ended, even if it's the same (1).
    AtEndAlways,
    /// Right away, even if it's the same (3).
    Always,
}

impl Switch {
    /// `different`: the next action plays another clip; `ended`: the
    /// playing clip has ended.
    pub fn applies(self, different: bool, ended: bool) -> bool {
        match self {
            Switch::Now => different || ended,
            Switch::AtEnd => different && ended,
            Switch::AtEndAlways => ended,
            Switch::Always => true,
        }
    }
}

/// Seconds the game blends back into READY.
pub const READY_BLEND: f32 = 0.066_667;
/// A recovery can go back into the combo up to this frame.
pub const RECOVERY_REENTRY_FRAME: f32 = 2.0;
/// A throw start hands over to the throw right away once past this frame.
pub const THROW_START_FRAMES: f32 = 2.0;

/// Logical attack buttons, as the controls report them.
pub mod button {
    pub const QUICK: u32 = 0x200;
    pub const POWER: u32 = 0x400;
    pub const ATTACKS: u32 = QUICK | POWER;
}

/// How far the target the controls aimed at is (the game recomputes these
/// every tick from the target search).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range(pub u32);

impl Range {
    /// Within 1 unit of the hero's reach (radius + 1 more while attacking).
    pub const CLOSE: u32 = 1;
    /// A low target (a short monster, a low generator) within reach: the
    /// low attacks and kick.
    pub const LOW: u32 = 2;
    /// Within 2 units of reach: close enough to lunge at.
    pub const MEDIUM: u32 = 4;
    /// Further, or nothing there.
    pub const FAR: u32 = 8;
    /// The target is a monster or level object.
    pub const MONSTER_OR_OBJECT: u32 = 0x10;
    /// The target is a generator or breakable.
    pub const GENERATOR: u32 = 0x20;

    pub fn has(self, bits: u32) -> bool {
        self.0 & bits != 0
    }
}

/// The per-hero state the chaining reads and keeps.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActionState {
    /// The action playing.
    pub action: Action,
    /// Attack buttons latched while any attack button is held.
    pub latched: u32,
    /// Attack buttons pressed since the last attack started.
    pub edges: u32,
    /// Attacks chained with a fresh press since the combo began.
    pub combo: u32,
    /// Range flags from this tick's target search.
    pub range: Range,
    /// Angle from the hero's facing to the target (or to where the stick
    /// points), radians, wrapped.
    pub target_angle: f32,
}

/// What the chaining needs to know besides the state.
#[derive(Clone, Copy, Debug)]
pub struct Env {
    /// Frame of the playing clip.
    pub frame: f32,
    /// The game's class index, if known.
    pub class: Option<usize>,
    /// The class has an ATTLOW2 clip.
    pub has_low2: bool,
    /// Magic was let go during MAGICS (the player's control flag 4): it
    /// becomes a blast rather than the potion throw.
    pub magic_released: bool,
    /// The playing clip has ended or come round since it started (the
    /// game's animation status).
    pub came_round: bool,
}

/// STUN2 can't be cut short in the first frames of its first pass.
const STUN_HOLD_FRAMES: f32 = 10.0;

/// The action to go to next and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Next {
    pub action: Action,
    pub switch: Switch,
    /// Seconds to blend from the old pose.
    pub blend: f32,
    /// The playing clip starts over at its end, which counts as a new
    /// start (the game's loop flag, set by the chooser: SSHOT2 while the
    /// crossbow's shot is asked for).
    pub again: bool,
}

impl ActionState {
    /// Records the attack buttons held this tick: presses of a button not
    /// yet latched become edges; releasing both clears the latch.
    pub fn observe_buttons(&mut self, held: u32) {
        let attack = held & button::ATTACKS;
        if attack == 0 {
            self.latched = 0;
        } else {
            self.edges |= held ^ self.latched;
            self.latched |= held;
        }
    }

    fn power_finisher(&self) -> Option<Action> {
        if self.edges & button::POWER == 0 || self.combo == 0 {
            return None;
        }
        Some(match self.combo {
            1 => Action::ATTPWRACLOSE,
            2 => Action::ATTPWRAMED,
            _ => Action::ATT360,
        })
    }

    /// The game's action state machine: which action follows the playing
    /// one given the requested one, and how it takes over. Also resets the
    /// combo count when the playing action isn't part of a combo.
    pub fn next(&mut self, requested: Action, env: &Env) -> Next {
        let (cur, req) = (self.action, requested);
        let req_cat = req.category().0;
        let mut state = cur;
        let mut switch = Switch::AtEnd;
        if req.0 > 0x72 && (1..11).contains(&cur.category().0) {
            state = Action::READY;
        }
        if (0x1D..0x20).contains(&req.0) {
            switch = Switch::Now;
        }
        // Knock-downs (1) and being grabbed (2) — never requested yet.
        let knocked: u8 = if (0x83..0x95).contains(&req.0) {
            switch = Switch::Now;
            if req.0 == 0x94 { 2 } else { 1 }
        } else {
            0
        };
        let state_cat = state.category().0;
        if (1..11).contains(&state_cat) && req_cat > 10 {
            state = Action::READY;
            switch = Switch::Now;
        }
        if !(2..=6).contains(&state_cat) && state_cat != 8 {
            self.combo = 0;
        }

        let mut next = req;
        let mut again = false;
        let range = self.range;
        let defend_now = req_cat == 1;
        let pick = |a: Action, b: Action, cond: bool| if cond { a } else { b };
        match state.0 {
            0x00 => switch = Switch::Now,
            0x01 => {
                switch = Switch::Now;
                if req == Action::READY {
                    switch = Switch::AtEndAlways;
                }
            }
            0x02 => {
                switch = Switch::Now;
                if req == Action::READY {
                    switch = Switch::AtEndAlways;
                    next = Action::IDLE2_LOOP;
                }
            }
            0x03 => {
                switch = Switch::Now;
                if req == Action::READY {
                    switch = Switch::AtEnd;
                    next = Action::IDLE2_LOOP;
                }
            }
            0x04..=0x07 | 0x77 | 0x78 => {
                switch = Switch::AtEndAlways;
                next = pick(Action::DEFEND2, Action::DEFENDR, state == Action::DEFEND1);
            }
            0x08 => {
                if req.0 > 0x1F {
                    switch = Switch::Now;
                }
            }
            // Strafe steps alternate like walking.
            0x09..=0x10 => {
                if req.0 > 0x1F || req_cat != 0 {
                    switch = Switch::Now;
                }
                next = alternate(state, req, 0x0C);
            }
            0x11..=0x13 => {
                if req.0 > 0x1A || req_cat != 0 {
                    switch = Switch::Now;
                }
                next = match state.0 {
                    0x11 if req == Action::WALK1 => Action::WALK2,
                    0x12 if req == Action::WALK1 => Action::WALK1,
                    0x13 if req == Action::RUN1 => Action::RUN2,
                    _ => req,
                };
            }
            0x14 => {
                if req == Action::HITREACT || req_cat != 0 {
                    switch = Switch::Now;
                }
                if req == Action::RUN1 {
                    next = Action::RUN1;
                }
            }
            0x15 | 0x17 | 0x18 => switch = Switch::Now,
            0x16 => {
                if req.0 > 0x1F || req_cat != 0 {
                    switch = Switch::Now;
                }
            }
            0x19 | 0x1A => {
                if req != Action::READY {
                    switch = Switch::Now;
                }
            }
            0x1B => {
                switch = if req.0 < 0x82 { Switch::Now } else { Switch::Always };
                if matches!(req, Action::READY | Action::WALK1 | Action::RUN1) {
                    switch = Switch::AtEnd;
                }
            }
            0x1C => {
                if req == cur {
                    next = Action::READY;
                }
            }
            0x20 => {
                if req_cat < 11 && !(9..=10).contains(&req_cat) {
                    next = Action::ATTSLOW1;
                } else {
                    switch = Switch::Now;
                }
            }
            0x21 => next = Action::ATTSLOW1R,
            0x22 | 0x2E | 0x2F | 0x32 | 0x33 | 0x36 | 0x37 | 0x3A | 0x3B | 0x3D | 0x43..=0x46 | 0x61 | 0x62 => {
                if defend_now {
                    switch = Switch::Now;
                }
            }
            0x23 => {
                if knocked < 2 {
                    switch = Switch::AtEnd;
                }
                next = Action::ATTPWRACLOSER;
            }
            0x24 | 0x26 | 0x55 | 0x64 => {
                if knocked == 0 {
                    switch = Switch::AtEnd;
                }
            }
            0x25 => {
                if knocked < 2 {
                    switch = Switch::AtEnd;
                }
                next = Action::ATTPWRAMEDR;
            }
            // The quick combo: another press or a held button keeps it
            // going, a power press finishes it, otherwise recover.
            0x27..=0x29 => {
                next = if let Some(finisher) = self.power_finisher() {
                    finisher
                } else if self.edges == 0 && self.latched == 0 || range.has(Range::FAR) {
                    pick(Action::ATTQUICK2R, Action::ATTQUICK3R, cur == Action::ATTQUICK2)
                } else if range.has(Range::MEDIUM) {
                    pick(Action::ATTSTEP3, Action::ATTSTEP2, cur == Action::ATTQUICK2)
                } else {
                    pick(Action::ATTQUICK3, Action::ATTQUICK2, cur == Action::ATTQUICK2)
                };
            }
            0x2A | 0x2B => {
                if let Some(finisher) = self.power_finisher() {
                    next = finisher;
                } else if self.edges != 0 && env.frame <= RECOVERY_REENTRY_FRAME && range.has(Range::CLOSE) {
                    next = pick(Action::ATTQUICK3, Action::ATTQUICK2, cur == Action::ATTQUICK2R);
                    switch = Switch::Now;
                }
                if defend_now {
                    switch = Switch::Now;
                }
            }
            0x2C => next = Action(0x2E),
            0x2D => next = Action(0x2F),
            0x30 => next = Action(0x32),
            0x31 => next = Action(0x33),
            0x34 => next = Action(0x36),
            0x35 => next = Action(0x37),
            0x38 => next = Action(0x3A),
            0x39 => next = Action(0x3B),
            0x3C => {
                next = if self.edges & button::POWER != 0 && self.combo != 0 {
                    Action::ATTPWRAMED
                } else {
                    Action::ATT360R
                };
            }
            0x3E..=0x40 => {
                next = if let Some(finisher) = self.power_finisher() {
                    finisher
                } else if self.edges == 0 && self.latched == 0 || range.has(Range::FAR) {
                    pick(Action::ATTSTEP2R, Action::ATTSTEP3R, cur == Action::ATTSTEP2)
                } else if range.has(Range::MEDIUM) {
                    pick(Action::ATTSTEP3, Action::ATTSTEP2, cur == Action::ATTSTEP2)
                } else {
                    pick(Action::ATTQUICK3, Action::ATTQUICK2, cur == Action::ATTSTEP2)
                };
            }
            0x41 | 0x42 => {
                if let Some(finisher) = self.power_finisher() {
                    next = finisher;
                } else if defend_now {
                    switch = Switch::Now;
                }
            }
            // Strafe attacks alternate like strafe steps.
            0x47..=0x4E => next = alternate(state, req, 0x4A),
            0x4F | 0x50 => {
                next = if req == Action::ATTLOW1 {
                    pick(Action::ATTLOW2, Action::ATTLOW1, cur == Action::ATTLOW1)
                } else {
                    Action::ATTLOWR
                };
            }
            0x51 => {
                if defend_now {
                    switch = Switch::Now;
                } else if req == Action::ATTLOW1 {
                    next = if env.has_low2 { Action::ATTLOW2 } else { Action::ATTLOW1 };
                } else {
                    switch = Switch::AtEndAlways;
                }
            }
            0x52 => {
                next = pick(Action::ATTPWRALOW, Action::ATTLOWKR, self.edges & button::POWER != 0 && self.combo != 0);
            }
            0x53 => {
                if self.edges & button::POWER != 0 && self.combo != 0 {
                    next = Action::ATTPWRALOW;
                } else {
                    switch = if defend_now { Switch::Now } else { Switch::AtEndAlways };
                }
            }
            0x54 => {
                if knocked < 2 {
                    switch = Switch::AtEnd;
                }
                next = Action::ATTPWRALOWR;
            }
            0x56 | 0x57 => {
                if knocked < 2 {
                    switch = Switch::AtEnd;
                }
            }
            // A throw's wind-up hands over to the throw.
            0x5B..=0x5E => {
                let second = state.0 >= 0x5D;
                let throw = pick(Action::THROW2, Action::THROW1, second);
                if req_cat < 2 || req_cat == 9 || req_cat == 10 {
                    if req.0 == 0x73 || req.0 == 0x75 || req.0 == 0x65 {
                        switch = Switch::Now;
                    } else {
                        next = throw;
                        if req.0 != 0x5B && env.frame >= THROW_START_FRAMES {
                            switch = Switch::Now;
                        }
                    }
                } else {
                    switch = Switch::Now;
                }
            }
            0x5F | 0x60 => next = pick(Action::THROW1R, Action::THROW2R, cur == Action::THROW1),
            0x63 => {
                if knocked < 2 {
                    switch = Switch::AtEnd;
                }
                next = Action::ATTPWRATHROWR;
            }
            // Magic: let go during MAGICS, the blast (MAGICR); still held,
            // the potion throw's wind-up (THROWPOTIONS), which ends in the
            // throw (THROWPOTIONR).
            0x73 => {
                if req == Action::THROWPOTIONS {
                    switch = Switch::Now;
                    next = req;
                } else if env.magic_released {
                    next = Action::MAGICR;
                } else {
                    next = Action::THROWPOTIONS;
                }
            }
            0x75 => next = Action::THROWPOTIONR,
            // Death's grab (the halo draining a Death): held while it's
            // asked for, then let go.
            0x1D | 0x1E => next = if req.0 == 0x1D { Action(0x1E) } else { Action(0x1F) },
            // A gauntlet's shot, the breath and the chop hand over to
            // their recoveries (the breath and the chop at once when
            // knocked, so they still go; a knock-down during a gauntlet's
            // shot is taken as from READY, above).
            0x67 | 0x68 | 0x6E | 0x70 => {
                if knocked == 0 {
                    switch = Switch::AtEnd;
                }
                next = match state.0 {
                    0x67 => Action::ATTFIRER,
                    0x68 => Action::ATTFIRERR,
                    0x6E => Action::ATTBREATHER,
                    _ => Action::ATTCHOPR,
                };
            }
            // STUN2 (a looping clip): asked to stand, it ends with its
            // clip; anything else cuts it, but not in the first frames of
            // its first pass.
            0x7A => {
                switch = if req == Action::READY || (env.frame < STUN_HOLD_FRAMES && !env.came_round) {
                    Switch::AtEnd
                } else {
                    Switch::Now
                };
            }
            // STUN1 and the hit reactions end in READY (asked for again,
            // they don't start over); a knock-down takes over at once.
            0x7F | 0x81 | 0x82 => {
                switch = if req.0 > 0x82 { Switch::Always } else { Switch::AtEnd };
                if req == cur {
                    next = Action::READY;
                }
            }
            // WEBREACT (looping) holds until something else is asked for.
            0x80 => {
                if req != Action::READY {
                    switch = Switch::Now;
                }
            }
            // The crossbow: SSHOT2, over and over, while its shot is asked
            // for; standing or moving, SSHOTR; anything else once the clip
            // is done (a hit reaction too).
            0x6B | 0x6C => {
                if req == Action::SSHOT1 {
                    next = Action::SSHOT2;
                    again = true;
                } else if req_cat == 0 {
                    next = Action::SSHOTR;
                } else {
                    switch = Switch::AtEndAlways;
                }
            }
            _ => {}
        }

        // Directional variants: a quick attack or lunge toward a target off
        // to the side or behind turns into the matching swing.
        next = match next.0 {
            0x5B if matches!(cur, Action::WALK1 | Action::RUN1) => Action::THROW2S_FROM_STRIDE,
            0x5C if matches!(cur, Action::WALK1 | Action::RUN1) => Action::THROW2S,
            0x3E => {
                let step = match cur.0 {
                    0x12 | 0x14 => Action::ATTWALK2,
                    0x27 | 0x29 => Action::ATTQ3TOSTEP1,
                    _ => next,
                };
                self.directional(step, false)
            }
            0x27 | 0x29 | 0x40 => self.directional(next, false),
            0x28 | 0x3F => self.directional(next, true),
            0x23 if range.has(Range::LOW) => Action::ATTPWRALOW,
            _ => next,
        };

        let blend = if next == Action::READY && cur != Action::READY && blends_to_ready(cur, env.class) {
            READY_BLEND
        } else {
            0.0
        };
        Next { action: next, switch, blend, again }
    }

    fn directional(&self, a: Action, second: bool) -> Action {
        const BEHIND: f32 = 3.0 * std::f32::consts::FRAC_PI_4;
        const SIDE: f32 = std::f32::consts::FRAC_PI_3;
        let t = self.target_angle;
        let pick = |q2: u8, q3: u8| Action(if second { q3 } else { q2 });
        if t > BEHIND {
            pick(0x34, 0x35)
        } else if t < -BEHIND {
            pick(0x38, 0x39)
        } else if t > SIDE {
            pick(0x2C, 0x2D)
        } else if t < -SIDE {
            pick(0x30, 0x31)
        } else {
            a
        }
    }

    /// Bookkeeping when `next` takes over: what the action that ended does
    /// (its strike), and the combo count and press edges.
    pub fn switched(&mut self, next: Action, class: Option<usize>) -> Strike {
        let old = self.action;
        let mut strike = Strike::default();
        match old.0 {
            0x21 | 0x3E..=0x40 | 0x43 | 0x45 => strike.0 |= Strike::STRONG,
            0x23 | 0x25 if class != Some(class::SOR) => strike.0 |= Strike::FINISHER,
            0x27..=0x29 | 0x2C | 0x2D | 0x30 | 0x31 | 0x34 | 0x35 | 0x38 | 0x39 | 0x3C | 0x4F | 0x50 => {
                strike.0 |= Strike::NORMAL
            }
            0x47..=0x4E | 0x5F | 0x60 | 0x65 | 0x66 => strike.0 |= Strike::SHOT,
            0x52 => strike.0 |= Strike::KICK,
            0x54 => strike.0 |= Strike::FINISHER,
            // Turbo attacks land like finishers (× 3, heavy).
            0x56 | 0x57 => strike.0 |= Strike::FINISHER,
            0x63 if class != Some(class::SOR) => strike.0 |= Strike::POWER_THROW,
            0x67 if next == Action::ATTFIRER => strike.0 |= Strike::GAUNTLET_LEFT,
            0x68 if next == Action::ATTFIRERR => strike.0 |= Strike::GAUNTLET_RIGHT,
            0x6B | 0x6C if matches!(next, Action::SSHOT2 | Action::SSHOTR) => strike.0 |= Strike::CROSSBOW,
            _ => {}
        }
        match next.0 {
            // Defending and strafing keep pending presses.
            0x04..=0x07 | 0x78 | 0x09..=0x10 | 0x77 => {}
            0x20 | 0x21 | 0x27..=0x29 | 0x2C | 0x2D | 0x30 | 0x31 | 0x34 | 0x35 | 0x38 | 0x39 | 0x3C
            | 0x3E..=0x40 | 0x43 | 0x45 | 0x4F | 0x50 | 0x52 => {
                strike.0 |= Strike::STARTED;
                self.combo = if self.edges == 0 { 0 } else { self.combo + 1 };
                self.edges = 0;
            }
            // Recoveries keep them.
            0x22 | 0x2A | 0x2B | 0x2E | 0x2F | 0x32 | 0x33 | 0x36 | 0x37 | 0x3A | 0x3B | 0x3D | 0x41 | 0x42
            | 0x44 | 0x46 | 0x51 | 0x53 => {}
            0x47..=0x4E | 0x5B..=0x5E | 0x65 | 0x66 | 0x6B | 0x6C => {
                strike.0 |= Strike::STARTED;
                self.edges = 0;
            }
            _ => self.edges = 0,
        }
        match next.0 {
            0x74 => strike.0 |= Strike::MAGIC,
            0x76 => strike.0 |= Strike::THROW_POTION,
            0x6E => strike.0 |= Strike::BREATH,
            0x71 => strike.0 |= Strike::CHOP,
            _ => {}
        }
        self.action = next;
        strike
    }
}

/// Strafe steps and strafe attacks come in pairs of first/second clips
/// (odd/even indices), grouped front/back (up to `split`) and left/right:
/// asking for a group's first clip plays the second after a first, the
/// first after a second.
fn alternate(state: Action, req: Action, split: u8) -> Action {
    let same_group = (state.0 <= split) == (req.0 <= split);
    if same_group && req.0 % 2 == 1 {
        Action(if state.0 % 2 == 1 { req.0 + 1 } else { req.0 })
    } else {
        req
    }
}

/// Whether returning to READY from `from` blends: not after the special
/// attacks and combos, a hit reaction, or the Archer's quick recovery.
fn blends_to_ready(from: Action, class: Option<usize>) -> bool {
    let a = from.0;
    !((0x56..=0x93).contains(&a) || a == 0x1B || a == 0x81 || a == 0x82 || class == Some(class::ARC) && a == 0x2A)
}

/// What an action does as it hands over to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strike(pub u32);

impl Strike {
    /// An attack action began.
    pub const STARTED: u32 = 1;
    /// A quick, directional, 360 or low swing lands.
    pub const NORMAL: u32 = 2;
    /// The slow attack or a lunge lands: double damage.
    pub const STRONG: u32 = 4;
    /// The kick lands.
    pub const KICK: u32 = 8;
    /// A combo finisher lands: triple damage.
    pub const FINISHER: u32 = 0x10;
    /// A strafe attack or throw releases a projectile.
    pub const SHOT: u32 = 0x100;
    /// SSHOT1 or SSHOT2 hands over to SSHOT2 (or starts over) or SSHOTR:
    /// the crossbow's bolt.
    pub const CROSSBOW: u32 = 0x800;
    /// The power throw releases its projectile.
    pub const POWER_THROW: u32 = 0x1000;
    /// A gauntlet's shot hands over to its recovery: the left one's
    /// lightning, the right one's acid.
    pub const GAUNTLET_LEFT: u32 = 0x2000;
    pub const GAUNTLET_RIGHT: u32 = 0x4000;
    /// MAGICR starts: a potion's blast (or shield).
    pub const MAGIC: u32 = 0x20000;
    /// THROWPOTIONR starts: the potion is thrown.
    pub const THROW_POTION: u32 = 0x40000;
    /// ATTBREATHE starts: the breath goes out (the game's `0x1000000`).
    pub const BREATH: u32 = 0x100_0000;
    /// ATTCHOPR starts: the hammer comes down (`0x2000000`).
    pub const CHOP: u32 = 0x200_0000;

    /// A melee blow is resolved now.
    pub fn melee(self) -> bool {
        self.0 & 0xFE != 0
    }

    pub fn projectile(self) -> bool {
        self.0 & 0xFF00 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env { frame: 0.0, class: Some(0), has_low2: true, magic_released: false, came_round: false }
    }

    fn state(action: Action, range: u32) -> ActionState {
        ActionState { action, range: Range(range), ..Default::default() }
    }

    #[test]
    fn table_names_match_the_indices_used() {
        assert_eq!(Action::ATTQUICK1.name(), "ATTQUICK1");
        assert_eq!(Action::ATTPWRALOWR.name(), "ATTPWRALOWR");
        assert_eq!(Action::DEFENDR.name(), "DEFENDR");
        assert_eq!(Action::THROW2R.name(), "THROW2R");
        assert_eq!(Action::STRAFE_ATKR2.name(), "STRAFE_ATKR2");
        assert_eq!(Action(0x94).name(), "GRABBED");
    }

    #[test]
    fn categories() {
        assert_eq!(Action::READY.category(), Category(0));
        assert_eq!(Action::DEFEND1.category(), Category(1));
        assert_eq!(Action::ATTPWRAMEDR.category(), Category(2));
        assert_eq!(Action::ATTQUICK3R.category(), Category(3));
        assert_eq!(Action::ATTQ3180L.category(), Category(4));
        assert_eq!(Action::ATT360R.category(), Category(5));
        assert_eq!(Action::ATTWALK2.category(), Category(6));
        assert_eq!(Action::STRAFE_ATKR2.category(), Category(7));
        assert_eq!(Action::ATTPWRALOWR.category(), Category(8));
        assert_eq!(Action::THROW2R.category(), Category(9));
    }

    #[test]
    fn locomotion_strides_alternate_and_wait_for_the_step() {
        let mut s = state(Action::RUN1, 0);
        let n = s.next(Action::RUN1, &env());
        assert_eq!((n.action, n.switch), (Action::RUN2, Switch::AtEnd));
        s.action = Action::RUN2;
        assert_eq!(s.next(Action::RUN1, &env()).action, Action::RUN1);
        s.action = Action::WALK1;
        assert_eq!(s.next(Action::WALK1, &env()).action, Action::WALK2);
        s.action = Action::RUN2;
        let stop = s.next(Action::READY, &env());
        assert_eq!((stop.action, stop.switch), (Action::READY, Switch::AtEnd));
        assert!(stop.blend > 0.0);
        s.action = Action::READY;
        let go = s.next(Action::WALK1, &env());
        assert_eq!((go.action, go.switch, go.blend), (Action::WALK1, Switch::Now, 0.0));
        // An attack cuts a stride short.
        s.action = Action::WALK2;
        assert_eq!(s.next(Action::ATTQUICK1, &env()).switch, Switch::Now);
    }

    #[test]
    fn quick_combo_continues_while_pressed_and_recovers_otherwise() {
        let mut s = state(Action::ATTQUICK1, Range::CLOSE);
        s.observe_buttons(button::QUICK);
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQUICK2);
        s.action = Action::ATTQUICK2;
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQUICK3);
        s.action = Action::ATTQUICK3;
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQUICK2);
        // Released and no fresh press: recover.
        s.observe_buttons(0);
        s.edges = 0;
        let n = s.next(Action::READY, &env());
        assert_eq!((n.action, n.switch), (Action::ATTQUICK3R, Switch::AtEnd));
        s.action = Action::ATTQUICK2;
        assert_eq!(s.next(Action::READY, &env()).action, Action::ATTQUICK2R);
        // Target out of reach: recover even while pressing; a bit further
        // away: lunge.
        s.observe_buttons(button::QUICK);
        s.range = Range(Range::FAR);
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQUICK2R);
        s.range = Range(Range::MEDIUM);
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTSTEP3);
    }

    #[test]
    fn recovery_reenters_the_combo_early_only() {
        let mut s = state(Action::ATTQUICK2R, Range::CLOSE);
        s.edges = button::QUICK;
        let early = s.next(Action::ATTQUICK1, &Env { frame: 1.0, ..env() });
        assert_eq!((early.action, early.switch), (Action::ATTQUICK3, Switch::Now));
        let late = s.next(Action::ATTQUICK1, &Env { frame: 3.0, ..env() });
        assert_eq!((late.action, late.switch), (Action::ATTQUICK1, Switch::AtEnd));
    }

    #[test]
    fn presses_count_the_combo_and_power_finishes_it() {
        let mut s = state(Action::READY, Range::CLOSE);
        s.observe_buttons(button::QUICK);
        s.switched(Action::ATTQUICK1, Some(0));
        assert_eq!(s.combo, 1);
        // Held without a fresh press: the count starts over.
        s.observe_buttons(button::QUICK);
        s.switched(Action::ATTQUICK2, Some(0));
        assert_eq!(s.combo, 0);
        // Tap, release, tap.
        s.observe_buttons(0);
        s.observe_buttons(button::QUICK);
        s.switched(Action::ATTQUICK3, Some(0));
        s.observe_buttons(0);
        s.observe_buttons(button::QUICK);
        assert_eq!(s.switched(Action::ATTQUICK2, Some(0)).0 & Strike::NORMAL, Strike::NORMAL);
        assert_eq!(s.combo, 2);
        s.observe_buttons(0);
        s.observe_buttons(button::POWER);
        let n = s.next(Action::ATTSTART, &env());
        assert_eq!(n.action, Action::ATTPWRAMED);
        assert_eq!(s.switched(n.action, Some(0)), Strike(Strike::NORMAL));
        s.observe_buttons(0);
        assert_eq!(s.next(Action::READY, &env()).action, Action::ATTPWRAMEDR);
        assert_eq!(s.switched(Action::ATTPWRAMEDR, Some(0)), Strike(Strike::FINISHER));
    }

    #[test]
    fn slow_attack_chain_strikes_double() {
        let mut s = state(Action::ATTSTART, Range::CLOSE);
        assert_eq!(s.next(Action::READY, &env()).action, Action::ATTSLOW1);
        s.switched(Action::ATTSLOW1, Some(0));
        assert_eq!(s.next(Action::READY, &env()).action, Action::ATTSLOW1R);
        assert_eq!(s.switched(Action::ATTSLOW1R, Some(0)), Strike(Strike::STRONG));
        let back = s.next(Action::READY, &env());
        assert_eq!((back.action, back.switch), (Action::READY, Switch::AtEnd));
        assert!(back.blend > 0.0);
    }

    #[test]
    fn targets_to_the_side_or_behind_pick_the_turning_swings() {
        let mut s = state(Action::ATTQUICK1, Range::CLOSE);
        s.observe_buttons(button::QUICK);
        s.target_angle = 1.2;
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQ2RIGHT);
        s.target_angle = -1.2;
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQ2LEFT);
        s.target_angle = 3.0;
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQ3180);
        s.action = Action::READY;
        s.target_angle = -3.0;
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQ2180L);
        s.target_angle = 0.3;
        assert_eq!(s.next(Action::ATTQUICK1, &env()).action, Action::ATTQUICK1);
    }

    #[test]
    fn a_low_target_gets_the_low_finisher() {
        let mut s = state(Action::ATTQUICK1, Range::CLOSE | Range::LOW);
        s.combo = 1;
        s.edges = button::POWER;
        assert_eq!(s.next(Action::ATTSTART, &env()).action, Action::ATTPWRALOW);
    }

    #[test]
    fn throws_wind_up_then_release() {
        let mut s = state(Action::THROW1S, Range::FAR);
        let n = s.next(Action::THROW1S, &env());
        assert_eq!((n.action, n.switch), (Action::THROW1, Switch::AtEnd));
        let n = s.next(Action::THROW1S, &Env { frame: 2.5, ..env() });
        assert_eq!((n.action, n.switch), (Action::THROW1, Switch::Now));
        s.switched(Action::THROW1, Some(0));
        assert_eq!(s.next(Action::READY, &env()).action, Action::THROW1R);
        assert!(s.switched(Action::THROW1R, Some(0)).projectile());
        s.action = Action::RUN1;
        assert_eq!(s.next(Action::THROW1S, &env()).action, Action::THROW2S);
    }

    #[test]
    fn strafe_steps_alternate() {
        let mut s = state(Action::STRAFE_WLKL1, 0);
        assert_eq!(s.next(Action::STRAFE_WLKL1, &env()).action, Action::STRAFE_WLKL2);
        s.action = Action::STRAFE_WLKL2;
        assert_eq!(s.next(Action::STRAFE_WLKL1, &env()).action, Action::STRAFE_WLKL1);
        s.action = Action::STRAFE_ATKB1;
        assert_eq!(s.next(Action::STRAFE_ATKB1, &env()).action, Action::STRAFE_ATKB2);
        s.action = Action::STRAFE_ATKB2;
        assert_eq!(s.next(Action::STRAFE_ATKB1, &env()).action, Action::STRAFE_ATKB1);
    }

    #[test]
    fn gauntlet_shots_go_as_their_recoveries_start() {
        let mut s = state(Action::ATTFIREL, 0);
        let n = s.next(Action::ATTFIREL, &env());
        assert_eq!((n.action, n.switch), (Action::ATTFIRER, Switch::AtEnd));
        assert_eq!(s.switched(n.action, Some(0)).0, Strike::GAUNTLET_LEFT);
        // Knocked down, the reaction takes over at once and the shot
        // doesn't go (the chooser treats a throw as READY then).
        let mut s = state(Action::ATTFIRELR, 0);
        let n = s.next(Action(0x85), &env());
        assert_eq!((n.action, n.switch), (Action(0x85), Switch::Now));
        assert!(!s.switched(n.action, Some(0)).projectile());
        // Only that hand-over raises it.
        let mut s = state(Action::ATTFIREL, 0);
        assert_eq!(s.switched(Action::READY, Some(0)).0, 0);
    }

    #[test]
    fn the_crossbow_shoots_while_asked() {
        // SSHOT1 hands over to SSHOT2, which starts over while asked for:
        // a bolt each time.
        let mut s = state(Action::SSHOT1, 0);
        let n = s.next(Action::SSHOT1, &env());
        assert_eq!((n.action, n.switch, n.again), (Action::SSHOT2, Switch::AtEnd, true));
        assert_eq!(s.switched(n.action, Some(0)).0, Strike::CROSSBOW | Strike::STARTED);
        let n = s.next(Action::SSHOT1, &env());
        assert_eq!((n.action, n.again), (Action::SSHOT2, true));
        assert_eq!(s.switched(n.action, Some(0)).0 & Strike::CROSSBOW, Strike::CROSSBOW);
        // Let go: SSHOTR at the clip's end, the last bolt.
        let n = s.next(Action::READY, &env());
        assert_eq!((n.action, n.switch, n.again), (Action::SSHOTR, Switch::AtEnd, false));
        assert_eq!(s.switched(n.action, Some(0)).0, Strike::CROSSBOW);
        // Any other attack waits for the clip's end, without a bolt.
        let mut s = state(Action::SSHOT2, 0);
        let n = s.next(Action::ATTQUICK1, &env());
        assert_eq!((n.action, n.switch), (Action::ATTQUICK1, Switch::AtEndAlways));
        assert_eq!(s.switched(n.action, Some(0)).0 & Strike::CROSSBOW, 0);
    }

    #[test]
    fn stuns_end_as_the_game_has_them() {
        // STUN2: standing waits for the clip; moving cuts it, but not
        // early in its first pass.
        let mut s = state(Action::STUN2, 0);
        assert_eq!(s.next(Action::READY, &Env { frame: 15.0, ..env() }).switch, Switch::AtEnd);
        assert_eq!(s.next(Action::WALK1, &Env { frame: 5.0, ..env() }).switch, Switch::AtEnd);
        assert_eq!(s.next(Action::WALK1, &Env { frame: 12.0, ..env() }).switch, Switch::Now);
        assert_eq!(s.next(Action::WALK1, &Env { frame: 5.0, came_round: true, ..env() }).switch, Switch::Now);
        // STUN1 asked for again goes to READY at its end; a knock-down
        // takes over at once.
        let mut s = state(Action::STUN1, 0);
        let n = s.next(Action::STUN1, &env());
        assert_eq!((n.action, n.switch), (Action::READY, Switch::AtEnd));
        assert_eq!(s.next(Action(0x85), &env()).switch, Switch::Always);
        // WEBREACT holds while standing.
        let mut s = state(Action::WEBREACT, 0);
        assert_eq!(s.next(Action::READY, &env()).switch, Switch::AtEnd);
        assert_eq!(s.next(Action::RUN1, &env()).switch, Switch::Now);
    }

    #[test]
    fn switch_modes() {
        assert!(Switch::Now.applies(true, false));
        assert!(!Switch::Now.applies(false, false));
        assert!(Switch::Now.applies(false, true));
        assert!(!Switch::AtEnd.applies(true, false));
        assert!(!Switch::AtEnd.applies(false, true));
        assert!(Switch::AtEnd.applies(true, true));
        assert!(Switch::AtEndAlways.applies(false, true));
        assert!(Switch::Always.applies(false, false));
    }

    #[test]
    fn attack_factors() {
        assert_eq!(factors(Action::ATTQUICK1, 0), (0.25, 0.0));
        assert_eq!(factors(Action::ATTSTART, 0), (0.0, 1.0));
        assert_eq!(factors(Action::ATTSTEP1, 0), (1.0, 0.25));
        assert_eq!(factors(Action::ATTPWRAMED, class::ARC), (0.25, 1.0));
        assert_eq!(factors(Action::ATTPWRAMED, class::SOR), (0.0, 0.0));
        assert_eq!(factors(Action::RUN2, 0), (1.3, 1.0));
        assert_eq!(factors(Action::STRAFE_WLKB1, 0), (0.667, 1.0));
        assert_eq!(stick_scale(Action::DEFEND1), 0.0);
        assert_eq!(stick_scale(Action::ATTQUICK1), 1.0);
    }
}
