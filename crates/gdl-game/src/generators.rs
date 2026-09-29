//! Generators and placed monsters (`docs/monsters.md`).
//!
//! A generator works while it's on screen (or always, if its placement has
//! flag bit 0): whenever it has fewer than its maximum alive and its wait
//! has run out, it makes a monster of its type at its strength's tier, just
//! outside itself — trying eight directions round it, starting from a
//! random one — then waits 6 × rate video fields, the wait growing by up
//! to double over each run of 2 × max monsters. Monsters placed directly
//! in the level (ENEMYINFO) appear once, when their spot comes on screen
//! within 50 units of the player.

use crate::play_camera::PlayCamera;
use bevy::prelude::*;
use gdl_formats::enemy::LevelEnemies;
use gdl_formats::enemy::{self, FIELDS_PER_TICK};
use gdl_formats::population::{ItemClass, PlacementParams, Population, rotation_matrix};
use gdl_formats::{LevelCollision, LevelTuning};

use crate::monsters::{
    Body, Monster, MonsterLevel, NewMonster, PLAYER_HEIGHT, PLAYER_RADIUS, add, bumps, despawn_monster, distance,
    game_view, on_screen, spawn_monster,
};
use crate::player::Player;
use crate::world::LevelGround;

/// A monster generator. Its model is drawn by the population view; this
/// entity carries its state.
#[derive(Component, Debug, Clone)]
pub struct Generator {
    /// Index of its placement in the level's population.
    pub placement: usize,
    /// Enemy type id it makes.
    pub enemy: i32,
    /// Strength 1–3: its monsters' tier (and its model, `GEN_<code><n>`).
    pub tier: i32,
    pub ai: i16,
    /// Most of its monsters alive at once.
    pub max: u8,
    /// Spawn rate: the wait after each monster is 6 × this (video fields).
    pub rate: u8,
    /// Its monsters alive now.
    pub alive: u8,
    /// Hit points (the item type's × strength × the level's scale), for
    /// when players can break generators.
    pub hit_points: f32,
    /// One strength level's worth of hit points (the item type's, scaled):
    /// losing that much drops the generator a level.
    pub hit_points_per_tier: f32,
    pub position: [f32; 3],
    /// Heading of its front (monsters come out this way first).
    pub yaw: f32,
    /// How far out monsters appear, beyond their own radius (the item
    /// type's first extent).
    pub reach: f32,
    /// Radius of its on-screen test (4 × the item's radius).
    pub screen_radius: f32,
    /// Placement flag bit 0: works even off screen.
    pub always_active: bool,
    /// Video fields until it may make another monster.
    pub wait: f32,
    /// Grows the wait: 0 → 1 by 1 / (2 × max) per monster, then wraps.
    pub ramp: f32,
}

/// A monster placed in the level, waiting to be seen.
#[derive(Debug, Clone)]
pub struct PlacedMonster {
    pub placement: usize,
    pub enemy: i32,
    /// The placement's level (0 counts as tier 1).
    pub tier: i32,
    pub ai: i16,
    /// Awareness override when positive (× the level's awareness scale).
    pub range: f32,
    pub position: [f32; 3],
    pub yaw: f32,
    pub screen_radius: f32,
    pub spawned: bool,
}

#[derive(Resource, Default)]
pub struct PlacedMonsters(pub Vec<PlacedMonster>);

/// Distance from the player within which a placed monster appears.
const PLACED_RANGE: f32 = 50.0;
/// Generators only make monsters when a player is within this.
const GENERATOR_RANGE: f32 = 1000.0;
/// A spawn spot's floor must be within this of the generator's centre.
const SPAWN_FLOOR_RANGE: f32 = 6.0;

/// The heading an item's forward (+Z) is turned to, the way the game reads
/// it off the placement matrix.
fn forward_heading(rotation: [f32; 3]) -> f32 {
    let m = rotation_matrix(rotation);
    m[6].atan2(m[8])
}

/// The game's radius for an item: half the larger of its first two extents.
fn item_radius(extent: [f32; 4]) -> f32 {
    0.5 * extent[0].max(extent[1])
}

/// The AI a new monster runs, as the game picks it from the type, the tier
/// and what the generator or placement asked for.
pub fn choose_ai(enemy: i32, tier: i32, ai: i16, random_bit: bool) -> i16 {
    match enemy {
        0 | 3 | 6 | 9 | 12 | 15 | 18 | 21 | 22 if ai != 2 && ai != 4 => {
            if random_bit {
                4
            } else {
                2
            }
        }
        1 | 4 | 5 | 8 | 10 | 11 | 13 | 16 | 19 | 20 | 25 | 32 | 33 if ai == 0 => match tier {
            5 => 0x11,
            4 => 0x17,
            6 => 0x12,
            _ => 7,
        },
        2 | 7 | 14 | 17 | 24 if ai == 0 => match tier {
            5 => 0x11,
            3 => 0x1E,
            4 => 0x17,
            6 => 0x12,
            _ => 7,
        },
        27 => 0x1F,
        29 => 0x13,
        30 => 3,
        31 => 0x1B,
        _ => ai,
    }
}

/// Generators and placed monsters from a level's population.
pub fn from_population(
    population: &Population,
    collision: &LevelCollision,
    tuning: &LevelTuning,
    enemies: &LevelEnemies,
) -> (Vec<Generator>, Vec<PlacedMonster>) {
    let mut generators = Vec::new();
    let mut placed = Vec::new();
    for (i, p) in population.placements.iter().enumerate() {
        let ty = population.resolved_type(p);
        let Some(named) = ty.enemy() else { continue };
        if !p.active_for(1) {
            continue; // needs more players
        }
        // The realm's own monster for the placeholder name. Generators
        // substitute with tier 0 (the field holds their live count, 0, at
        // that point); placed monsters with their level.
        let id = match p.params(ty.class) {
            PlacementParams::Enemy { level, .. } => enemies.substitute(named, level as i32),
            _ => enemies.substitute(named, 0),
        };
        if !enemies.has(id) || enemy::enemy_stats(id).is_none_or(|s| s.id >= enemy::FIRST_SPECIAL_TYPE) {
            // Not loaded on this level (the game refuses these), or a boss
            // or scripted type (the critter system drives those).
            continue;
        }
        let mut position = p.position;
        if !ty.keeps_height()
            && let Some(y) = collision.floor_height(position)
        {
            position[1] = y + 0.1;
        }
        let yaw = forward_heading(p.rotation);
        let default_ai = enemy::enemy_stats(named).map_or(7, |s| s.default_ai);
        match p.params(ty.class) {
            PlacementParams::Generator { strength, ai, max, rate } if ty.class == ItemClass::Generator => {
                let strength = strength.max(1);
                let (default_max, default_rate) = enemy::generator_defaults(strength);
                let max = if max == 0 { default_max } else { max };
                let rate = if rate == 0 { default_rate } else { rate };
                generators.push(Generator {
                    placement: i,
                    enemy: id,
                    tier: strength as i32,
                    ai: if ai < 0 { default_ai } else { ai },
                    max: (max as f32 * tuning.generator_max) as u8,
                    rate: (rate as f32 * tuning.generator_rate) as u8,
                    alive: 0,
                    hit_points: (ty.hit_points as i32 * strength as i32) as f32 * tuning.generator_hit_points,
                    hit_points_per_tier: ty.hit_points as f32 * tuning.generator_hit_points,
                    position,
                    yaw,
                    reach: ty.extent[0],
                    screen_radius: 4.0 * item_radius(ty.extent),
                    always_active: p.flags & 1 != 0,
                    wait: 0.0,
                    ramp: 0.0,
                });
            }
            PlacementParams::Enemy { level, ai, range, .. } => placed.push(PlacedMonster {
                placement: i,
                enemy: id,
                tier: level as i32,
                ai: if ai < 0 { default_ai } else { ai },
                range,
                position,
                yaw,
                screen_radius: 4.0 * item_radius(ty.extent),
                spawned: false,
            }),
            _ => {}
        }
    }
    (generators, placed)
}

/// The eight spots round a generator, by the game's order: straight out,
/// behind, left, right, then the diagonals — (direction multiplier, angle
/// offset of the new monster's facing).
fn spawn_direction(k: u32, dir: [f32; 2]) -> ([f32; 2], f32) {
    let [x, z] = dir;
    let h = std::f32::consts::FRAC_1_SQRT_2;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    match k {
        1 => ([-x, -z], PI),
        2 => ([-z, x], -FRAC_PI_2),
        3 => ([z, -x], FRAC_PI_2),
        4 => ([h * x + h * z, h * -x + h * z], FRAC_PI_4),
        5 => ([h * -z + h * x, h * z + h * x], -FRAC_PI_4),
        6 => ([h * z + h * -x, h * -z + h * -x], 3.0 * FRAC_PI_4),
        7 => ([h * -x + h * -z, h * x + h * -z], -3.0 * FRAC_PI_4),
        _ => (dir, 0.0),
    }
}

/// Types whose generators only let monsters out of the front and front
/// diagonals.
fn front_only(enemy: i32) -> bool {
    matches!(enemy, 1 | 4 | 5 | 7 | 8 | 10 | 11 | 14 | 15 | 19 | 24 | 25)
}

/// The game's spawn spot test: nothing solid between the generator's
/// centre and the spot, a floor under it near the generator's height, and
/// no player or monster standing there.
fn test_spot(
    collision: &LevelCollision,
    from: [f32; 3],
    offset: [f32; 3],
    radius: f32,
    step: f32,
    players: &[[f32; 3]],
    bodies: &[Body],
) -> Option<[f32; 3]> {
    let mut to = add(from, offset);
    if collision.wall(from, to, radius).is_some() {
        return None;
    }
    let floor = collision.floor_probe(to, 4.0, -10.0, 0.1, 2)?;
    if (floor.point[1] - from[1]).abs() > SPAWN_FLOOR_RANGE {
        return None;
    }
    to[1] = floor.point[1];
    let feet = [from[0], floor.point[1], from[2]];
    let crowded = players.iter().any(|p| bumps(feet, to, *p, radius + 0.5 + PLAYER_RADIUS, step + PLAYER_HEIGHT))
        || bodies.iter().any(|b| bumps(feet, to, b.feet, 0.5 * radius + b.radius, step + b.step));
    (!crowded).then_some(to)
}

fn find_spot(
    level: &mut MonsterLevel,
    collision: &LevelCollision,
    g: &Generator,
    players: &[[f32; 3]],
    bodies: &[Body],
) -> Option<([f32; 3], f32)> {
    let stats = enemy::enemy_stats(g.enemy)?;
    let from = [g.position[0], g.position[1] + stats.center_height, g.position[2]];
    let out = g.reach + stats.radius;
    let dir = [g.yaw.sin(), g.yaw.cos()];
    let skip: u32 = if front_only(g.enemy) { 0xFFCE } else { 0 };
    let first = level.random(8);
    let mut k = first;
    loop {
        if skip & (1 << k) == 0 {
            let (d, offset) = spawn_direction(k, dir);
            let offset_to = [d[0] * out, 0.0, d[1] * out];
            if let Some(p) = test_spot(collision, from, offset_to, stats.radius, stats.step(), players, bodies) {
                return Some((p, offset));
            }
        }
        k = (k + 1) % 8;
        if k == first {
            return None;
        }
    }
}

/// Makes room for a new monster: a free slot, or the monster the game
/// would recycle — the one furthest from its target, placed ones last and
/// those near the screen only for a requester that's on screen itself.
/// `requester` is −1 for an off-screen generator, 0 on screen, 1 placed.
fn claim_slot(
    level: &MonsterLevel,
    monsters: &Query<(Entity, &Monster)>,
    requester: i32,
) -> Result<Option<Entity>, ()> {
    if monsters.iter().count() < level.slots {
        return Ok(None);
    }
    let mut worst: Option<(f32, Entity, bool)> = None;
    for (e, m) in monsters {
        let mut score = if m.target.is_some() { m.target_distance } else { 1.0e5 };
        if m.placed {
            score *= 0.01;
        } else if !m.near_screen {
            score += 10_000.0;
        }
        if worst.is_none_or(|w| score > w.0) {
            worst = Some((score, e, m.near_screen));
        }
    }
    match worst {
        Some((_, e, near)) if requester >= near as i32 => Ok(Some(e)),
        _ => Err(()),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn tick_generators(
    mut commands: Commands,
    level: Option<ResMut<MonsterLevel>>,
    ground: Option<Res<LevelGround>>,
    players: Query<&Player>,
    camera: Option<Res<PlayCamera>>,
    monsters: Query<(Entity, &Monster)>,
    mut generators: Query<(Entity, &mut Generator)>,
) {
    let (Some(mut level), Some(ground)) = (level, ground) else { return };
    let view = game_view(camera.as_deref());
    let frustum = view.as_ref();
    let feet: Vec<[f32; 3]> = players.iter().map(|p| p.mover.position).collect();
    let mut bodies: Vec<Body> = monsters
        .iter()
        .map(|(e, m)| Body { entity: e, feet: m.position, radius: m.stats.radius, step: m.stats.step })
        .collect();
    let mut live = bodies.len();
    let mut recycled: Vec<Entity> = Vec::new();
    let mut freed: Vec<Entity> = Vec::new();

    for (entity, mut g) in &mut generators {
        if g.hit_points <= 0.0 || g.alive >= g.max {
            continue;
        }
        let visible = on_screen(frustum, g.position, g.screen_radius);
        if !visible && !g.always_active {
            continue;
        }
        if g.wait > 0.0 {
            g.wait -= FIELDS_PER_TICK;
            continue;
        }
        if !feet.iter().any(|p| distance(*p, g.position) <= GENERATOR_RANGE) {
            continue;
        }
        let victim = if live < level.slots {
            None
        } else {
            match claim_slot(&level, &monsters, if visible { 0 } else { -1 }) {
                Ok(Some(v)) if !recycled.contains(&v) => Some(v),
                _ => continue,
            }
        };
        // The game frees the slot first: the recycled monster goes even if
        // no spot is found.
        if let Some(v) = victim {
            if let Ok((_, m)) = monsters.get(v)
                && let Some(owner) = m.generator
            {
                freed.push(owner);
            }
            commands.entity(v).try_despawn();
            recycled.push(v);
            bodies.retain(|b| b.entity != v);
            live -= 1;
        }
        let Some((spot, offset)) = find_spot(&mut level, &ground.0, &g, &feet, &bodies) else { continue };
        let tier = g.tier;
        let random_bit = level.random(2) == 1;
        let new = NewMonster {
            enemy: g.enemy,
            tier,
            ai: choose_ai(g.enemy, tier, g.ai, random_bit),
            position: spot,
            facing: crate::locomotion::wrap(g.yaw + offset),
            generator: Some(entity),
            placed: false,
            awareness: None,
            freeze: 0.0,
        };
        let Some(e) = spawn_monster(&mut level, new, &mut commands) else { continue };
        debug!("generator {} made enemy {} tier {tier} at {spot:?}", g.placement, g.enemy);
        let stats = enemy::enemy_stats(g.enemy).expect("spawned a known type");
        bodies.push(Body { entity: e, feet: spot, radius: stats.radius, step: stats.step() });
        live += 1;
        g.alive += 1;
        g.wait = enemy::generator_wait(g.rate, g.ramp);
        g.ramp += enemy::generator_ramp_step(g.max);
        if g.ramp > 1.0 {
            g.ramp = 0.0;
        }
    }
    for owner in freed {
        if let Ok((_, mut g)) = generators.get_mut(owner) {
            g.alive = g.alive.saturating_sub(1);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn tick_placed(
    mut commands: Commands,
    level: Option<ResMut<MonsterLevel>>,
    placed: Option<ResMut<PlacedMonsters>>,
    players: Query<&Player>,
    camera: Option<Res<PlayCamera>>,
    monsters: Query<(Entity, &Monster)>,
    mut generators: Query<&mut Generator>,
) {
    let (Some(mut level), Some(mut placed)) = (level, placed) else { return };
    let view = game_view(camera.as_deref());
    let frustum = view.as_ref();
    let feet: Vec<[f32; 3]> = players.iter().map(|p| p.mover.position).collect();
    let mut live = monsters.iter().count();
    // Monsters recycled this tick (their despawn hasn't applied yet).
    let mut recycled: Vec<Entity> = Vec::new();
    for p in placed.0.iter_mut().filter(|p| !p.spawned) {
        if !on_screen(frustum, p.position, p.screen_radius)
            || !feet.iter().any(|f| distance(*f, p.position) <= PLACED_RANGE)
        {
            continue;
        }
        if live >= level.slots {
            match claim_slot(&level, &monsters, 1) {
                Ok(Some(v)) if !recycled.contains(&v) => {
                    if let Ok((_, m)) = monsters.get(v) {
                        despawn_monster(&mut commands, v, m, &mut generators);
                    }
                    recycled.push(v);
                    live -= 1;
                }
                _ => continue,
            }
        }
        let random_bit = level.random(2) == 1;
        let awareness = (p.range > 0.0).then(|| p.range * level.scales.awareness);
        let new = NewMonster {
            enemy: p.enemy,
            tier: p.tier.max(1),
            ai: choose_ai(p.enemy, p.tier, p.ai, random_bit),
            position: p.position,
            facing: p.yaw,
            generator: None,
            placed: true,
            awareness,
            // Placed monsters stand still for 30 fields first.
            freeze: if p.tier < 4 { 30.0 } else { 0.0 },
        };
        p.spawned = true;
        debug!("placed monster {} (enemy {} level {}) appears", p.placement, p.enemy, p.tier);
        if spawn_monster(&mut level, new, &mut commands).is_some() {
            live += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_directions_turn_the_front() {
        let dir = [0.0, 1.0];
        let (d, a) = spawn_direction(2, dir);
        assert_eq!((d, a), ([-1.0, 0.0], -std::f32::consts::FRAC_PI_2));
        // The direction's heading plus the offset is the new heading.
        for k in 0..8 {
            let (d, a) = spawn_direction(k, [0.3f32.sin(), 0.3f32.cos()]);
            let h = d[0].atan2(d[1]);
            assert!(crate::locomotion::wrap(h - (0.3 + a)).abs() < 1e-5, "{k}");
        }
    }

    #[test]
    fn ai_choice_follows_the_game() {
        assert_eq!(choose_ai(4, 1, 0, false), 7);
        assert_eq!(choose_ai(2, 3, 0, false), 0x1E);
        assert_eq!(choose_ai(0, 1, 7, true), 4);
        assert_eq!(choose_ai(4, 1, 9, false), 9);
    }
}
