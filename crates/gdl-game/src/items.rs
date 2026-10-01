//! What the hero touches: pickups, doors, locked chests, exits and
//! transporters, run the way the game's item code does it
//! (`docs/items.md`), plus the item models' own animation (powerups spin
//! and bob through their atree's looping `ACTIVE` action; doors and chests
//! play their open actions; a transporter's swirl and a force field's
//! glow are textures its actions change).
//!
//! Each tick, after the player has moved: every live item the hero
//! reaches is touched (the item type's shape and extents against the
//! hero's radius and height; secret walls by their own collision
//! triangles), blocking items push the hero back out, and at most one
//! powerup is picked up. Rock falls, sinking rocks and falling leaves the
//! hero comes near — and walls shot down — fall spinning out of the level
//! (`docs/items.md`, "Falling obstacles"). A key opens a locked chest and
//! what it holds comes out: a powerup hangs in it, growing as the lid
//! opens, till the hero walks into the open chest and takes it — and the
//! chest goes with it (`docs/items.md`, "Containers").

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::anim::{Atree, Track, rotation_matrix as pose_matrix};
use gdl_formats::texmod::TexModKind;
use gdl_formats::population::{
    ItemClass, ItemType, PlacementParams, REALM_LETTERS, level_for_code, rotation_matrix,
};
use gdl_formats::collision::NodePose;
use gdl_formats::{CollisionTriangle, LevelCollision, MoveParams};

use crate::audio::{CALL_VOLUME, HERO_LINE_VOLUME, LoopSoundAt, PlaySoundAt, QueueHeroLine};
use crate::combat::hit_kind;
use crate::character;
use crate::effects::EffectAt;
use crate::exits::ChangeLevelTo;
use crate::hints::{Hint, Hints, ShowHint};
use crate::mechanics::LevelNodes;
use crate::message_box::ShowMessage;
use crate::pickup_notices::PickupNotice;
use crate::level_material::LevelMaterial;
use crate::player::{Player, PlayerTick};
use crate::player_state::{Cry, FIELDS_PER_TICK, Heal, HurtHero, PlayerState, TimeStop, power};
use crate::quest;
use crate::population::{ContentModels, ItemRig, LevelPopulation, PlacementIndex};
use crate::world::LevelGround;

pub struct ItemsPlugin;

/// The item tick; runs after the player's.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemTick;

impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LevelItems>()
            .configure_sets(FixedUpdate, ItemTick.after(PlayerTick))
            .add_systems(FixedUpdate, tick.in_set(ItemTick))
            .add_systems(
                Update,
                (
                    build_items.run_if(resource_exists_and_changed::<LevelPopulation>),
                    attach_models,
                    pose_items,
                    show_rocks,
                    place_falling,
                    show_contents,
                )
                    .chain(),
            );
    }
}

// Item flags (the record's `+0xC4`, starting from the type's `+0x46`).
/// Activated: a door or chest opened, an exit in use.
pub const USED: u16 = 0x1;
/// A blow on a safe rock its armour swallows still takes this off.
const ROCK_LEAST_BLOW: f32 = 1.0;
/// Powerups flagged so (`+0xC4 & 0x8100`) are never held by a critter.
const HOLD_REFUSED: u16 = 0x8100;
/// How near a placed critter's spot a powerup must be for it to hold it:
/// across, and up or down.
const HOLD_ACROSS: f32 = 2.0;
const HOLD_UP: f32 = 3.0;
/// Collides even off screen (doors, exits, placements with flag bit 0).
pub const ALWAYS_ACTIVE: u16 = 0x40;
/// A locked container: a key opens it on touch.
const LOCKED: u16 = 0x10;
/// An exit the quest hasn't opened: it shows `EXIT_OFF` and goes nowhere.
pub const CLOSED: u16 = 0x8000;

/// The Pojo's special bit, and the food that poisons it (`CHICKEN`: 100).
const POJO: u32 = 0x400;
/// Skorne's horns, mask and gauntlets (special bits): one at a time.
const SKORNE: u32 = 0xF000;
const POJO_POISON: &str = "CHICKEN";
const POJO_POISON_AMOUNT: f32 = -100.0;

/// The container that explodes once opened (CHESTEXP), and the tick it
/// plays as it's set off.
pub const CHEST_EXP: i32 = 0x2C;
pub const CHEST_EXP_TICK: &str = "S_TICKY";

/// Powerup subtypes: the obelisk (never made), and the quest's pieces —
/// legendary items, scrolls, gems, gargoyle pieces.
const OBELISK: i32 = 12;
const LEGENDARY: i32 = 13;
const SCROLL: i32 = 14;
const GEM: i32 = 15;
const GARGOYLE_PIECE: i32 = 16;
/// Obstacle subtype: a boss level's safe rock, drawn by its stage
/// (`SAFEROCK3`…`SAFEROCK0`).
pub const SAFE_ROCK: i32 = 0x29;

/// Fields a picked-up item lingers before it's freed: 8, or 15 when a
/// player let it out (a chest's contents, and the chest with them).
const PICKUP_LINGER: i32 = 8;
const RELEASED_LINGER: i32 = 15;
/// Fields before a container's powerup can be picked up.
const CONTENTS_DELAY: i32 = 30;
/// Container subtypes: the barrel, the gold chest (its gold is its
/// amount) and the silver chest (gold in it makes it a gold chest).
const BARREL: i32 = 0x2B;
const GOLD_CHEST: i32 = 0x2F;
const SILVER_CHEST: i32 = 0x30;
/// Powerup subtypes: gold and keys; more than one key comes as the
/// `KEYRING` type.
const GOLD: i32 = 1;
const KEY: i32 = 2;
const KEY_RING: &str = "KEYRING";
/// Contents hanging in a chest are this big till it opens, growing to
/// their own size over its opening action.
const CONTENTS_SMALL: f32 = 0.2;
/// A transport takes this many fields; the hero moves when it's half done.
const TRANSPORT_FIELDS: i32 = 60;

/// Stand-ins for the player mover's collision values, the same ones
/// `player.rs` uses (not decoded yet).
const MOVER_RADIUS: f32 = 1.0;
const MOVER_STEP: f32 = 2.0;

/// The door sounds by realm id and door subtype (a `main.dol` table; empty
/// rows are realms whose doors are silent).
const DOOR_SOUNDS: [[&str; 4]; 13] = [
    ["", "", "", ""],
    ["S_GATEA4", "S_GATEA2", "S_GATEA3", "S_GATEA1"],
    ["S_GATEB1", "S_GATEB1", "S_GATEB1", "S_GATEB1"],
    ["S_GATEC1", "S_GATEC2", "S_GATEC3", "S_GATEC1"],
    ["S_GATED1", "S_GATED1", "S_GATED1", "S_GATED1"],
    ["", "", "", ""],
    ["", "", "", ""],
    ["S_GATEWOODG", "S_GATEWOODG", "S_GATEWOODG", "S_GATEMETG"],
    ["S_GATEWOODH", "S_GATEWOODH", "S_GATEWOODH", "S_GATEWOODH"],
    ["S_GATEWOODI", "S_GATEWOODI", "S_GATEWOODI", "S_GATEWOODI"],
    ["S_GATEWOODJ", "S_GATEWOODJ", "S_GATEWOODJ", "S_GATEMETJ"],
    ["S_GATEWOODK", "S_GATEWOODK", "S_GATEWOODK", "S_GATEWOODK"],
    ["", "", "", ""],
];

/// The transporter sound by realm id.
const TRANSPORT_SOUNDS: [&str; 14] = [
    "",
    "S_TRANSPORTA",
    "",
    "S_TRANSPORTC",
    "",
    "",
    "",
    "S_TRANSPORTG",
    "S_TRANSPORTH",
    "S_TRANSPORTI",
    "S_TRANSPORTJ",
    "S_TRANSPORTK",
    "S_TRANSPORTS3",
    "",
];

/// The base classes, in the game's order, whose eating sounds are in its
/// tables.
const EATERS: [&str; 8] = ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"];

/// An item type's collision shape (`+0x08`) and extents, placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    /// 0 none, 1 upright cylinder, 2 sphere, 3 box, 4 walls (their own
    /// collision triangles, `Wall`).
    pub kind: u16,
    /// `extent[0]`: radius.
    pub radius: f32,
    /// `extent[1]`: vertical reach above and below the centre.
    pub reach: f32,
    /// `extent[2]`, `extent[3]`: box half widths along the item's X and Z.
    pub half: [f32; 2],
    pub centre: [f32; 3],
    /// The item's X and Z axes in the world.
    pub axes: [[f32; 3]; 2],
}

impl Shape {
    fn of(ty: &ItemType, position: [f32; 3], rotation: [f32; 9]) -> Self {
        // The centre offset is raised by 1 and turned with the item.
        let off = [ty.center_offset[0], ty.center_offset[1] + 1.0, ty.center_offset[2]];
        let centre = std::array::from_fn(|j| position[j] + (0..3).map(|i| off[i] * rotation[i * 3 + j]).sum::<f32>());
        Self {
            kind: u16::from_le_bytes([ty.raw[8], ty.raw[9]]),
            radius: ty.extent[0],
            reach: ty.extent[1],
            half: [ty.extent[2], ty.extent[3]],
            centre,
            axes: [[rotation[0], rotation[1], rotation[2]], [rotation[6], rotation[7], rotation[8]]],
        }
    }

    fn local(&self, p: [f32; 3]) -> [f32; 2] {
        let (dx, dz) = (p[0] - self.centre[0], p[2] - self.centre[2]);
        [dx * self.axes[0][0] + dz * self.axes[0][2], dx * self.axes[1][0] + dz * self.axes[1][2]]
    }

    /// Whether a sphere of radius `r` at `p` touches the shape (a
    /// missile, `LevelItems::rock_in_way`).
    fn touches(&self, p: [f32; 3], r: f32) -> bool {
        let reach = self.radius + r;
        let (dx, dy, dz) = (p[0] - self.centre[0], p[1] - self.centre[1], p[2] - self.centre[2]);
        let dist = dx.hypot(dz);
        if dist > reach {
            return false;
        }
        match self.kind {
            1 => dy.abs() <= self.reach + r,
            2 => dist.hypot(dy) <= reach,
            3 => dy.abs() <= self.reach + r && self.in_box(p, r),
            _ => false,
        }
    }

    fn in_box(&self, p: [f32; 3], r: f32) -> bool {
        let l = self.local(p);
        l[0].abs() <= self.half[0] + r && l[1].abs() <= self.half[1] + r
    }
}

/// A touch: how far the hero is from the item's edge, and where the hero
/// would be pushed back to if the item blocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub clearance: f32,
    pub out: [f32; 3],
}

/// The game's item touch test for a hero of radius `r` and half height `h`
/// moving from `from` to `to`. Pads, damage tiles, exits and transporters
/// (`pass_through`) never push back; for everything else, moving away from
/// an item already reached isn't a touch, and `out` slides the hero around
/// a round item or out of a box along its shallowest side.
pub fn contact(s: &Shape, pass_through: bool, from: [f32; 3], to: [f32; 3], r: f32, h: f32) -> Option<Contact> {
    let reach = s.radius + r;
    // The game tests from the hero's collision centre, 2.5 above `to`.
    let (dx, dy, dz) = (to[0] - s.centre[0], to[1] + HERO_CENTRE - s.centre[1], to[2] - s.centre[2]);
    if reach * reach < dx * dx + dz * dz {
        return None;
    }
    if s.kind != 2 && dy.abs() > s.reach + h {
        return None;
    }
    let dist = dx.hypot(dz);
    if reach < dist {
        return None;
    }
    let inside = match s.kind {
        1 => true,
        2 => dist.hypot(dy) <= reach,
        3 => s.in_box(to, r),
        _ => false,
    };
    if !inside {
        return None;
    }
    let clearance = (dist - s.radius).max(0.0);
    if pass_through {
        return Some(Contact { clearance, out: to });
    }

    // Already touching at the start (always, for a cylinder) and moving
    // away: let the hero go.
    let already = match s.kind {
        1 => true,
        3 => s.in_box(from, r),
        _ => (from[0] - s.centre[0]).hypot(from[2] - s.centre[2]) <= reach,
    };
    let mut step = [to[0] - from[0], to[2] - from[2]];
    let mut toward = [s.centre[0] - from[0], s.centre[2] - from[2]];
    if already {
        normalize(&mut step);
        normalize(&mut toward);
        if step[0] * toward[0] + step[1] * toward[1] < 0.0 {
            return None;
        }
    }

    let out = if s.kind == 3 {
        let l = s.local(to);
        let pen = [s.half[0] + r - l[0].abs(), s.half[1] + r - l[1].abs()];
        let push = |axis: usize| {
            let d = if l[axis] <= 0.0 { -pen[axis] } else { pen[axis] };
            let a = s.axes[axis];
            [to[0] + a[0] * d, to[1] + a[1] * d, to[2] + a[2] * d]
        };
        if pen[0] < pen[1] && pen[0] > 0.0 {
            push(0)
        } else if pen[1] < pen[0] && pen[1] > 0.0 {
            push(1)
        } else {
            from
        }
    } else {
        // Slide along the tangent at the hero, on the side the move goes.
        let mut t = if step[1] * toward[0] - step[0] * toward[1] <= 0.0 {
            [toward[1], -toward[0]]
        } else {
            [-toward[1], toward[0]]
        };
        normalize(&mut t);
        let along = step[0] * t[0] + step[1] * t[1];
        [from[0] + t[0] * along, from[1], from[2] + t[1] * along]
    };
    Some(Contact { clearance, out })
}

fn normalize(v: &mut [f32; 2]) {
    let l = v[0].hypot(v[1]);
    if l > 0.0 {
        v[0] /= l;
        v[1] /= l;
    }
}

/// An item's own collision (shape 4, the secret walls): the level's
/// collision triangles its placement names (`+0x04` first, `+0x06`
/// count), in the item's frame, and where the item stands.
struct Wall {
    triangles: Arc<[CollisionTriangle]>,
    /// Where the item is now (a shot-down one falls).
    position: [f32; 3],
    /// Row vectors: world = local · rotation + position.
    rotation: [f32; 9],
}

impl Wall {
    /// The placement's triangles, if it names any.
    fn of(collision: &LevelCollision, links: [i16; 2], position: [f32; 3], rotation: [f32; 9]) -> Option<Self> {
        let (first, count) = (usize::try_from(links[0]).ok()?, usize::try_from(links[1]).ok()?);
        let triangles: Arc<[CollisionTriangle]> = collision.triangles.get(first..first.checked_add(count)?)?.into();
        (!triangles.is_empty()).then_some(Self { triangles, position, rotation })
    }

    fn to_local(&self, p: [f32; 3]) -> [f32; 3] {
        let d = [p[0] - self.position[0], p[1] - self.position[1], p[2] - self.position[2]];
        let r = &self.rotation;
        std::array::from_fn(|i| d[0] * r[i * 3] + d[1] * r[i * 3 + 1] + d[2] * r[i * 3 + 2])
    }

    fn to_world(&self, p: [f32; 3]) -> [f32; 3] {
        let r = &self.rotation;
        std::array::from_fn(|j| self.position[j] + (0..3).map(|i| p[i] * r[i * 3 + j]).sum::<f32>())
    }

    fn direction_to_world(&self, v: [f32; 3]) -> [f32; 3] {
        let r = &self.rotation;
        std::array::from_fn(|j| (0..3).map(|i| v[i] * r[i * 3 + j]).sum::<f32>())
    }
}

/// The game's test of a hero against a shape-4 item: its collision centre
/// (`HERO_CENTRE` above the feet) swept from `from` to `to` with radius
/// `r` against the wall's triangles (front faces) in the wall's frame; the
/// nearest hit pushes the hero out along its normal, level, until it's `r`
/// from the point it touched.
fn wall_contact(w: &Wall, from: [f32; 3], to: [f32; 3], r: f32) -> Option<Contact> {
    let centre = |p: [f32; 3]| [p[0], p[1] + HERO_CENTRE, p[2]];
    let (a, b) = (w.to_local(centre(from)), w.to_local(centre(to)));
    let (dist, point, normal) = w
        .triangles
        .iter()
        .filter_map(|t| t.sweep(a, b, r).map(|(d, p)| (d, p, t.normal)))
        .min_by(|x, y| x.0.total_cmp(&y.0))?;
    let point = w.to_world(point);
    let n = w.direction_to_world(normal);
    let mut level = [n[0], n[2]];
    normalize(&mut level);
    let c = centre(to);
    let push = (point[0] - c[0]) * level[0] + (point[2] - c[2]) * level[1] + r;
    let out = if push > 0.0 { [to[0] + level[0] * push, to[1], to[2] + level[1] * push] } else { to };
    Some(Contact { clearance: dist.sqrt(), out })
}

/// The hero's collision centre above its feet (`+0x64`), where the item
/// query tests from.
const HERO_CENTRE: f32 = 2.5;

/// Obstacles that fall once set off (`docs/items.md`, "Falling
/// obstacles"): rock falls (and crumbling floors), falling leaves, the
/// boss's debris (E2; set off by the boss, not wired), walls that fall
/// once shot down, sinking rocks.
const ROCK_FALL: i32 = 0x28;
const LEAF_FALL: i32 = 0x31;
const DEBRIS: i32 = 0x33;
const SHOT_FALL: i32 = 0x34;
const ROCK_SINK: i32 = 0x35;

fn falls(subtype: i32) -> bool {
    matches!(subtype, ROCK_FALL | LEAF_FALL | SHOT_FALL | ROCK_SINK)
}

/// What a fall takes off its speed each update, and its spin (radians a
/// second, times a step of [`SPIN_STEPS`]): leaves 1 and 10°, sinking
/// rocks 2 and 1°, the rest 2 and 20°.
fn fall_rates(subtype: i32) -> (f32, f32) {
    match subtype {
        LEAF_FALL => (1.0, 10f32.to_radians()),
        ROCK_SINK => (2.0, 1f32.to_radians()),
        _ => (2.0, 20f32.to_radians()),
    }
}

/// The spin's steps; an item spins about its X by the one its slot picks
/// (`& 7`) and about its Z by the one its slot's complement picks.
const SPIN_STEPS: [f32; 8] = [-4.0, -3.0, -2.0, -1.0, 1.0, 2.0, 3.0, 4.0];
/// A falling item is gone this far below the level's kill height.
const FALL_GONE: f32 = 200.0;
/// The falls' requested volume.
const FALL_VOLUME: u8 = 0xE0;

/// A rock fall's or sinking rock's sound when it's set off, by realm id
/// (A–K = 1–11; F2 and I5 have their own).
fn rock_fall_sound(realm: usize, level: usize) -> Option<&'static str> {
    Some(match (realm, level) {
        (6, 1) => "S_ROCKBREAKF2",
        (9, 4) => "S_ICEBREAKY",
        (1, _) => "S_FALLAWAY",
        (2, _) => "S_ROCKBREAK",
        (3, _) => "S_LIMBBREAKC",
        (4, _) => "S_LIMBBREAK",
        (5, _) => "S_ROCKBREAKE",
        (6, _) => "S_ROCKBREAKF",
        (7, _) => "S_ROCKBREAKG",
        (8, _) => "S_LIMBBREAKH",
        (9, _) => "S_ICEBREAK",
        _ => return None,
    })
}

/// Falling leaves' sound, by realm id: the forest's and the ice realm's.
fn leaf_fall_sound(realm: usize) -> Option<&'static str> {
    match realm {
        4 => Some("S_LEAFBREAK"),
        9 => Some("S_WOODBREAKI"),
        _ => None,
    }
}

/// One update of a falling item (the game's obstacle update, run while
/// it's on screen or always active): it spins — its matrix taken to the
/// locator builder's angles, X and Z turned on, built again — its fall
/// speeds up, it moves; once well below the level it's gone.
fn fall(item: &mut Item, dt: f32, kill: f32) -> bool {
    let (gravity, spin) = fall_rates(item.ty.subtype);
    let slot = item.placement;
    let mut a = crate::population::locator_euler(item.rotation);
    a[0] += spin * SPIN_STEPS[slot & 7] * dt;
    a[2] += spin * SPIN_STEPS[!slot & 7] * dt;
    item.rotation = crate::population::locator_matrix(a);
    let v = item.falling.get_or_insert([0.0; 3]);
    v[1] -= gravity;
    let v = *v;
    item.position = std::array::from_fn(|i| item.position[i] + v[i] * dt);
    item.shape = Shape::of(&item.ty, item.position, item.rotation);
    if let Some(w) = item.wall.as_mut() {
        w.position = item.position;
        w.rotation = item.rotation;
    }
    item.position[1] < kill - FALL_GONE
}

/// Where an item that rides a moving floor stands relative to it: its
/// position and turn (row vectors) with the floor's node at rest.
#[derive(Clone, Copy, Debug)]
struct Ride {
    node: usize,
    position: [f32; 3],
    rotation: [f32; 9],
}

/// One placed item's run-time state (the game's `0xF0`-byte item record).
struct Item {
    placement: usize,
    ty: ItemType,
    params: PlacementParams,
    shape: Shape,
    /// `+0xC4`.
    flags: u16,
    /// `+0xC8`: the state its animation has reached; `+0xCA`: the action
    /// it's playing.
    state: usize,
    action: usize,
    /// Frame of `action`, and whether it has ended (or, looping, finished
    /// a cycle).
    frame: f32,
    done: bool,
    /// `+0xC6`, fields.
    timer: i32,
    /// `+0xE0` for powerups: amount (gold, keys...), counted down as taken.
    amount: i32,
    /// `+0xEC`: fields until a powerup can be picked up.
    delay: i32,
    /// `+0xDE` for obstacles: the placement's count — a safe rock's stage
    /// (3 whole … 0 broken; −1 not made yet).
    stage: i16,
    /// `+0xD0` hit points and `+0xCF` armour (−1: blows don't hurt it):
    /// a safe rock's are its type's × its stage, and none once broken.
    hit_points: i16,
    armor: i8,
    /// Picked up or opened for good: lingering `timer` fields, then gone.
    leaving: bool,
    gone: bool,
    /// Held by a placed critter (the game's `+0xCD` = 10): hidden and out
    /// of reach till the critter drops it.
    held: bool,
    /// A sleeping critter's statue (a golem's, a gargoyle's): it blocks,
    /// and walking into it wakes the critter (the touch handler's placed
    /// monster case).
    statue: bool,
    atree: Option<Arc<Atree>>,
    model: Option<Entity>,
    /// A container's contents, resolved.
    contents: Option<ItemType>,
    /// Shape 4: its own collision triangles.
    wall: Option<Wall>,
    /// Where it stands and how it's turned (row vectors), and — falling —
    /// its speed (units a second).
    position: [f32; 3],
    rotation: [f32; 9],
    falling: Option<[f32; 3]>,
    /// The node of the floor the level's drop found under it, and — a
    /// moving one (flag 0x1000) — how it rides it.
    floor_node: Option<usize>,
    ride: Option<Ride>,
    /// The clip the model build started is still on (it goes round).
    first_loops: bool,
    /// A container's contents while they're in it (the game's `+0xE8`
    /// both ways): on the contents, the container (placement) they hang
    /// in; on the container, what it holds.
    inside: Option<usize>,
    holds: Option<usize>,
    /// Contents whose model has been asked for (`show_contents`).
    shown: bool,
    /// Its model has a contents node to hang what it lets out on.
    hangs: bool,
}

impl Item {
    fn class(&self) -> ItemClass {
        self.ty.class
    }

    /// A safe rock, by the item's own subtype (the placement's, else the
    /// type's).
    fn is_safe_rock(&self) -> bool {
        let own = match self.params {
            PlacementParams::Obstacle { subtype, .. } if subtype >= 1 => i32::from(subtype),
            _ => self.ty.subtype,
        };
        self.class() == ItemClass::Obstacle && own == SAFE_ROCK
    }

    /// Starts `action` of the item's atree from its first frame. A
    /// restart takes the clip's loop flag from the action again.
    fn play(&mut self, action: usize) {
        self.action = action;
        self.frame = 0.0;
        self.done = false;
        self.first_loops = false;
    }

    /// Whether the current clip goes round (the game's clip flag, anim
    /// `+0x34` = item `+0xA4`): set from the action's own flag (`+0x24`)
    /// at each start, but the model build sets it once the first clip
    /// has started, so an item's first action goes round until the item
    /// moves to another — keys, scrolls and the icons turn for good
    /// though their `ACTIVE` doesn't loop. A trigger's update clears it
    /// every frame; an exit's sets it for actions 0, 1 and 3.
    fn loops(&self, own: bool) -> bool {
        match self.class() {
            ItemClass::Trigger => false,
            ItemClass::Exit => matches!(self.action, 0 | 1 | 3),
            _ => own || self.first_loops,
        }
    }

    /// Advances the current action by `dt` seconds at its own rate.
    fn advance(&mut self, dt: f32) {
        let Some(a) = self.atree.as_ref().and_then(|t| t.actions.get(self.action)) else {
            self.done = true;
            return;
        };
        let loops = self.loops(a.loops());
        // A looping action finishing a cycle counts as its end for whatever
        // waits on it.
        let wrapped = character::advance_clip(&mut self.frame, dt, a.frames, a.rate, loops);
        if wrapped || (!loops && self.frame >= character::clip_end(a.frames)) {
            self.done = true;
        }
    }

    fn action_count(&self) -> usize {
        self.atree.as_ref().map_or(0, |t| t.actions.len())
    }
}

/// An exit the hero is taking: fields left before the level changes, and
/// whether it's a secret one.
struct Leaving {
    to: String,
    fields: i32,
    secret: bool,
}

/// A placed sound item (class 13) that plays its sound about it: the
/// placement's name is the sound, `+0x30` how far it carries at full
/// volume, `+0x34` 0 (others pick the music, not done), `+0x38` flags.
struct Ambient {
    name: String,
    at: Vec3,
    reach: f32,
    playing: bool,
}

impl Ambient {
    /// From a sound placement: its name and reach, when it's an ambient one.
    fn of(name: &str, params: &[u8; 12], at: Vec3) -> Option<Self> {
        let reach = f32::from_le_bytes(params[0..4].try_into().ok()?);
        let zone = i16::from_le_bytes([params[4], params[5]]);
        (zone == 0 && !name.is_empty()).then(|| Self { name: name.to_ascii_uppercase(), at, reach, playing: false })
    }
}

/// A transport under way.
struct Transport {
    to: [f32; 3],
    fields: i32,
}

/// The current level's items.
#[derive(Resource, Default)]
pub struct LevelItems {
    items: Vec<Item>,
    realm: usize,
    /// The level within its realm (0 the first).
    level: usize,
    doors: usize,
    /// The hero's feet at the end of the last tick.
    last_feet: Option<[f32; 3]>,
    /// The transporter touched this tick, and the cooldown until the next
    /// one works (set on arrival, cleared by stepping off).
    transport: Option<Transport>,
    transport_cooldown: i32,
    leaving: Option<Leaving>,
    /// The hero stood in an open exit this tick (its flame burns).
    in_exit: bool,
    flame: bool,
    /// The placed sound items' loops, and whether this is the secret
    /// realm's first level (where they're louder).
    ambient: Vec<Ambient>,
    secret_first: bool,
    /// Items released so far this level (container contents).
    released: usize,
    /// The level's scroll texts (`SCROLLSA1`).
    scrolls: String,
    /// Statues walked into since the critters last looked (placements).
    woken: Vec<usize>,
    /// The types letting out contents can change to: the gold chest a
    /// silver chest holding gold becomes (the level's last container type
    /// of subtype `0x2F`), and the key ring more than one key comes as.
    gold_chest: Option<ItemType>,
    key_ring: Option<ItemType>,
    /// Model swaps waiting for `show_contents` (placement, model), and
    /// monsters let out of containers waiting for `breakables.rs`.
    swaps: Vec<(usize, &'static str)>,
    let_out: Vec<(ItemType, [f32; 3])>,
}

/// What the level's other item code (`mechanics.rs`, `hazards.rs`,
/// `breakables.rs`) sees of one item.
pub struct ItemView<'a> {
    /// The placement it came from; released items get numbers from
    /// [`RELEASED_BASE`] on.
    pub placement: usize,
    pub ty: &'a ItemType,
    pub params: &'a PlacementParams,
    /// Its touch shape, where it stands (floor-dropped).
    pub shape: Shape,
    /// `+0xC4`.
    pub flags: u16,
    /// `+0xC8`: the action it has reached; `+0xCA`: the one playing.
    pub state: usize,
    pub action: usize,
    /// The playing action has finished (or come round, looping).
    pub done: bool,
    /// How many actions its atree has (0 without one).
    pub actions: usize,
    /// Neither picked up, opened for good nor freed.
    pub live: bool,
    /// Its armour now (`+0xCF`: −1 blows don't hurt it).
    pub armor: i8,
    /// A safe rock that's been made (standing or broken).
    pub rock: bool,
    /// A sleeping critter's statue.
    pub statue: bool,
    /// What it holds (containers).
    pub contents: Option<&'a ItemType>,
    /// Its model, while it has one.
    pub model: Option<Entity>,
    /// The node of the floor the level's drop put it on.
    pub floor_node: Option<usize>,
}

/// Placement numbers of items released at run time (container contents)
/// start here, clear of the level's own.
pub const RELEASED_BASE: usize = 1 << 20;

impl LevelItems {
    fn find(&self, placement: usize) -> Option<&Item> {
        self.items.iter().find(|i| i.placement == placement)
    }

    fn find_mut(&mut self, placement: usize) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.placement == placement)
    }

    fn view_of(item: &Item) -> ItemView<'_> {
        ItemView {
            placement: item.placement,
            ty: &item.ty,
            params: &item.params,
            shape: item.shape,
            flags: item.flags,
            state: item.state,
            action: item.action,
            done: item.done,
            actions: item.action_count(),
            live: !item.gone && !item.leaving && !item.held && item.inside.is_none(),
            armor: item.armor,
            rock: item.is_safe_rock() && item.stage >= 0,
            statue: item.statue,
            contents: item.contents.as_ref(),
            floor_node: item.floor_node,
            model: item.model,
        }
    }

    pub fn view(&self, placement: usize) -> Option<ItemView<'_>> {
        self.find(placement).map(Self::view_of)
    }

    /// The items riding a moving floor (placement, the floor's node), but
    /// those falling.
    pub fn riders(&self) -> Vec<(usize, usize)> {
        self.items
            .iter()
            .filter(|i| !i.gone && i.falling.is_none() && !(falls(i.ty.subtype) && i.class() == ItemClass::Obstacle && i.flags & USED != 0))
            .filter_map(|i| i.ride.map(|r| (i.placement, r.node)))
            .collect()
    }

    /// Puts a riding item where its floor's `pose` takes it: its touch
    /// shape (and a wall's triangles) follow. Its model, position and turn
    /// (row vectors) for the caller to place the model.
    pub fn ride(&mut self, placement: usize, pose: &NodePose) -> Option<(Option<Entity>, [f32; 3], [f32; 9])> {
        let i = self.find_mut(placement)?;
        let r = i.ride?;
        i.position = pose.apply(r.position);
        // World turn = rest turn · poseᵀ (row vectors).
        let m = &pose.rotation;
        i.rotation = std::array::from_fn(|k| {
            let (row, col) = (k / 3, k % 3);
            (0..3).map(|j| r.rotation[row * 3 + j] * m[col * 3 + j]).sum()
        });
        i.shape = Shape::of(&i.ty, i.position, i.rotation);
        if let Some(w) = i.wall.as_mut() {
            w.position = i.position;
            w.rotation = i.rotation;
        }
        Some((i.model, i.position, i.rotation))
    }

    /// Every item not freed yet.
    pub fn views(&self) -> impl Iterator<Item = ItemView<'_>> {
        self.items.iter().filter(|i| !i.gone).map(Self::view_of)
    }

    /// The realm id (`REALM_LETTERS`) of the level.
    pub fn realm(&self) -> usize {
        self.realm
    }

    /// Starts action `action` of the item's atree (its model follows).
    pub fn play(&mut self, placement: usize, action: usize) {
        if let Some(i) = self.find_mut(placement) {
            i.play(action);
        }
    }

    /// Sets the state its animation has reached (`+0xC8`).
    pub fn set_state(&mut self, placement: usize, state: usize) {
        if let Some(i) = self.find_mut(placement) {
            i.state = state;
        }
    }

    /// Sets flag bits (`+0xC4`): [`USED`] starts an opened door, chest or
    /// broken barrel stepping through its actions.
    pub fn set_flags(&mut self, placement: usize, flags: u16) {
        if let Some(i) = self.find_mut(placement) {
            i.flags |= flags;
        }
    }

    /// Sets what taking a powerup gives (`+0xE0`): gold blown to junk,
    /// food spoiled by gas (`breakables.rs`).
    pub fn set_amount(&mut self, placement: usize, amount: i32) {
        if let Some(i) = self.find_mut(placement) {
            i.amount = amount;
        }
    }

    /// Lets go of the item's model, for a new one to take its place: the
    /// caller despawns it and spawns the new one with the item's placement
    /// number, which binds it to the item as the level's models are
    /// (`ContentModels::spawn`).
    pub fn take_model(&mut self, placement: usize) -> Option<Entity> {
        let i = self.find_mut(placement)?;
        i.atree = None;
        i.model.take()
    }

    /// A placed golem, gargoyle or general holds the nearest powerup whose
    /// centre is within [`HOLD_ACROSS`] across and [`HOLD_UP`] up or down
    /// of its spot (the game's level-load pass): hidden and out of reach
    /// till it's dropped. Returns its placement number.
    pub fn hold_nearest(&mut self, at: [f32; 3]) -> Option<usize> {
        let near = |i: &Item| {
            let c = i.shape.centre;
            ((c[0] - at[0]).powi(2) + (c[2] - at[2]).powi(2)).sqrt()
        };
        let item = self
            .items
            .iter_mut()
            .filter(|i| i.class() == ItemClass::Powerup && !i.gone && !i.held && i.flags & HOLD_REFUSED == 0)
            .filter(|i| near(i) < HOLD_ACROSS && (i.shape.centre[1] - at[1]).abs() < HOLD_UP)
            .min_by(|a, b| near(a).total_cmp(&near(b)))?;
        item.held = true;
        Some(item.placement)
    }

    /// Stands the item at `position` (a thrown item in flight): its touch
    /// shape goes with it. Returns its model, for the caller to move.
    pub fn place(&mut self, placement: usize, position: [f32; 3]) -> Option<Entity> {
        let i = self.find_mut(placement)?;
        i.shape = Shape::of(&i.ty, position, rotation_matrix([0.0; 3]));
        i.model
    }

    /// Its critter died at `at`: the held item is let go there (and can be
    /// picked up after `delay` fields). Returns its model, for the caller
    /// to move there and show.
    pub fn drop_held(&mut self, placement: usize, at: [f32; 3], delay: i32) -> Option<Entity> {
        let i = self.find_mut(placement)?;
        i.held = false;
        i.delay = delay;
        i.shape = Shape::of(&i.ty, at, rotation_matrix([0.0; 3]));
        i.model
    }

    /// The placement stands as a critter's statue (`critters.rs`).
    pub fn mark_statue(&mut self, placement: usize) {
        if let Some(i) = self.find_mut(placement) {
            i.statue = true;
        }
    }

    /// The statues walked into since the last call.
    pub fn take_woken(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.woken)
    }

    /// The nearest safe rock a missile going from `from` to `to` (its
    /// radius `radius`, kind `kind`) runs into: how far along (0..1), and
    /// its placement. Standing rocks stop every missile but magic
    /// (`0x200`); a broken one (armour −1) only an explosive one (`0x400`)
    /// — the game's item filter for missiles.
    pub fn rock_in_way(&self, from: [f32; 3], to: [f32; 3], radius: f32, kind: u32) -> Option<(f32, usize)> {
        if kind & 0x200 != 0 {
            return None;
        }
        let (a, b) = (Vec3::from(from), Vec3::from(to));
        let steps = ((b - a).length() / (0.5 * radius.max(0.25))).ceil().clamp(1.0, 64.0) as usize;
        self.items
            .iter()
            .filter(|i| i.is_safe_rock() && i.stage >= 0 && !i.gone && (i.armor >= 0 || kind & 0x400 != 0))
            .filter_map(|i| {
                (0..=steps).map(|k| k as f32 / steps as f32).find(|&t| i.shape.touches(a.lerp(b, t).to_array(), radius)).map(|t| (t, i.placement))
            })
            .min_by(|x, y| x.0.total_cmp(&y.0))
    }

    /// The nearest sleeping critter's statue a missile going from `from` to
    /// `to` (its radius `radius`) runs into: how far along (0..1), and its
    /// placement. Only a missile that doesn't pass most items (the
    /// monsters', flag `0x100`) is stopped: `pass_items` false.
    pub fn statue_in_way(&self, from: [f32; 3], to: [f32; 3], radius: f32, pass_items: bool) -> Option<(f32, usize)> {
        if pass_items {
            return None;
        }
        let (a, b) = (Vec3::from(from), Vec3::from(to));
        let steps = ((b - a).length() / (0.5 * radius.max(0.25))).ceil().clamp(1.0, 64.0) as usize;
        self.items
            .iter()
            .filter(|i| i.statue && !i.gone)
            .filter_map(|i| {
                (0..=steps).map(|k| k as f32 / steps as f32).find(|&t| i.shape.touches(a.lerp(b, t).to_array(), radius)).map(|t| (t, i.placement))
            })
            .min_by(|x, y| x.0.total_cmp(&y.0))
    }

    /// A blow landed on a critter's statue (the game's item damage
    /// routine, the placed monster case): it wakes.
    pub fn strike_statue(&mut self, placement: usize) {
        if self.find(placement).is_some_and(|i| i.statue && !i.gone) {
            self.woken.push(placement);
        }
    }

    /// A blow of `damage` on a safe rock (the game's item damage routine):
    /// its armour comes off, leaving at least 1 (armour −1: nothing), and
    /// that comes off its hit points; then it's restaged — 0 hit points
    /// broken (0: walked over, no armour), up to its type's 1, up to twice
    /// 2, more 3. Returns the new stage when it changed.
    pub fn hit_rock(&mut self, placement: usize, damage: f32) -> Option<i16> {
        let i = self.find_mut(placement).filter(|i| i.is_safe_rock() && i.stage >= 0)?;
        if i.armor < 0 {
            return None;
        }
        let blow = (damage - f32::from(i.armor)).max(ROCK_LEAST_BLOW);
        i.hit_points = (i.hit_points - blow.round() as i16).max(0);
        let per = i.ty.hit_points;
        let stage = match i.hit_points {
            0 => 0,
            hp if hp <= per => 1,
            hp if hp <= per.saturating_mul(2) => 2,
            _ => 3,
        };
        if stage == i.stage {
            return None;
        }
        i.stage = stage;
        if stage == 0 {
            i.armor = -1;
        }
        Some(stage)
    }

    /// Frees the item at once, model and all (the game's `+0xC4 = 0xFFFF`),
    /// and what it holds hanging in it.
    pub fn free(&mut self, placement: usize, commands: &mut Commands) {
        let Some(i) = self.find_mut(placement) else { return };
        i.gone = true;
        if let Some(m) = i.model.take() {
            commands.entity(m).try_despawn();
        }
        if let Some(held) = i.holds.take() {
            self.free(held, commands);
        }
    }

    /// A boss that makes the level's safe rocks (the yeti's boulders)
    /// starts without them: each is hidden and walked through until it's
    /// made (`docs/critters.md`, "Safe rocks"). Returns how many.
    pub fn hide_safe_rocks(&mut self) -> usize {
        let mut n = 0;
        for item in self.items.iter_mut().filter(|i| i.is_safe_rock()) {
            item.stage = -1;
            n += 1;
        }
        n
    }

    /// The level's safe rocks, in order: placement, centre, and whether it
    /// stands (hit points and a stage above 0).
    pub fn safe_rocks(&self) -> Vec<(usize, [f32; 3], bool)> {
        self.items
            .iter()
            .filter(|i| i.is_safe_rock() && !i.gone)
            .map(|i| (i.placement, i.shape.centre, i.stage > 0 && i.hit_points > 0))
            .collect()
    }

    /// A rock the boss threw down lands (docs/critters.md "Safe rocks"):
    /// shown, stage 3, three times its type's hit points, its type's armour.
    /// Returns its type's name, for its model.
    pub fn make_rock(&mut self, placement: usize) -> Option<String> {
        let i = self.find_mut(placement).filter(|i| i.is_safe_rock())?;
        i.stage = 3;
        i.hit_points = i.ty.hit_points.saturating_mul(3);
        i.armor = i.ty.armor;
        Some(i.ty.name.clone())
    }

    /// A new item of type `ty` standing at `position` (turned by the
    /// placement-style `rotation`), the way the game releases a container's
    /// contents: `amount` overrides the type's (keys take the container's
    /// count), and it can't be picked up for `delay` fields. Returns its
    /// placement number; its model, if one was built for the level, is
    /// spawned by the caller with it ([`ContentModels::spawn`]).
    pub fn release(&mut self, ty: ItemType, position: [f32; 3], rotation: [f32; 9], amount: Option<i32>, delay: i32) -> usize {
        let (hit_points_of_type, armor_of_type) = (ty.hit_points, ty.armor);
        let placement = RELEASED_BASE + self.released;
        self.released += 1;
        let params = match ty.class {
            ItemClass::Powerup => PlacementParams::Powerup { count: amount.unwrap_or(ty.amount as i32) as i16 },
            _ => PlacementParams::None,
        };
        self.items.push(Item {
            placement,
            shape: Shape::of(&ty, position, rotation),
            amount: amount.unwrap_or(ty.amount as i32),
            flags: ty.flags,
            ty,
            params,
            state: 0,
            action: 0,
            frame: 0.0,
            done: false,
            timer: 0,
            delay,
            stage: 1,
            hit_points: hit_points_of_type,
            armor: armor_of_type,
            leaving: false,
            gone: false,
            held: false,
            statue: false,
            atree: None,
            model: None,
            contents: None,
            wall: None,
            position,
            rotation,
            falling: None,
            floor_node: None,
            ride: None,
            first_loops: true,
            inside: None,
            holds: None,
            shown: false,
            hangs: false,
        });
        placement
    }

    /// The monsters let out of containers since the last call: their type
    /// and where (the container's centre), for `breakables.rs` to make.
    pub fn take_let_out(&mut self) -> Vec<(ItemType, [f32; 3])> {
        std::mem::take(&mut self.let_out)
    }

    /// What container `i` holds comes out, as the game lets it out when a
    /// key opens a chest (`docs/items.md`, "Containers"). A silver chest
    /// holding gold becomes the level's gold chest (its model the realm's
    /// `CHESTSG`) and a gold chest keeps the gold, both taken by walking
    /// into the open chest; more than one key comes as the key ring. A
    /// powerup hangs on the chest's contents node (its model's `NULL1`)
    /// till it's taken; one without a model to hang on is let out where
    /// the chest stands, as a barrel's is; a monster is let out there too.
    fn release_contents(&mut self, i: usize) {
        let Some(ty) = self.items[i].contents.clone() else { return };
        let chest = &self.items[i];
        let (subtype, placement, centre) = (chest.ty.subtype, chest.placement, chest.shape.centre);
        let count = match chest.params {
            PlacementParams::Container { param, .. } => i32::from(param),
            _ => 0,
        };
        let hangs = chest.hangs;
        let (position, rotation) = (chest.position, chest.rotation);
        let powerup = ty.class == ItemClass::Powerup;
        if subtype == SILVER_CHEST && powerup && ty.subtype == GOLD {
            let gold = self.gold_chest.clone();
            let chest = &mut self.items[i];
            if let Some(t) = gold {
                chest.ty = t;
            }
            chest.amount = i32::from(ty.amount);
            self.swaps.push((placement, crate::population::SILVER_GOLD_CHEST));
            return;
        }
        if subtype == GOLD_CHEST {
            self.items[i].amount = i32::from(ty.amount);
            return;
        }
        if ty.class == ItemClass::EnemyInfo {
            self.let_out.push((ty, centre));
            return;
        }
        let ty = match &self.key_ring {
            Some(ring) if powerup && ty.subtype == KEY && count > 1 => ring.clone(),
            _ => ty,
        };
        // Keys come as many as the container says (at least one), a
        // scroll as its number.
        let amount = match ty.subtype {
            KEY if powerup => Some(count.max(1)),
            SCROLL if powerup => Some(count),
            _ => None,
        };
        let delay = if powerup { CONTENTS_DELAY } else { 0 };
        if powerup && hangs {
            let new = self.release(ty, centre, rotation_matrix([0.0; 3]), amount, delay);
            if let Some(item) = self.find_mut(new) {
                item.inside = Some(placement);
            }
            self.items[i].holds = Some(new);
        } else {
            self.release(ty, position, rotation, amount, delay);
        }
    }
}

/// Plays an item's atree actions on its model.
#[derive(Component)]
struct ItemPose {
    action: usize,
    frame: f32,
    tracks: Option<(usize, Vec<Option<Track>>)>,
    /// Each flipbook node's frame on show: (action, frame of its run; none
    /// hidden).
    shown: Vec<(usize, Option<usize>)>,
    /// The texture its actions last put in place of one of its own (the
    /// game keeps one per object), and its copies of the materials drawing
    /// that texture, by part.
    texture: Option<(u16, AssetId<Image>)>,
    copies: HashMap<Entity, Handle<LevelMaterial>>,
}

pub(crate) fn build_items(
    mut items: ResMut<LevelItems>,
    population: Res<LevelPopulation>,
    ground: Option<Res<LevelGround>>,
    nodes: Option<Res<LevelNodes>>,
    state: Option<Res<PlayerState>>,
) {
    let pop = &population.population;
    // The items are dropped with the movers at their off heights (the
    // game runs the mover update once first), and those landing on a
    // moving node's floor (flag 0x1000) ride it.
    let starts = nodes.as_ref().map(|n| crate::mechanics::start_poses(pop, n)).unwrap_or_default();
    let dropping = ground.as_ref().map(|g| {
        let mut c = (*g.0).clone();
        for (&node, &pose) in &starts {
            c.set_pose(node, pose);
        }
        c
    });
    let realm = population
        .level
        .strip_prefix("level")
        .and_then(|s| s.chars().next())
        .and_then(|c| REALM_LETTERS.iter().find(|(l, _)| l.eq_ignore_ascii_case(&c)))
        .map_or(0, |(_, id)| *id as usize);
    let mut out = Vec::new();
    let mut ambient = Vec::new();
    for (index, placement) in pop.placements.iter().enumerate() {
        // One player: the rest are hidden and never touched.
        if !placement.active_for(1) {
            continue;
        }
        let ty = pop.resolved_type(placement).clone();
        // The tower's gems aren't placed once the town has opened.
        if ty.class == ItemClass::Powerup
            && ty.subtype == GEM
            && realm == quest::TOWER as usize
            && state.as_ref().is_some_and(|s| s.quest.crystals_open(1))
        {
            continue;
        }
        // An obelisk is never made (the game frees it as it's built).
        if ty.class == ItemClass::Powerup && ty.subtype == OBELISK {
            continue;
        }
        let rotation = rotation_matrix(placement.rotation);
        let mut position = placement.position;
        // Every item is dropped onto the floor below it (4 above to 10
        // below, radius 1) and lifted 0.1 — left where it is, 0.1 up, with
        // none — unless its type keeps its height.
        let mut floor_node = None;
        let mut ride = None;
        if !ty.keeps_height() {
            match dropping.as_ref().and_then(|c| c.floor_probe(position, 4.0, -10.0, 1.0, 0).map(|h| (h, c))) {
                Some((hit, c)) => {
                    position[1] = hit.point[1] + 0.1;
                    floor_node = Some(hit.node);
                    if c.nodes[hit.node].flags & gdl_formats::collision::node_flags::MOVES != 0 {
                        let at = starts.get(&hit.node).copied().unwrap_or(NodePose::REST);
                        let m = &at.rotation;
                        ride = Some(Ride {
                            node: hit.node,
                            position: at.inverse_apply(position),
                            // Rest turn = world turn · pose (row vectors).
                            rotation: std::array::from_fn(|k| {
                                let (row, col) = (k / 3, k % 3);
                                (0..3).map(|j| rotation[row * 3 + j] * m[j * 3 + col]).sum()
                            }),
                        });
                    }
                }
                None => position[1] += 0.1,
            }
        }
        let mut flags = ty.flags;
        if placement.flags & 1 != 0 {
            flags |= ALWAYS_ACTIVE;
        }
        let params = placement.params(ty.class);
        let contents = match params {
            PlacementParams::Container { contents: Some(c), .. } if c < pop.item_types.len() => {
                Some(pop.resolve(c).clone())
            }
            _ => None,
        };
        let mut amount = ty.amount as i32;
        let stage = match params {
            PlacementParams::Obstacle { count, .. } => count,
            _ => 1,
        };
        let rock = matches!(params, PlacementParams::Obstacle { subtype, .. } if (if subtype >= 1 { i32::from(subtype) } else { ty.subtype }) == SAFE_ROCK);
        let hit_points = if rock { ty.hit_points.saturating_mul(stage.max(0)) } else { ty.hit_points };
        let armor = if rock && stage == 0 { -1 } else { ty.armor };
        match (ty.class, &params) {
            (ItemClass::Exit, _) => {
                flags = (flags & !USED) | ALWAYS_ACTIVE;
                // An exit the quest hasn't opened is shut (`quest.rs`) — in
                // the tower only: the game switches exits off as the tower
                // loads, and a realm level's own exits are always open.
                if realm == quest::TOWER as usize
                    && let (PlacementParams::Exit { destination: Some(code) }, Some(state)) = (&params, &state)
                    && let Some((to_realm, to_level)) = exit_destination(code)
                    && !state.exit_open(to_realm, to_level)
                {
                    flags |= CLOSED;
                }
            }
            // Keys come as many as the placement says (at least one; the
            // game shows several as the key ring).
            (ItemClass::Powerup, PlacementParams::Powerup { count }) if ty.subtype == 2 => {
                amount = (*count as i32).max(1)
            }
            // A scroll's is the page of the level's scroll texts it shows,
            // numbered from 1.
            (ItemClass::Powerup, PlacementParams::Powerup { count }) if ty.subtype == SCROLL => amount = *count as i32,
            // A sound item has no model: it stays where it's placed.
            (ItemClass::Sound, _) => {
                ambient.extend(Ambient::of(&placement.name, &placement.params, Vec3::from(placement.position)));
            }
            _ => {}
        }
        let shape = Shape::of(&ty, position, rotation);
        let wall = (shape.kind == 4)
            .then(|| ground.as_ref().and_then(|g| Wall::of(&g.0, placement.links, position, rotation)))
            .flatten();
        out.push(Item {
            placement: index,
            shape,
            ty,
            params,
            flags,
            state: 0,
            action: 0,
            frame: 0.0,
            done: false,
            timer: 0,
            amount,
            delay: 0,
            stage,
            hit_points,
            armor,
            leaving: false,
            gone: false,
            held: false,
            statue: false,
            atree: None,
            model: None,
            contents,
            wall,
            position,
            rotation,
            falling: None,
            floor_node,
            ride,
            // The build starts action 0; one made used starts action 1
            // instead (unless its type steps actions, flag 4), a restart.
            first_loops: flags & USED == 0 || flags & 4 != 0,
            inside: None,
            holds: None,
            shown: false,
            hangs: false,
        });
    }
    let doors = out.iter().filter(|i| i.class() == ItemClass::Door).count();
    let shut = out.iter().filter(|i| i.flags & CLOSED != 0).count();
    info!("{}: {} items in play ({doors} doors, {shut} exits shut)", population.level, out.len());
    let scrolls = format!("SCROLLS{}", population.level.strip_prefix("level").unwrap_or_default().to_ascii_uppercase());
    if !ambient.is_empty() {
        info!("{} ambient sounds: {:?}", ambient.len(), ambient.iter().map(|a| a.name.as_str()).collect::<Vec<_>>());
    }
    // The secret realm's first level record is levelS1.
    let secret_first = population.level.eq_ignore_ascii_case(SECRET_FIRST_LEVEL);
    let level = population.level.chars().last().and_then(|c| c.to_digit(10)).map_or(0, |d| d.saturating_sub(1) as usize);
    let gold_chest = pop.item_types.iter().rev().find(|t| t.class == ItemClass::Container && t.subtype == GOLD_CHEST).cloned();
    let key_ring = pop
        .item_types
        .iter()
        .find(|t| t.name == KEY_RING && t.class == ItemClass::Powerup && t.subtype == KEY)
        .cloned();
    *items = LevelItems { items: out, realm, level, doors, scrolls, ambient, secret_first, gold_chest, key_ring, ..default() };
}

/// An exit's destination code (`g1`) as a realm id and level (0 the
/// first).
pub fn exit_destination(code: &str) -> Option<(u32, u32)> {
    let mut chars = code.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let digit = chars.next()?.to_digit(10)?;
    let realm = REALM_LETTERS.iter().find(|(l, _)| *l == letter)?.1;
    Some((realm, digit.checked_sub(1)?))
}

/// A model just spawned for an item: its placement, rig, where it stands,
/// and whether it's a shut exit's `EXIT_OFF`.
type NewModel<'a> = (Entity, &'a PlacementIndex, Option<&'a ItemRig>, &'a Transform, Has<ShutExitModel>);

/// Links newly spawned models to their items (and removes the models of
/// items not in a one-player game).
fn attach_models(
    mut commands: Commands,
    mut items: ResMut<LevelItems>,
    contents: Option<Res<ContentModels>>,
    models: Query<NewModel, Added<PlacementIndex>>,
) {
    for (entity, &PlacementIndex(placement), rig, transform, shut_model) in &models {
        match items.items.iter_mut().find(|i| i.placement == placement) {
            // A shut exit shows `EXIT_OFF` in its place, as the game swaps
            // the model.
            Some(item) if item.flags & CLOSED != 0 && !shut_model => {
                commands.entity(entity).despawn();
                item.model = None;
                if let Some(off) = contents.as_ref().and_then(|c| c.spawn(EXIT_OFF, *transform, placement, &mut commands)) {
                    commands.entity(off).insert(ShutExitModel);
                }
            }
            Some(item) => {
                item.model = Some(entity);
                item.atree = rig.map(|r| r.atree.clone());
                item.hangs = rig.is_some_and(|r| r.atree.node_index(crate::population::CONTENTS_NODE).is_some());
                // Where the item's drop put it (with the movers at their
                // off heights); contents hang at their chest's node.
                let place = match item.inside {
                    Some(_) => Transform::from_scale(transform.scale),
                    None => Transform {
                        translation: Vec3::from(item.position),
                        rotation: Quat::from_mat3(&Mat3::from_cols_array(&item.rotation)),
                        scale: transform.scale,
                    },
                };
                commands.entity(entity).insert(place);
                let shown = vec![(0, Some(0)); rig.map_or(0, |r| r.flipbooks.len())];
                let pose = ItemPose { action: 0, frame: 0.0, tracks: None, shown, texture: None, copies: HashMap::new() };
                commands.entity(entity).insert(pose);
            }
            None => commands.entity(entity).despawn(),
        }
    }
}

/// The model of a shut exit.
pub const EXIT_OFF: &str = "EXIT_OFF";

/// Marks the `EXIT_OFF` model standing in for a shut exit's own.
#[derive(Component)]
struct ShutExitModel;

/// Poses every animated item model at its item's action and frame: its
/// bones, the frame each flipbook node shows (a barrel breaking), and the
/// texture its action puts on it.
fn pose_items(
    mut commands: Commands,
    items: Res<LevelItems>,
    mut models: Query<(&ItemRig, &mut ItemPose)>,
    mut bones: Query<&mut Transform>,
    mut drawn: Query<&mut MeshMaterial3d<LevelMaterial>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
) {
    for item in &items.items {
        let Some((rig, mut pose)) = item.model.and_then(|m| models.get_mut(m).ok()) else { continue };
        pose.action = item.action;
        pose.frame = item.frame;
        if let Some((binding, image)) = action_texture(item, rig)
            && pose.texture != Some((binding, image.id()))
        {
            pose.texture = Some((binding, image.id()));
            let pose = &mut *pose;
            for (part, b, shared) in &rig.texmod_parts {
                let Ok(mut material) = drawn.get_mut(*part) else { continue };
                // Another of its textures is replaced now: this one is the
                // bank's again.
                if *b != binding {
                    material.0 = shared.clone();
                    continue;
                }
                let copy = match pose.copies.get(part) {
                    Some(copy) => copy.clone(),
                    None => {
                        let Some(own) = materials.get(shared).cloned() else { continue };
                        let copy = materials.add(own);
                        pose.copies.insert(*part, copy.clone());
                        copy
                    }
                };
                if let Some(m) = materials.get_mut(&copy) {
                    m.diffuse = Some(image.clone());
                }
                material.0 = copy;
            }
        }
        for (k, (holder, node, frames, facing)) in rig.flipbooks.iter().enumerate() {
            let Some(list) = frames.get(pose.action) else { continue };
            let start = rig.atree.flipbook_entry(*node, pose.action).map_or(0, |e| e.param);
            let playing = rig.atree.actions.get(pose.action);
            let frame = playing.and_then(|a| character::flipbook_frame(a, pose.frame, start, list.len()));
            let now = (pose.action, frame);
            if pose.shown.get(k) == Some(&now) {
                continue;
            }
            if let Some(s) = pose.shown.get_mut(k) {
                *s = now;
            }
            debug!(
                "item {} flipbook {}: action {} frame {frame:?} ({} parts)",
                item.placement,
                rig.atree.nodes[*node].name,
                pose.action,
                frame.and_then(|f| list.get(f)).map_or(0, Vec::len)
            );
            commands.entity(*holder).despawn_related::<Children>();
            for p in frame.and_then(|f| list.get(f)).into_iter().flatten() {
                let e = commands.spawn((Mesh3d(p.mesh.clone()), MeshMaterial3d(p.material.clone()), ChildOf(*holder))).id();
                if let Some(b) = facing {
                    commands.entity(e).insert((*b, Transform::default()));
                }
            }
        }
        if pose.tracks.as_ref().is_none_or(|(a, _)| *a != pose.action) {
            let tracks = (0..rig.atree.nodes.len())
                .map(|n| rig.atree.clip_bone(n).and_then(|b| rig.atree.track(b, pose.action).ok().flatten()))
                .collect();
            pose.tracks = Some((pose.action, tracks));
        }
        let Some((_, tracks)) = &pose.tracks else { continue };
        for (n, &bone) in rig.bones.iter().enumerate() {
            let (Some(Some(track)), Ok(mut t)) = (tracks.get(n), bones.get_mut(bone)) else { continue };
            let p = track.sample(pose.frame);
            let m = Mat4::from_cols_array(&pose_matrix(p.rotation, track.flags));
            *t = Transform {
                translation: Vec3::from(rig.atree.nodes[n].offset) + Vec3::from(p.translation),
                rotation: Quat::from_mat4(&m),
                scale: Vec3::from(p.scale),
            };
        }
    }
}

/// The texture an item's action puts on its model: each of the action's
/// modifiers in turn sets the object's one replaced texture to its
/// flipbook's frame at the action's frame (rounded; counted from the end
/// when the action runs backwards; wrapped round a looping action), so the
/// last one listed wins (`docs/rendering.md`, "Texture animation"). None
/// when the action runs none: the last one stays.
fn action_texture(item: &Item, rig: &ItemRig) -> Option<(u16, Handle<Image>)> {
    let list = rig.texmods.as_ref()?.actions.get(item.action)?;
    let action = rig.atree.actions.get(item.action)?;
    let mut f = (item.frame + 0.5) as i32;
    if action.backwards() {
        f = i32::from(action.frames) - f - 1;
    }
    let mut shown = None;
    for (m, frames) in list {
        let length = i32::from(m.count) * m.period as i32;
        if length < f && action.params[0] != 0 && length > 1 {
            f %= length;
        }
        if let TexModKind::Frames(_) = m.kind
            && let Some(Some(image)) = frames.get(m.action_frame(f) as usize)
        {
            shown = Some((m.binding, image.clone()));
        }
    }
    shown
}

/// A safe rock not made yet (stage −1) and an item a critter holds aren't
/// drawn: their parts are hidden (the model itself follows the population
/// view).
fn show_rocks(items: Res<LevelItems>, children: Query<&Children>, mut parts: Query<&mut Visibility>) {
    for item in items.items.iter().filter(|i| i.is_safe_rock() || i.class() == ItemClass::Powerup) {
        let Some(model) = item.model else { continue };
        let hidden = item.held || item.is_safe_rock() && item.stage < 0;
        let want = if hidden { Visibility::Hidden } else { Visibility::Inherited };
        for part in children.iter_descendants(model) {
            if let Ok(mut v) = parts.get_mut(part)
                && *v != want
            {
                *v = want;
            }
        }
    }
}

/// What touching an item did to the hero.
enum Touch {
    /// Nothing more (or the item is walked through).
    Pass,
    /// The item blocks the hero.
    Block,
    /// The hero stands in an exit or on a transporter.
    Stand,
}

/// Sounds and hints a tick raises, sent when it ends.
struct Out<'a> {
    sounds: Vec<PlaySoundAt>,
    /// The hero's own lines, for the heroes' voice queue.
    voices: Vec<String>,
    /// Poison the hero has eaten, dealt as blows once the items are done.
    poison: Vec<f32>,
    hints: Vec<Hint>,
    messages: Vec<ShowMessage>,
    /// The sparkle a pickup gives off (placed at the item by its caller),
    /// and those placed.
    sparkle: Option<&'static str>,
    effects: Vec<(&'static str, [f32; 3])>,
    /// Statues walked into (placements).
    woken: Vec<usize>,
    /// The plates the pickups show over the panel (`pickup_notices.rs`).
    notices: Vec<PickupNotice>,
    seen: &'a Hints,
    /// The level's scroll texts.
    scrolls: String,
    /// Game seconds.
    now: f32,
    /// The level's realm id.
    realm: usize,
}

impl Out<'_> {
    /// A pickup's sound: centred, at the call's own volume.
    fn sound(&mut self, name: &str) {
        self.sound_as(name, CALL_VOLUME);
    }

    /// Centred, at `volume`.
    fn sound_as(&mut self, name: &str, volume: u8) {
        if !name.is_empty() {
            self.sounds.push(PlaySoundAt::centred(name, volume));
        }
    }

    /// Faded and panned at `at`, at the call's own volume (the chests',
    /// doors' and transporters').
    fn sound_at(&mut self, name: &str, at: [f32; 3]) {
        if !name.is_empty() {
            self.sounds.push(PlaySoundAt::faded(name, Vec3::from(at), CALL_VOLUME));
        }
    }

    /// A line of the hero's, queued in the heroes' voice queue.
    fn voice(&mut self, name: &str) {
        if !name.is_empty() {
            self.voices.push(name.into());
        }
    }

    fn hint(&mut self, hint: Hint) {
        self.hints.push(hint);
    }

    /// The pickup sparkle (`POWERUPS`) for the item just taken.
    fn sparkle(&mut self, name: &'static str) {
        self.sparkle = Some(name);
    }

    /// Places the pending sparkle at `at`.
    fn sparkle_at(&mut self, at: [f32; 3]) {
        if let Some(name) = self.sparkle.take() {
            self.effects.push((name, at));
        }
    }

    fn message(&mut self, group: &str, index: usize, voice: Option<&'static str>) {
        self.messages.push(ShowMessage { group: group.into(), index: Some(index), voice });
    }

    /// The plate for a pickup: its subtype and the game's value for it.
    fn notice(&mut self, subtype: i32, value: i32) {
        self.notices.push(PickupNotice { subtype, value });
    }
}

#[allow(clippy::too_many_arguments)]
fn tick(
    time: Res<Time>,
    mut items: ResMut<LevelItems>,
    mut state: ResMut<PlayerState>,
    ground: Option<Res<LevelGround>>,
    mut players: Query<&mut Player>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut commands: Commands,
    (mut sounds, mut loops): (MessageWriter<PlaySoundAt>, MessageWriter<LoopSoundAt>),
    mut voices: MessageWriter<QueueHeroLine>,
    (mut hints, mut messages): (MessageWriter<ShowHint>, MessageWriter<ShowMessage>),
    seen: Res<Hints>,
    mut change: MessageWriter<ChangeLevelTo>,
    mut effects: MessageWriter<EffectAt>,
    mut hurt: MessageWriter<HurtHero>,
    mut notices: MessageWriter<PickupNotice>,
    (stop, camera): (Res<TimeStop>, Option<Res<crate::play_camera::PlayCamera>>),
) {
    let dt = time.delta_secs();
    let items = &mut *items;
    let scrolls = items.scrolls.clone();
    let now = time.elapsed_secs();
    let mut out = Out {
        sounds: Vec::new(),
        voices: Vec::new(),
        poison: Vec::new(),
        hints: Vec::new(),
        messages: Vec::new(),
        sparkle: None,
        effects: Vec::new(),
        woken: Vec::new(),
        notices: Vec::new(),
        seen: &seen,
        scrolls,
        now,
        realm: items.realm,
    };
    update_items(items, dt, &mut commands);
    run(items, dt, &mut state, ground.as_deref(), &mut players, &cameras, &mut out, &mut change);
    fall_items(items, dt, ground.as_deref(), &cameras, &mut commands);
    items.woken.append(&mut out.woken);
    // Poison eaten is a poison blow on the hero, through its armour powers
    // and its reactions (the gold armour's heal comes back negative); the
    // food's own line is its only cry.
    if let Ok(mut player) = players.single_mut() {
        for amount in out.poison.drain(..) {
            let taken = player.take_blow(amount, hit_kind::POISON, Vec3::ZERO);
            if taken != 0.0 {
                hurt.write(HurtHero { amount: taken, kind: hit_kind::POISON, cry: Cry::Silent });
            }
        }
    }
    let feet = players.iter().next().map(|p| Vec3::from(p.mover.position));
    sounds.write_batch(out.sounds);
    // The hero's own lines, panned from where it is.
    if let Some(at) = feet {
        voices.write_batch(out.voices.into_iter().map(|line| QueueHeroLine { line, volume: HERO_LINE_VOLUME, at }));
    }
    exit_flame(items, feet, &mut loops);
    let quiet = stop.0 || camera.is_some_and(|c| c.in_cut());
    ambient_sounds(items, feet.filter(|_| state.alive), quiet, &mut loops);
    hints.write_batch(out.hints.into_iter().map(ShowHint));
    messages.write_batch(out.messages);
    notices.write_batch(out.notices);
    effects.write_batch(out.effects.into_iter().map(|(name, at)| EffectAt {
        name,
        bank: Some(SPARKLE_BANK),
        at: Vec3::from(at),
        facing: 0.0,
        scale: 1.0,
    }));
}

/// Where the pickup sparkles are.
const SPARKLE_BANK: &str = "POWERUPS";
/// A gem's sparkle by its crystal counter (1 orange … 8 black; effects
/// `0x46`–`0x4D`), a gargoyle piece's and a runestone's.
const GEM_SPARKLES: [&str; 9] = [
    "",
    "GETGEMORANGE",
    "GETGEMRED",
    "GETGEMPURPLE",
    "GETGEMBLUE",
    "GETGEMGREEN",
    "GETGEMYELLOW",
    "GETGEMWHITE",
    "GETGEMBLACK",
];
const GARGOYLE_SPARKLE: &str = "GETGARG";
const RUNE_SPARKLE: &str = "GETRUNE";

/// One tick of the hero against the items.
#[allow(clippy::too_many_arguments)]
fn run(
    items: &mut LevelItems,
    dt: f32,
    state: &mut PlayerState,
    ground: Option<&LevelGround>,
    players: &mut Query<&mut Player>,
    cameras: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    out: &mut Out,
    change: &mut MessageWriter<ChangeLevelTo>,
) {
    items.in_exit = false;
    let Ok(mut player) = players.single_mut() else {
        items.last_feet = None;
        return;
    };
    if let Some(leaving) = &mut items.leaving {
        leaving.fields -= FIELDS_PER_TICK;
        if leaving.fields <= 0 {
            info!("exit to {}", leaving.to);
            change.write(ChangeLevelTo::finishing(leaving.to.clone()));
            items.leaving = None;
        }
        return;
    }
    let to = player.mover.position;
    // A hero moved instantly (put back at the start, a test's hop) is
    // touched where it is now, not swept along the jump.
    let from = if std::mem::take(&mut player.teleported) { to } else { items.last_feet.unwrap_or(to) };
    if !state.alive {
        items.last_feet = Some(to);
        return;
    }

    // A transport under way holds the hero still, then moves them when
    // it's half done.
    if let Some(t) = &mut items.transport {
        let before = t.fields;
        t.fields -= 2 * FIELDS_PER_TICK;
        let at = if before >= TRANSPORT_FIELDS / 2 && t.fields < TRANSPORT_FIELDS / 2 {
            out.hint(Hint::Transporter);
            items.transport_cooldown = 1;
            t.to
        } else {
            from
        };
        if t.fields <= 0 {
            items.transport = None;
        }
        player.mover.position = at;
        items.last_feet = Some(at);
        return;
    }

    let visible = |c: [f32; 3]| on_screen(cameras, c);
    let (r, h) = (state.radius, state.half_height);
    let mut blocked: Option<Contact> = None;
    let mut picked = false;
    let mut on_transporter = None;
    let mut on_exit = Vec::new();
    for i in 0..items.items.len() {
        let item = &items.items[i];
        // Contents still in their chest are only taken through it.
        if item.gone || item.leaving || item.held || item.inside.is_some() {
            continue;
        }
        if !(item.flags & ALWAYS_ACTIVE != 0 || visible(item.shape.centre)) {
            continue;
        }
        if !touchable(item) {
            continue;
        }
        // A sleeping critter's statue is touched out to its placement's
        // range — the game's touch shape for a placed monster is a
        // cylinder that wide — which wakes it (a negative range never
        // does); it blocks only up close, within its own shape.
        if let PlacementParams::Enemy { range, .. } = item.params
            && item.class() == ItemClass::EnemyInfo
            && range >= 0.0
            && contact(&Shape { kind: 1, radius: range, ..item.shape }, true, from, to, r, h).is_some()
        {
            out.woken.push(item.placement);
        }
        let pass = matches!(
            item.class(),
            ItemClass::Trigger | ItemClass::DamageTile | ItemClass::Exit | ItemClass::Transporter
        );
        let c = match &item.wall {
            Some(w) => wall_contact(w, from, to, r),
            None => contact(&item.shape, pass, from, to, r, h),
        };
        let Some(c) = c else { continue };
        match touch(items, i, state, from, to, &mut picked, out) {
            Touch::Pass => {}
            Touch::Block => {
                if blocked.is_none_or(|b| c.clearance < b.clearance) {
                    blocked = Some(c);
                }
            }
            Touch::Stand => match items.items[i].class() {
                ItemClass::Transporter => on_transporter = Some(i),
                _ => on_exit.push(i),
            },
        }
    }

    // Pushed back by what blocks, through the level's walls again.
    let mut feet = to;
    if let Some(c) = blocked {
        let delta = [c.out[0] - from[0], to[1] - from[1], c.out[2] - from[2]];
        feet = match ground {
            Some(g) => {
                let p = MoveParams::new(MOVER_RADIUS, MOVER_STEP, 16.0 * dt);
                let d = g.0.move_actor(from, delta, &p).delta;
                std::array::from_fn(|k| from[k] + d[k])
            }
            None => std::array::from_fn(|k| from[k] + delta[k]),
        };
        player.mover.position = feet;
    }

    items.in_exit = exits(items, &on_exit, feet, out);
    if let Some(leaving) = &items.leaving
        && player.going_out.is_none()
    {
        player.going_out = Some(crate::going_out::GoingOut::new(feet[1], leaving.fields));
    }
    if let Some(t) = on_transporter {
        start_transport(items, t, r, ground.map(|g| &*g.0), cameras, out);
    } else if items.transport_cooldown > 0 {
        items.transport_cooldown -= 1;
    }
    items.last_feet = Some(feet);
}

/// Items the touch test skips outright: freed ones, open doors, burst
/// barrels, sounds.
fn touchable(item: &Item) -> bool {
    match item.class() {
        ItemClass::Door => !(item.state > 1 || (item.state == 1 && item.timer > 30)),
        ItemClass::Obstacle => !((43..46).contains(&item.ty.subtype) && item.state > 0),
        ItemClass::Container => !(item.ty.subtype == 0x2B && item.state == 2),
        ItemClass::Sound => false,
        ItemClass::EnemyInfo => item.statue,
        _ => true,
    }
}

/// Is `centre` inside the play camera's view? The game only lets items
/// that are on screen (or flagged always-active) be touched.
fn on_screen(cameras: &Query<(&Camera, &GlobalTransform), With<Camera3d>>, centre: [f32; 3]) -> bool {
    let Some((camera, at)) = cameras.iter().find(|(c, _)| c.is_active) else { return true };
    camera
        .world_to_ndc(at, Vec3::from(centre))
        .is_some_and(|n| n.x.abs() <= 1.1 && n.y.abs() <= 1.1 && n.z > 0.0 && n.z <= 1.0)
}

/// The game's touch handler for item `i`.
fn touch(
    items: &mut LevelItems,
    i: usize,
    state: &mut PlayerState,
    from: [f32; 3],
    to: [f32; 3],
    picked: &mut bool,
    out: &mut Out,
) -> Touch {
    let (doors, realm, level) = (items.doors, items.realm, items.level);
    let item = &mut items.items[i];
    match item.class() {
        ItemClass::Powerup => {
            // One powerup per hero per tick, once its release delay is over.
            if item.delay < 1 && !*picked {
                *picked = true;
                let (sub, value, name) = (item.ty.subtype, item.ty.value, item.ty.name.clone());
                let dur = item.ty.duration as f32;
                let taken = pick_up(state, sub, value, &name, &mut item.amount, dur, doors, out);
                out.sparkle_at(item.shape.centre);
                if taken {
                    item.leaving = true;
                    item.timer = PICKUP_LINGER;
                    info!(
                        "picked up {name}: health {:.0}, gold {}, keys {}, potions {}",
                        state.health,
                        state.gold,
                        state.keys,
                        state.potions.len()
                    );
                }
            }
            Touch::Pass
        }
        ItemClass::Container => {
            if item.state == 2 && item.ty.subtype == GOLD_CHEST {
                // An opened gold chest: its gold, and the chest goes.
                let gold = item.amount.max(0) as u32;
                if gold > 24 {
                    out.hint(Hint::CollectGold);
                }
                state.add_gold(gold);
                out.notice(1, gold as i32);
                info!("took {gold} gold from {}", item.ty.name);
                out.sound("S_PICKUPMAGIC");
                item.leaving = true;
                item.timer = PICKUP_LINGER;
            } else if item.state == 2
                && let Some(held) = item.holds
            {
                // Open and holding what came out: walked into, it's taken
                // (one powerup per hero per tick).
                if !*picked {
                    *picked = true;
                    take_contents(items, i, held, state, out);
                }
                return Touch::Pass;
            } else if item.flags & LOCKED != 0 && item.state == 0 && item.flags & USED == 0 {
                if state.use_key() {
                    info!("chest {} opened with a key; {} left", item.ty.name, state.keys);
                    out.sound_at("S_CHEST", item.shape.centre);
                    item.flags |= USED;
                    open_chest(items, i, out);
                } else {
                    out.hint(Hint::UseKeyOnChest);
                }
            }
            Touch::Block
        }
        ItemClass::Door => {
            if item.state != 0 || item.flags & USED != 0 {
                return Touch::Block;
            }
            // Only walking into the door opens it.
            let toward =
                (to[0] - from[0]) * (item.shape.centre[0] - from[0]) + (to[2] - from[2]) * (item.shape.centre[2] - from[2]);
            if toward < 0.0 {
                return Touch::Block;
            }
            if state.use_key() {
                info!("door {} opened with a key; {} left", item.ty.name, state.keys);
                item.flags |= USED;
                let sub = item.ty.subtype.clamp(0, 3) as usize;
                out.sound_at(DOOR_SOUNDS.get(realm).map_or("", |row| row[sub]), item.shape.centre);
                Touch::Pass
            } else {
                out.hint(Hint::UseKeyOnDoor);
                Touch::Block
            }
        }
        ItemClass::Generator => Touch::Block,
        ItemClass::Obstacle => match item.ty.subtype {
            // A rock fall, sinking rock or falling leaves is set off (with
            // its realm's sound) and walked through.
            ROCK_FALL | LEAF_FALL | ROCK_SINK => {
                if item.flags & USED == 0 {
                    item.flags |= USED;
                    let name = if item.ty.subtype == LEAF_FALL { leaf_fall_sound(realm) } else { rock_fall_sound(realm, level) };
                    if let Some(name) = name {
                        out.sounds.push(PlaySoundAt::faded(name, Vec3::from(item.position), FALL_VOLUME));
                    }
                }
                Touch::Pass
            }
            // Debris and shot-down walls are walked through.
            DEBRIS | SHOT_FALL => Touch::Pass,
            // A safe rock only while it stands.
            SAFE_ROCK if item.stage <= 0 => Touch::Pass,
            _ => Touch::Block,
        },
        ItemClass::Exit | ItemClass::Transporter => Touch::Stand,
        // A sleeping critter's statue blocks; walking into it wakes the
        // critter unless its placement's range is negative.
        ItemClass::EnemyInfo => {
            if matches!(item.params, PlacementParams::Enemy { range, .. } if range >= 0.0) {
                out.woken.push(item.placement);
            }
            Touch::Block
        }
        _ => Touch::Pass,
    }
}

/// A key opened chest `i`: it plays its opening action and lets out what
/// it holds (`LevelItems::release_contents`). A CHESTEXP releases
/// nothing: it ticks (and keeps running off screen) until it's open, then
/// explodes (`breakables.rs`).
fn open_chest(items: &mut LevelItems, i: usize, out: &mut Out) {
    let chest = &mut items.items[i];
    chest.play(1.min(chest.action_count().saturating_sub(1)));
    if chest.ty.subtype == CHEST_EXP {
        chest.flags |= ALWAYS_ACTIVE;
        out.sound_as(CHEST_EXP_TICK, CHEST_EXP_TICK_VOLUME);
        return;
    }
    items.release_contents(i);
}

/// The hero walked into open chest `chest` holding contents `placement`:
/// they're picked up like a powerup on the floor, and if they're taken
/// the chest goes with them, both after [`RELEASED_LINGER`] fields.
fn take_contents(items: &mut LevelItems, chest: usize, placement: usize, state: &mut PlayerState, out: &mut Out) {
    let doors = items.doors;
    let Some(k) = items.items.iter().position(|i| i.placement == placement && !i.gone && !i.leaving) else { return };
    let item = &mut items.items[k];
    let (sub, value, name, duration) = (item.ty.subtype, item.ty.value, item.ty.name.clone(), item.ty.duration as f32);
    let taken = pick_up(state, sub, value, &name, &mut item.amount, duration, doors, out);
    out.sparkle_at(item.shape.centre);
    if taken {
        info!("took {name} from {}", items.items[chest].ty.name);
        for j in [k, chest] {
            items.items[j].leaving = true;
            items.items[j].timer = RELEASED_LINGER;
        }
    }
}

/// Picks up a powerup of `subtype`: returns `true` if it's used up (the
/// item goes), `false` if it stays (full health, key ring or potions).
#[allow(clippy::too_many_arguments)]
fn pick_up(
    state: &mut PlayerState,
    subtype: i32,
    value: i32,
    name: &str,
    amount: &mut i32,
    duration: f32,
    doors: usize,
    out: &mut Out,
) -> bool {
    match subtype {
        // Gold.
        1 => {
            let gold = (*amount).max(0) as u32;
            state.add_gold(gold);
            out.notice(1, gold as i32);
            out.sound(gold_sound(out.realm, gold));
            if gold > 24 {
                out.hint(Hint::CollectGold);
            }
            true
        }
        // Keys: what fits on the ring; the rest stays on the floor.
        2 => {
            let want = (*amount).max(0) as u32;
            let taken = state.take_keys(want);
            if taken == 0 {
                out.hint(Hint::KeysFull);
                return false;
            }
            out.hint(if doors == 0 { Hint::UseKeyOnChest } else { Hint::SaveKeys });
            out.sound("S_PICKUPKEY");
            *amount -= taken as i32;
            // The game's plate: the keys taken, or those left when not all fit.
            out.notice(2, if taken == want { want as i32 } else { *amount });
            taken == want
        }
        // Food: refused at full health; negative food is poison, a blow of
        // kind poison on the hero (its gas mask or invulnerability stops
        // it). The Pojo can't eat chicken: to it that's 100 of poison.
        3 => {
            let pojo = state.bits.special & POJO != 0 && name.eq_ignore_ascii_case(POJO_POISON);
            let health = if pojo { POJO_POISON_AMOUNT } else { *amount as f32 };
            if health < 0.0 {
                out.poison.push(-health);
            } else if state.heal(health) == Heal::Refused {
                out.hint(Hint::HealthFull);
                return false;
            }
            match health as i32 {
                a if a >= 100 => out.hint(Hint::EatMeat),
                a if a >= 50 => out.hint(Hint::EatFruit),
                a if a < 0 => out.hint(Hint::PoisonedFood),
                _ => {}
            }
            out.voice(&eat_sound(&state.class, name, health < 0.0));
            out.notice(3, health as i32);
            true
        }
        // Potions (magic).
        4 => {
            if state.take_potions(value, (*amount).max(0) as u32) == 0 {
                out.hint(Hint::MagicFull);
                return false;
            }
            let hint = [Hint::UseMagic, Hint::ThrowMagic, Hint::MagicShield].into_iter().find(|h| !out.seen.seen(*h));
            if let Some(h) = hint {
                out.hint(h);
            }
            out.sound("S_PICKUPMAGIC");
            out.notice(4, 0);
            true
        }
        // Weapon, armour, speed, magic and special powers: one of Skorne's
        // pieces stays on the floor while the hero holds any of them.
        5..=9 => {
            let value = value as u32;
            if subtype == power::SPECIAL && value & SKORNE != 0 && state.bits.special & SKORNE != 0 {
                return false;
            }
            state.grant_power(subtype, value, *amount as f32, duration);
            if let Some(h) = Hint::for_power(subtype, value) {
                out.hint(h);
            }
            let (sound, volume) = power_sound(subtype, value);
            out.sound_as(sound, volume);
            out.notice(subtype, 0);
            true
        }
        // Runestones: one of each.
        10 => {
            if state.runestones.contains(amount) {
                return false;
            }
            state.runestones.push(*amount);
            out.sound("S_PICKUPRUNE");
            out.notice(10, *amount);
            out.sparkle(RUNE_SPARKLE);
            true
        }
        // A realm's legendary item (numbered by the realm whose boss it's
        // for): its bit, for the boss intro, and its name spoken.
        LEGENDARY => {
            if (0..16).contains(amount) {
                state.quest.legendary |= 1 << *amount;
            }
            if (1..=11).contains(amount) {
                out.hint(Hint::Legendary(*amount as u8));
            }
            out.sound("S_PICKUPMAGIC");
            out.notice(13, *amount);
            true
        }
        // A scroll shows its text: this level's, numbered from 1.
        SCROLL => {
            if let Ok(n) = usize::try_from(*amount - 1) {
                let group = out.scrolls.clone();
                out.message(&group, n, None);
            }
            true
        }
        // Gems count toward their colour's realm, gargoyle pieces toward
        // their tower section (`quest.rs`).
        GEM => {
            if let Some(c) = state.quest.add_gem(*amount) {
                info!("{} crystals: {}/{}", quest::CRYSTAL_COLOURS[c], state.quest.crystals[c], quest::CRYSTALS_NEEDED[c]);
                state.popup = Some((c as u16, out.now));
                out.sparkle(GEM_SPARKLES[c.min(8)]);
            }
            out.sound("S_PICKUPMAGIC");
            out.notice(15, *amount);
            true
        }
        GARGOYLE_PIECE => {
            if let Some(p) = state.quest.add_gargoyle(*amount) {
                info!("gargoyle pieces {p}: {}/{}", state.quest.gargoyle[p], quest::GARGOYLE_NEEDED[p]);
                state.popup = Some((0x100 + p as u16, out.now));
            }
            out.sparkle(GARGOYLE_SPARKLE);
            out.sound("S_PICKUPMAGIC");
            out.notice(16, *amount);
            true
        }
        _ => false,
    }
}

/// The sound a powerup plays and its requested volume (the growth and the
/// shrink louder).
fn power_sound(subtype: i32, value: u32) -> (&'static str, u8) {
    match subtype {
        9 if value & 1 != 0 => ("S_LEVITATEUP", CALL_VOLUME),
        9 if value & 0x100 != 0 => ("S_GROW", LOUD_POWER_VOLUME),
        9 if value & 0x200 != 0 => ("S_SHRINK", LOUD_POWER_VOLUME),
        9 if value & 0x400 != 0 => ("S_POJO", CALL_VOLUME),
        6 if value & 0x20_0000 != 0 => ("S_PICKUPSHIELD", CALL_VOLUME),
        _ => ("S_PICKUPSPECIAL", CALL_VOLUME),
    }
}
const LOUD_POWER_VOLUME: u8 = 0xB4;

/// Gold's sound: in the secret realm player 1's coin sound by the amount
/// (50 bronze, 100 silver, else gold), elsewhere `S_PICKUPMAGIC`.
fn gold_sound(realm: usize, gold: u32) -> &'static str {
    if realm != SECRET_REALM {
        return "S_PICKUPMAGIC";
    }
    match gold {
        50 => "S_PKUPBRONZE1",
        100 => "S_PKUPSILVER1",
        _ => "S_PKUPGOLD1",
    }
}
/// The secret realm's id, and its first level record (`SECRET.WAD`'s).
const SECRET_REALM: usize = 12;
const SECRET_FIRST_LEVEL: &str = "levelS1";
/// A CHESTEXP ticks at this requested volume, centred.
const CHEST_EXP_TICK_VOLUME: u8 = 0xE0;

/// The hero's eating sound: the class's eating effect, or — one time in
/// four in the game — its voice line (the archer has one per fruit).
/// Poisoned food plays the class's poisoned sound. All three are the
/// hero's own lines, which wait in the heroes' voice queue.
fn eat_sound(class: &str, food: &str, poisoned: bool) -> String {
    if !EATERS.contains(&class) {
        return String::new();
    }
    if poisoned {
        return format!("S_{class}POISON");
    }
    let voice = rand_quarter();
    match (voice, class) {
        (false, _) => format!("S_{class}EATSFX"),
        (true, "ARC") => {
            let kind = match food {
                "APPLE" => 2,
                "BANANA" => 3,
                "PINEAPPLE" => 4,
                _ => 1,
            };
            format!("S_ARCEAT{kind}")
        }
        (true, _) => format!("S_{class}EAT"),
    }
}

/// One time in four.
fn rand_quarter() -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEED: AtomicU32 = AtomicU32::new(0x1234_5678);
    let s = SEED.load(Ordering::Relaxed).wrapping_mul(1_103_515_245).wrapping_add(12345);
    SEED.store(s, Ordering::Relaxed);
    (s >> 16) & 3 == 0
}

/// Runs the items' own clocks: animations, opening doors and chests,
/// pickup delays, and freeing picked-up items.
fn update_items(items: &mut LevelItems, dt: f32, commands: &mut Commands) {
    for item in &mut items.items {
        if item.gone {
            continue;
        }
        item.advance(dt);
        if item.delay > 0 {
            item.delay -= FIELDS_PER_TICK;
        }
        if item.leaving {
            item.timer -= FIELDS_PER_TICK;
            if item.timer <= 0 {
                item.gone = true;
                // (Contents hanging in a chest go with its model.)
                if let Some(m) = item.model.take() {
                    commands.entity(m).try_despawn();
                }
            }
            continue;
        }
        match item.class() {
            // Falling obstacles fall (`fall_items`).
            ItemClass::Obstacle if falls(item.ty.subtype) => {}
            // Broken barrels and obstacles step through their actions like
            // opened doors and chests (`breakables.rs` marks them used).
            ItemClass::Door | ItemClass::Container | ItemClass::Obstacle if item.flags & USED != 0 => open_step(item),
            _ => {}
        }
        // An open chest holding nothing goes at once (the game's
        // container update): one that held a monster, or nothing; not a
        // barrel, a gold chest (it goes as its gold is taken) or a
        // CHESTEXP (it explodes).
        if item.class() == ItemClass::Container
            && item.state == 2
            && item.holds.is_none()
            && !matches!(item.ty.subtype, BARREL | GOLD_CHEST | CHEST_EXP)
        {
            debug!("open chest {} ({}) holds nothing: gone", item.placement, item.ty.name);
            item.gone = true;
            if let Some(m) = item.model.take() {
                commands.entity(m).try_despawn();
            }
        }
    }
}

/// Falling obstacles set off — touched, or shot down (`breakables.rs`
/// marks them used) — fall while they're on screen (or always active).
fn fall_items(
    items: &mut LevelItems,
    dt: f32,
    ground: Option<&LevelGround>,
    cameras: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    commands: &mut Commands,
) {
    let kill = ground.map_or(f32::MIN, |g| g.0.kill_height());
    for item in &mut items.items {
        if item.gone || item.class() != ItemClass::Obstacle || !falls(item.ty.subtype) || item.flags & USED == 0 {
            continue;
        }
        if item.flags & ALWAYS_ACTIVE == 0 && !on_screen(cameras, item.shape.centre) {
            continue;
        }
        if fall(item, dt, kill) {
            debug!("falling item {} gone below the level", item.placement);
            item.gone = true;
            if let Some(m) = item.model.take() {
                commands.entity(m).try_despawn();
            }
        }
    }
}

/// A falling item's model goes where it is now.
fn place_falling(items: Res<LevelItems>, mut models: Query<&mut Transform>) {
    for item in items.items.iter().filter(|i| i.falling.is_some() && !i.gone) {
        let Some(mut t) = item.model.and_then(|m| models.get_mut(m).ok()) else { continue };
        t.translation = Vec3::from(item.position);
        t.rotation = Quat::from_mat3(&Mat3::from_cols_array(&item.rotation));
    }
}

/// How big contents hanging in a chest are by how far it has opened (the
/// game's container update), from the chest's action, its frame and that
/// action's length: a fifth of their size till it starts opening, then
/// 0.2 + 0.8 × (frame + 1) / frames through its opening action, and their
/// own size once it's open.
fn contents_scale(action: usize, frame: f32, frames: u16) -> f32 {
    match action {
        0 => CONTENTS_SMALL,
        1 if frames >= 2 => (CONTENTS_SMALL + (1.0 - CONTENTS_SMALL) * (frame + 1.0) / f32::from(frames)).min(1.0),
        _ => 1.0,
    }
}

/// What a chest lets out hangs on its model's contents node (`NULL1`),
/// sized by how far it has opened ([`contents_scale`]), till it's taken;
/// and a silver chest holding gold takes the gold-filled chest's model as
/// it opens.
fn show_contents(
    mut commands: Commands,
    mut items: ResMut<LevelItems>,
    contents: Option<Res<ContentModels>>,
    rigs: Query<&ItemRig>,
    mut transforms: Query<&mut Transform>,
) {
    for (placement, name) in std::mem::take(&mut items.swaps) {
        let Some(models) = contents.as_ref() else { continue };
        let Some(pose) = items.find(placement).map(|i| Transform {
            translation: Vec3::from(i.position),
            rotation: Quat::from_mat3(&Mat3::from_cols_array(&i.rotation)),
            ..default()
        }) else {
            continue;
        };
        if models.spawn(name, pose, placement, &mut commands).is_none() {
            debug!("no {name} model in this level");
            continue;
        }
        if let Some(old) = items.take_model(placement) {
            commands.entity(old).try_despawn();
        }
    }
    for k in 0..items.items.len() {
        let item = &items.items[k];
        if item.gone {
            continue;
        }
        let Some(chest) = item.inside.and_then(|c| items.find(c)) else { continue };
        let frames = chest.atree.as_ref().and_then(|a| a.actions.get(chest.action)).map_or(0, |a| a.frames);
        let scale = Vec3::splat(contents_scale(chest.action, chest.frame, frames));
        if let Some(m) = item.model {
            if let Ok(mut t) = transforms.get_mut(m) {
                t.scale = scale;
            }
            continue;
        }
        if item.shown {
            continue;
        }
        let (Some(models), Some(bone)) =
            (contents.as_ref(), chest.model.and_then(|m| rigs.get(m).ok()).and_then(crate::population::contents_bone))
        else {
            continue;
        };
        if let Some(e) = models.spawn(&item.ty.name, Transform::from_scale(scale), item.placement, &mut commands) {
            commands.entity(e).insert(ChildOf(bone));
        }
        items.items[k].shown = true;
    }
}

/// An opened door or chest steps through its actions: the opening action,
/// then the open one, where it stays (its state follows the action that
/// has finished). Stand-in: the game waits a timer between the two whose
/// length comes from undecoded animation fields; here it's none. Without
/// an atree a door just stops blocking.
fn open_step(item: &mut Item) {
    let count = item.action_count();
    if count < 2 {
        item.state = 2;
        if item.class() == ItemClass::Door {
            item.leaving = true;
            item.timer = 0;
        }
        return;
    }
    if item.action == 0 {
        item.play(1);
    } else if item.done {
        item.state = item.state.max(item.action);
        if item.action + 1 < count {
            let next = item.action + 1;
            item.play(next);
        }
    }
}

/// Where an exit takes the heroes (`docs/items.md`, "Exits"): the tower's
/// portals to the level their code names; a secret exit to that level of
/// the secret realm; every other exit back to the tower — but E1 and F1
/// on to E2 and F2, and a secret-realm level on to the next.
fn exit_goes_to(realm: usize, level: usize, secret: bool, code: Option<&str>) -> Option<String> {
    let letter = |realm: usize| REALM_LETTERS.iter().find(|(_, id)| *id as usize == realm).map(|(l, _)| *l);
    if secret {
        let (_, level) = exit_destination(code?)?;
        return Some(format!("level{}{}", letter(SECRET_REALM)?, level + 1));
    }
    if realm == crate::quest::TOWER as usize {
        return level_for_code(code?);
    }
    match (realm, level) {
        (SECRET_REALM, _) | (REALM_E | REALM_F, 0) => Some(format!("level{}{}", letter(realm)?, level + 2)),
        _ => Some(crate::frontend::TOWER.to_string()),
    }
}
const REALM_E: usize = 5;
const REALM_F: usize = 6;

/// Exits the hero stands in this tick: the portal steps through its
/// actions while the hero stays, and when the last one has played the hero
/// goes out (`going_out.rs`) to where the exit takes it ([`exit_goes_to`]),
/// `S_TUNNEL` panned at its feet. Secret exits go at once. Whether the hero
/// stands in an open exit (not a secret one: those have no flame).
fn exits(items: &mut LevelItems, on_exit: &[usize], feet: [f32; 3], out: &mut Out) -> bool {
    let (realm, level) = (items.realm, items.level);
    let mut go = None;
    let mut standing = false;
    for (i, item) in items.items.iter_mut().enumerate() {
        if item.class() != ItemClass::Exit || item.gone || item.flags & CLOSED != 0 {
            continue;
        }
        let here = on_exit.contains(&i);
        let secret = item.ty.subtype == 0x32;
        if !here {
            if item.action != 0 && !secret {
                item.play(0);
                item.state = 0;
            }
            continue;
        }
        standing |= !secret;
        let code = match &item.params {
            PlacementParams::Exit { destination } => destination.as_deref(),
            _ => None,
        };
        let dest = exit_goes_to(realm, level, secret, code);
        if secret {
            item.flags |= USED;
            go = dest.map(|to| (to, true));
            break;
        }
        let last = item.action_count().saturating_sub(1).min(4);
        if item.action == 0 {
            item.play(1.min(last));
        } else if item.done && item.action < last {
            let next = item.action + 1;
            item.play(next);
        } else if item.done || last == 0 {
            item.flags |= USED;
            go = dest.map(|to| (to, false));
            break;
        }
    }
    if let Some((to, secret)) = go {
        // Going out takes 50 fields before the level changes (a secret
        // exit none); the first hero out goes with the tunnel's sound.
        let fields = if secret { 0 } else { crate::going_out::FIELDS };
        items.leaving = Some(Leaving { to, fields, secret });
        out.sounds.push(PlaySoundAt::panned(TUNNEL_SOUND, Vec3::from(feet), CALL_VOLUME));
    }
    standing
}

/// Going out through an exit.
const TUNNEL_SOUND: &str = "S_TUNNEL";
/// The exit's flame: a loop at the hero standing in an exit (its top
/// point, 4.4 above its feet), louder than its call.
const EXIT_FLAME: &str = "S_EXITFLAME";
const EXIT_FLAME_CHANNEL: &str = "exit_flame";
const EXIT_FLAME_VOLUME: u8 = 0xE0;
const HERO_TOP: f32 = 4.4;

/// The exit's flame burns while the hero stands in an open exit, and as it
/// goes out through one.
fn exit_flame(items: &mut LevelItems, feet: Option<Vec3>, loops: &mut MessageWriter<LoopSoundAt>) {
    let burning = items.in_exit || items.leaving.as_ref().is_some_and(|l| !l.secret);
    match feet.filter(|_| burning) {
        Some(feet) => {
            loops.write(LoopSoundAt::at(EXIT_FLAME_CHANNEL, EXIT_FLAME, feet + Vec3::Y * HERO_TOP, EXIT_FLAME_VOLUME));
        }
        None if items.flame => {
            loops.write(LoopSoundAt::stop(EXIT_FLAME_CHANNEL));
        }
        None => {}
    }
    items.flame = burning;
}

/// The placed sound items' loops: on while the hero is near (full within
/// the item's reach, fading out to half again as far), at 224 × that —
/// 16 while time stands still or a cut is on; four times louder (64 to
/// 255) on the secret realm's first level — following the volume as the
/// hero moves, and all stopped once the hero goes out.
fn ambient_sounds(items: &mut LevelItems, feet: Option<Vec3>, quiet: bool, loops: &mut MessageWriter<LoopSoundAt>) {
    let gone_out = items.leaving.is_some();
    let secret_first = items.secret_first;
    for (i, a) in items.ambient.iter_mut().enumerate() {
        let distance = feet.map_or(NO_HERO, |f| f.distance(a.at));
        let near = ambient_near(distance, a.reach);
        if near > 0.0 && !gone_out {
            let volume = ambient_volume(near, quiet, secret_first);
            loops.write(LoopSoundAt::at(AMBIENT_CHANNEL, a.name.clone(), a.at, volume).slot(i as u32).follow_volume());
            a.playing = true;
        } else if a.playing {
            loops.write(LoopSoundAt::stop(AMBIENT_CHANNEL).slot(i as u32));
            a.playing = false;
        }
    }
}
const AMBIENT_CHANNEL: &str = "ambient";
/// The distance taken with no hero in play.
const NO_HERO: f32 = 1000.0;

/// How near the hero is for an ambient sound reaching `reach`: 1 within
/// it (or for a reach of 2 or less), falling to 0 at 1.5 × the reach.
fn ambient_near(distance: f32, reach: f32) -> f32 {
    if distance < reach || reach <= 2.0 { 1.0 } else { 2.0 * (1.5 * reach - distance) / reach }
}

/// An ambient sound's requested volume at nearness `near` (the level
/// record's scale, 1 on every level, left out).
fn ambient_volume(near: f32, quiet: bool, secret_first: bool) -> u8 {
    let mut v = if quiet { 16 } else { (224.0 * near).max(0.0) as i32 };
    if secret_first {
        v = (v * 4).clamp(64, 255);
    }
    v.clamp(0, 255) as u8
}

/// The hero stands on transporter `t`: if its partner is on screen and
/// there's floor there, the transport starts.
fn start_transport(
    items: &mut LevelItems,
    t: usize,
    radius: f32,
    ground: Option<&LevelCollision>,
    cameras: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    out: &mut Out,
) {
    if items.transport_cooldown > 0 {
        items.transport_cooldown = 1;
        return;
    }
    let PlacementParams::Transporter { destination, .. } = items.items[t].params else { return };
    let Some(partner) = items.items.iter().position(|j| {
        !j.gone && matches!(j.params, PlacementParams::Transporter { id, .. } if id == destination)
    }) else {
        return;
    };
    let p = &items.items[partner];
    if !on_screen(cameras, p.shape.centre) {
        return;
    }
    let mut to = [p.shape.centre[0], p.shape.centre[1] - 1.0, p.shape.centre[2]];
    let Some(floor) = ground.and_then(|g| g.floor_probe(to, 4.0, -10.0, radius, 0)) else { return };
    to[1] = floor.point[1];
    // Faded at the transporter gone to (stand-in: where the hero lands,
    // under its centre, for its place).
    out.sound_at(TRANSPORT_SOUNDS.get(items.realm).copied().unwrap_or(""), to);
    items.transport = Some(Transport { to, fields: TRANSPORT_FIELDS });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(kind: u16) -> Shape {
        Shape {
            kind,
            radius: 1.0,
            reach: 2.0,
            half: [3.0, 1.0],
            centre: [0.0, 1.0, 0.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    fn obstacle(subtype: i32) -> ItemType {
        ItemType {
            class: ItemClass::Obstacle,
            subtype,
            name: String::new(),
            choices: Vec::new(),
            extent: [15.0, 20.0, 0.0, 0.0],
            center_offset: [0.0; 3],
            value: 0,
            amount: 0,
            armor: -1,
            hit_points: 0,
            flags: 0,
            duration: 0,
            raw: [0; 0x50],
        }
    }

    #[test]
    fn a_rock_fall_spins_and_drops_out_of_the_level() {
        let mut items = LevelItems { realm: 2, ..default() };
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let placement = items.release(obstacle(ROCK_FALL), [5.0, 30.0, -2.0], identity, None, 0);
        let item = items.items.iter_mut().find(|i| i.placement == placement).unwrap();
        // Two units a second slower each update, from rest: after a second
        // (30 updates) it's going 60 down and has fallen 31.
        for _ in 0..30 {
            assert!(!fall(item, 1.0 / 30.0, -10.0));
        }
        assert_eq!(item.falling.map(|v| v[1]), Some(-60.0));
        assert!((item.position[1] - (30.0 - 31.0)).abs() < 1e-3, "{:?}", item.position);
        assert_eq!((item.position[0], item.position[2]), (5.0, -2.0));
        // Spinning about its X and Z by its slot's steps (20° a second each),
        // its matrix still a rotation; its touch centre follows it.
        let r = item.rotation;
        let det = r[0] * (r[4] * r[8] - r[5] * r[7]) - r[1] * (r[3] * r[8] - r[5] * r[6]) + r[2] * (r[3] * r[7] - r[4] * r[6]);
        assert!((det - 1.0).abs() < 1e-4 && r != identity, "{r:?}");
        // (Its centre is 1 up the item's own Y: row 1 of its matrix.)
        assert!((0..3).all(|k| (item.shape.centre[k] - (item.position[k] + r[3 + k])).abs() < 1e-4), "{:?}", item.shape.centre);
        // Gone once 200 below the kill height.
        let mut n = 0;
        while !fall(item, 1.0 / 30.0, -10.0) {
            n += 1;
            assert!(n < 300);
        }
        assert!(item.position[1] < -210.0);
    }

    #[test]
    fn first_clips_go_round_until_the_action_changes() {
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let mut items = LevelItems::default();
        let key = ItemType { class: ItemClass::Powerup, subtype: 2, ..obstacle(0) };
        let k = items.release(key, [0.0; 3], identity, None, 0);
        let item = items.items.iter_mut().find(|i| i.placement == k).unwrap();
        // The key's ACTIVE has no loop flag, but it's the clip the build
        // started: round it goes, till another action starts.
        assert!(item.loops(false));
        item.play(1);
        assert!(!item.loops(false) && item.loops(true));
        // A pad never goes round; an exit's waiting and open actions do.
        let pad = ItemType { class: ItemClass::Trigger, ..obstacle(0x18) };
        let p = items.release(pad, [0.0; 3], identity, None, 0);
        assert!(!items.items.iter().find(|i| i.placement == p).unwrap().loops(true));
        let exit = ItemType { class: ItemClass::Exit, ..obstacle(0) };
        let e = items.release(exit, [0.0; 3], identity, None, 0);
        let item = items.items.iter_mut().find(|i| i.placement == e).unwrap();
        let by_action: Vec<bool> = (0..5)
            .map(|a| {
                item.play(a);
                item.loops(false)
            })
            .collect();
        assert_eq!(by_action, [true, true, false, true, false]);
    }

    /// What the touch handler reports to (nothing seen yet).
    fn out(seen: &Hints) -> Out<'_> {
        Out {
            sounds: Vec::new(),
            voices: Vec::new(),
            poison: Vec::new(),
            hints: Vec::new(),
            messages: Vec::new(),
            sparkle: None,
            effects: Vec::new(),
            woken: Vec::new(),
            notices: Vec::new(),
            seen,
            scrolls: String::new(),
            now: 0.0,
            realm: 1,
        }
    }

    /// A locked chest of `subtype` holding `contents` (`count` of them) at
    /// the origin, and its index.
    fn chest(items: &mut LevelItems, subtype: i32, contents: ItemType, count: i16) -> usize {
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let ty = ItemType { class: ItemClass::Container, flags: LOCKED | 6, extent: [3.9, 2.0, 1.2, 1.0], ..obstacle(subtype) };
        let p = items.release(ty, [0.0; 3], identity, None, 0);
        let i = items.items.iter().position(|i| i.placement == p).unwrap();
        let c = &mut items.items[i];
        c.contents = Some(contents);
        c.params = PlacementParams::Container { contents: Some(0), param: count };
        c.hangs = true;
        i
    }

    fn powerup(subtype: i32, name: &str, amount: i16) -> ItemType {
        ItemType { class: ItemClass::Powerup, name: name.into(), amount, extent: [0.5, 2.0, 0.0, 0.0], ..obstacle(subtype) }
    }

    #[test]
    fn a_chest_lets_out_its_contents_and_goes_with_them() {
        let seen = Hints::default();
        let mut out = out(&seen);
        let mut state = PlayerState::default();
        let mut items = LevelItems::default();
        let c = chest(&mut items, 0x2E, powerup(GOLD, "TREAS_GOLD", 200), 0);
        // A key opens it: the gold hangs in it, out of reach on its own,
        // and can't be picked up for 30 fields.
        open_chest(&mut items, c, &mut out);
        let held = items.items[c].holds.expect("the chest holds what came out");
        let g = items.items.iter().position(|i| i.placement == held).unwrap();
        assert_eq!(items.items[g].inside, Some(items.items[c].placement));
        assert_eq!((items.items[g].amount, items.items[g].delay), (200, CONTENTS_DELAY));
        assert!(!LevelItems::view_of(&items.items[g]).live);
        // Shut, it blocks; open, the hero walks in and takes the gold, and
        // the chest goes with it, both after 15 fields.
        let mut picked = false;
        let at = items.items[c].shape.centre;
        assert!(matches!(touch(&mut items, c, &mut state, at, at, &mut picked, &mut out), Touch::Block));
        items.items[c].state = 2;
        assert!(matches!(touch(&mut items, c, &mut state, at, at, &mut picked, &mut out), Touch::Pass));
        assert_eq!(state.gold, 200);
        for i in [c, g] {
            assert!(items.items[i].leaving && items.items[i].timer == RELEASED_LINGER, "{i}");
        }
    }

    #[test]
    fn contents_grow_as_the_chest_opens() {
        assert_eq!(contents_scale(0, 0.0, 0), CONTENTS_SMALL);
        // Through the 50-frame opening: 0.2 + 0.8 × (frame + 1) / 50.
        assert!((contents_scale(1, 0.0, 50) - 0.216).abs() < 1e-5);
        assert!((contents_scale(1, 24.0, 50) - 0.6).abs() < 1e-5);
        assert_eq!(contents_scale(1, 49.0, 50), 1.0);
        assert_eq!(contents_scale(2, 0.0, 0), 1.0);
    }

    #[test]
    fn chests_by_what_they_hold() {
        let seen = Hints::default();
        let mut out = out(&seen);
        let gold_chest = ItemType { class: ItemClass::Container, name: "CHESTG1".into(), ..obstacle(GOLD_CHEST) };
        let key_ring = powerup(KEY, KEY_RING, 1);
        let mut items = LevelItems { gold_chest: Some(gold_chest), key_ring: Some(key_ring), ..default() };
        // A silver chest holding gold becomes a gold chest, its model the
        // gold-filled silver chest; the gold stays in it.
        let s = chest(&mut items, SILVER_CHEST, powerup(GOLD, "TREAS_JUNK", 10), 0);
        open_chest(&mut items, s, &mut out);
        let p = items.items[s].placement;
        assert_eq!((items.items[s].ty.subtype, items.items[s].amount, items.items[s].holds), (GOLD_CHEST, 10, None));
        assert_eq!(items.swaps, [(p, crate::population::SILVER_GOLD_CHEST)]);
        // More than one key comes as the key ring, as many as it says.
        let k = chest(&mut items, 0x2E, powerup(KEY, "KEY", 1), 3);
        open_chest(&mut items, k, &mut out);
        let held = items.items[k].holds.and_then(|h| items.find(h)).expect("keys hang in it");
        assert_eq!((held.ty.name.as_str(), held.amount), (KEY_RING, 3));
        // A Death is let out where the chest stands, and the open chest,
        // holding nothing, goes at once.
        let death = ItemType { class: ItemClass::EnemyInfo, name: "DEATH".into(), ..obstacle(0) };
        let d = chest(&mut items, 0x2E, death, 0);
        open_chest(&mut items, d, &mut out);
        assert_eq!(items.take_let_out().len(), 1);
        items.items[d].flags |= USED;
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let world = World::new();
        let mut commands = Commands::new(&mut queue, &world);
        update_items(&mut items, 1.0 / 30.0, &mut commands);
        assert!(items.items[d].gone);
        // A gold chest stays open with its gold; a chest without a node to
        // hang them on lets its contents out where it stands.
        assert!(!items.items[s].gone);
        let n = chest(&mut items, 0x2E, powerup(3, "HAM", 200), 0);
        items.items[n].hangs = false;
        let before = items.items.len();
        open_chest(&mut items, n, &mut out);
        assert_eq!((items.items.len(), items.items[n].holds), (before + 1, None));
        assert_eq!(items.items.last().map(|i| i.inside), Some(None));
    }

    #[test]
    fn falls_by_kind_and_their_sounds() {
        assert_eq!(fall_rates(LEAF_FALL), (1.0, 10f32.to_radians()));
        assert_eq!(fall_rates(ROCK_SINK), (2.0, 1f32.to_radians()));
        assert_eq!(fall_rates(SHOT_FALL), (2.0, 20f32.to_radians()));
        assert!(falls(ROCK_FALL) && falls(SHOT_FALL) && !falls(DEBRIS) && !falls(SAFE_ROCK));
        assert_eq!(rock_fall_sound(1, 0), Some("S_FALLAWAY"));
        assert_eq!(rock_fall_sound(6, 0), Some("S_ROCKBREAKF"));
        assert_eq!(rock_fall_sound(6, 1), Some("S_ROCKBREAKF2"));
        assert_eq!(rock_fall_sound(9, 4), Some("S_ICEBREAKY"));
        assert_eq!(rock_fall_sound(10, 0), None);
        assert_eq!(leaf_fall_sound(4), Some("S_LEAFBREAK"));
        assert_eq!(leaf_fall_sound(1), None);
    }

    #[test]
    fn cylinder_touch_and_slide() {
        let s = shape(1);
        // Out of reach, then just in reach walking in.
        assert!(contact(&s, false, [-3.0, 0.0, 0.0], [-2.6, 0.0, 0.0], 1.5, 2.5).is_none());
        let c = contact(&s, false, [-2.6, 0.0, 0.1], [-2.4, 0.0, 0.1], 1.5, 2.5).unwrap();
        assert!((c.clearance - (2.4f32.hypot(0.1) - 1.0)).abs() < 1e-5);
        // Walking straight at it slides (almost) nowhere; walking away is
        // no touch at all.
        assert!(c.out[0] < -2.5);
        assert!(contact(&s, false, [-2.4, 0.0, 0.0], [-2.5, 0.0, 0.0], 1.5, 2.5).is_none());
        // Too far above.
        assert!(contact(&s, false, [-2.0, 6.0, 0.0], [-1.9, 6.0, 0.0], 1.5, 2.5).is_none());
        // Pads and exits are stood in, never pushed back.
        let c = contact(&s, true, [-2.4, 0.0, 0.0], [-2.5, 0.0, 0.0], 1.5, 2.5).unwrap();
        assert_eq!(c.out, [-2.5, 0.0, 0.0]);
    }

    #[test]
    fn box_pushes_out_the_short_way() {
        // A door 6 wide (X) and 2 deep (Z), reach 4 around its centre.
        let s = Shape { radius: 4.0, ..shape(3) };
        // Walking into its face along +Z is pushed back out along -Z.
        let c = contact(&s, false, [0.5, 0.0, -2.6], [0.5, 0.0, -2.4], 1.5, 2.5).unwrap();
        assert!((c.out[2] - -2.5).abs() < 1e-5 && (c.out[0] - 0.5).abs() < 1e-5, "{c:?}");
        // Beside the box: no touch though inside the radius.
        assert!(contact(&s, false, [4.6, 0.0, -2.4], [4.6, 0.0, -2.3], 1.5, 2.5).is_none());
    }

    #[test]
    fn pickup_and_ambient_sounds() {
        assert_eq!(power_sound(9, 0x100), ("S_GROW", 0xB4));
        assert_eq!(power_sound(9, 1), ("S_LEVITATEUP", 0x7F));
        assert_eq!(gold_sound(12, 50), "S_PKUPBRONZE1");
        assert_eq!(gold_sound(12, 100), "S_PKUPSILVER1");
        assert_eq!(gold_sound(12, 25), "S_PKUPGOLD1");
        assert_eq!(gold_sound(1, 50), "S_PICKUPMAGIC");
        // Full within the reach, gone at 1.5 × it.
        assert_eq!(ambient_near(10.0, 20.0), 1.0);
        assert_eq!(ambient_near(25.0, 20.0), 0.5);
        assert!(ambient_near(31.0, 20.0) < 0.0);
        assert_eq!(ambient_near(100.0, 2.0), 1.0);
        assert_eq!(ambient_volume(1.0, false, false), 224);
        assert_eq!(ambient_volume(0.5, false, false), 112);
        assert_eq!(ambient_volume(1.0, true, false), 16);
        assert_eq!(ambient_volume(0.05, false, true), 64);
        assert_eq!(ambient_volume(1.0, false, true), 255);
        let mut params = [0u8; 12];
        params[0..4].copy_from_slice(&15.0f32.to_le_bytes());
        let a = Ambient::of("s_waterfall", &params, Vec3::ZERO).unwrap();
        assert_eq!((a.name.as_str(), a.reach), ("S_WATERFALL", 15.0));
        params[4] = 1;
        assert!(Ambient::of("S_WATERFALL", &params, Vec3::ZERO).is_none());
    }

    /// A collision triangle with corners `a`, `b`, `c` (counter-clockwise
    /// round `normal`), its corners in the plane frame as the file has them.
    fn triangle(normal: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> CollisionTriangle {
        let mut t = CollisionTriangle { height_range: [i16::MIN, i16::MAX], frame_scale: 1.0, normal, origin: a, corners: [[0; 2]; 2] };
        for (k, v) in [b, c].iter().enumerate() {
            let p = t.to_plane([v[0] - a[0], v[1] - a[1], v[2] - a[2]]);
            t.corners[k] = [(p[0] * 64.0).round() as i16, (p[2] * 64.0).round() as i16];
        }
        t
    }

    #[test]
    fn secret_walls_block() {
        // A wall 10 wide and 10 high across z = 0, facing +Z, standing at
        // (0, 0, 20) unturned.
        let n = [0.0, 0.0, 1.0];
        let triangles = vec![
            triangle(n, [-5.0, 0.0, 0.0], [5.0, 0.0, 0.0], [5.0, 10.0, 0.0]),
            triangle(n, [-5.0, 0.0, 0.0], [5.0, 10.0, 0.0], [-5.0, 10.0, 0.0]),
        ];
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let w = Wall { triangles: triangles.into(), position: [0.0, 0.0, 20.0], rotation: identity };
        // Walking into it: pushed back out to a radius from it.
        let c = wall_contact(&w, [0.0, 0.0, 23.0], [0.0, 0.0, 21.0], 1.5).expect("a contact");
        assert!((c.out[2] - 21.5).abs() < 1e-3 && c.out[0].abs() < 1e-3, "{c:?}");
        // Out of reach, past its side, or walking away: none.
        assert!(wall_contact(&w, [0.0, 0.0, 25.0], [0.0, 0.0, 24.0], 1.5).is_none());
        assert!(wall_contact(&w, [9.0, 0.0, 23.0], [9.0, 0.0, 21.0], 1.5).is_none());
        assert!(wall_contact(&w, [0.0, 0.0, 21.0], [0.0, 0.0, 23.0], 1.5).is_none());
        // Turned half round (facing −Z), it blocks from the other side.
        let turned = [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0];
        let w = Wall { rotation: turned, ..w };
        let c = wall_contact(&w, [0.0, 0.0, 17.0], [0.0, 0.0, 19.0], 1.5).expect("a contact");
        assert!((c.out[2] - 18.5).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn sphere_reach_is_3d() {
        // A sphere at height 1 reaching 2.5 (its radius and the hero's)
        // touches the hero's collision centre — 2.5 above its feet — 2
        // above it (feet at 0.5), not 3 (feet at 1.5).
        let s = shape(2);
        assert!(contact(&s, true, [0.0, 0.5, 0.0], [0.0, 0.5, 0.0], 1.5, 2.5).is_some());
        assert!(contact(&s, true, [0.0, 1.5, 0.0], [0.0, 1.5, 0.0], 1.5, 2.5).is_none());
    }

    #[test]
    fn touches_reach_from_the_collision_centre() {
        // A pad's centre 2.4 below the hero's feet and 7.9 above them are
        // both within its reach of 3 and the hero's 2.5 from 2.5 up: the
        // window is 3 below the feet to 8 above.
        let s = Shape { reach: 3.0, ..shape(1) };
        let at = |feet: f32| contact(&s, true, [0.0, feet, 0.0], [0.0, feet, 0.0], 1.5, 2.5).is_some();
        assert!(at(1.0 + 2.4) && at(1.0 - 7.9));
        assert!(!at(1.0 + 3.1) && !at(1.0 - 8.1));
    }
}
