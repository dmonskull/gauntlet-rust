//! What a landed blow does (`docs/combat.md`, `docs/monsters.md`): the
//! hero's `Hit` messages take hit points off monsters and generators.
//!
//! Monsters (the game's monster-damage routine): hit points go down by the damage;
//! below two thirds and one third of their full hit points they hit for
//! 0.667 / 0.333 of their damage; at 0 they die — their generator slot is
//! freed at once and the body plays out its death (`deaths.rs`). Generators take
//! whole hit points (the blow scaled by the hero's level against the
//! level's, less their armour, at least 1: `enemy::item_damage`), lose a
//! strength level for each item-type's worth (their monsters come out
//! weaker) and at 0 leave their broken model (`GEN_<code>0`) where the
//! game has one, else are gone with their model. Blows on them earn five
//! times a blow on one of their monsters.
//!
//! Monster blows (`MonsterHit`) hurt the hero through `HurtHero`, voiced
//! as the game's monster blow does it (`blow_cry`).
//!
//! The flinch/knockdown and push the blow causes play out in the monster's
//! next tick (`monsters.rs`).
//!
//! Every blow — on a hero (`Player::take_blow`), a monster or a critter —
//! goes through the game's armour and resistance routine ([`resist`]):
//! armour, the armour powers, elements.
//!
//! A blow that does damage plays the monster's hit sound, the killing one
//! its death sound (`MonsterSounds`): the far versions for thrown blows —
//! faded and panned at its feet as the game plays them
//! (`docs/audio-format.md`, "Positional sounds"), as are a generator's
//! and a critter's; Death's dying is panned only.
//!
//! Stand-ins: the level-versus-player-level damage scale and blocking
//! while defending aren't applied; there's no score yet. What happens when
//! the hero dies is the front end's (`frontend.rs`).

use bevy::prelude::*;
use gdl_formats::enemy;

use crate::audio::{CALL_VOLUME, PlaySoundAt};
use crate::combat::{Hit, TargetKind, Targetable, hit_kind};
use crate::critters::{Critter, CritterLevel, CritterSphere};
use crate::deaths;
use crate::effects::EffectAt;
use crate::generators::Generator;
use crate::hints::{Hint, ShowHint};
use crate::monsters::{self, DeathDrain, Monster, MonsterHit, MonsterLevel};
use crate::player::Player;
use crate::party::Party;
use crate::player_state::{Cry, EnemyScale, HealPlayer, HurtHero, PlayerState};
use crate::player::PlayerTick;
use crate::population::{GeneratorLooks, PlacementIndex};

pub struct DamagePlugin;

impl Plugin for DamagePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (apply_hits.after(PlayerTick), hurt_hero, death_drains));
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_hits(
    mut commands: Commands,
    mut hits: MessageReader<Hit>,
    mut party: ResMut<Party>,
    level: Option<Res<MonsterLevel>>,
    mut monsters: Query<&mut Monster>,
    mut generators: Query<&mut Generator>,
    models: Query<(Entity, &PlacementIndex, Option<&GeneratorLooks>, Option<&Children>)>,
    mut critters: Query<&mut Critter>,
    spheres: Query<&CritterSphere>,
    critter_level: Option<Res<CritterLevel>>,
    mut sounds: MessageWriter<PlaySoundAt>,
    mut effects: MessageWriter<EffectAt>,
    heroes: Query<&Player>,
    (enemies, mut hints, mut items): (Res<EnemyScale>, MessageWriter<ShowHint>, Option<ResMut<crate::items::LevelItems>>),
) {
    for hit in hits.read() {
        // Only a hero's blows earn experience (a monster's bomb or blast
        // earns nobody any).
        let by_hero = heroes.contains(hit.attacker);
        // The record of the hero who landed it: its experience.
        let mut state = heroes.get(hit.attacker).ok().map(|p| p.slot).and_then(|slot| party.state_mut(slot));
        match hit.target_kind {
            TargetKind::Monster => {
                let Ok(mut m) = monsters.get_mut(hit.target) else { continue };
                // Already dead from an earlier blow this tick.
                if m.hit_points <= 0.0 {
                    continue;
                }
                if m.enemy == monsters::DEATH_TYPE {
                    let hero = heroes.get(hit.attacker).ok();
                    if blow_on_death(&mut m, hit, hero, state.as_deref_mut(), &mut hints) {
                        sounds.write(PlaySoundAt::panned(DEATH_DIES, Vec3::from(m.position), CALL_VOLUME));
                        if let Some(mut g) = m.generator.and_then(|g| generators.get_mut(g).ok()) {
                            g.alive = g.alive.saturating_sub(1);
                        }
                        m.die(hit.kind, hit.at.to_array());
                        commands.entity(hit.target).try_remove::<Targetable>();
                        if let Some(state) = state.as_deref_mut() {
                            state.kills += 1;
                        }
                        info!("Death dies");
                    }
                    continue;
                }
                // Through its armour (only Death has any) and its
                // elements' factors; a hero's blow does at least a point.
                let boss_level = level.as_ref().is_some_and(|l| l.boss >= 0);
                let armour = if m.enemy == DEATH { DEATH_ARMOUR } else { 0.0 };
                let mut kind = hit.kind;
                let mut damage = resist(hit.damage, &mut kind, armour, 0, boss_level);
                if enemies.shrunk() {
                    damage *= SHRUNK_TAKE;
                }
                if by_hero && damage < HERO_BLOW_LEAST {
                    damage = HERO_BLOW_LEAST;
                }
                m.take_hit(damage, kind, hit.push.to_array());
                if damage > 0.0 {
                    m.hits = m.hits.saturating_add(1);
                    if let Some(s) = level.as_ref().and_then(|l| l.sounds.get(&m.enemy)) {
                        let name = if m.hit_points > 0.0 { s.hit(m.strength, m.hits, hit.ranged) } else { s.die(m.strength, hit.ranged) };
                        sounds.write(PlaySoundAt::faded(name, Vec3::from(m.position), MONSTER_VOLUME));
                    }
                }
                // Every blow earns experience; the killing one earns the
                // kill's.
                if by_hero && let (Some(state), Some(level)) = (state.as_mut(), level.as_ref()) {
                    let (at, scale) = level.experience;
                    let xp = enemy::experience(m.enemy, m.hit_points <= 0.0, state.level, at, scale);
                    let gained = state.add_experience(xp);
                    if gained > 0 {
                        info!("the hero reaches level {}", state.level);
                    }
                }
                if m.hit_points > 0.0 {
                    // Wounded monsters hit softer.
                    if let (Some(stats), Some(level)) = (enemy::enemy_stats(m.enemy), level.as_ref()) {
                        let full = stats.base_hit_points(&level.scales);
                        let base = stats.damage * level.scales.damage;
                        m.stats.damage = if m.hit_points <= 0.333 * full {
                            0.333 * base
                        } else if m.hit_points <= 0.667 * full {
                            0.667 * base
                        } else {
                            m.stats.damage
                        };
                    }
                    // The blow's effect: a blood spray, or the element's.
                    if damage > 0.0
                        && let Some(name) = deaths::hit_effect(m.enemy, kind)
                    {
                        let at = deaths::effect_origin(m.stats.step, m.centre(), hit.at);
                        let scale = deaths::die_effect_scale(m.enemy, m.stats.step);
                        effects.write(EffectAt { name, bank: None, at, facing: 0.0, scale });
                    }
                    debug!("monster {:?} hit for {:.1}: {:.1} left", hit.target, damage, m.hit_points);
                    continue;
                }
                // Dead: its slot is free now, it can't be targeted, and the
                // body plays out its death (`deaths.rs`).
                if let Some(mut g) = m.generator.and_then(|g| generators.get_mut(g).ok()) {
                    g.alive = g.alive.saturating_sub(1);
                }
                m.die(kind, hit.at.to_array());
                commands.entity(hit.target).try_remove::<Targetable>();
                // A hero's killing blow counts for its tally.
                if by_hero && let Some(state) = state.as_mut() {
                    state.kills += 1;
                }
                debug!("monster {:?} dies", hit.target);
            }
            TargetKind::Generator => {
                let Ok(mut g) = generators.get_mut(hit.target) else { continue };
                if g.hit_points <= 0.0 {
                    continue;
                }
                let hero_level = state.as_ref().map_or(1, |s| s.level);
                let was = g.hit_points;
                let at = level.as_ref().map_or(0.0, |l| l.experience.0);
                g.hit_points -= enemy::item_damage(hit.damage, g.armor, hero_level, at) as f32;
                // Five times a blow on one of its monsters.
                if by_hero && let (Some(state), Some(level)) = (state.as_mut(), level.as_ref()) {
                    let (at, scale) = level.experience;
                    let xp = enemy::generator_experience(g.enemy, g.hit_points <= 0.0, state.level, at, scale);
                    if state.add_experience(xp) > 0 {
                        info!("the hero reaches level {}", state.level);
                    }
                }
                // Its realm's sounds; none for destroying one on a boss level.
                if let Some(level) = level.as_ref()
                    && let Some((hurt, destroyed)) = enemy::generator_sounds(level.realm, g.enemy)
                    && g.hit_points < was
                {
                    // Above it (stand-in: its place, not its centre, raised).
                    let at = Vec3::from(g.position) + Vec3::Y * GENERATOR_SOUND_RISE;
                    if g.hit_points > 0.0 {
                        sounds.write(PlaySoundAt::faded(hurt, at, GENERATOR_HURT_VOLUME));
                    } else if level.boss < 0 {
                        sounds.write(PlaySoundAt::faded(destroyed, at, CALL_VOLUME));
                    }
                }
                if g.hit_points <= 0.0 {
                    // Strength 0: the game swaps in its broken model
                    // (`GEN_<code>0`) where it has one — the wreck stays,
                    // still in the way, but makes nothing and takes no
                    // more blows — and frees the item where it hasn't.
                    let mut wrecked = false;
                    for (e, model, looks, children) in &models {
                        if model.0 != g.placement {
                            continue;
                        }
                        match looks.and_then(GeneratorLooks::broken) {
                            Some(parts) => {
                                show_look(e, parts, children, &mut commands);
                                wrecked = true;
                            }
                            None => commands.entity(e).try_despawn(),
                        }
                    }
                    commands.entity(hit.target).try_despawn();
                    if !wrecked && let Some(items) = items.as_deref_mut() {
                        items.free(g.placement, &mut commands);
                    }
                    if by_hero && let Some(state) = state.as_mut() {
                        state.generators += 1;
                    }
                    debug!("generator {} destroyed", g.placement);
                } else if g.hit_points_per_tier > 0.0 {
                    let tier = (g.hit_points / g.hit_points_per_tier).ceil().clamp(1.0, 3.0) as i32;
                    if tier != g.tier {
                        g.tier = tier;
                        // Its model steps down with it.
                        for (e, model, looks, children) in &models {
                            let (true, Some(looks)) = (model.0 == g.placement, looks) else { continue };
                            let Some(parts) = looks.0.get(tier as usize) else { continue };
                            show_look(e, parts, children, &mut commands);
                        }
                        debug!("generator {} drops to strength {tier}", g.placement);
                    }
                }
            }
            TargetKind::Object => {
                // Critters (`critters.rs`), on the body or a hit sphere:
                // block, armour, experience.
                let (target, sphere, part) = match spheres.get(hit.target) {
                    Ok(s) => (s.critter, s.node, s.part),
                    Err(_) => (hit.target, None, None),
                };
                let Ok(mut c) = critters.get_mut(target) else { continue };
                let mut hit_sounds = Vec::new();
                let level = critter_level.as_deref();
                let xp = c.take_hit(hit.damage, hit.kind, hit.push.to_array(), sphere, part, hit.ranged, level, &mut hit_sounds);
                info!("the hero hits a critter for {:.1}: {:.0} hit points left", hit.damage, c.hit_points);
                // At its root, faded.
                let at = Vec3::from(c.position);
                for s in hit_sounds {
                    sounds.write(PlaySoundAt::faded(s, at, CRITTER_VOLUME));
                }
                if let Some(state) = state.as_mut()
                    && state.add_experience(xp) > 0
                {
                    info!("the hero reaches level {}", state.level);
                }
            }
            _ => {}
        }
    }
}

/// Armour and resistance bits, as the resistance routine reads them: a
/// hero's armour powers (`PowerBits::armour`), a critter type's
/// resistances. The element bits are in [`ELEMENTS`].
pub mod resists {
    /// Blows of more than a point heal a tenth of themselves instead (the
    /// gold invulnerability).
    pub const GOLD: u32 = 0x10_0000;
    /// No blow does anything.
    pub const INVULNERABLE: u32 = 0x1_0000;
    /// Magic does nothing.
    pub const MAGIC_PROOF: u32 = 0x1000;
    /// Poison does nothing (the gas mask).
    pub const POISON_PROOF: u32 = 0x2000;
    /// Blows lose their knockback kinds.
    pub const STEADY: u32 = 0x4_0000;
    /// Magic is resisted.
    pub const MAGIC_RESIST: u32 = 0x10;
}

/// Swaps a generator model's meshes for one strength level's look.
fn show_look(
    model: Entity,
    parts: &[(Handle<Mesh>, Handle<crate::level_material::LevelMaterial>)],
    children: Option<&Children>,
    commands: &mut Commands,
) {
    for c in children.into_iter().flatten() {
        commands.entity(*c).try_despawn();
    }
    for (mesh, material) in parts {
        commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), ChildOf(model)));
    }
}

/// By a blow's element (fire, lightning, light, acid: its kind's low four
/// bits, 1–4): the bit that resists it, the one that stops it, and the
/// ones that leave the target weak to it.
pub const ELEMENTS: [(u32, u32, u32); 4] = [
    (0x1, 0x100, 0x2 | 0x200),
    (0x2, 0x200, 0x1 | 0x100),
    (0x4, 0x400, 0x8 | 0x800),
    (0x8, 0x800, 0x4 | 0x400),
];

/// An elemental blow's factors, resisted / neither / weak: outside boss
/// levels, and on them.
const FACTORS: [f32; 3] = [0.5, 1.5, 2.0];
const BOSS_FACTORS: [f32; 3] = [0.75, 1.25, 1.5];

/// A gold-armoured target shrugs off blows of this or less.
const GOLD_SHRUGS: f32 = 1.0;
/// … and a bigger one heals it by this much of the blow.
const GOLD_HEAL: f32 = 0.1;

/// The game's armour and resistance routine (`docs/powers.md`, "The
/// resistance routine"), for heroes, monsters and critters alike: `bits`
/// are the target's [`resists`] and element bits, `armour` its armour
/// value. Returns what the blow takes — negative for the gold armour's
/// heal — and drops the knockback kinds from `kind` for a steady target.
///
/// In order: gold turns a blow of more than a point into a tenth of it
/// healed; invulnerability, or proof against the blow's magic or poison,
/// stops it; resisted magic is scaled; armour comes off anything but magic
/// and poison (a blow no stronger than it does nothing); what's left is
/// scaled by its element — every elemental blow the target neither
/// resists, is immune to nor is weak to does 1.5 times (1.25 on a boss
/// level).
pub fn resist(damage: f32, kind: &mut u32, armour: f32, bits: u32, boss_level: bool) -> f32 {
    use resists::*;
    if bits & GOLD != 0 {
        return if damage <= GOLD_SHRUGS { 0.0 } else { -GOLD_HEAL * damage };
    }
    let k = *kind;
    if bits & INVULNERABLE != 0
        || (bits & MAGIC_PROOF != 0 && k & hit_kind::MAGIC != 0)
        || (bits & POISON_PROOF != 0 && k & hit_kind::POISON != 0)
    {
        return 0.0;
    }
    let [resisted, neutral, weak] = if boss_level { BOSS_FACTORS } else { FACTORS };
    if bits & STEADY != 0 {
        *kind &= !hit_kind::KNOCKS;
    }
    let mut d = damage;
    if bits & MAGIC_RESIST != 0 && k & hit_kind::MAGIC != 0 {
        d *= resisted;
    }
    if k & (hit_kind::MAGIC | hit_kind::POISON) == 0 {
        d = if d < 0.0 {
            -d
        } else if d > armour {
            d - armour
        } else {
            0.0
        };
    }
    let element = ((k & hit_kind::ELEMENT) as usize).wrapping_sub(1);
    if d > 0.0
        && let Some(&(resist, immune, weak_to)) = ELEMENTS.get(element)
    {
        d = if bits & resist != 0 {
            d * resisted
        } else if bits & immune != 0 {
            0.0
        } else if bits & weak_to != 0 {
            d * weak
        } else {
            d * neutral
        };
    }
    d
}

/// Death (monster type `0x1E`) is the one monster with armour (its blows
/// are its own: [`blow_on_death`]).
const DEATH: i32 = 0x1E;
const DEATH_ARMOUR: f32 = 1.0;
/// Any blow but magic takes this from Death, and the halo's drain gives it
/// to the hero.
const DEATH_BLOW: f32 = 1.0;
/// A hero above this level is healed by a Death it kills with magic: its
/// hit points × (0.2 + 0.032 a level above).
const DEATH_HEAL_FROM: u32 = 75;
const DEATH_HEAL_BASE: f32 = 0.2;
const DEATH_HEAL_PER_LEVEL: f32 = 0.032;
/// The halo's armour bit.
const HALO: u32 = 0x8_0000;
/// The sound of Death dying.
const DEATH_DIES: &str = "S_DEATHDIE";
/// Requested volumes: a monster's hit and death sounds, a generator hurt
/// (destroyed: the call's own), a critter's sounds; the generators' play
/// this far above their place.
const MONSTER_VOLUME: u8 = 0xE0;
const GENERATOR_HURT_VOLUME: u8 = 0xB4;
const CRITTER_VOLUME: u8 = 0xE0;
const GENERATOR_SOUND_RISE: f32 = 2.0;

/// A blow on Death (the game's blow routine, Death's branch; no reaction
/// or experience): magic kills it outright — healing a hero above level 75
/// by its hit points' share — anything else takes exactly a point, which a
/// hero wearing the halo drinks (a point of health, or for a tier-2 Death
/// a drain's worth of experience) and any other hero is told to use magic.
/// True when this blow killed it.
fn blow_on_death(
    m: &mut Monster,
    hit: &Hit,
    hero: Option<&Player>,
    state: Option<&mut PlayerState>,
    hints: &mut MessageWriter<ShowHint>,
) -> bool {
    if hit.kind & hit_kind::MAGIC != 0 {
        if hero.is_some()
            && let Some(state) = state
            && state.level > DEATH_HEAL_FROM
        {
            let share = DEATH_HEAL_BASE + DEATH_HEAL_PER_LEVEL * (state.level - DEATH_HEAL_FROM) as f32;
            state.heal(m.hit_points * share);
        }
        m.hit_points = 0.0;
    } else {
        m.hit_points -= DEATH_BLOW;
        match (hero, state) {
            (Some(p), Some(state)) if p.armour_bits & HALO != 0 => {
                if m.strength == 2 {
                    let step = state.drain_step();
                    state.add_experience(step);
                } else {
                    state.health += DEATH_BLOW;
                }
            }
            (Some(_), _) => {
                hints.write(ShowHint::to(hero.map_or(0, |p| p.slot), Hint::KillDeathWithMagic));
            }
            _ => {}
        }
    }
    m.hit_points <= 0.0
}

/// A Death's drain lands on a hero: a tier-2 Death takes a drain's worth of
/// experience (levels can go); any other one its damage in health as the
/// game's negative blow of kind `0x1000` — through no armour, stopped only
/// by gold and invulnerability. Each drain raises its hint.
fn death_drains(
    mut drains: MessageReader<DeathDrain>,
    mut players: Query<&mut Player>,
    mut party: ResMut<Party>,
    mut damage: MessageWriter<HurtHero>,
    mut hints: MessageWriter<ShowHint>,
) {
    for d in drains.read() {
        let Ok(mut p) = players.get_mut(d.hero) else { continue };
        if d.experience {
            if let Some(state) = party.state_mut(p.slot) {
                let step = state.drain_step();
                if state.lose_experience(step) > 0 {
                    info!("Death drains the hero down to level {}", state.level);
                }
            }
            hints.write(ShowHint::to(p.slot, Hint::DeathDrainsExperience));
        } else {
            let amount = p.take_blow(-d.amount, hit_kind::DRAIN, Vec3::ZERO);
            if amount != 0.0 {
                damage.write(HurtHero { slot: p.slot, amount, kind: hit_kind::DRAIN, cry: Cry::Hurt });
            }
            hints.write(ShowHint::to(p.slot, Hint::DeathDrainsHealth));
        }
        debug!("Death {:?} drains the hero", d.death);
    }
}
/// A hero's blow on a monster takes at least this.
const HERO_BLOW_LEAST: f32 = 1.0;
/// Shrunk (`EnemyScale`), monsters take twice the damage (and deal half:
/// `monsters.rs`).
const SHRUNK_TAKE: f32 = 2.0;

/// The monster type whose blows knock heroes down.
const KNOCKDOWN_MONSTER: i32 = 0x1D;

fn hurt_hero(
    mut hits: MessageReader<MonsterHit>,
    monsters: Query<&Monster>,
    mut players: Query<&mut Player>,
    mut damage: MessageWriter<HurtHero>,
    enemies: Res<EnemyScale>,
    (mut turned, mut heal, mut sounds): (MessageWriter<Hit>, MessageWriter<HealPlayer>, MessageWriter<PlaySoundAt>),
) {
    for hit in hits.read() {
        let Ok(mut p) = players.get_mut(hit.player) else { continue };
        // The blow's kind, as the monster's attack sets it: a big monster's
        // strong attack knocks back, one type knocks down, small monsters'
        // blows only make the hero flinch — and shrunk, every one is a
        // small monster's.
        let (mut flags, mut push, mut blow, mut cry) = (0u32, Vec3::ZERO, hit.damage, Cry::Hurt);
        if let Ok(m) = monsters.get(hit.monster) {
            let big = m.stats.step > monsters::BIG_STEP;
            // How the blow sounds: a big grunt's, knight's or lizard's
            // with its own hurt sound at the hero's feet, the hero
            // silent.
            let (own, voiced) = blow_cry(m.enemy, big, m.tier);
            if let Some(name) = own {
                sounds.write(PlaySoundAt::panned(name, Vec3::from(p.mover.position), CALL_VOLUME));
            }
            cry = voiced;
            if enemies.shrunk() {
                flags = hit_kind::SMALL_MONSTER;
            } else {
                if hit.strong && big {
                    flags |= 0x10;
                }
                if m.enemy == KNOCKDOWN_MONSTER {
                    flags |= 0x20;
                }
                if !big {
                    flags |= hit_kind::SMALL_MONSTER;
                }
            }
            // The Hand of Death turns the blow back on the monster; the
            // Health Vampire too, as magic, healing the hero by it. The
            // hero takes nothing.
            if let Some(vampire) = p.turns_blows() {
                turned.write(Hit {
                    target: hit.monster,
                    attacker: Entity::PLACEHOLDER,
                    damage: blow,
                    kind: if vampire { hit_kind::MAGIC } else { 0 },
                    push: Vec3::ZERO,
                    at: m.centre(),
                    target_kind: TargetKind::Monster,
                    ranged: false,
                });
                if vampire {
                    heal.write(HealPlayer { slot: p.slot, amount: blow });
                }
                debug!("the hero's hand turns a blow of {blow:.1} back on the monster");
                (blow, flags) = (0.0, hit_kind::SMALL_MONSTER);
            }
            if flags & 0x130 != 0 {
                let d = Vec3::from(p.mover.position) - Vec3::from(m.position);
                push = Vec3::new(d.x, 0.0, d.z).normalize_or_zero();
            }
        }
        let amount = p.take_blow(blow, flags, push);
        if amount != 0.0 {
            damage.write(HurtHero { slot: p.slot, amount, kind: flags, cry });
        }
    }
}

/// Monster types by how their blows on a hero sound (the game's monster
/// blow): a big grunt, knight or lizard plays a hurt sound of its own; a
/// big demon, mummy or tree — and every small monster but the "it" — its
/// strike or bite (`S_<name>STRIKE`/`BITE`, not built here); the rest
/// leave it to the hero's cries.
const OWN_HURT_TYPES: [i32; 3] = [4, 5, 10];
const STRIKING_TYPES: [i32; 3] = [2, 8, 11];
const IT_TYPE: i32 = 0x1F;

/// A monster's blow on a hero: the hurt sound it plays itself (at the
/// hero's feet) and how the hero's cries go — silent when the monster's
/// own sound answers for it.
fn blow_cry(enemy: i32, big: bool, tier: i32) -> (Option<&'static str>, Cry) {
    if big && OWN_HURT_TYPES.contains(&enemy) {
        (Some(if tier < 2 { "S_PLYRDMG5" } else { "S_PLYRDMG4" }), Cry::Silent)
    } else if (big && STRIKING_TYPES.contains(&enemy)) || (!big && enemy != IT_TYPE) {
        (None, Cry::Silent)
    } else {
        (None, Cry::Hurt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn how_monster_blows_sound() {
        // A big grunt: its own hurt sound by tier, the hero silent.
        assert_eq!(blow_cry(4, true, 1), (Some("S_PLYRDMG5"), Cry::Silent));
        assert_eq!(blow_cry(10, true, 3), (Some("S_PLYRDMG4"), Cry::Silent));
        // A big demon or any small monster: its strike (not built).
        assert_eq!(blow_cry(2, true, 1), (None, Cry::Silent));
        assert_eq!(blow_cry(4, false, 1), (None, Cry::Silent));
        // The "it", and other big monsters: the hero's cries.
        assert_eq!(blow_cry(IT_TYPE, false, 1), (None, Cry::Hurt));
        assert_eq!(blow_cry(24, true, 1), (None, Cry::Hurt));
    }

    fn plain(damage: f32, armour: f32) -> f32 {
        resist(damage, &mut 0, armour, 0, false)
    }

    #[test]
    fn armour_takes_its_value_off_or_stops_the_blow() {
        assert_eq!(plain(5.0, 1.5), 3.5);
        assert_eq!(plain(1.0, 1.5), 0.0);
        assert_eq!(plain(-2.0, 1.5), 2.0);
        // Magic and poison go straight through.
        assert_eq!(resist(1.0, &mut { hit_kind::POISON }, 1.5, 0, false), 1.0);
    }

    #[test]
    fn invulnerability_stops_everything_and_gold_heals() {
        assert_eq!(resist(50.0, &mut 0x421, 0.0, resists::INVULNERABLE, false), 0.0);
        let gold = resists::INVULNERABLE | resists::GOLD;
        assert_eq!(resist(50.0, &mut 0, 3.0, gold, false), -5.0);
        assert_eq!(resist(1.0, &mut 0, 0.0, gold, false), 0.0);
    }

    #[test]
    fn the_gas_mask_stops_poison_and_resists_acid() {
        let mask = 0x2008;
        assert_eq!(resist(30.0, &mut { hit_kind::POISON }, 0.0, mask, false), 0.0);
        assert_eq!(resist(30.0, &mut 4, 0.0, mask, false), 15.0);
        // Light, which acid resistance leaves it weak to.
        assert_eq!(resist(30.0, &mut 3, 0.0, mask, false), 60.0);
    }

    #[test]
    fn elements_scale_by_the_level() {
        // Fire on a target with no resistances, then after armour.
        assert_eq!(resist(10.0, &mut 1, 0.0, 0, false), 15.0);
        assert_eq!(resist(10.0, &mut 1, 0.0, 0, true), 12.5);
        assert_eq!(resist(12.0, &mut 0x421, 2.0, 0, false), 15.0);
        // Immune, resisted, weak.
        assert_eq!(resist(10.0, &mut 2, 0.0, 0x200, false), 0.0);
        assert_eq!(resist(10.0, &mut 2, 0.0, 0x2, true), 7.5);
        assert_eq!(resist(10.0, &mut 2, 0.0, 0x100, false), 20.0);
    }

    #[test]
    fn a_steady_target_loses_the_knockback() {
        let mut kind = 0x10 | 0x100 | 0x80;
        resist(10.0, &mut kind, 0.0, resists::STEADY, false);
        assert_eq!(kind, 0x80);
    }
}
