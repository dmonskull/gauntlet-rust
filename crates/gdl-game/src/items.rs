//! What the hero touches: pickups, doors, locked chests, exits and
//! transporters, run the way the game's item code does it
//! (`docs/items.md`), plus the item models' own animation (powerups spin
//! and bob through their atree's looping `ACTIVE` action; doors and chests
//! play their open actions; a transporter's swirl and a force field's
//! glow are textures its actions change).
//!
//! Each tick, after the player has moved: every live item the hero
//! reaches is touched (the item type's shape and extents against the
//! hero's radius and height), blocking items push the hero back out,
//! and at most one powerup is picked up.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::anim::{Atree, Track, rotation_matrix as pose_matrix};
use gdl_formats::texmod::TexModKind;
use gdl_formats::population::{
    ItemClass, ItemType, PlacementParams, REALM_LETTERS, level_for_code, rotation_matrix,
};
use gdl_formats::{LevelCollision, MoveParams};

use crate::audio::{PlaySound, QueueVoice};
use crate::combat::hit_kind;
use crate::character;
use crate::effects::EffectAt;
use crate::exits::ChangeLevelTo;
use crate::hints::{Hint, Hints, ShowHint};
use crate::message_box::ShowMessage;
use crate::level_material::LevelMaterial;
use crate::player::{Player, PlayerTick};
use crate::player_state::{DamagePlayer, FIELDS_PER_TICK, Heal, PlayerState};
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
                (build_items.run_if(resource_exists_and_changed::<LevelPopulation>), attach_models, pose_items, show_rocks)
                    .chain(),
            );
    }
}

// Item flags (the record's `+0xC4`, starting from the type's `+0x46`).
/// Activated: a door or chest opened, an exit in use.
pub const USED: u16 = 0x1;
/// Collides even off screen (doors, exits, placements with flag bit 0).
pub const ALWAYS_ACTIVE: u16 = 0x40;
/// A locked container: a key opens it on touch.
const LOCKED: u16 = 0x10;
/// An exit the quest hasn't opened: it shows `EXIT_OFF` and goes nowhere.
pub const CLOSED: u16 = 0x8000;

/// The Pojo's special bit, and the food that poisons it (`CHICKEN`: 100).
const POJO: u32 = 0x400;
const POJO_POISON: &str = "CHICKEN";
const POJO_POISON_AMOUNT: f32 = -100.0;

/// The container that explodes once opened (CHESTEXP), and the tick it
/// plays as it's set off.
pub const CHEST_EXP: i32 = 0x2C;
pub const CHEST_EXP_TICK: &str = "S_TICKY";

/// Powerup subtypes of the quest's pieces.
const LEGENDARY: i32 = 13;
const SCROLL: i32 = 14;
const GEM: i32 = 15;
const GARGOYLE_PIECE: i32 = 16;
/// Obstacle subtype: a boss level's safe rock, drawn by its stage
/// (`SAFEROCK3`…`SAFEROCK0`).
pub const SAFE_ROCK: i32 = 0x29;

/// Fields a picked-up item lingers before it's freed: 8, or 15 when a
/// player dropped it.
const PICKUP_LINGER: i32 = 8;
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
    /// 0 none, 1 upright cylinder, 2 sphere, 3 box, 4 the obstacle test
    /// (walls; not ported).
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
    let (dx, dy, dz) = (to[0] - s.centre[0], to[1] - s.centre[1], to[2] - s.centre[2]);
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
    /// Picked up or opened for good: lingering `timer` fields, then gone.
    leaving: bool,
    gone: bool,
    atree: Option<Arc<Atree>>,
    model: Option<Entity>,
    /// A container's contents, resolved.
    contents: Option<ItemType>,
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

    /// Starts `action` of the item's atree from its first frame.
    fn play(&mut self, action: usize) {
        self.action = action;
        self.frame = 0.0;
        self.done = false;
    }

    /// Advances the current action by `dt` seconds at its own rate.
    fn advance(&mut self, dt: f32) {
        let Some(a) = self.atree.as_ref().and_then(|t| t.actions.get(self.action)) else {
            self.done = true;
            return;
        };
        // A looping action finishing a cycle counts as its end for whatever
        // waits on it.
        let wrapped = character::advance_clip(&mut self.frame, dt, a.frames, a.rate, a.loops());
        if wrapped || (!a.loops() && self.frame >= character::clip_end(a.frames)) {
            self.done = true;
        }
    }

    fn action_count(&self) -> usize {
        self.atree.as_ref().map_or(0, |t| t.actions.len())
    }
}

/// An exit the hero is taking: fields left before the level changes.
struct Leaving {
    to: String,
    fields: i32,
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
    doors: usize,
    /// The hero's feet at the end of the last tick.
    last_feet: Option<[f32; 3]>,
    /// The transporter touched this tick, and the cooldown until the next
    /// one works (set on arrival, cleared by stepping off).
    transport: Option<Transport>,
    transport_cooldown: i32,
    leaving: Option<Leaving>,
    /// Items released so far this level (container contents).
    released: usize,
    /// The level's scroll texts (`SCROLLSA1`).
    scrolls: String,
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
    /// What it holds (containers).
    pub contents: Option<&'a ItemType>,
    /// Its model, while it has one.
    pub model: Option<Entity>,
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
            live: !item.gone && !item.leaving,
            contents: item.contents.as_ref(),
            model: item.model,
        }
    }

    pub fn view(&self, placement: usize) -> Option<ItemView<'_>> {
        self.find(placement).map(Self::view_of)
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

    /// Moves the item's touch shape to `centre` (a pad riding its lift);
    /// returns how far it moved and its model, for the caller to move too.
    pub fn move_centre(&mut self, placement: usize, centre: [f32; 3]) -> Option<([f32; 3], Option<Entity>)> {
        let i = self.find_mut(placement)?;
        let old = i.shape.centre;
        i.shape.centre = centre;
        Some(([centre[0] - old[0], centre[1] - old[1], centre[2] - old[2]], i.model))
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

    /// Frees the item at once, model and all (the game's `+0xC4 = 0xFFFF`).
    pub fn free(&mut self, placement: usize, commands: &mut Commands) {
        if let Some(i) = self.find_mut(placement) {
            i.gone = true;
            if let Some(m) = i.model.take() {
                commands.entity(m).try_despawn();
            }
        }
    }

    /// A new item of type `ty` standing at `position` (turned by the
    /// placement-style `rotation`), the way the game releases a container's
    /// contents: `amount` overrides the type's (keys take the container's
    /// count), and it can't be picked up for `delay` fields. Returns its
    /// placement number; its model, if one was built for the level, is
    /// spawned by the caller with it ([`crate::population::ItemModels`]).
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

    pub fn release(&mut self, ty: ItemType, position: [f32; 3], rotation: [f32; 9], amount: Option<i32>, delay: i32) -> usize {
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
            leaving: false,
            gone: false,
            atree: None,
            model: None,
            contents: None,
        });
        placement
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
    state: Option<Res<PlayerState>>,
) {
    let pop = &population.population;
    let realm = population
        .level
        .strip_prefix("level")
        .and_then(|s| s.chars().next())
        .and_then(|c| REALM_LETTERS.iter().find(|(l, _)| l.eq_ignore_ascii_case(&c)))
        .map_or(0, |(_, id)| *id as usize);
    let mut out = Vec::new();
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
        let rotation = rotation_matrix(placement.rotation);
        let mut position = placement.position;
        // Items with a model are dropped to the floor (+0.1) unless their
        // type keeps its height — the same as the models are placed.
        if placement.flags & 2 == 0
            && !ty.keeps_height()
            && let Some(y) = ground.as_ref().and_then(|g| g.0.floor_height(position))
        {
            position[1] = y + 0.1;
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
        match (ty.class, &params) {
            (ItemClass::Exit, _) => {
                flags = (flags & !USED) | ALWAYS_ACTIVE;
                // An exit the quest hasn't opened is shut (`quest.rs`).
                if let (PlacementParams::Exit { destination: Some(code) }, Some(state)) = (&params, &state)
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
            _ => {}
        }
        out.push(Item {
            placement: index,
            shape: Shape::of(&ty, position, rotation),
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
            leaving: false,
            gone: false,
            atree: None,
            model: None,
            contents,
        });
    }
    let doors = out.iter().filter(|i| i.class() == ItemClass::Door).count();
    let shut = out.iter().filter(|i| i.flags & CLOSED != 0).count();
    info!("{}: {} items in play ({doors} doors, {shut} exits shut)", population.level, out.len());
    let scrolls = format!("SCROLLS{}", population.level.strip_prefix("level").unwrap_or_default().to_ascii_uppercase());
    *items = LevelItems { items: out, realm, doors, scrolls, ..default() };
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

/// A safe rock not made yet (stage −1) isn't drawn: its parts are hidden
/// (the model itself follows the population view).
fn show_rocks(items: Res<LevelItems>, children: Query<&Children>, mut parts: Query<&mut Visibility>) {
    for item in items.items.iter().filter(|i| i.is_safe_rock()) {
        let Some(model) = item.model else { continue };
        let want = if item.stage < 0 { Visibility::Hidden } else { Visibility::Inherited };
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
    sounds: Vec<String>,
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
    seen: &'a Hints,
    /// The level's scroll texts.
    scrolls: String,
    /// Game seconds.
    now: f32,
}

impl Out<'_> {
    fn sound(&mut self, name: &str) {
        if !name.is_empty() {
            self.sounds.push(name.into());
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
    mut sounds: MessageWriter<PlaySound>,
    mut voices: MessageWriter<QueueVoice>,
    mut hints: MessageWriter<ShowHint>,
    mut messages: MessageWriter<ShowMessage>,
    seen: Res<Hints>,
    mut change: MessageWriter<ChangeLevelTo>,
    mut effects: MessageWriter<EffectAt>,
    mut hurt: MessageWriter<DamagePlayer>,
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
        seen: &seen,
        scrolls,
        now,
    };
    update_items(items, dt, &mut commands);
    run(items, dt, &mut state, ground.as_deref(), &mut players, &cameras, &mut out, &mut change);
    // Poison eaten is a poison blow on the hero, through its armour powers
    // and its reactions (the gold armour's heal comes back negative).
    if let Ok(mut player) = players.single_mut() {
        for amount in out.poison.drain(..) {
            let taken = player.take_blow(amount, hit_kind::POISON, Vec3::ZERO);
            if taken != 0.0 {
                hurt.write(DamagePlayer { amount: taken });
            }
        }
    }
    sounds.write_batch(out.sounds.into_iter().map(PlaySound));
    voices.write_batch(out.voices.into_iter().map(QueueVoice::hero));
    hints.write_batch(out.hints.into_iter().map(ShowHint));
    messages.write_batch(out.messages);
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

    let Ok(mut player) = players.single_mut() else {
        items.last_feet = None;
        return;
    };
    if let Some(leaving) = &mut items.leaving {
        leaving.fields -= FIELDS_PER_TICK;
        if leaving.fields <= 0 {
            info!("exit to {}", leaving.to);
            change.write(ChangeLevelTo(leaving.to.clone()));
            items.leaving = None;
        }
        return;
    }
    let to = player.mover.position;
    let from = items.last_feet.unwrap_or(to);
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
        if item.gone || item.leaving || !(item.flags & ALWAYS_ACTIVE != 0 || visible(item.shape.centre)) {
            continue;
        }
        if !touchable(item) {
            continue;
        }
        let pass = matches!(
            item.class(),
            ItemClass::Trigger | ItemClass::DamageTile | ItemClass::Exit | ItemClass::Transporter
        );
        let Some(c) = contact(&item.shape, pass, from, to, r, h) else { continue };
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

    exits(items, &on_exit);
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
        ItemClass::Sound | ItemClass::EnemyInfo => false,
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
    let (doors, realm) = (items.doors, items.realm);
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
            if item.state == 2 && item.ty.subtype == 0x2F {
                // An opened gold chest: its gold, and the chest goes.
                let gold = item.amount.max(0) as u32;
                if gold > 24 {
                    out.hint(Hint::CollectGold);
                }
                state.add_gold(gold);
                info!("took {gold} gold from {}", item.ty.name);
                out.sound("S_PICKUPMAGIC");
                item.leaving = true;
                item.timer = PICKUP_LINGER;
            } else if item.flags & LOCKED != 0 && item.state == 0 && item.flags & USED == 0 {
                if state.use_key() {
                    info!("chest {} opened with a key; {} left", item.ty.name, state.keys);
                    out.sound("S_CHEST");
                    item.flags |= USED;
                    open_chest(items, i, state, out);
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
                out.sound(DOOR_SOUNDS.get(realm).map_or("", |row| row[sub]));
                Touch::Pass
            } else {
                out.hint(Hint::UseKeyOnDoor);
                Touch::Block
            }
        }
        ItemClass::Generator => Touch::Block,
        ItemClass::Obstacle => match item.ty.subtype {
            // Crumbling floors and the like are walked over.
            0x28 | 0x31 | 0x33..=0x35 => Touch::Pass,
            // A safe rock only while it stands.
            SAFE_ROCK if item.stage <= 0 => Touch::Pass,
            _ => Touch::Block,
        },
        ItemClass::Exit | ItemClass::Transporter => Touch::Stand,
        _ => Touch::Pass,
    }
}

/// A key opened chest `i`: it plays its opening action and releases what
/// it holds. Stand-in: the contents go straight to the hero rather than
/// appearing as an item in the chest (see `docs/items.md`). A CHESTEXP
/// releases nothing: it ticks (and keeps running off screen) until it's
/// open, then explodes (`breakables.rs`).
fn open_chest(items: &mut LevelItems, i: usize, state: &mut PlayerState, out: &mut Out) {
    let doors = items.doors;
    let chest = &mut items.items[i];
    chest.play(1.min(chest.action_count().saturating_sub(1)));
    if chest.ty.subtype == CHEST_EXP {
        chest.flags |= ALWAYS_ACTIVE;
        out.sound(CHEST_EXP_TICK);
        return;
    }
    let Some(ty) = chest.contents.clone() else { return };
    if chest.ty.subtype == 0x2F {
        // Gold chests keep the gold until the hero comes back for it.
        chest.amount = ty.amount as i32;
        return;
    }
    if ty.class == ItemClass::Powerup {
        let mut amount = ty.amount as i32;
        pick_up(state, ty.subtype, ty.value, &ty.name, &mut amount, ty.duration as f32, doors, out);
        out.sparkle_at(chest.shape.centre);
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
            out.sound("S_PICKUPMAGIC");
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
            true
        }
        // Weapon, armour, speed, magic and special powers.
        5..=9 => {
            state.grant_power(subtype, value as u32, *amount as f32, duration);
            out.sound(power_sound(subtype, value as u32));
            true
        }
        // Runestones: one of each.
        10 => {
            if state.runestones.contains(amount) {
                return false;
            }
            state.runestones.push(*amount);
            out.sound("S_PICKUPRUNE");
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
            true
        }
        GARGOYLE_PIECE => {
            if let Some(p) = state.quest.add_gargoyle(*amount) {
                info!("gargoyle pieces {p}: {}/{}", state.quest.gargoyle[p], quest::GARGOYLE_NEEDED[p]);
                state.popup = Some((0x100 + p as u16, out.now));
            }
            out.sparkle(GARGOYLE_SPARKLE);
            out.sound("S_PICKUPMAGIC");
            true
        }
        _ => false,
    }
}

/// The sound a powerup plays.
fn power_sound(subtype: i32, value: u32) -> &'static str {
    match subtype {
        9 if value & 1 != 0 => "S_LEVITATEUP",
        9 if value & 0x100 != 0 => "S_GROW",
        9 if value & 0x200 != 0 => "S_SHRINK",
        9 if value & 0x400 != 0 => "S_POJO",
        6 if value & 0x20_0000 != 0 => "S_PICKUPSHIELD",
        _ => "S_PICKUPSPECIAL",
    }
}

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
                if let Some(m) = item.model.take() {
                    commands.entity(m).despawn();
                }
            }
            continue;
        }
        match item.class() {
            // Broken barrels and obstacles step through their actions like
            // opened doors and chests (`breakables.rs` marks them used).
            ItemClass::Door | ItemClass::Container | ItemClass::Obstacle if item.flags & USED != 0 => open_step(item),
            _ => {}
        }
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

/// Exits the hero stands in this tick: the portal steps through its
/// actions while the hero stays, and when the last one has played the hero
/// leaves for the exit's level. Secret exits go at once.
fn exits(items: &mut LevelItems, on_exit: &[usize]) {
    let mut go = None;
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
        let dest = match &item.params {
            PlacementParams::Exit { destination: Some(code) } => level_for_code(code),
            // Stand-in: an exit without a code (a realm's last level) goes
            // back to the hub.
            _ => Some("levelL1".to_string()),
        };
        if secret {
            item.flags |= USED;
            go = dest;
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
            go = dest;
            break;
        }
    }
    if let Some(to) = go {
        // The hero's exit takes 50 fields before the level changes.
        items.leaving = Some(Leaving { to, fields: 50 });
    }
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
    out.sound(TRANSPORT_SOUNDS.get(items.realm).copied().unwrap_or(""));
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
    fn sphere_reach_is_3d() {
        let s = shape(2);
        assert!(contact(&s, true, [0.0, 3.0, 0.0], [0.0, 3.0, 0.0], 1.5, 2.5).is_some());
        assert!(contact(&s, true, [0.0, 4.0, 0.0], [0.0, 4.0, 0.0], 1.5, 2.5).is_none());
    }
}
