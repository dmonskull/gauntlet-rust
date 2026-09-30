//! A boss's loot (`docs/critters.md`, "The boss's loot"). As the blow of
//! kind 9 on its DEATH lands, the level's items take an unseen blast (out
//! to 1000, 1000 at its heart, over 5 s: generators, barrels and the like
//! go; treasure and food stand) and the loot is flung out from the body:
//! the first Skorne throws his four pieces, every other boss its realm's
//! coins. They fly, bounce and slide to a stop, and can be picked up two
//! seconds after they're thrown.

use bevy::prelude::*;
use gdl_formats::population::rotation_matrix;

use crate::effects::SweepItems;
use crate::items::LevelItems;
use crate::monsters::MonsterTick;
use crate::player::Player;
use crate::population::{ContentModels, LevelPopulation};
use crate::world::LevelGround;

pub struct LootPlugin;

impl Plugin for LootPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BossLoot>()
            .init_resource::<Flung>()
            .add_systems(FixedUpdate, (spray_loot, fly_loot).chain().after(MonsterTick))
            .add_systems(Update, forget_flung.run_if(resource_exists_and_changed::<LevelPopulation>));
    }
}

/// What a dying boss throws (`critters.rs`): from `at`, at `velocity`
/// (its forward turned and tilted by the blow, × the blow's speed),
/// spread up to `spread` radians either side.
#[derive(Message, Clone, Copy, Debug)]
pub struct BossLoot {
    pub at: Vec3,
    pub velocity: Vec3,
    pub spread: f32,
    /// The boss's type and its realm's number.
    pub boss: i32,
    pub realm: u32,
}

/// The first Skorne: his pieces instead of coins.
const SKORNE: i32 = 0x2A;
/// His pieces, in the order they're thrown: the right gauntlet, the mask,
/// the horns, the left gauntlet (special powers `0x4000`, `0x2000`,
/// `0x1000`, `0x8000`).
const SKORNE_PIECES: [&str; 4] = ["BGNTR_IC", "BMASK_IC", "BHORN_IC", "BGNTL_IC"];
/// Coins per player by realm number (bronze, silver, gold).
const COINS: [[usize; 3]; 12] = [
    [0, 0, 0],
    [2, 1, 1],
    [0, 5, 0],
    [2, 1, 2],
    [2, 0, 2],
    [0, 0, 0],
    [0, 0, 4],
    [4, 1, 0],
    [0, 0, 0],
    [0, 3, 2],
    [0, 0, 3],
    [0, 4, 1],
];
const COIN_NAMES: [&str; 3] = ["COIN_BRONZE", "COIN_SILVER", "COIN_GOLD"];
/// What each coin is worth (set on it, whatever its type says).
const COIN_WORTH: [i32; 3] = [500, 1000, 5000];
/// The gold coins' speed: this share of the throw's, and up to this much
/// more at random along each axis. (The game works the same out for the
/// bronze and the silver, then throws them at the throw's own speed.)
const GOLD_SPEED: f32 = 0.75;
const GOLD_SPEED_RANDOM: f32 = 0.1;
/// The level's items' sweep.
const SWEEP_DAMAGE: f32 = 1000.0;
const SWEEP_RADIUS: f32 = 1000.0;
const SWEEP_LIFE: f32 = 5.0;
/// Fields before a thrown item can be picked up.
const PICKUP_DELAY: i32 = 120;
/// At most this many items fly at once.
const MOST_FLUNG: usize = 32;
/// The flight: falling at 32 units/s²; landing (under 0.1 above its
/// resting height, 1 above the floor) it bounces back up at 0.4 of its
/// fall's speed, stopping under 0.1; across the floor it slows by 4 of its
/// speed a second (0.5 in the air), stopping once that's more than it has.
const GRAVITY: f32 = 32.0;
const BOUNCE: f32 = 0.4;
const REST_ABOVE: f32 = 1.0;
const LANDED: f32 = 0.1;
const GROUND_SLOWING: f32 = 4.0;
const AIR_SLOWING: f32 = 0.5;
/// Its height above its rest when it isn't coming down.
const AIRBORNE: f32 = 10.0;

/// One item to throw: its type's name, velocity and what it's worth
/// (`None`: its type's).
#[derive(Clone, Debug, PartialEq)]
struct Toss {
    name: &'static str,
    velocity: Vec3,
    worth: Option<i32>,
}

/// Turns `v` about the vertical by `angle` the game's way (x cos − z sin,
/// z cos + x sin).
fn turned(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(v.x * c - v.z * s, v.y, v.z * c + v.x * s)
}

/// What a boss throws: the first Skorne his four pieces a quarter of the
/// spread apart, centred on the throw; any other its realm's coins, each
/// kind spread evenly across the whole spread (`players` × the realm's
/// count). `random` gives 0..1.
fn tosses(loot: &BossLoot, players: usize, mut random: impl FnMut() -> f32) -> Vec<Toss> {
    let spread = loot.spread;
    if loot.boss == SKORNE {
        let step = spread / 2.0;
        return SKORNE_PIECES
            .iter()
            .enumerate()
            .map(|(i, &name)| Toss { name, velocity: turned(loot.velocity, -0.75 * spread + i as f32 * step), worth: None })
            .collect();
    }
    let counts = COINS.get(loot.realm as usize).copied().unwrap_or_default();
    let mut out = Vec::new();
    for (kind, &count) in counts.iter().enumerate() {
        let n = count * players;
        if n == 0 {
            continue;
        }
        let step = 2.0 * spread / n as f32;
        for i in 0..n {
            let angle = 0.5 * step - spread + i as f32 * step;
            let velocity = if kind == 2 {
                let mut scale = || GOLD_SPEED + GOLD_SPEED_RANDOM * random();
                let v = loot.velocity;
                turned(Vec3::new(v.x * scale(), v.y * scale(), v.z * scale()), angle)
            } else {
                turned(loot.velocity, angle)
            };
            out.push(Toss { name: COIN_NAMES[kind], velocity, worth: Some(COIN_WORTH[kind]) });
        }
    }
    out
}

/// One tick of a thrown item's flight (`dt` seconds), given the floor
/// under where it's got to (`None`: none; it keeps falling).
fn fly(position: &mut Vec3, velocity: &mut Vec3, floor: impl Fn(Vec3) -> Option<f32>, dt: f32) {
    *position += *velocity * dt;
    let mut above = AIRBORNE;
    let mut slowing = AIR_SLOWING * dt;
    if velocity.y <= 0.0
        && let Some(f) = floor(*position)
    {
        above = position.y - (REST_ABOVE + f);
        if above < LANDED {
            velocity.y *= -BOUNCE;
            if velocity.y < LANDED {
                velocity.y = 0.0;
            }
            position.y = f + REST_ABOVE;
            slowing = GROUND_SLOWING * dt;
        }
    }
    if above >= LANDED {
        velocity.y -= GRAVITY * dt;
    }
    for v in [&mut velocity.x, &mut velocity.z] {
        *v = if v.abs() <= slowing { 0.0 } else { *v - slowing * *v };
    }
}

/// The items in flight: placement number, where it is, how it's going.
#[derive(Resource, Default)]
struct Flung(Vec<(usize, Vec3, Vec3, bool)>);

/// A boss's loot: the items' sweep, then the loot thrown.
#[allow(clippy::too_many_arguments)]
fn spray_loot(
    mut commands: Commands,
    mut loot: MessageReader<BossLoot>,
    items: Option<ResMut<LevelItems>>,
    (population, contents): (Option<Res<LevelPopulation>>, Option<Res<ContentModels>>),
    heroes: Query<(), With<Player>>,
    mut flung: ResMut<Flung>,
    mut sweeps: MessageWriter<SweepItems>,
    mut seed: Local<u32>,
) {
    let (Some(mut items), Some(population)) = (items, population) else { return };
    for l in loot.read() {
        sweeps.write(SweepItems { at: l.at, damage: SWEEP_DAMAGE, radius: SWEEP_RADIUS, life: SWEEP_LIFE });
        *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) | 1;
        let mut state = *seed;
        let random = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state & 0x7FFF) as f32 / 32767.0
        };
        for t in tosses(l, heroes.iter().count().max(1), random) {
            if flung.0.len() >= MOST_FLUNG {
                break;
            }
            let Some(ty) = population.population.item_types.iter().find(|ty| ty.name == t.name).cloned() else {
                warn!("no item type {} for the boss's loot", t.name);
                continue;
            };
            let placement = items.release(ty, l.at.to_array(), rotation_matrix([0.0; 3]), t.worth, PICKUP_DELAY);
            if let Some(models) = contents.as_deref() {
                models.spawn(t.name, Transform::from_translation(l.at), placement, &mut commands);
            }
            debug!("the boss throws {} at {:?}", t.name, t.velocity);
            flung.0.push((placement, l.at, t.velocity, false));
        }
        info!("the boss throws its loot: {} items", flung.0.len());
    }
}

/// A new level: nothing's in flight.
fn forget_flung(mut flung: ResMut<Flung>) {
    flung.0.clear();
}

/// The thrown items fly till they're picked up (the game's never quite
/// rest: at 30 ticks a second a landed item keeps a hop of a hundredth of
/// a unit, as the game's own arithmetic has it).
fn fly_loot(
    time: Res<Time>,
    mut flung: ResMut<Flung>,
    items: Option<ResMut<LevelItems>>,
    ground: Option<Res<LevelGround>>,
    mut models: Query<&mut Transform>,
) {
    let Some(mut items) = items else {
        flung.0.clear();
        return;
    };
    let dt = time.delta_secs();
    let floor = |at: Vec3| ground.as_ref().and_then(|g| g.0.floor_height(at.to_array()));
    flung.0.retain_mut(|(placement, position, velocity, settled)| {
        if !items.view(*placement).is_some_and(|v| v.live) {
            return false;
        }
        fly(position, velocity, floor, dt);
        if !*settled && velocity.x == 0.0 && velocity.z == 0.0 {
            *settled = true;
            debug!("thrown item {placement} comes to rest at {position:?}");
        }
        if let Some(model) = items.place(*placement, position.to_array())
            && let Ok(mut t) = models.get_mut(model)
        {
            t.translation = *position;
        }
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loot(boss: i32, realm: u32, spread: f32) -> BossLoot {
        BossLoot { at: Vec3::ZERO, velocity: Vec3::new(0.0, 30.0, 10.0), spread, boss, realm }
    }

    #[test]
    fn skorne_throws_his_pieces_across_the_spread() {
        let t = tosses(&loot(SKORNE, 5, 0.4), 1, || 0.5);
        let names: Vec<&str> = t.iter().map(|t| t.name).collect();
        assert_eq!(names, SKORNE_PIECES);
        // A quarter of the spread apart, centred: −0.3, −0.1, 0.1, 0.3.
        let first = t[0].velocity;
        let heading = first.x.atan2(first.z);
        assert!((heading - 0.3).abs() < 1e-5, "{heading}");
        assert!(t.iter().all(|t| t.worth.is_none() && (t.velocity.y - 30.0).abs() < 1e-5));
    }

    #[test]
    fn coins_by_realm_and_players() {
        // Realm 1: two bronze, a silver and a gold each.
        let t = tosses(&loot(0x23, 1, 1.0), 2, || 0.0);
        let count = |n: &str| t.iter().filter(|t| t.name == n).count();
        assert_eq!((count("COIN_BRONZE"), count("COIN_SILVER"), count("COIN_GOLD")), (4, 2, 2));
        assert!(t.iter().filter(|t| t.name == "COIN_GOLD").all(|t| t.worth == Some(5000)));
        // The bronze spread evenly across ±spread; they fly at the throw's
        // speed, the gold at 0.75 of it (plus the random part).
        let bronze: Vec<f32> = t.iter().filter(|t| t.name == "COIN_BRONZE").map(|t| t.velocity.x.atan2(t.velocity.z)).collect();
        assert_eq!(bronze.len(), 4);
        assert!((t[0].velocity.y - 30.0).abs() < 1e-5);
        let gold = t.iter().find(|t| t.name == "COIN_GOLD").unwrap();
        assert!((gold.velocity.y - 22.5).abs() < 1e-4);
        // A realm with none (the second Skorne's is gold only; realm 5
        // none).
        assert!(tosses(&loot(0x2B, 5, 1.0), 1, || 0.0).is_empty());
    }

    #[test]
    fn a_thrown_item_lands_bounces_and_stops() {
        let (mut p, mut v) = (Vec3::new(0.0, 5.0, 0.0), Vec3::new(10.0, 0.0, 0.0));
        let floor = |_: Vec3| Some(0.0);
        let dt = 1.0 / 30.0;
        let mut bounced = false;
        for _ in 0..600 {
            let falling = v.y < 0.0;
            fly(&mut p, &mut v, floor, dt);
            bounced |= falling && v.y > 0.0;
        }
        assert!(bounced, "it bounces");
        // It stops across the floor, and settles 1 above it (hopping a
        // hundredth of a unit, as the game's own arithmetic does).
        assert_eq!((v.x, v.z), (0.0, 0.0));
        assert!((p.y - REST_ABOVE).abs() < 0.05, "{p:?}");
        assert!(v.y.abs() < 1.0, "{v:?}");
    }
}
