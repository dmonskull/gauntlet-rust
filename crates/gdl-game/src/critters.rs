//! Critters: the game's scripted monsters, run from their `CRITTER/*.WAD`
//! data (`docs/critters.md` has the game's code behind all of this). This
//! runs the placed golems, gargoyles and generals and the level's boss.
//!
//! A golem or gargoyle stands as a statue until it's woken; a general is
//! made as soon as its spot is seen within 50 of a hero; a boss
//! is made at the level's boss locator and sleeps (playing INIT) until two
//! seconds have passed and its targets are within its wake distance. Then
//! every 30 Hz tick a critter
//!
//! 1. scores the players against its type's target condition: the golem
//!    keeps the best, a boss every one that passes (up to four); players a
//!    critter hit in the last quarter second score worse;
//! 2. picks a move: the forced ones first (INIT → START, DEATH, a move's
//!    follow-up, a boss's READY after START, then a knockdown, knockback,
//!    roar or flinch for the blows it took), else a block when its target
//!    is attacking, else the next step of a running pattern or the least
//!    recently used pattern or attack whose condition a target meets, else
//!    the movement move whose condition scores its target best, else a
//!    taunt (unhurt) or READY — every move only when its cooldown since it
//!    last ended has run out;
//! 3. switches to it the way the move's transition allows (at once, or
//!    when the animation playing ends); a boss holds a move at least its
//!    hold time;
//! 4. lands the move's blows on their frames — a sphere on the move's node
//!    swept from its last position, a breath cone along the node, a
//!    missile (with the critter's own effect model, aimed with an arc at
//!    its target, faster the angrier it is, splashing and bursting where
//!    it stops) or a still effect set down as its record says (a ring on
//!    the ground, an attached attack: a blast growing over its life) — and
//!    plays its effects' sounds; a player a critter hit can't be hit by
//!    one again for 0.25 s;
//! 5. walks at the move's speed in the move's direction plus its knockback
//!    (a golem on the level's walls and floor, stopping short of players;
//!    a boss kept within its leash of home) and turns toward its target at
//!    the move's rate.
//!
//! Blows from the hero reach it through `damage.rs` ([`Critter::take_hit`]):
//! critters whose type has hit spheres are hit on those ([`CritterSphere`],
//! each with its share of hit points and damage scale), others on the
//! body; a blocking critter takes a quarter, its armour comes off each
//! blow, the hero earns a share of its experience per blow and a fifth of
//! it for the kill; the blows' kinds pick its hit reaction and push it
//! (bosses aren't pushed). At 0 hit points it plays DEATH and is removed
//! when that ends (a boss when its hold after it ends).
//!
//! A statue wakes when a wake trigger (flag 0x2000) next to it comes on or
//! a hero walks into it (it blocks); it comes alive once its spot is seen
//! within 50 of a hero. A placed golem, gargoyle or general holds the
//! powerup lying on its spot, hidden, and drops it where it dies (a
//! gargoyle holding none leaves a gargoyle piece).
//! `GDL_WAKE_STATUES=<range>` (a testing aid) also wakes them when the
//! hero comes that close.
//!
//! A hero who brings the realm's legendary item gets the boss intro
//! (`GDL_LEGENDARY=1`, a testing aid, pretends so): once START is over
//! the level darkens, the boss idles 1–3 s, roars, and fights (the
//! chimera only after a missile hits it); a missile in the dark freezes
//! the dragon or stuns the djinn and cuts the intro short.
//!
//! When the boss's body goes, the realm is won: its shard shows where the
//! boss was made, the wizard appears by the heroes and speaks (his
//! messages timed as the game types them), and a short countdown ends
//! the level.
//!
//! A boss has a health meter across the top of the screen, a golem or
//! gargoyle a 3D one over its head (`meter.rs`).
//!
//! Stand-ins (see the doc): a woken statue goes straight to its ACTIVE
//! animation and its critter appears when that ends; critter missiles
//! fly the missiles' three seconds; the chimera's wake
//! timer starts at once; the tower's first level follows a boss. The
//! heroes' highlights in the intro, breaking nodes, look nodes, fading,
//! its blows on other monsters and pushing players aside aren't done.

use gdl_formats::detmath::{Det, sync_bits, sync_bits64};
use std::collections::HashMap;
use std::sync::Arc;

use bevy::math::Affine3A;
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use gdl_formats::anim::{AnimFile, Track, rotation_matrix as clip_rotation};
use gdl_formats::text::TextRom;
use gdl_formats::collision::{approx_hypot, node_flags, push_out};
use gdl_formats::critter::{self, CritterDamage, CritterFile, CritterMove, Condition, class, kind};
use gdl_formats::population::{ItemClass, LocatorKind, PlacementParams, REALM_LETTERS, rotation_matrix};
use gdl_formats::texmod::TexMod;
use gdl_formats::{LevelCollision, ModelFile, enemy};

use crate::audio::{PlaySoundAt, QueueVoice, VoiceQueues};
use crate::character::{Animate, Animator, CharacterData, CharacterModel, advance_clip, clip_end, clip_fps};
use crate::combat::{CritterAim, SphereAim, TargetKind, Targetable};
use crate::effects::{BankEffect, CritterBlast, OneShot, effect_life};
use crate::exits::ChangeLevelTo;
use crate::flash::{self, Flash, FlashColours};
use crate::message_box::{self, ShowCaption, TextFile};
use crate::hints::{Hint, ShowHint};
use crate::items::LevelItems;
use crate::loot::BossLoot;
use crate::texanim::LevelTexAnims;
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion;
use crate::mechanics::Mechanics;
use crate::monsters::{MonsterLevel, MonsterTick, game_view, on_screen};
use crate::play_camera::PlayCamera;
use crate::player::{GrabHero, Player, ReleaseHero, ThrowHero};
use crate::party::Party;
use crate::tick_places::{Between, TickPlaces};
use crate::player_state::{Cry, EnemyScale, HurtHero, TimeStop};
use crate::population::{ContentModels, LevelPopulation};
use crate::projectiles::{CritterMissile, CritterStop, cylinder_hit, load_atree, spawn_critter_missile};
use crate::world::{LevelEntity, LevelGround};

mod meter;

pub struct CrittersPlugin;

impl Plugin for CrittersPlugin {
    fn build(&self, app: &mut App) {
        meter::plugin(app);
        app.init_resource::<BossWatch>()
            .add_systems(
                FixedUpdate,
                (
                    touch_statues.before(tick_critters),
                    tick_critters.after(MonsterTick),
                    drop_items.after(tick_critters),
                    rock_blows.after(tick_critters),
                    run_victory.after(tick_critters),
                    watch_boss.after(run_victory),
                ),
            )
            .add_systems(
            Update,
            (setup_level.run_if(resource_added::<MonsterLevel>).after(crate::items::build_items), interpolate).chain(),
        )
        .add_systems(FixedPreUpdate, interpolate.in_set(TickPlaces::Movers))
        .add_systems(Update, pose_parts.after(Animate));
    }
}

/// A part playing a move of its own poses its subtree of the body's model
/// with its own clip, over the body's animation (the game animates each
/// part's subtree apart; copying the body, it shows the body's pose).
fn pose_parts(critters: Query<(&Critter, &Animator)>, mut bones: Query<&mut Transform>) {
    for (c, animator) in &critters {
        for p in c.parts.iter().filter(|p| !p.mirrored && p.current.is_some()) {
            let body = p.body();
            let Some(tracks) = body.tracks.get(p.clock.action) else { continue };
            let last = f32::from(p.clock.frames.saturating_sub(1));
            let at = p.clock.frame.min(last);
            for (k, &n) in body.subtree.iter().enumerate() {
                let Some(mut t) = animator.bone(n).and_then(|b| bones.get_mut(b).ok()) else { continue };
                *t = match tracks.get(k).and_then(Option::as_ref) {
                    Some(track) => {
                        let pose = track.sample(at);
                        let m = Mat4::from_cols_array(&clip_rotation(pose.rotation, track.flags));
                        Transform {
                            translation: body.rest[k] + Vec3::from(pose.translation),
                            rotation: Quat::from_mat4(&m),
                            scale: Vec3::from(pose.scale),
                        }
                    }
                    None => Transform::from_translation(body.rest[k]),
                };
            }
        }
    }
}

/// The end of a boss level, once the boss is gone (the game's end
/// sequence; its steps as the game numbers them):
///
/// - 0: the heroes win the realm; the key shows where the boss was made
///   (not after the skornes or garm), with its sound, its second model
///   taking over for 30 s when its clip is done;
/// - 1–2: 5 s (10 s after the first skorne and garm);
/// - 3–4: the wizard appears halfway between the boss's spot and the
///   heroes, 3 above them (at a fixed spot for the skornes), and fades in;
/// - 5: he speaks and his first message's pages type out; half a second
///   after them, 6: his second speech and message (how many of the realm's
///   runestones the heroes hold), and a second after its pages
/// - 8: the last countdown starts, 2 s (10 s after the first skorne when
///   the heroes hold all twelve runestones); it ends the level (which
///   then waits for the voice queues, as every level change does);
/// - 9: once the voice queues are empty (his speeches done),
/// - 10: at 35 fields left the heroes still standing teleport out in the
///   death light (`going_out.rs`, stepping 0.5).
///
/// His speeches wait in the announcer's voice queue; from his appearance
/// the announcer's own lines (hints) are refused. His messages show a page
/// at a time for as long as the game types each. Stand-ins: he doesn't
/// fade; the next level is the tower's first.
#[allow(clippy::too_many_arguments)]
fn run_victory(
    mut commands: Commands,
    level: Option<ResMut<CritterLevel>>,
    mut party: ResMut<Party>,
    mut players: Query<&mut Player>,
    mut animators: Query<&mut Animator>,
    mut sounds: MessageWriter<PlaySoundAt>,
    (mut voices, mut queues): (MessageWriter<QueueVoice>, ResMut<VoiceQueues>),
    mut messages: MessageWriter<ShowCaption>,
    mut change: MessageWriter<ChangeLevelTo>,
) {
    let Some(mut level) = level else { return };
    let level = &mut *level;
    let Some(mut v) = level.victory.take() else { return };
    let now = level.now;
    let spawn = |model: &Arc<CharacterModel>, at: [f32; 3], commands: &mut Commands| {
        let e = model.spawn(Transform::from_translation(Vec3::from(at)), commands);
        commands.entity(e).insert(LevelEntity);
        e
    };
    let runes = party.states().fold(0, |bits, (_, s)| bits | s.runestone_bits());
    // The first skorne asks for the twelve runestones of the realms.
    let all_twelve = runes & ALL_RUNESTONES == ALL_RUNESTONES;

    // The key: its clip, then the second model for 30 s (its clip over
    // and over).
    if let Some((e, until, second)) = v.key {
        if now >= until {
            commands.entity(e).try_despawn();
            v.key = match (&level.end.key_after, second) {
                (Some(m), false) => Some((spawn(m, v.key_at, &mut commands), now + KEY_AFTER, true)),
                _ => None,
            };
            debug!("the boss key {}", if v.key.is_some() { "turns to its second model" } else { "goes" });
        } else if second
            && let Ok(mut a) = animators.get_mut(e)
            && a.finished()
        {
            a.play(0);
        }
    }
    match v.step {
        0 => {
            // Every player's record (the boss's death marks them all).
            for (_, s) in party.states_mut() {
                s.realms_beaten |= 1 << level.realm_id;
            }
            // The health meters go.
            level.meters.hidden = true;
            info!("realm {} beaten", level.realm_id);
            if let Some((m, life)) = &level.end.key {
                v.key = Some((spawn(m, v.key_at, &mut commands), now + life, false));
                // Panned from where the key shows (the boss's spot).
                sounds.write(PlaySoundAt::panned(format!("S_BOSSKEY{}", level.realm), Vec3::from(v.key_at), SFXX_VOLUME));
                info!("the boss key shows at {:?} for {life:.2} s", v.key_at);
            }
            v.step = 1;
        }
        1 => {
            let wait = if matches!(level.boss_type, SKORNE | GARM) { WIZARD_DELAY_LONG } else { WIZARD_DELAY };
            v.timer = now + wait;
            v.step = 2;
        }
        2 if now >= v.timer => v.step = 3,
        3 => {
            let at = if matches!(level.boss_type, SKORNE | SKORNE2) {
                WIZARD_SKORNE_SPOT
            } else {
                let heroes: Vec<[f32; 3]> = players
                    .iter()
                    .filter(|p| party.state(p.slot).is_some_and(|s| s.alive))
                    .map(|p| p.mover.position)
                    .collect();
                let mut at = wizard_spot(level.boss_spot, &heroes);
                at[1] += WIZARD_RISE;
                at
            };
            v.wizard = level.end.wizard.as_ref().map(|m| spawn(m, at, &mut commands));
            v.wizard_at = at;
            info!("the wizard appears at {at:?}");
            queues.close_announcer();
            v.fade = WIZARD_FADE;
            v.step = 4;
        }
        4 => {
            v.fade -= WIZARD_FADE_STEP;
            if v.fade <= 0 {
                let speech = if level.boss_type == SKORNE && all_twelve { 1 } else { 0 };
                speak(level, speech, &mut voices);
                let pages = show_message(level, first_message(level.boss_type), &mut messages);
                v.timer = now + pages + AFTER_FIRST_SPEECH;
                v.step = 5;
            }
        }
        5 if now >= v.timer => {
            let held = runes_held(level.realm_id, runes);
            if level.boss_type < SKORNE {
                speak(level, held + 1, &mut voices);
            }
            let pages = show_message(level, second_message(level.boss_type, held, all_twelve), &mut messages);
            v.timer = now + pages + if pages > 0.0 { AFTER_SECOND_SPEECH } else { 0.0 };
            v.step = 6;
        }
        6 if now >= v.timer => {
            v.countdown = if level.boss_type == SKORNE && all_twelve { COUNTDOWN_LONG } else { COUNTDOWN };
            v.step = 9;
        }
        9..=11 => {
            v.countdown -= DT;
            if v.step == 9 && !queues.busy() {
                v.step = 10;
            }
            if v.step == 10 && v.countdown <= TELEPORT_LEFT {
                debug!("the heroes teleport out");
                for mut p in &mut players {
                    if party.state(p.slot).is_some_and(|s| s.alive) {
                        p.light = Some(crate::going_out::DeathLight::new(crate::going_out::TELEPORT_LIGHT_STEP));
                    }
                }
                v.step = 11;
            }
            if v.countdown <= 0.0 && !v.over {
                // The key and the wizard go with the level.
                info!("the boss level is over: to {AFTER_BOSS_LEVEL}");
                change.write(ChangeLevelTo::finishing(AFTER_BOSS_LEVEL));
                v.over = true;
            }
        }
        _ => {}
    }
    // The wizard plays his first clip over and over.
    if let Some(mut a) = v.wizard.and_then(|w| animators.get_mut(w).ok())
        && a.finished()
    {
        a.play(0);
    }
    level.victory = Some(v);
}

/// Queues the wizard's speech `n` in the announcer's voice queue: it plays
/// once the lines before it are over.
fn speak(level: &CritterLevel, n: usize, voices: &mut MessageWriter<QueueVoice>) {
    let Some(name) = level.end.speeches.get(n).cloned().flatten() else { return };
    info!("the wizard says {name}");
    voices.write(QueueVoice::announcer(name, SPEECH_MOST_WAIT));
}

/// Types one of the wizard's messages in the top bar, a page at a time,
/// each held a second once typed (`message_box.rs`): how long that takes,
/// 0 without one.
fn show_message(level: &CritterLevel, group: Option<&'static str>, messages: &mut MessageWriter<ShowCaption>) -> f32 {
    let Some((group, pages)) = group.and_then(|g| Some((g, level.end.texts.get(g)?))) else { return 0.0 };
    info!("the wizard's message {group}: {:?}", pages.join(" "));
    messages.write(ShowCaption { file: TextFile::English, group: group.into(), index: None, y: SPEECH_Y, stay: false, text: None });
    message_box::caption_seconds(pages)
}

/// The wizard's first message for the boss (a `TEXT/ENGLISH.ROM` group).
fn first_message(boss_type: i32) -> Option<&'static str> {
    Some(match boss_type {
        DRAGON => "DRAGON_SPEECH",
        CHIMERA => "CHIMERA_SPEECH",
        DJINN => "DJINN_SPEECH",
        DRIDER => "DRIDER_SPEECH",
        PBOSS => "PBOSS_SPEECH",
        YETI => "YETI_SPEECH",
        WRAITH => "WRAITH_SPEECH",
        LICH => "LICH_SPEECH",
        SKORNE => "SKORNE1_SPEECH",
        SKORNE2 => "SKORNE2_SPEECH",
        GARM => "GARM2_SPEECH",
        _ => return None,
    })
}

/// His second: by how many of the realm's runestones the heroes hold
/// ([`runes_held`]); after the first skorne, whether they hold all twelve.
fn second_message(boss_type: i32, held: usize, all_twelve: bool) -> Option<&'static str> {
    match boss_type {
        DRAGON..=LICH => ["RUNE_PHRASE0", "RUNE_PHRASE1", "RUNE_PHRASE1B", "RUNE_PHRASE2"].get(held).copied(),
        SKORNE => Some(if all_twelve { "SKORNE1_RUNE_YES" } else { "SKORNE1_RUNE_NO" }),
        _ => None,
    }
}

/// Where the wizard stands: the mean of the boss's spot and the heroes,
/// at the first hero's height (the game counts it twice in place of the
/// spot's).
fn wizard_spot(spot: [f32; 3], heroes: &[[f32; 3]]) -> [f32; 3] {
    let mut sum = spot;
    for (i, h) in heroes.iter().enumerate() {
        sum = add(sum, *h);
        if i == 0 {
            sum[1] = 2.0 * h[1];
        }
    }
    scale(sum, 1.0 / (1 + heroes.len()) as f32)
}

/// Seconds per 30 Hz tick: critters keep time in seconds.
const DT: f32 = 1.0 / 30.0;
/// Enemy types of the placed critters, and the first boss type.
const GOLEM: i32 = enemy::GOLEM;
const GARGOYLE: i32 = enemy::GARGOYLE;
const GENERAL: i32 = enemy::GENERAL;
const FIRST_BOSS: i32 = 0x22;
/// A placed critter comes (its statue starts to wake, or the general is
/// made) once its spot is on screen — a sphere of this many times its
/// radius — and within this of a hero (as placed monsters do).
const PLACED_SCREEN_RADIUS: f32 = 4.0;
const PLACED_RANGE: f32 = 50.0;
/// A `DAMG` of this kind throws down a safe rock (the yeti's boulders).
const MAKES_ROCK: i16 = 6;
/// A `DAMG` of this kind is a boss's loot (on its DEATH).
const LOOT: i16 = 9;
/// A `DAMG` of this kind is a held attack: its slot hurts steadily over
/// its whole radius (the others set down grow as blasts).
const HELD_ATTACK: i16 = 2;
/// Boss types the intro and the end treat apart.
const DRAGON: i32 = 0x22;
const CHIMERA: i32 = 0x23;
const DJINN: i32 = 0x24;
const DRIDER: i32 = 0x25;
const PBOSS: i32 = 0x26;
const YETI: i32 = 0x27;
const WRAITH: i32 = 0x28;
const LICH: i32 = 0x29;
const SKORNE: i32 = 0x2A;
const SKORNE2: i32 = 0x2B;
const GARM: i32 = 0x2C;
/// Bosses up to this type have an intro (not the second skorne or garm).
const LAST_INTRO_BOSS: i32 = 0x2A;

/// The boss intro's states (the game's `r13-0x725c`, `docs/critters.md`
/// "The boss intro").
mod intro {
    /// No intro: no hero brought the realm's legendary item.
    pub const NONE: i32 = 0;
    /// Until the boss's START ends.
    pub const START: i32 = 1;
    /// A wait while the level darkens.
    pub const WAIT: i32 = 2;
    /// The boss roars (still dark).
    pub const ROAR: i32 = 3;
    /// The roar is over; the level update moves 4 on to 5 at once.
    pub const ROARED: i32 = 4;
    pub const AFTER: i32 = 5;
    pub const FIGHT: i32 = 6;
    /// The boss is dead.
    pub const OVER: i32 = 99;
}
/// The intro's wait: 1 s for the chimera, the lich and the first skorne,
/// 3 s for the rest.
const INTRO_WAIT_SHORT: f32 = 1.0;
const INTRO_WAIT: f32 = 3.0;
/// Seconds in state 5 before the djinn, P-boss, yeti, wraith and first
/// skorne go on to 6.
const INTRO_AFTER: f32 = 29.0;
/// A missile hitting the dragon in states 2–3 freezes it, the djinn and the
/// P-boss are stunned (1200, 1800 and 18000 fields at 60 a second).
const DRAGON_FREEZE: f32 = 20.0;
const DJINN_STUN: f32 = 30.0;
const PBOSS_STUN: f32 = 300.0;
/// A stunned critter turns at this share of its rate.
const STUNNED_TURN: f32 = 0.1;
/// The level light's darkening in states 2–3: each tick asks for this
/// offset for this long; the offset moves at most these steps a tick; an
/// expired target decays by this factor and snaps to 0 below the last.
const DARKEN_TO: f32 = -0.8;
const DARKEN_HOLD: f32 = 0.1;
const DARKEN_STEP: f32 = -0.25;
const BRIGHTEN_STEP: f32 = 0.05;
const DARKEN_DECAY: f32 = 0.6;
const DARKEN_SNAP: f32 = 0.05;

/// The end of a boss level: the second key model shows this long; the
/// wizard appears 5 s after the boss goes (10 s after the first skorne and
/// garm), 3 above the heroes' centre (at a fixed spot for the skornes),
/// fading in 4 of 255 a tick; half a second after his first speech and a
/// second after his second the last countdown starts; the heroes teleport
/// out 35 fields before its end.
const KEY_AFTER: f32 = 30.0;
const WIZARD_DELAY: f32 = 5.0;
const WIZARD_DELAY_LONG: f32 = 10.0;
const WIZARD_RISE: f32 = 3.0;
const WIZARD_SKORNE_SPOT: [f32; 3] = [0.0, -12.0, 6.0];
const WIZARD_FADE: i32 = 255;
const WIZARD_FADE_STEP: i32 = 4;
const AFTER_FIRST_SPEECH: f32 = 0.5;
const AFTER_SECOND_SPEECH: f32 = 1.0;
const COUNTDOWN: f32 = 2.0;
const COUNTDOWN_LONG: f32 = 10.0;
/// The line his words are typed on: the top bar.
const SPEECH_Y: f32 = 16.0;
/// Runestones 0–11 (the first skorne wants them all).
const ALL_RUNESTONES: u32 = 0xFFF;
const TELEPORT_LEFT: f32 = 35.0 / 60.0;
/// A speech is dropped when it would wait longer than this, seconds.
const SPEECH_MOST_WAIT: f32 = 10.0;
/// Where the heroes go after a boss (stand-in: the game picks the first
/// level of world 13 flagged for it).
const AFTER_BOSS_LEVEL: &str = "levelL1";

/// A wake trigger wakes the nearest statue within this (horizontally,
/// less the statue item's radius).
const WAKE_REACH: f32 = 10.0;
/// A boss sleeps at least this long before it may wake.
const BOSS_WAKE_DELAY: f32 = 2.0;
/// `TYPE +0x5C` flags: hit spheres, a boxed leash, a wake timer started by
/// the floor, free turning.
const TYPE_SPHERES: u32 = 0x2;
const TYPE_BOX_LEASH: u32 = 0x20;
const TYPE_FLOOR_WAKE: u32 = 0x80;
const TYPE_FREE_TURN: u32 = 0x400;

/// Anger: 0.5 at full health up to 5 near death.
const ANGER_SPAN: f32 = 4.5;
const ANGER_BASE: f32 = 0.5;
/// Below this anger an idle critter taunts.
const TAUNT_ANGER: f32 = 0.8;
/// Damage taken lately that makes it roar (× the player-count table, 1 for
/// one player).
const ROAR_DAMAGE: f32 = 50.0;
/// Damage taken is forgotten this long after the last blow.
const DAMAGE_MEMORY: f32 = 3.0;
/// A player a critter hit can't be hit by one again for this long, and
/// scores × [`RECENTLY_HIT`] as a target meanwhile.
const HIT_GUARD: f32 = 0.25;
const RECENTLY_HIT: f32 = 1000.0;
/// Scores at or above this mean a condition failed.
const REJECTED: f32 = 1.0e21;
/// Bosses track at most this many players.
const MAX_TARGETS: usize = 4;
/// A blocking critter takes this share of a blow.
const BLOCKED: f32 = 0.25;
/// Share of its experience every player earns for the kill.
const KILL_EXPERIENCE: f32 = 0.2;
/// A boss takes blows × this for 0..4 players (outside its intro).
const BOSS_DAMAGE_BY_PLAYERS: [f32; 5] = [1.0, 1.0, 0.5, 0.3, 0.2];
/// The roar damage is × this for 0..4 players.
const ROAR_BY_PLAYERS: [f32; 5] = [1.0, 1.0, 1.5, 2.0, 2.0];
/// Knockback per unit of push by blow kind, the golem's reduction, cap,
/// decay per tick, stop threshold and upward fall per second.
const KNOCK_HEAVY: f32 = 10.0;
const KNOCK_KNOCKDOWN: f32 = 7.5;
const KNOCK_STRONG: f32 = 5.0;
const KNOCK_DEAD: f32 = 20.0;
const KNOCK_GOLEM: f32 = 5.0;
const KNOCK_MAX: f32 = 40.0;
const KNOCK_DECAY: f32 = 0.8;
const KNOCK_STOP: f32 = 0.01;
const KNOCK_FALL: f32 = 100.0;
/// Seconds per animation frame when recording when a move ends.
const FRAME_TIME: f32 = 1.0 / 30.0;
/// Ticks after its event before `GDL_CRITTER_SHOT` saves its screenshot.
const SHOT_DELAY: u32 = 8;
/// When a move that has never run "ended".
const NEVER: f32 = -1.0e6;
/// A critter drops at most this fast (units/s).
const MAX_DROP: f32 = 16.0;
/// Missiles: the span of the launch speed range anger reaches (from 0.5
/// to 1.5 anger), and the launch direction's drop when it aims at nothing.
const SPEED_SPAN: f32 = 0.75;
const ANGER_CAP: f32 = 1.5;
const UNAIMED_DROP: f32 = -0.5;

/// Blow kind bits.
/// Shrunk (`EnemyScale`), critters other than bosses take twice the
/// damage and deal half.
const SHRUNK_TAKE: f32 = 2.0;
const SHRUNK_DEAL: f32 = 0.5;
const KIND_STRONG: u32 = 0x10;
const KIND_KNOCKDOWN: u32 = 0x20;
const KIND_HEAVY: u32 = 0x100;
const KIND_REACTIONS: u32 = 0x130;
/// Blow kinds that don't land on a hit sphere (they hit the body, and a
/// blow it lives through flashes it).
const KIND_BODY: u32 = 0x100320;
/// Blows of this kind play no hit effect and flash nothing.
const KIND_NO_HIT_LOOK: u32 = 0x1000000;

/// One critter file loaded for the level, with a model per body.
pub struct CritterKind {
    pub file: CritterFile,
    bodies: Vec<Option<Body>>,
    /// The statue a placed one stands as until it wakes.
    statue: Option<CharacterModel>,
    /// Missile models: effect atrees named by its `SFXX` records; and
    /// each one's clip (its first action's frames and rate), also for the
    /// effects the folder lacks that the effect table's own bank holds.
    effects: HashMap<usize, Arc<CharacterModel>>,
    effect_clips: HashMap<usize, (u16, u16)>,
    /// Its folder (`MONSTERS/DRAGON`: its 2D meter's sprites too), and its
    /// 3D health meter (`GMETER`) when a type has one.
    folder: String,
    solid_meter: Option<CharacterModel>,
}

/// A body's model and what its moves animate.
struct Body {
    /// The model (parts have none: they move a subtree of their body's).
    model: Option<CharacterModel>,
    /// A part's subtree of its body's skeleton: the node named by `TYPE
    /// +0x10` and everything under it; their rest offsets, and their tracks
    /// by action.
    subtree: Vec<usize>,
    rest: Vec<Vec3>,
    tracks: Vec<Vec<Option<Track>>>,
    /// Per move (within the type): its action (a missing one plays the
    /// first, as the game does), and its node.
    actions: Vec<usize>,
    nodes: Vec<Option<usize>>,
    /// Per `NODE` record of the type: its skeleton node (none: the root).
    spheres: Vec<Option<usize>>,
    /// Per action: frames, rate, loops.
    clips: Vec<(u16, u16, bool)>,
}

/// A placed critter waiting as a statue.
struct Statue {
    placement: usize,
    enemy: i32,
    position: [f32; 3],
    yaw: f32,
    /// The item type's radius (the wake reach is measured to its edge).
    radius: f32,
    entity: Option<Entity>,
    waking: bool,
    done: bool,
    /// The powerup it holds (its placement number), dropped when its
    /// critter dies.
    holds: Option<usize>,
}

#[derive(Component)]
struct StatueModel;

/// What a dead placed critter leaves: the item it held, or — a gargoyle
/// holding nothing — a gargoyle piece of its kind.
struct CritterDrop {
    held: Option<usize>,
    class: i16,
    at: [f32; 3],
}

/// Fields before a dropped item can be picked up (a stand-in: the game
/// tosses it out, and it can be had where it lands).
const DROP_DELAY: i32 = 30;

/// A target on a critter: one of its hit spheres (its type's `NODE`
/// records, numbered across the body and its parts), or — `node` none —
/// the body of the critter or of its part `part`, at its centre, which the
/// game falls back on when none of its spheres will do.
#[derive(Component, Clone, Copy, Debug)]
pub struct CritterSphere {
    pub critter: Entity,
    pub node: Option<usize>,
    pub part: Option<usize>,
}

/// The level's critter state.
#[derive(Resource)]
pub struct CritterLevel {
    kinds: HashMap<i32, Arc<CritterKind>>,
    statues: Vec<Statue>,
    /// Seconds since the level started.
    now: f32,
    realm: char,
    hit_point_scale: f32,
    speed_scale: f32,
    damage_scale: f32,
    /// The enemies' scale this tick (the shrink power's, `EnemyScale`),
    /// and whether time is stopped (`TimeStop`).
    enemy_scale: f32,
    time_stopped: bool,
    /// Per player: until when blows can't hit it — the game's one guard
    /// (`+0x8E8`) that the missiles and blasts set too (taken from and
    /// given back to `projectiles::PlayerGuard` each tick; fixed-clock
    /// seconds, `clock` now).
    guard: HashMap<Entity, f64>,
    clock: f64,
    /// Players in the game as the level began (the bosses' blows and roar
    /// scale by them).
    pub players: usize,
    /// The boss, once made; whether it has died (its DEATH has played:
    /// the game's `r13-0x7784`).
    pub boss: Option<Entity>,
    pub boss_dead: bool,
    /// The level's boss type (`LEVL +0x44`, −1 for none) and its realm's
    /// number.
    boss_type: i32,
    realm_id: u32,
    /// The boss intro: its state ([`intro`]) and timer, and whether the
    /// legendary item that started it has been used up.
    pub intro: i32,
    intro_timer: f32,
    legendary_used: bool,
    /// The scene light's darkening during the intro.
    light: Darkening,
    /// The boss locator's position; the end's models; the end, once the
    /// boss is gone.
    boss_spot: [f32; 3],
    end: EndModels,
    victory: Option<Victory>,
    /// Events this tick, for `GDL_CRITTER_SHOT_ON`.
    events: Vec<&'static str>,
    /// Stand-in look for critter missiles: their effect models are drawn by
    /// the effects system (textures it supplies), which isn't done.
    glow: (Handle<Mesh>, Handle<StandardMaterial>),
    rng: u32,
    /// The items the placed critters hold, and what the dead ones leave
    /// this tick; the realm's gargoyle piece (`GARG<kind>`).
    held: HashMap<Entity, usize>,
    drops: Vec<CritterDrop>,
    gargoyle_piece: String,
    /// Placements whose critters were made this tick (their items go).
    made: Vec<usize>,
    /// A dying boss's loot, for `loot.rs`.
    loot: Vec<BossLoot>,
    /// Blows at the safe rocks this tick (kinds 5 and 6); the rocks the
    /// boss threw down, with the seconds till each is made; the one it
    /// threw last, by its number among the rocks (−1 none: the round
    /// starts at a random one as the boss gathers them).
    rock_blows: Vec<RockBlow>,
    rock_timers: Vec<(usize, f32)>,
    last_rock: i32,
    /// The critters' still effects set down this tick (`critter_effects`),
    /// and whether one of the effects started shakes the camera.
    area_blasts: Vec<CritterBlast>,
    shake: bool,
    /// Effects the critters' folders don't hold, from the effect bank.
    bank_effects: Vec<BankEffect>,
    /// Heroes grabbed, thrown and let go this tick (`player.rs`).
    grabs: Vec<GrabHero>,
    throws: Vec<ThrowHero>,
    releases: Vec<Entity>,
    /// The 2D health meters (`meter.rs`).
    meters: meter::Meters,
}

/// A blow of kind 5 (at every safe rock) or 6 (throwing one down): its
/// effect at the rock(s), a blast hurting the heroes about it.
struct RockBlow {
    critter: Entity,
    every: bool,
    /// The feet of the blow's target, if it has one.
    target: Option<[f32; 3]>,
    damage: f32,
    radius: f32,
    kind: u32,
    effect: Option<Arc<CharacterModel>>,
    /// The effect's clip: its frames (a thrown rock is made as they run
    /// out) and how long it lasts.
    frames: u16,
    life: f32,
    /// Its `SFXX` record's sounds, played at each rock.
    sounds: Vec<PlaySoundAt>,
}

/// An `SFXX` record's flags: 2 shakes the camera as it starts; 1 (or
/// 0x800) attaches its effect to the critter's root, 0x80 puts it at the
/// critter's spawn point, 0x40 at the move's node.
const SFXX_SHAKES: u32 = 0x2;
const SFXX_ON_ROOT: u32 = 0x801;
const SFXX_AT_SPAWN: u32 = 0x80;
const SFXX_AT_NODE: u32 = 0x40;
/// The shake it starts: the target round a 0.1 circle for 90 fields,
/// priority 100.
const SFXX_SHAKE: crate::play_camera::Shake =
    crate::play_camera::Shake { amplitude: 0.1, what: 0, delay: 0.0, fields: 90.0, priority: 100 };

/// A missile blow's flags: 0x1000 flies past the heroes, 0x40 through
/// walls and items.
const NO_PLAYERS: u16 = 0x1000;
const NO_LEVEL: u16 = 0x40;

/// The most safe rocks a boss gathers.
const MAX_ROCKS: usize = 16;
/// A critter's `SFXX` sounds play at this requested volume, from where it
/// stands, faded by its distance from the heroes (panned only during its
/// DEATH move: not done yet, `audio::PlaySoundAt`).
const SFXX_VOLUME: u8 = 0xE0;

impl CritterLevel {
    /// The damage after which a critter roars, by the players in the game.
    fn roar_at(&self) -> f32 {
        ROAR_DAMAGE * ROAR_BY_PLAYERS[self.players.min(4)]
    }

    /// The offset the game adds to the scene's brightness (0 normally, down
    /// to −0.8 in the boss intro; the brightness is clamped to 0..1). The
    /// renderer doesn't apply it yet.
    pub fn light_offset(&self) -> f32 {
        self.light.offset
    }

    /// What the machines compare online (`online.rs`).
    pub fn sync_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.rng, sync_bits(self.now), sync_bits64(self.clock), self.intro, self.boss_dead, self.players).hash(&mut h);
        h.finish()
    }

    fn random(&mut self) -> f32 {
        // xorshift32; the game has its own generator.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// The scene-light offset (the game's `r13-0x7170`): a caller asks for a
/// target for a while; every frame the offset moves toward the target,
/// quickly down and slowly up, and a target nobody asks for any more
/// decays to 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Darkening {
    offset: f32,
    target: f32,
    until: f32,
}

impl Darkening {
    fn ask(&mut self, now: f32, hold: f32, target: f32) {
        let until = now + hold + DT;
        if until > self.until {
            self.until = until;
            self.target = target;
        }
    }

    fn step(&mut self, now: f32) {
        if self.target != 0.0 && self.until < now {
            self.target *= DARKEN_DECAY;
            if self.target.abs() < DARKEN_SNAP {
                self.target = 0.0;
            }
        }
        self.offset += (self.target - self.offset).clamp(DARKEN_STEP, BRIGHTEN_STEP);
    }
}

/// The models of a boss level's end, from the level's own item set
/// (`ITEMS/<level>`): the key (and its clip's length), its second model,
/// the wizard; and the wizard's speeches for this boss.
#[derive(Default)]
struct EndModels {
    key: Option<(Arc<CharacterModel>, f32)>,
    key_after: Option<Arc<CharacterModel>>,
    wizard: Option<Arc<CharacterModel>>,
    speeches: Vec<Option<String>>,
    /// The wizard's messages for this boss: pages by group.
    texts: HashMap<&'static str, Vec<String>>,
}

/// The end of a boss level, from the boss's removal: the game's end
/// sequence, its steps numbered as the game numbers them.
struct Victory {
    step: u8,
    timer: f32,
    /// Where the key shows: the boss's spawn position plus `TYPE +0xD0`.
    key_at: [f32; 3],
    /// The key's model, until when it shows, and whether it's the second.
    key: Option<(Entity, f32, bool)>,
    wizard: Option<Entity>,
    /// Where the wizard stands.
    wizard_at: [f32; 3],
    fade: i32,
    countdown: f32,
    /// The countdown has run out and the level change is asked for.
    over: bool,
}

/// What the boss camera follows (`boss_camera.rs`), refreshed each tick:
/// the boss, its spot, whether it has woken (the game's `r13-0x7788`, kept
/// once set) and died (`r13-0x7784`), and the end's key and wizard.
#[derive(Resource, Default, Clone)]
pub struct BossWatch {
    pub boss: Option<crate::boss_camera::Boss>,
    pub spot: Option<[f32; 3]>,
    pub awake: bool,
    pub dead: bool,
    pub ending: bool,
    pub key: Option<[f32; 3]>,
    pub wizard: Option<[f32; 3]>,
    /// The boss it was made for (a new level's starts over).
    of: Option<Entity>,
}

pub(crate) fn watch_boss(level: Option<Res<CritterLevel>>, critters: Query<&Critter>, mut watch: ResMut<BossWatch>) {
    let Some(level) = level else {
        *watch = BossWatch::default();
        return;
    };
    if watch.of != level.boss {
        *watch = BossWatch { of: level.boss, ..BossWatch::default() };
    }
    let boss = level.boss.and_then(|e| critters.get(e).ok());
    watch.spot = level.boss.map(|_| level.boss_spot);
    watch.boss = boss.map(|c| crate::boss_camera::Boss {
        position: c.position,
        spawn: c.spawned_at,
        yaw: c.yaw,
        height: c.kind.file.types[c.ty].height,
        dying: c.state == CritterState::Dying,
    });
    watch.awake |= boss.is_some_and(|c| c.state != CritterState::New);
    watch.dead = level.boss_dead;
    watch.ending = level.victory.is_some();
    watch.key = level.victory.as_ref().and_then(|v| v.key.map(|_| v.key_at));
    watch.wizard = level.victory.as_ref().and_then(|v| v.wizard.map(|_| v.wizard_at));
}

/// The game's instance state (0 new, 1 dying, 3 active).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CritterState {
    New,
    Dying,
    Active,
}

/// A target the critter tracks (the game keeps up to four).
#[derive(Clone, Copy, Debug)]
struct Tracked {
    player: Entity,
    /// Horizontal distance from its centre, and the unit direction.
    distance: f32,
    direction: [f32; 2],
    position: [f32; 3],
    score: f32,
    /// Scales the score (the game weighs damage dealt against taken).
    weight: f32,
    /// The player's centre (missiles aim here).
    centre: [f32; 3],
}

/// A critter's own animation clock (the game's controller: frame, whether
/// the clip has come to its end).
#[derive(Clone, Copy, Debug)]
struct Clock {
    action: usize,
    frame: f32,
    frames: u16,
    rate: u16,
    loops: bool,
    ended: bool,
}

impl Clock {
    fn start(&mut self, action: usize, clip: (u16, u16, bool)) {
        *self = Clock { action, frame: 0.0, frames: clip.0, rate: clip.1, loops: clip.2, ended: false };
    }

    /// Moves the clip on at its rate (900 / rate frames a second): a loop
    /// has ended on the tick it starts over, any other clip once it's past
    /// its last frame.
    fn advance(&mut self, dt: f32) {
        let wrapped = advance_clip(&mut self.frame, dt, self.frames, self.rate, self.loops);
        self.ended = if self.loops { wrapped } else { self.frame >= clip_end(self.frames) };
    }
}

/// A live critter.
#[derive(Component)]
pub struct Critter {
    kind: Arc<CritterKind>,
    ty: usize,
    pub state: CritterState,
    pub hit_points: f32,
    pub full_hit_points: f32,
    /// Root position (feet, plus its hover height), and where it was made.
    pub position: [f32; 3],
    pub(crate) spawned_at: [f32; 3],
    pub yaw: f32,
    home_yaw: f32,
    pub(crate) home: [f32; 3],
    previous: ([f32; 3], f32),
    pub(crate) floor: f32,
    /// The collision node of the floor it stands on: a moving one carries
    /// it (`mechanics.rs`).
    pub(crate) ground_node: Option<usize>,
    /// Current and chosen move (within the type).
    current: Option<usize>,
    next: Option<usize>,
    /// The attack choice's target for the chosen move.
    pick: Option<Entity>,
    /// The current move's target.
    move_target: Option<Entity>,
    switched: bool,
    /// When each move last ended; when each pattern last started.
    ends: Vec<f32>,
    pattern_starts: Vec<f32>,
    /// The pattern running and its step, and the one just chosen.
    pattern: Option<(usize, usize)>,
    chosen_pattern: Option<usize>,
    /// A boss holds its move until this time.
    hold_until: f32,
    /// A sleeping boss may wake from this time.
    wake_at: f32,
    /// Blows and sounds done this move (bits 1, 2).
    blows_done: u8,
    sounds_done: u8,
    tracked: Vec<Tracked>,
    anger: f32,
    /// Damage taken lately, its kinds, summed push, time of the last blow.
    damage_taken: f32,
    kinds: u32,
    push: [f32; 3],
    last_blow: f32,
    knock: [f32; 3],
    clock: Clock,
    /// The move node's world position this tick and last.
    node_at: Option<[f32; 3]>,
    node_was: Option<[f32; 3]>,
    /// Per hit sphere: damage taken (it stops counting at its share of hit
    /// points), its entity, and whether it's spent (no longer a target).
    sphere_damage: Vec<f32>,
    spheres: Vec<Entity>,
    spent: Vec<bool>,
    /// Its body's target, at its centre.
    aim: Entity,
    blows_dealt: u32,
    /// The hero its grab holds (the game's `+0x128`).
    held: Option<Entity>,
    /// Hit points at the start of the last tick.
    hp_before: f32,
    /// The critter clock at its last tick (seconds since the level began).
    now: f32,
    /// A missile hit it since its last tick (the intro reacts).
    missile_hit: bool,
    /// Seconds it stays frozen (its animation and moves stop) and stunned
    /// (it turns slowly): the intro's reactions to missiles.
    frozen: f32,
    stunned: f32,
    /// A body's parts (the chimera's heads), updated with it.
    parts: Vec<Critter>,
    /// A body's hit spheres, all of them: whose (a part, or the body) and
    /// which of its own.
    sphere_owner: Vec<(Option<usize>, usize)>,
    /// A part copies its body's animation this tick (the usual case)
    /// rather than playing its own move.
    mirrored: bool,
    /// Its hit flash (`flash.rs`): a body's over its whole model, a part's
    /// over its subtree.
    flash: Flash,
    /// Its 2D health meter, by number (`meter.rs`).
    meter: Option<usize>,
}

impl Critter {
    fn moves(&self) -> &[CritterMove] {
        self.kind.file.type_moves(self.ty)
    }

    fn body(&self) -> &Body {
        self.kind.bodies[self.ty].as_ref().expect("critters are only made with a body")
    }

    fn class(&self) -> i16 {
        self.kind.file.desc.class
    }

    fn move_kind(&self, i: Option<usize>) -> Option<i32> {
        i.and_then(|i| self.moves().get(i)).map(|m| m.kind)
    }

    /// A blow from the hero, on hit sphere `sphere` (numbered across a body
    /// and its parts), or the body of part `part`, or the body (`ranged`: a
    /// missile or thrown weapon): returns the experience it earns. `sounds`
    /// gets the names of the sounds to play. A part's blow also comes off
    /// its body while the part lives on; a body's is shared out over its
    /// living parts, half of it split between them.
    #[allow(clippy::too_many_arguments)]
    pub fn take_hit(
        &mut self,
        damage: f32,
        kind_bits: u32,
        push: [f32; 3],
        sphere: Option<usize>,
        part: Option<usize>,
        ranged: bool,
        level: Option<&CritterLevel>,
        sounds: &mut Vec<String>,
    ) -> u32 {
        let owner = sphere.and_then(|g| self.sphere_owner.get(g).copied());
        let on_part = match owner {
            Some((k, local)) => k.map(|k| (k, Some(local))),
            None => part.map(|k| (k, None)),
        };
        if let Some((k, local)) = on_part {
            let Some(p) = self.parts.get_mut(k) else { return 0 };
            let (mut xp, dealt) = p.hit_self(damage, kind_bits, push, local, ranged, level, sounds);
            if dealt > 0.0 && p.hit_points > 0.0 && self.state == CritterState::Active {
                self.hit_points -= dealt;
                if self.hit_points <= 0.0 {
                    xp += self.die();
                }
            }
            return xp;
        }
        let local = owner.map(|(_, i)| i).or(sphere);
        let (mut xp, dealt) = self.hit_self(damage, kind_bits, push, local, ranged, level, sounds);
        let living = self.parts.iter().filter(|p| p.state == CritterState::Active).count();
        if dealt > 0.0 && self.hit_points > 0.0 && living > 0 {
            let each = 0.5 * dealt / living as f32;
            for p in self.parts.iter_mut().filter(|p| p.state == CritterState::Active) {
                p.hit_points -= each;
                if p.hit_points <= 0.0 {
                    xp += p.die();
                }
            }
        }
        xp
    }

    /// Dies: the experience every player earns for the kill (a fifth of
    /// its type's). A body's parts die with it.
    fn die(&mut self) -> u32 {
        self.state = CritterState::Dying;
        for p in &mut self.parts {
            if p.state != CritterState::Dying {
                p.hit_points = p.hit_points.min(0.0);
                p.state = CritterState::Dying;
            }
        }
        (KILL_EXPERIENCE * self.kind.file.types[self.ty].experience) as u32
    }

    /// A blow on this critter alone: the experience and the hit points it
    /// took.
    #[allow(clippy::too_many_arguments)]
    fn hit_self(
        &mut self,
        damage: f32,
        kind_bits: u32,
        push: [f32; 3],
        sphere: Option<usize>,
        ranged: bool,
        level: Option<&CritterLevel>,
        sounds: &mut Vec<String>,
    ) -> (u32, f32) {
        if self.state != CritterState::Active || self.hit_points <= 0.0 {
            return (0, 0.0);
        }
        let (realm, intro) = level.map_or(('A', intro::NONE), |l| (l.realm, l.intro));
        self.missile_hit |= ranged;
        let ty = self.kind.file.types[self.ty].clone();
        let boss = self.class() == class::BOSS;
        let (mut damage, mut kind_bits) = (damage, kind_bits);
        if self.move_kind(self.current) == Some(kind::BLOCK) {
            kind_bits &= !KIND_REACTIONS;
            damage *= BLOCKED;
        }
        let boss_level = level.is_some_and(|l| l.boss_type >= 0);
        damage = crate::damage::resist(damage, &mut kind_bits, ty.armor, ty.resist, boss_level);
        // Shrunk, all but bosses take twice.
        if !boss && level.is_some_and(|l| l.enemy_scale < 1.0) {
            damage *= SHRUNK_TAKE;
        }
        self.damage_taken += damage;
        // Bosses take less with more players, except in their intro's
        // first four states.
        let players = level.map_or(1, |l| l.players).min(4);
        if boss && !(intro::START..=intro::ROARED).contains(&intro) {
            damage *= BOSS_DAMAGE_BY_PLAYERS[players];
        }
        let share = damage.clamp(0.0, self.hit_points.max(0.0)) / (1.0 + self.full_hit_points);
        let mut xp = (share * ty.experience) as u32;
        if boss {
            xp *= players as u32;
        }
        // A blow on a hit sphere that still has hit points is scaled by it,
        // up to what it has left; once a sphere is spent, blows on it land
        // on the body in full (the game skips the sphere then).
        if kind_bits & KIND_BODY == 0
            && let Some(n) = sphere
            && let Some(node) = self.kind.file.type_nodes(self.ty).get(n)
        {
            let full = node.hit_points * self.full_hit_points;
            let taken = self.sphere_damage[n];
            if taken < full {
                damage = (damage * node.damage_scale).min(full - taken);
                self.sphere_damage[n] = taken + damage;
            }
        }
        if damage <= 0.0 {
            return (xp, 0.0);
        }
        self.kinds |= kind_bits;
        self.push = add(self.push, push);
        self.last_blow = self.now;
        self.hit_points -= damage;
        if self.hit_points <= 0.0 {
            return (xp + self.die(), damage);
        }
        // Blows past the hit spheres flash it. (A blow on a hit sphere
        // flashes that sphere's node in the game: not done, as which model
        // node a sphere names isn't traced.)
        if kind_bits & KIND_BODY != 0 && kind_bits & KIND_NO_HIT_LOOK == 0 {
            self.flash.start();
        }
        if let Ok(s) = usize::try_from(ty.hit_effects[0]) {
            sound_chain(&self.kind.file, s, realm, sounds);
        }
        (xp, damage)
    }
}

/// An `SFXX` record starting (the game's effect start): its sound and
/// those of the records chained after it, from where the critter stands
/// at 0xE0 — faded by its distance from the heroes, panned only during its
/// DEATH move. Returns whether one of them shakes the camera (flag 2).
fn effect_sounds(c: &Critter, record: usize, realm: char, out: &mut Vec<PlaySoundAt>) -> bool {
    let mut names = Vec::new();
    sound_chain(&c.kind.file, record, realm, &mut names);
    let at = Vec3::from(c.position);
    let dying = c.move_kind(c.current) == Some(kind::DEATH);
    out.extend(names.into_iter().map(|n| {
        if dying { PlaySoundAt::panned(n, at, SFXX_VOLUME) } else { PlaySoundAt::faded(n, at, SFXX_VOLUME) }
    }));
    let (mut next, mut shakes, mut guard) = (Some(record), false, 0);
    while let Some(r) = next.and_then(|i| c.kind.file.sounds.get(i)) {
        shakes |= r.flags & SFXX_SHAKES != 0;
        next = usize::try_from(r.next).ok();
        guard += 1;
        if guard > 8 {
            break;
        }
    }
    shakes
}

/// The names of an `SFXX` record's sounds and those chained after it.
fn sound_chain(file: &CritterFile, first: usize, realm: char, out: &mut Vec<String>) {
    let mut next = Some(first);
    let mut guard = 0;
    while let Some(i) = next {
        let Some(s) = file.sounds.get(i) else { break };
        if !s.sound.is_empty() {
            out.push(s.sound.replace("%c", &realm.to_string()));
        }
        next = usize::try_from(s.next).ok();
        guard += 1;
        if guard > 8 {
            break;
        }
    }
}

/// Loads the critter files the level's enemy slots name (the golem and the
/// boss for now), their models; places the statues and makes the boss.
#[allow(clippy::too_many_arguments)]
fn setup_level(
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    monsters: Res<MonsterLevel>,
    population: Option<Res<LevelPopulation>>,
    ground: Option<Res<LevelGround>>,
    party: Res<Party>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut items: ResMut<LevelItems>,
    texanims: Option<ResMut<LevelTexAnims>>,
) {
    let (Some(population), Some(ground)) = (population, ground) else { return };
    let realm_id = REALM_LETTERS.iter().find(|(l, _)| *l == monsters.realm).map_or(1, |r| r.1);
    let realm_items = format!("level{}", monsters.realm);
    let mut kinds = HashMap::new();
    let mut animated = Vec::new();
    let mut bank = None;
    let wanted = monsters.enemies.loaded.iter().map(|e| e.0).filter(|&e| matches!(e, GOLEM | GARGOYLE | GENERAL) || e >= FIRST_BOSS);
    let gargoyle = monsters.enemies.gargoyle.as_str();
    for enemy in wanted {
        if kinds.contains_key(&enemy) {
            continue;
        }
        let Some(file_name) = critter::file_for_enemy(enemy, realm_id, gargoyle) else { continue };
        let path = format!("CRITTER/{file_name}");
        let file = match game.install.read(&path).map_err(|e| e.to_string()).and_then(|b| CritterFile::parse(&b).map_err(|e| e.to_string())) {
            Ok(f) => f,
            Err(e) => {
                warn!("{path}: {e}");
                continue;
            }
        };
        // A boss that makes its own safe rocks starts without the level's.
        if enemy >= FIRST_BOSS && file.damage.iter().any(|d| d.kind == MAKES_ROCK) {
            let n = items.hide_safe_rocks();
            info!("{path}: the boss throws down the safe rocks; {n} hidden until then");
        }
        let folder = format!("MONSTERS/{}", critter::model_folder(&file.desc, &realm_items, gargoyle));
        let Some((anim, model, textures, texmods)) = read_folder(&mut game, &folder) else {
            warn!("{folder}: can't read the critter's models");
            continue;
        };
        let data = |name: &str| -> Option<CharacterData> {
            let tree = anim.atrees.iter().find(|a| a.name.eq_ignore_ascii_case(name))?;
            Some(CharacterData {
                name: format!("{folder}/{name}"),
                class: String::new(),
                colour: String::new(),
                skeleton: tree.clone(),
                clips: Arc::new(tree.clone()),
                model: model.clone(),
                textures: textures.clone(),
            })
        };
        // Every model from the folder runs its running texture animations.
        let mut build = |d: &CharacterData| {
            let (model, anims) = CharacterModel::build_animated(d, &texmods, &mut meshes, &mut materials, &mut images);
            animated.extend(anims);
            model
        };
        let bodies = (0..file.types.len())
            .map(|ty| {
                let owner = file.types[ty].parent.unwrap_or(ty);
                let d = data(&file.atree_name(owner))?;
                let moves = file.type_moves(ty);
                let actions = moves
                    .iter()
                    .map(|m| d.clips.actions.iter().position(|a| a.name == m.anim).unwrap_or(0))
                    .collect();
                let node = |name: &str| (!name.is_empty()).then(|| d.skeleton.node_index(name)).flatten();
                let nodes = moves.iter().map(|m| node(&m.node)).collect();
                let spheres = file.type_nodes(ty).iter().map(|n| node(&n.name)).collect();
                let clips = d.clips.actions.iter().map(|a| (a.frames, a.rate, a.loops())).collect();
                let part = file.types[ty].parent.is_some();
                let subtree = if part { subtree_of(&d.skeleton, &part_node(&file.types[ty])) } else { Vec::new() };
                let rest = subtree.iter().map(|&n| Vec3::from(d.skeleton.nodes[n].offset)).collect();
                let tracks = if part {
                    (0..d.clips.actions.len())
                        .map(|a| {
                            subtree
                                .iter()
                                .map(|&n| {
                                    let bone = d.clips.node_index(&d.skeleton.nodes[n].name).and_then(|j| d.clips.clip_bone(j))?;
                                    d.clips.track(bone, a).ok().flatten()
                                })
                                .collect()
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let model = (!part).then(|| build(&d));
                Some(Body { model, subtree, rest, tracks, actions, nodes, spheres, clips })
            })
            .collect::<Vec<_>>();
        if bodies.first().is_none_or(Option::is_none) {
            warn!("{folder}: no atree {}", file.atree_name(0));
            continue;
        }
        // The golem and the gargoyle stand as statues till woken; the
        // general has none.
        let statue = match file.desc.class {
            class::GOLEM => data("GOL_STATUE").map(|d| build(&d)),
            class::GARGOYLE => data("GAR_STATUE").map(|d| build(&d)),
            _ => None,
        };
        // Missile models: the effect a projectile blow starts names an
        // atree of the critter's own folder (or of the effect table's bank:
        // NULLFX, unseen).
        let mut effects = HashMap::new();
        let mut effect_clips = HashMap::new();
        let records = file.damage.iter().filter(|d| d.kind != LOOT).flat_map(|d| [d.effects[0], d.effects[1]]);
        for e in records {
            let Ok(e) = usize::try_from(e) else { continue };
            let Some(name) = file.sounds.get(e).map(|s| s.effect.clone()) else { continue };
            if effects.contains_key(&e) || name.is_empty() {
                continue;
            }
            match data(&name) {
                Some(md) => {
                    debug!(
                        "{folder}: missile model {name}: nodes {:?}, objects {:?}",
                        md.skeleton.nodes.iter().map(|n| (n.name.as_str(), n.has_model(), n.hidden(), n.render_flags)).collect::<Vec<_>>(),
                        md.model
                            .objects
                            .iter()
                            .filter(|o| o.name.starts_with(&name))
                            .map(|o| (o.name.as_str(), o.submeshes.len(), o.flags))
                            .collect::<Vec<_>>()
                    );
                    if let Some(a) = md.clips.actions.first() {
                        effect_clips.insert(e, (a.frames, a.rate));
                    }
                    effects.insert(e, Arc::new(build(&md)));
                }
                None => match bank_clip(&mut game, &mut bank, &name) {
                    Some(clip) => {
                        debug!("{folder}: effect {name} from {EFFECT_BANK}, clip {clip:?}");
                        effect_clips.insert(e, clip);
                    }
                    None => debug!("{folder}: no missile model {name}"),
                },
            }
        }
        // The 3D health meter, for types that have one.
        let solid_meter = if file.types.iter().any(|t| t.flags & meter::SOLID != 0) {
            let m = data(meter::SOLID_MODEL).map(|d| build(&d));
            if m.is_none() {
                warn!("{folder}: no {}", meter::SOLID_MODEL);
            }
            m
        } else {
            None
        };
        info!(
            "critter {} ({path}) from {folder}: {} moves, statue {}, {} missile models, 3D meter {}",
            file.desc.name,
            file.moves.len(),
            statue.is_some(),
            effects.len(),
            solid_meter.is_some()
        );
        kinds.insert(enemy, Arc::new(CritterKind { file, bodies, statue, effects, effect_clips, folder, solid_meter }));
    }
    if let Some(mut texanims) = texanims {
        info!("{} texture animations on the level's critters", animated.len());
        texanims.extend(animated);
    }

    // Placed critters stand as statues (the general comes as soon as it's
    // seen).
    let pop = &population.population;
    let mut statues = Vec::new();
    for (placement, p) in pop.placements.iter().enumerate() {
        let ty = pop.resolved_type(p);
        if ty.class != ItemClass::EnemyInfo || !p.active_for(population.players) {
            continue;
        }
        let Some(enemy) = ty.enemy() else { continue };
        let Some(kind) = kinds.get(&enemy) else { continue };
        if kind.file.desc.class == class::BOSS {
            continue;
        }
        let mut position = p.position;
        if let Some(y) = ground.0.floor_height(position) {
            position[1] = y;
        }
        let m = rotation_matrix(p.rotation);
        let yaw = m[6].datan2(m[8]);
        let range = match p.params(ty.class) {
            PlacementParams::Enemy { range, .. } => range,
            _ => 0.0,
        };
        let entity = kind.statue.as_ref().map(|s| {
            let t = Transform::from_translation(Vec3::from(position)).with_rotation(Quat::from_rotation_y(yaw));
            let e = s.spawn(t, &mut commands);
            commands.entity(e).insert((StatueModel, LevelEntity));
            e
        });
        if entity.is_some() {
            items.mark_statue(placement);
        }
        debug!("critter {enemy:#x} statue at placement {placement} {position:?} (range {range})");
        statues.push(Statue {
            placement,
            enemy,
            position,
            yaw,
            radius: 0.5 * ty.extent[0].max(ty.extent[1]),
            entity,
            waking: kind.statue.is_none(),
            done: false,
            holds: items.hold_nearest(p.position),
        });
    }
    info!("critters: {} kinds, {} statues", kinds.len(), statues.len());
    let t = &monsters.tuning;
    // `GDL_CRITTER_HP=<scale>`: a testing aid that scales critters' hit
    // points (to see one die sooner).
    let testing_scale = std::env::var("GDL_CRITTER_HP").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let mut level = CritterLevel {
        players: party.len().max(1),
        kinds,
        statues,
        now: 0.0,
        realm: monsters.realm,
        hit_point_scale: t.monster_hit_points * testing_scale,
        speed_scale: t.monster_speed,
        damage_scale: t.monster_damage,
        enemy_scale: 1.0,
        time_stopped: false,
        guard: HashMap::new(),
        clock: 0.0,
        boss: None,
        boss_dead: false,
        boss_type: monsters.boss,
        realm_id,
        intro: intro::NONE,
        intro_timer: 0.0,
        legendary_used: false,
        light: Darkening::default(),
        boss_spot: [0.0; 3],
        end: EndModels::default(),
        victory: None,
        events: Vec::new(),
        glow: (
            meshes.add(Sphere::new(1.0)),
            standard.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.55, 0.1),
                emissive: LinearRgba::rgb(4.0, 1.6, 0.2),
                unlit: true,
                ..default()
            }),
        ),
        rng: 0x2545_F491,
        held: HashMap::new(),
        drops: Vec::new(),
        made: Vec::new(),
        loot: Vec::new(),
        gargoyle_piece: format!("GARG{}", monsters.enemies.gargoyle.to_ascii_uppercase()),
        rock_blows: Vec::new(),
        rock_timers: Vec::new(),
        last_rock: -1,
        area_blasts: Vec::new(),
        shake: false,
        bank_effects: Vec::new(),
        grabs: Vec::new(),
        throws: Vec::new(),
        releases: Vec::new(),
        meters: meter::Meters::default(),
    };

    // The boss appears at the level's boss locator, dropped onto the floor
    // below it.
    if monsters.boss >= FIRST_BOSS
        && let Some(kind) = level.kinds.get(&monsters.boss).cloned()
        && let Some(spot) = pop.locators.iter().find(|l| l.kind == LocatorKind::Boss)
    {
        // The boss's spot is turned by the game's locator builder (not
        // the placements'): its yaw is the stored one.
        let m = crate::population::locator_matrix(spot.rotation);
        let yaw = m[6].datan2(m[8]);
        let mut at = spot.position;
        if let Some(h) = ground.0.floor_probe(at, 4.0, -1000.0, 5.0, 2) {
            at[1] = h.point[1];
        }
        level.boss = spawn_critter(&mut level, &kind, at, yaw, &mut commands);
        level.boss_spot = spot.position;
        // The boss gathers the level's safe rocks: the round its throws
        // take with no target starts at a random one.
        let rocks = items.safe_rocks().len().min(MAX_ROCKS) as i32;
        if rocks > 0 {
            level.last_rock = (level.random() * rocks as f32) as i32 % rocks;
        }
        level.end = load_end_models(&mut game, &population.level, monsters.boss, monsters.realm, &mut meshes, &mut materials, &mut images);
        info!("boss {} at {at:?} facing {:.0}°", kind.file.desc.name, yaw.to_degrees());
        // The intro runs when a hero brings the realm's legendary item
        // (`GDL_LEGENDARY=1`, a testing aid, pretends one does); the second
        // skorne and garm have none.
        let carried = party.any(|s| s.quest.legendary & (1 << realm_id) != 0);
        let pretend = std::env::var("GDL_LEGENDARY").is_ok_and(|v| v == "1");
        if (0..=LAST_INTRO_BOSS).contains(&monsters.boss) && (carried || pretend) {
            level.intro = intro::START;
            info!("the boss intro will run (the hero brings the legendary item{})", if carried { "" } else { ": GDL_LEGENDARY" });
        }
    }
    commands.insert_resource(level);
}

/// A critter folder's atrees, models, texture bytes and texture modifiers.
type CritterFolder = (AnimFile, ModelFile, Vec<u8>, Vec<TexMod>);

/// The effect table's own bank: its effects are there for every critter's
/// `SFXX` records too.
const EFFECT_BANK: &str = "WEAPONS";

/// An effect's clip (its first action's frames and rate) from the effect
/// table's bank, read once (`bank`).
fn bank_clip(game: &mut LoadedGame, bank: &mut Option<Option<AnimFile>>, name: &str) -> Option<(u16, u16)> {
    let anim = bank.get_or_insert_with(|| {
        game.install.read(&format!("{EFFECT_BANK}/ANIM.PS2")).ok().and_then(|b| AnimFile::parse(&b).ok())
    });
    let tree = anim.as_ref()?.atrees.iter().find(|a| a.name.eq_ignore_ascii_case(name))?;
    tree.actions.first().map(|a| (a.frames, a.rate))
}

fn read_folder(game: &mut LoadedGame, folder: &str) -> Option<CritterFolder> {
    let bytes = game.install.read(&format!("{folder}/ANIM.PS2")).ok()?;
    let anim = AnimFile::parse(&bytes).ok()?;
    let texmods = TexMod::parse_all(&bytes).unwrap_or_default();
    let model = ModelFile::parse(&game.install.read(&format!("{folder}/objects.ngc")).ok()?).ok()?;
    let textures = game.install.read(&format!("{folder}/textures.ngc")).ok()?;
    Some((anim, model, textures, texmods))
}

/// Loads the models of the boss level's end from the level's own item set:
/// the key (bosses before the first skorne) and the wizard; and the names
/// of the wizard's speeches.
fn load_end_models(
    game: &mut LoadedGame,
    level: &str,
    boss_type: i32,
    letter: char,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> EndModels {
    let folder = format!("ITEMS/{level}");
    let mut build = |name: &str| -> Option<(Arc<CharacterModel>, f32)> {
        let data = load_atree(game, &folder, name)?;
        // A clip without frames plays 30.
        let life = data.clips.actions.first().map_or(1.0, |a| effect_life(a.frames, a.rate));
        debug!(
            "{folder}/{name}: {} actions {:?}",
            data.clips.actions.len(),
            data.clips.actions.iter().map(|a| (a.name.as_str(), a.frames, a.rate, a.loops())).collect::<Vec<_>>()
        );
        Some((Arc::new(CharacterModel::build(&data, meshes, materials, images)), life))
    };
    let has_key = boss_type < SKORNE;
    let key = if has_key { build("BOSSKEY") } else { None };
    let key_after = if has_key { build("BOSSKEY2").map(|m| m.0) } else { None };
    let wizard = build("WIZARD").map(|m| m.0);
    let speeches = (0..5).map(|n| wizard_speech(boss_type, letter, n)).collect();
    let rom = game.install.read("TEXT/ENGLISH.ROM").ok().and_then(|b| TextRom::parse(&b).ok());
    let groups = [first_message(boss_type)]
        .into_iter()
        .chain((0..4).map(|n| second_message(boss_type, n, false)))
        .chain([second_message(boss_type, 0, true)])
        .flatten();
    let texts: HashMap<&'static str, Vec<String>> = groups
        .filter_map(|g| Some((g, rom.as_ref()?.group(g)?.strings.clone())))
        .collect();
    info!(
        "boss end from {folder}: key {}, second key {}, wizard {}, {} messages",
        key.is_some(),
        key_after.is_some(),
        wizard.is_some(),
        texts.len()
    );
    EndModels { key, key_after, wizard, speeches, texts }
}

/// The wizard's speech `n` after the boss of this type falls, in realm
/// `letter` (the game's table by boss type): 0 when he appears, then 1 +
/// how many of the realm's runestones the heroes hold ([`runes_held`]).
/// The skornes and garm have one each.
fn wizard_speech(boss_type: i32, letter: char, n: usize) -> Option<String> {
    let name = match (boss_type, n) {
        (DRAGON..=LICH, 0) => format!("S_DEFEATVOX{letter}"),
        (DRAGON..=LICH, 1) => format!("S_RUNEVOX0{letter}"),
        (DRAGON..=LICH, 2) => format!("S_RUNEVOX1{letter}"),
        (DRAGON..=DRIDER, 3 | 4) => format!("S_RUNEVOX1{letter}"),
        (PBOSS..=LICH, 3 | 4) => format!("S_RUNEVOX2{letter}"),
        (SKORNE, 0) => "S_E2VOXA".into(),
        (SKORNE, 1) => "S_E2VOXB".into(),
        (SKORNE2, 0) => "S_ENDVOX".into(),
        (GARM, 0) => "S_GRMDESTVOX".into(),
        _ => return None,
    };
    Some(name)
}

/// How many of the realm's runestones the heroes hold (`bits`: stone n →
/// bit n): 0 none, 1 some, 2 all, 3 all of a realm with two or more (the
/// game's realm table: which stones, how many).
fn runes_held(realm_id: u32, bits: u32) -> usize {
    let (mask, count) = match realm_id {
        1 => (0x1, 1),
        2 => (0x8, 1),
        3 => (0x40, 1),
        4 => (0x200, 1),
        7 => (0x180, 2),
        8 => (0x1000, 1),
        9 => (0x30, 2),
        10 => (0xC00, 2),
        11 => (0x6, 2),
        _ => (0, 0),
    };
    if bits & mask == mask {
        if count < 2 { 2 } else { 3 }
    } else if bits & mask == 0 {
        0
    } else {
        1
    }
}

/// Makes a critter of `kind` standing at `position` facing `yaw`: its
/// hit points, its home (the type's, or here), its hit spheres, its health
/// meter; then its parts (the types chained by `TYPE +0x11C`: the
/// chimera's heads), which move subtrees of its model, each with its own
/// meter. Hit spheres are numbered across the body and its parts.
fn spawn_critter(level: &mut CritterLevel, kind: &Arc<CritterKind>, position: [f32; 3], yaw: f32, commands: &mut Commands) -> Option<Entity> {
    let ty = 0;
    let body = kind.bodies[ty].as_ref()?;
    let t = &kind.file.types[ty];
    let position = [position[0], position[1] + t.hover, position[2]];
    let transform = Transform::from_translation(Vec3::from(position)).with_rotation(Quat::from_rotation_y(yaw));
    let root = body.model.as_ref()?.spawn(transform, commands);
    let mut owners = Vec::new();
    let mut c = new_critter(level, kind, ty, position, yaw, root, None, &mut owners, commands);
    c.meter = level.meters.add(t, c.full_hit_points);
    if t.flags & meter::FLAT != 0 {
        level.meters.folder = kind.folder.clone();
    }
    if t.flags & meter::SOLID != 0
        && let Some(m) = &kind.solid_meter
    {
        meter::add_solid(m, root, t, commands);
    }
    let mut part = t.child;
    let mut guard = 0;
    while let Some(pt) = part.filter(|&p| p < kind.file.types.len() && guard < 8) {
        if kind.bodies[pt].is_some() {
            let k = c.parts.len();
            let mut p = new_critter(level, kind, pt, position, yaw, root, Some(k), &mut owners, commands);
            p.meter = level.meters.add(&kind.file.types[pt], p.full_hit_points);
            if p.meter.is_some() {
                level.meters.folder = kind.folder.clone();
            }
            debug!("boss part {} ({} nodes under {})", kind.file.types[pt].name, p.body().subtree.len(), part_node(&kind.file.types[pt]));
            c.parts.push(p);
        }
        part = kind.file.types[pt].child;
        guard += 1;
    }
    c.sphere_owner = owners;
    commands.entity(root).insert((c, LevelEntity));
    Some(root)
}

/// The node a part's subtree hangs from (`TYPE +0x10`).
fn part_node(t: &gdl_formats::critter::CritterType) -> String {
    let raw = t.raw.get(0x10..0x20).unwrap_or_default();
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

/// A skeleton node and every node under it (none when it's missing).
fn subtree_of(skeleton: &gdl_formats::anim::Atree, name: &str) -> Vec<usize> {
    let Some(top) = skeleton.node_index(name) else { return Vec::new() };
    let under = |mut n: usize| -> bool {
        let mut guard = 0;
        loop {
            if n == top {
                return true;
            }
            match skeleton.nodes.get(n).and_then(|x| x.parent) {
                Some(p) if guard < 256 => n = p,
                _ => return false,
            }
            guard += 1;
        }
    };
    (0..skeleton.nodes.len()).filter(|&n| under(n)).collect()
}

/// A critter of type `ty` (a body, or part `part` of the body `root`):
/// its state and its hit spheres (entities on `root`, numbered on from
/// `owners`, which records whose each is).
#[allow(clippy::too_many_arguments)]
fn new_critter(
    level: &CritterLevel,
    kind: &Arc<CritterKind>,
    ty: usize,
    position: [f32; 3],
    yaw: f32,
    root: Entity,
    part: Option<usize>,
    owners: &mut Vec<(Option<usize>, usize)>,
    commands: &mut Commands,
) -> Critter {
    let t = &kind.file.types[ty];
    let hp = t.hit_points * level.hit_point_scale;
    let moves = kind.file.type_moves(ty).len();
    let nodes = kind.file.type_nodes(ty);
    // Its body's target at its centre (its cylinder's radius and height),
    // then its spheres, grouped with it.
    let centre = centre_at(position, yaw, t.center);
    let aim = commands.spawn((Transform::from_translation(Vec3::from(centre)), CritterSphere { critter: root, node: None, part }, LevelEntity)).id();
    let body = CritterAim { body: aim, sphere: None };
    let boss = kind.file.desc.class == class::BOSS;
    commands.entity(aim).insert(Targetable::new(TargetKind::Object, t.radius, t.height).of_critter(body).of_boss(boss));
    let spheres: Vec<Entity> = if t.flags & TYPE_SPHERES != 0 {
        nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let sphere = SphereAim { index: i, reach: n.reach, weight: n.weight };
                let target = Targetable::new(TargetKind::Object, n.radius, n.radius)
                    .of_critter(CritterAim { body: aim, sphere: Some(sphere) })
                    .of_boss(boss);
                let sphere = CritterSphere { critter: root, node: Some(owners.len()), part };
                owners.push((part, i));
                commands.spawn((Transform::from_translation(Vec3::from(position)), target, sphere, LevelEntity)).id()
            })
            .collect()
    } else {
        Vec::new()
    };
    Critter {
        kind: kind.clone(),
        ty,
        state: CritterState::New,
        hit_points: hp,
        full_hit_points: hp,
        position,
        spawned_at: position,
        yaw,
        home_yaw: yaw,
        home: t.fixed_home().unwrap_or(position),
        previous: (position, yaw),
        floor: position[1] - t.hover,
        ground_node: None,
        current: None,
        next: None,
        pick: None,
        move_target: None,
        switched: false,
        // Never used yet: every move is ready (the game's clock has run since
        // boot, ours since the level began).
        ends: vec![NEVER; moves],
        pattern_starts: vec![NEVER; kind.file.type_patterns(ty).len()],
        pattern: None,
        chosen_pattern: None,
        hold_until: 0.0,
        wake_at: 0.0,
        blows_done: 0,
        sounds_done: 0,
        tracked: Vec::new(),
        anger: ANGER_BASE,
        damage_taken: 0.0,
        kinds: 0,
        push: [0.0; 3],
        last_blow: 0.0,
        knock: [0.0; 3],
        clock: Clock { action: 0, frame: 0.0, frames: 1, rate: 30, loops: false, ended: true },
        node_at: None,
        node_was: None,
        sphere_damage: vec![0.0; nodes.len()],
        spent: vec![false; spheres.len()],
        aim,
        blows_dealt: 0,
        held: None,
        hp_before: hp,
        now: level.now,
        spheres,
        missile_hit: false,
        frozen: 0.0,
        stunned: 0.0,
        parts: Vec::new(),
        sphere_owner: Vec::new(),
        mirrored: true,
        flash: Flash::default(),
        meter: None,
    }
}

/// Tags a critter's model with its flashes: all of it while the body's
/// shows, a part's subtree while the part's does.
fn show_flashes(c: &Critter, animator: &Animator, colour: u32, tags: &mut Query<&mut MeshTag>, commands: &mut Commands) {
    let lit = |node: usize| {
        c.flash.on() || c.parts.iter().any(|p| p.flash.on() && p.body().subtree.contains(&node))
    };
    for &(node, e) in animator.meshes() {
        flash::set_tag(e, if lit(node) { colour } else { 0 }, tags, commands);
    }
}

/// A player as the critters see it.
#[derive(Clone, Copy)]
struct Hero {
    entity: Entity,
    feet: [f32; 3],
    radius: f32,
    half: f32,
    attacking: bool,
    /// Invisible: only bosses track it.
    hidden: bool,
}

/// A blow for a player: damage, kind bits, push.
type Blow = (Entity, f32, u32, [f32; 3]);

#[allow(clippy::too_many_arguments)]
fn tick_critters(
    mut commands: Commands,
    level: Option<ResMut<CritterLevel>>,
    ground: Option<Res<LevelGround>>,
    mechanics: Option<ResMut<Mechanics>>,
    population: Option<Res<LevelPopulation>>,
    mut party: ResMut<Party>,
    mut players: Query<(Entity, &mut Player)>,
    mut critters: Query<(Entity, &mut Critter, &mut Animator), Without<StatueModel>>,
    mut statues: Query<&mut Animator, With<StatueModel>>,
    mut spheres: Query<&mut Transform, (With<CritterSphere>, Without<Critter>)>,
    bones: Query<&GlobalTransform>,
    mut hurt: MessageWriter<HurtHero>,
    mut sounds: MessageWriter<PlaySoundAt>,
    (mut death_shot, camera): (Local<Option<u32>>, Option<Res<PlayCamera>>),
    (colours, mut tags, enemies, stop): (Res<FlashColours>, Query<&mut MeshTag>, Res<EnemyScale>, Res<TimeStop>),
    (time, mut guard): (Res<Time>, ResMut<crate::projectiles::PlayerGuard>),
) {
    let (Some(mut level), Some(ground)) = (level, ground) else { return };
    let level = &mut *level;
    // The heroes' guard, shared with the missiles and blasts.
    level.guard = guard.snapshot();
    level.clock = time.elapsed_secs_f64();
    level.now += DT;
    level.enemy_scale = enemies.0;
    level.time_stopped = stop.0;
    let now = level.now;
    update_intro(level);
    // The hero who brought the legendary item uses it up once the level
    // darkens (the game clears its bit in the player's record).
    if matches!(level.intro, intro::WAIT | intro::ROAR) && !level.legendary_used {
        level.legendary_used = true;
        let bit = 1u16 << level.realm_id;
        if let Some((slot, s)) = party.states_mut().find(|(_, s)| s.quest.legendary & bit != 0) {
            s.quest.legendary &= !bit;
            info!("player {}'s legendary item is used up", slot + 1);
        }
    }
    let heroes: Vec<Hero> = players
        .iter()
        .filter_map(|(e, p)| {
            let s = party.state(p.slot).filter(|s| s.alive)?;
            let c = p.actions.action.category().0;
            Some(Hero {
                entity: e,
                feet: p.mover.position,
                radius: s.radius,
                half: s.half_height,
                attacking: (1..=12).contains(&c),
                hidden: p.special_bits & crate::player_state::power::INVISIBLE != 0,
            })
        })
        .collect();

    let view = game_view(camera.as_deref());
    wake_statues(level, mechanics, population.as_deref(), &heroes, &view, &mut statues, &mut commands);

    let intro_before = level.intro;
    let mut blows: Vec<Blow> = Vec::new();
    let mut to_play: Vec<PlaySoundAt> = Vec::new();
    for (entity, mut c, mut animator) in &mut critters {
        let c = &mut *c;
        c.now = now;
        c.previous = (c.position, c.yaw);
        c.switched = false;
        let ty = type_info(&c.kind.file, c.ty);
        let boss = ty.class == class::BOSS;

        // The blows taken become knockback; old damage is forgotten.
        knockback(c);
        let cur_kind = c.move_kind(c.current);
        forget_damage(c, now);

        // Targets and anger; the parts share the body's place and wake
        // with it.
        let centre = centre_of(c, &ty);
        track(c, &ty, centre, &heroes, level);
        c.anger = anger_of(c);
        if c.state == CritterState::New {
            if !boss {
                c.state = CritterState::Active;
            } else if wake_boss(c, &ty, now) {
                c.state = CritterState::Active;
                info!("the boss {} wakes", c.kind.file.desc.name);
                let at = |c: &Critter| -> Vec<(String, Vec3)> {
                    let names = c.kind.file.type_nodes(c.ty);
                    c.spheres.iter().zip(names).filter_map(|(s, n)| Some((n.name.clone(), spheres.get(*s).ok()?.translation))).collect()
                };
                debug!("boss hit spheres: {:?}", at(c).into_iter().chain(c.parts.iter().flat_map(at)).collect::<Vec<_>>());
            }
        }
        let (position, yaw, awake) = (c.position, c.yaw, c.state != CritterState::New);
        for p in &mut c.parts {
            p.now = now;
            p.position = position;
            p.yaw = yaw;
            p.previous = (position, yaw);
            p.switched = false;
            forget_damage(p, now);
            let pty = type_info(&p.kind.file, p.ty);
            let pc = centre_of(p, &pty);
            track(p, &pty, pc, &heroes, level);
            p.anger = anger_of(p);
            if awake && p.state == CritterState::New {
                p.state = CritterState::Active;
            }
            intro_reactions(p, level);
        }
        intro_reactions(c, level);
        let mut flashes = c.flash.step();
        for p in &mut c.parts {
            flashes |= p.flash.step();
        }
        if flashes {
            show_flashes(c, &animator, colours.body().unwrap_or(0), &mut tags, &mut commands);
        }
        // A body whose parts are all dead dies.
        if c.state == CritterState::Active && !c.parts.is_empty() && c.parts.iter().all(|p| p.state == CritterState::Dying) {
            info!("the boss's parts are all dead");
            c.die();
        }

        // Dead and done: a golem when DEATH ends, a boss when its hold does
        // (it counts as dead from the end of DEATH).
        if boss && level.boss == Some(entity) && c.move_kind(c.current) == Some(kind::DEATH) && c.clock.ended && !level.boss_dead {
            level.boss_dead = true;
        }
        if c.move_kind(c.current) == Some(kind::DEATH) && c.clock.ended && (!boss || now >= c.hold_until) {
            debug!("critter {entity:?} is gone");
            if level.boss == Some(entity) {
                let key_at = add(c.spawned_at, c.kind.file.types[c.ty].key_offset);
                level.victory = Some(Victory {
                    step: 0,
                    timer: 0.0,
                    key_at,
                    key: None,
                    wizard: None,
                    wizard_at: [0.0; 3],
                    fade: 0,
                    countdown: 0.0,
                    over: false,
                });
                info!("the boss is gone");
            }
            if !boss {
                level.drops.push(CritterDrop { held: level.held.remove(&entity), class: c.class(), at: c.position });
            }
            for s in c.spheres.iter().chain([&c.aim]).chain(c.parts.iter().flat_map(|p| p.spheres.iter().chain([&p.aim]))) {
                commands.entity(*s).try_despawn();
            }
            commands.entity(entity).try_despawn();
            continue;
        }

        // Choose, switch.
        c.next = None;
        c.pick = None;
        c.chosen_pattern = None;
        forced(c, &ty, now, level.intro, level.boss_type, level.roar_at());
        // Time stopped, no pattern is chosen; only a start or death move
        // switches in, and only a death plays on.
        let stopped = level.time_stopped;
        if c.state == CritterState::Active && !stopped {
            if c.next.is_none() {
                choose_block(c, now, &heroes);
            }
            if c.next.is_none() {
                choose_attack(c, now);
                choose_parts(c, now, level.intro, level.boss_type, level.roar_at());
            }
            let attacking = cur_kind.is_some_and(|k| k >= kind::ATTACK_FIRST);
            if c.next.is_none() && !(boss && attacking) {
                choose_movement(c, centre, now);
            }
            if c.next.is_none() {
                c.next = idle(c, now);
            }
        }
        if c.next.is_none() && !boss {
            c.next = c.current;
        }
        let was = c.current;
        let parts_were: Vec<Option<usize>> = c.parts.iter().map(|p| p.current).collect();
        // Frozen, it keeps its move and frame.
        let starts_or_dies = |c: &Critter, m: Option<usize>| matches!(c.move_kind(m), Some(kind::START | kind::DEATH));
        if c.frozen <= 0.0 && (!stopped || starts_or_dies(c, c.current) || starts_or_dies(c, c.next)) {
            switch(c, now, Some(&mut animator), &mut level.intro, true);
        }
        switch_parts(c, now, &mut level.intro);
        // The intro also moves on once the boss's START (without a
        // follow-up) or ROAR has played out, before anything replaces it.
        if level.boss == Some(entity)
            && c.clock.ended
            && let Some(m) = c.current.map(|i| &c.moves()[i])
        {
            if m.kind == kind::ROAR && level.intro == intro::ROAR {
                level.intro = intro::ROARED;
            } else if m.kind == kind::START && m.next < 0 && level.intro == intro::START {
                level.intro = intro::WAIT;
            }
        }
        if c.switched && c.move_kind(c.current) == Some(kind::DEATH) {
            level.events.push("death");
        }
        let hp_now = c.hit_points + c.parts.iter().map(|p| p.hit_points).sum::<f32>();
        if hp_now < c.hp_before {
            level.events.push("hurt");
        }
        c.hp_before = hp_now;
        if c.state == CritterState::Dying {
            for s in c.spheres.iter().chain([&c.aim]) {
                commands.entity(*s).try_remove::<Targetable>();
            }
        }
        for p in c.parts.iter().filter(|p| p.state == CritterState::Dying) {
            for s in p.spheres.iter().chain([&p.aim]) {
                commands.entity(*s).try_remove::<Targetable>();
            }
        }
        if c.current != was && let Some(i) = c.current {
            debug!(
                "critter {entity:?} {} → {} (anger {:.2}, {:.0} hp)",
                was.map_or("-", |w| c.moves()[w].name.as_str()),
                c.moves()[i].name,
                c.anger,
                c.hit_points
            );
        }
        for (k, p) in c.parts.iter().enumerate() {
            if p.current != parts_were[k] && let Some(i) = p.current {
                if p.moves()[i].is_attack() {
                    level.events.push("part");
                }
                debug!(
                    "critter {entity:?} part {} {} → {} ({:.0} hp)",
                    p.kind.file.types[p.ty].name,
                    parts_were[k].map_or("-", |w| p.moves()[w].name.as_str()),
                    p.moves()[i].name,
                    p.hit_points
                );
            }
        }

        // Held by the time stop (all but a death), its clip holds too.
        let held = stopped && c.current.is_some() && c.move_kind(c.current) != Some(kind::DEATH);
        if animator.hold != held {
            animator.hold = held;
        }
        let root = Affine3A::from_rotation_translation(Quat::from_rotation_y(c.yaw), Vec3::from(c.position));
        let bone_matrix = |n: Option<usize>| -> Affine3A {
            n.and_then(|n| animator.bone(n)).and_then(|b| bones.get(b).ok()).map_or(root, |g| g.affine())
        };
        let bone_entity = |n: Option<usize>| n.and_then(|n| animator.bone(n));
        // The parts' moves: target, node, blows and sounds; their hit
        // spheres follow their nodes.
        for p in &mut c.parts {
            follow_spheres(p, &bone_matrix, &mut spheres, &mut commands);
            if !p.mirrored
                && !level.time_stopped
                && let Some(pcur) = p.current
            {
                let pmv = p.moves()[pcur].clone();
                act(p, entity, &pmv, pcur, (&bone_matrix, &bone_entity), &heroes, level, &mut blows, &mut to_play, &mut commands);
                if p.frozen <= 0.0 {
                    p.clock.advance(DT);
                }
            }
        }

        let Some(cur) = c.current else { continue };
        if held {
            continue;
        }
        let mv = c.moves()[cur].clone();
        trace!(
            "critter {entity:?} {} frame {:.1}/{} ended {} next {:?} hold {:.2}",
            mv.name,
            c.clock.frame,
            c.clock.frames,
            c.clock.ended,
            c.next.map(|n| c.moves()[n].name.clone()),
            c.hold_until - now
        );

        // A boss holds a move its hold time past the animation's end.
        if mv.hold <= 0.0 {
            c.hold_until = 0.0;
        } else if !c.clock.ended || c.hold_until == 0.0 {
            c.hold_until = now + mv.hold;
        }

        follow_spheres(c, &bone_matrix, &mut spheres, &mut commands);
        act(c, entity, &mv, cur, (&bone_matrix, &bone_entity), &heroes, level, &mut blows, &mut to_play, &mut commands);

        // Walk and turn.
        if boss {
            walk_leashed(c, &mv, &ty, level.speed_scale);
        } else {
            walk(c, &mv, &ty, &ground.0, &heroes, level.speed_scale);
        }
        // Holding a hero, it doesn't turn. Dying, it lets go (a safety:
        // the game has nothing let go but the throw).
        if c.held.is_none() {
            turn(c, &mv, &heroes);
        }
        if c.state == CritterState::Dying
            && let Some(h) = c.held.take()
        {
            level.releases.push(h);
        }
        if c.frozen > 0.0 {
            c.frozen = (c.frozen - DT).max(0.0);
        } else {
            c.clock.advance(DT);
        }
        c.stunned = (c.stunned - DT).max(0.0);
        // Parts copying the body keep its clock.
        let clock = c.clock;
        for p in c.parts.iter_mut().filter(|p| p.mirrored) {
            p.clock = clock;
        }
    }

    if level.intro != intro_before {
        info!("boss intro {intro_before} → {} at {now:.2} s", level.intro);
    }
    for (player, amount, kind_bits, push) in blows {
        let Ok((_, mut p)) = players.get_mut(player) else { continue };
        let amount = p.take_blow(amount, kind_bits, Vec3::from(push));
        // Voiced as the game's critter blows are: the hurt sound by kind.
        if amount != 0.0 {
            hurt.write(HurtHero { slot: p.slot, amount, kind: kind_bits, cry: Cry::Hurt });
        }
        info!("a critter hits the hero for {amount:.1} (kind {kind_bits:#x})");
    }
    // `GDL_CRITTER_SHOT=<png>`: a testing aid that saves a screenshot a
    // few ticks after the first critter event named by
    // `GDL_CRITTER_SHOT_ON` (`death`, the default; `missile`; `hurt`; `part`, a
    // boss part starting an attack; `rocks`, a blow at the safe rocks;
    // `effect`, a still effect set down).
    let wanted = std::env::var("GDL_CRITTER_SHOT_ON").unwrap_or_else(|_| "death".into());
    if death_shot.is_none() && level.events.iter().any(|e| *e == wanted) {
        let delay = std::env::var("GDL_CRITTER_SHOT_DELAY").ok().and_then(|v| v.parse().ok());
        *death_shot = Some(delay.unwrap_or(SHOT_DELAY).max(1));
    }
    level.events.clear();
    if let Some(left) = death_shot.as_mut()
        && *left > 0
    {
        *left -= 1;
        if *left == 0
            && let Some(path) = std::env::var_os("GDL_CRITTER_SHOT")
        {
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(std::path::PathBuf::from(path)));
        }
    }
    sounds.write_batch(to_play);
    guard.merge(&level.guard);
}

/// Old damage is forgotten after 3 s, and during ROAR and hit reactions.
fn forget_damage(c: &mut Critter, now: f32) {
    let cur_kind = c.move_kind(c.current);
    if (c.last_blow > 0.0 && now - c.last_blow > DAMAGE_MEMORY)
        || cur_kind == Some(kind::ROAR)
        || cur_kind.is_some_and(|k| (0x40..0x7F).contains(&k))
    {
        c.damage_taken = 0.0;
        c.kinds = 0;
        c.last_blow = 0.0;
    }
}

/// Anger: 0.5 at full health up to 5 near death.
fn anger_of(c: &Critter) -> f32 {
    (1.0 - c.hit_points.max(0.0) / (1.0 + c.full_hit_points)) * ANGER_SPAN + ANGER_BASE
}

/// Its hit spheres follow their nodes, and its body's target its centre;
/// a sphere whose share of hit points is spent stops being a target (the
/// game's searches pass it over).
fn follow_spheres(
    c: &mut Critter,
    bone_matrix: &dyn Fn(Option<usize>) -> Affine3A,
    spheres: &mut Query<&mut Transform, (With<CritterSphere>, Without<Critter>)>,
    commands: &mut Commands,
) {
    let centre = centre_at(c.position, c.yaw, c.kind.file.types[c.ty].center);
    if let Ok(mut t) = spheres.get_mut(c.aim) {
        t.translation = Vec3::from(centre);
    }
    for (i, s) in c.spheres.iter().enumerate() {
        let Some(n) = c.kind.file.type_nodes(c.ty).get(i) else { continue };
        if !c.spent[i] && c.sphere_damage[i] >= n.hit_points * c.full_hit_points {
            c.spent[i] = true;
            commands.entity(*s).try_remove::<Targetable>();
        }
        let m = bone_matrix(c.body().spheres[i]);
        if let Ok(mut t) = spheres.get_mut(*s) {
            t.translation = m.transform_point3(Vec3::from(n.offset));
        }
    }
}

/// Doing the move (a body's or a part's; `me` is the body): its target
/// and node, and the blows and sounds on their frames.
#[allow(clippy::too_many_arguments)]
fn act(
    c: &mut Critter,
    me: Entity,
    mv: &CritterMove,
    cur: usize,
    (bone_matrix, bone_entity): Bones,
    heroes: &[Hero],
    level: &mut CritterLevel,
    blows: &mut Vec<Blow>,
    to_play: &mut Vec<PlaySoundAt>,
    commands: &mut Commands,
) {
    if c.move_target.is_none() || c.switched {
        c.move_target = c.pick.or_else(|| best_target(c, &mv.condition, true));
    }
    if c.switched {
        c.blows_done = 0;
        c.sounds_done = 0;
        c.node_was = None;
    }
    let node_matrix = bone_matrix(c.body().nodes[cur]);
    let node_bone = bone_entity(c.body().nodes[cur]);
    c.node_was = c.node_at.filter(|_| c.node_was.is_some() || !c.switched);
    c.node_at = Some(node_matrix.translation.into());
    if c.node_was.is_none() {
        c.node_was = c.node_at;
    }
    let frame = c.clock.frame as i32;
    let bits = blow_bits(mv, frame, c.blows_done);
    for (slot, bit) in [(0usize, 1u8), (1, 2)] {
        if bits & bit == 0 {
            continue;
        }
        let first = c.blows_done & bit == 0;
        c.blows_done |= bit;
        let Ok(d) = usize::try_from(mv.damage[slot]) else { continue };
        let Some(dmg) = c.kind.file.damage.get(d).cloned() else { continue };
        deal(c, me, &dmg, (slot, first), (node_matrix, node_bone), heroes, level, blows, to_play, commands);
    }
    for (k, (s, at)) in mv.sounds.iter().enumerate() {
        let bit = 1 << k;
        if c.sounds_done & bit == 0 && *s >= 0 && i32::from(*at) <= frame {
            c.sounds_done |= bit;
            level.shake |= effect_sounds(c, *s as usize, level.realm, to_play);
        }
    }
}

/// A critter's bones by its skeleton's node: world matrix and entity.
type Bones<'a> = (&'a dyn Fn(Option<usize>) -> Affine3A, &'a dyn Fn(Option<usize>) -> Option<Entity>);

/// The idle move: TAUNT while unhurt, else READY.
fn idle(c: &Critter, now: f32) -> Option<usize> {
    let taunt = if c.anger < TAUNT_ANGER { find(c, kind::TAUNT, Find::Ready, now) } else { None };
    taunt.or_else(|| find(c, kind::READY, Find::Nearest, now))
}

/// The parts' moves, after the body's attack choice: while the body runs
/// a pattern, the step of their own pattern of the same number; when the
/// body chose nothing, their forced moves, their attacks, or idling. A
/// part attacking makes the body play TOGETHER.
fn choose_parts(c: &mut Critter, now: f32, intro: i32, boss_type: i32, roar_at: f32) {
    if c.parts.is_empty() {
        return;
    }
    let (pattern, free) = (c.pattern, c.next.is_none());
    let mut attacking = 0;
    for p in &mut c.parts {
        p.next = None;
        p.pick = None;
        p.chosen_pattern = None;
        match pattern {
            None if free => {
                forced_part(p, now, intro, boss_type, roar_at);
                if p.next.is_none() {
                    choose_attack(p, now);
                }
                if p.next.is_none() {
                    p.next = idle(p, now);
                } else if p.pattern.is_some() || p.move_kind(p.next).is_some_and(|k| k >= kind::ATTACK_FIRST) {
                    attacking += 1;
                }
            }
            None => {}
            Some((n, step)) => {
                if let Some(own) = p.kind.file.type_patterns(p.ty).get(n) {
                    p.pattern = Some((n, step));
                    p.next = own.moves.get(step).and_then(|&m| usize::try_from(m).ok());
                }
            }
        }
    }
    if attacking > 0 {
        c.next = find(c, kind::TOGETHER, Find::Nearest, now);
    }
}

/// A part's forced moves: DEATH when dying; else its move's follow-up, or
/// READY for the chimera's heads in the intro's states 3–5; then its
/// reactions to the blows it took.
fn forced_part(p: &mut Critter, now: f32, intro: i32, boss_type: i32, roar_at: f32) {
    let chimera_waits = (intro::ROAR..=intro::AFTER).contains(&intro) && boss_type == CHIMERA;
    p.next = if p.state == CritterState::Dying {
        find(p, kind::DEATH, Find::Nearest, now)
    } else if chimera_waits {
        find(p, kind::READY, Find::Ready, now)
    } else {
        p.current.map(|i| p.moves()[i].next).and_then(|n| usize::try_from(n).ok())
    };
    reactions(p, now, roar_at);
}

/// The parts after the body's switch: a dying part plays DEATH; while the
/// body plays TOGETHER or runs a pattern (not in the intro's dark), a part
/// with a move of its own switches as a body does; otherwise it copies the
/// body's animation.
fn switch_parts(c: &mut Critter, now: f32, intro: &mut i32) {
    let body_kind = c.move_kind(c.current);
    let in_pattern = c.pattern.is_some();
    let own = (body_kind == Some(kind::TOGETHER) || in_pattern) && !(intro::WAIT..=intro::ROAR).contains(intro);
    let switched = c.switched;
    for p in &mut c.parts {
        if p.state == CritterState::Dying {
            p.next = find(p, kind::DEATH, Find::Nearest, now);
        } else if !(own && (p.current.is_some() || p.next.is_some())) {
            p.mirrored = true;
            p.switched = switched;
            p.current = None;
            p.pattern = None;
            if body_kind == Some(kind::TOGETHER) {
                p.next = Some(0);
            }
            continue;
        }
        p.mirrored = false;
        if p.frozen <= 0.0 {
            switch(p, now, None, intro, !in_pattern);
        }
    }
}

/// A sleeping boss wakes once its two seconds are up and every player it
/// tracks is within its wake distance (at once when that's 0). With the
/// type's flag 0x80 (the chimera) the game starts the two seconds only
/// when the boss stands on floor whose node has flag 0x10 in its byte
/// `+0x16`; that byte isn't read here, so they start at once (stand-in).
fn wake_boss(c: &mut Critter, ty: &TypeInfo, now: f32) -> bool {
    if c.wake_at == 0.0 {
        c.wake_at = now + BOSS_WAKE_DELAY;
        if ty.flags & TYPE_FLOOR_WAKE != 0 {
            debug!("boss wake timer started at once (stand-in for its floor's flag)");
        }
        return false;
    }
    if now < c.wake_at {
        return false;
    }
    let furthest = c.tracked.iter().map(|t| t.distance).fold(0.0f32, f32::max);
    let furthest = if furthest <= 0.0 { REJECTED } else { furthest };
    ty.wake_distance <= 0.0 || furthest < ty.wake_distance
}

/// The level's side of the boss intro, each tick before the critters move
/// (the game's level update): 99 once the boss is dead; in 2 a wait (from
/// the first tick) then 3; 4 goes on to 5 at once, noting the time; after
/// 29 s in 5 the djinn, P-boss, yeti, wraith and first skorne go on to 6
/// (the dragon, chimera, drider and lich stay in 5; a missile moves the
/// chimera on). The level darkens through 2–3.
fn update_intro(level: &mut CritterLevel) {
    let now = level.now;
    let was = level.intro;
    if level.boss_dead {
        level.intro = intro::OVER;
    }
    match level.intro {
        intro::ROARED => {
            level.intro = intro::AFTER;
            level.intro_timer = now;
        }
        intro::AFTER => {
            if now - level.intro_timer >= INTRO_AFTER && matches!(level.boss_type, DJINN | PBOSS | YETI | WRAITH | SKORNE) {
                level.intro = intro::FIGHT;
            }
        }
        intro::WAIT => {
            if level.intro_timer == 0.0 {
                let wait = if matches!(level.boss_type, CHIMERA | LICH | SKORNE) { INTRO_WAIT_SHORT } else { INTRO_WAIT };
                level.intro_timer = now + wait;
            } else if level.intro_timer <= now {
                level.intro = intro::ROAR;
                level.intro_timer = 0.0;
            }
            level.light.ask(now, DARKEN_HOLD, DARKEN_TO);
        }
        intro::ROAR => level.light.ask(now, DARKEN_HOLD, DARKEN_TO),
        _ => {}
    }
    level.light.step(now);
    if level.intro != was {
        info!("boss intro {was} → {} at {now:.2} s (light {:+.2})", level.intro, level.light_offset());
    }
}

/// What a missile hitting the boss does in its intro (the game checks it
/// where effects hit critters): in 2–3 it ends the intro for the dragon
/// (frozen) and the djinn (stunned), and stuns the P-boss; in 5 it starts
/// the chimera's fight. The P-boss's stun ends with its intro.
fn intro_reactions(c: &mut Critter, level: &mut CritterLevel) {
    if std::mem::take(&mut c.missile_hit)
        && let Some((next, freeze, stun)) = missile_reaction(level.intro, level.boss_type)
    {
        level.intro = next;
        if freeze > 0.0 {
            c.frozen = freeze;
        }
        if stun > 0.0 {
            c.stunned = stun;
        }
        info!("a missile hits the boss in its intro: now {next} (frozen {:.0} s, stunned {:.0} s)", c.frozen, c.stunned);
    }
    if level.boss_type == PBOSS && level.intro >= intro::FIGHT {
        c.stunned = 0.0;
    }
}

/// A missile hit in the intro: the next state, and the seconds it freezes
/// and stuns the critter hit.
fn missile_reaction(state: i32, boss_type: i32) -> Option<(i32, f32, f32)> {
    match (state, boss_type) {
        (intro::WAIT | intro::ROAR, DRAGON) => Some((intro::ROARED, DRAGON_FREEZE, 0.0)),
        (intro::WAIT | intro::ROAR, DJINN) => Some((intro::ROARED, 0.0, DJINN_STUN)),
        (intro::WAIT | intro::ROAR, PBOSS) => Some((state, 0.0, PBOSS_STUN)),
        (intro::AFTER, CHIMERA) => Some((intro::FIGHT, 0.0, 0.0)),
        _ => None,
    }
}

/// Blows at the safe rocks (kinds 5 and 6): the blow's effect at every
/// rock (the P-boss's spouts) or at the one thrown down (the yeti's
/// boulders), each a blast growing over the effect's clip that hurts the
/// heroes about it (`effects::CritterBlast`), with the effect's sounds
/// from the critter, once a rock. The rock thrown down is, of those not
/// standing, the one nearest the blow's target (the game's quick distance
/// from the target's feet); with no target, the round from the last one
/// thrown — the game tests the rocks from the first but takes the one that
/// many after the last, so it can pick a standing rock. It's made as the
/// effect's frames run out (less one, 30 a second), at stage 3.
#[allow(clippy::too_many_arguments)]
fn rock_blows(
    mut commands: Commands,
    level: Option<ResMut<CritterLevel>>,
    items: Option<ResMut<LevelItems>>,
    contents: Option<Res<ContentModels>>,
    transforms: Query<&Transform>,
    mut blasts: MessageWriter<CritterBlast>,
    mut sounds: MessageWriter<PlaySoundAt>,
    (mut shakes, mut banked): (MessageWriter<crate::play_camera::Shake>, MessageWriter<BankEffect>),
    (mut grabs, mut throws, mut releases): (MessageWriter<GrabHero>, MessageWriter<ThrowHero>, MessageWriter<ReleaseHero>),
) {
    let Some(mut level) = level else { return };
    let level = &mut *level;
    grabs.write_batch(std::mem::take(&mut level.grabs));
    throws.write_batch(std::mem::take(&mut level.throws));
    releases.write_batch(std::mem::take(&mut level.releases).into_iter().map(|hero| ReleaseHero { hero }));
    // The still effects the critters set down this tick, the bank's
    // effects they show, and a shake.
    blasts.write_batch(std::mem::take(&mut level.area_blasts));
    banked.write_batch(std::mem::take(&mut level.bank_effects));
    if std::mem::take(&mut level.shake) {
        shakes.write(SFXX_SHAKE);
    }
    let Some(mut items) = items else { return };
    // Where a rock stands: its model's pose (its node), else its centre.
    let pose_of = |items: &LevelItems, placement: usize, centre: [f32; 3]| {
        items
            .view(placement)
            .and_then(|v| v.model)
            .and_then(|m| transforms.get(m).ok())
            .copied()
            .unwrap_or_else(|| Transform::from_translation(Vec3::from(centre)))
    };
    // Rocks thrown down are made as their effects end.
    let mut made = Vec::new();
    level.rock_timers.retain_mut(|(placement, left)| {
        *left -= DT;
        let done = *left <= 0.0;
        if done {
            made.push(*placement);
        }
        !done
    });
    for placement in made {
        let centre = items.view(placement).map_or([0.0; 3], |v| v.shape.centre);
        let pose = pose_of(&items, placement, centre);
        let Some(name) = items.make_rock(placement) else { continue };
        if let Some(old) = items.take_model(placement) {
            commands.entity(old).try_despawn();
        }
        if let Some(models) = contents.as_deref() {
            models.spawn(&format!("{name}3"), pose, placement, &mut commands);
        }
        info!("the boss's rock at placement {placement} is made");
    }
    for b in std::mem::take(&mut level.rock_blows) {
        let mut rocks = items.safe_rocks();
        rocks.truncate(MAX_ROCKS);
        let n = rocks.len();
        if n == 0 {
            continue;
        }
        let spots: Vec<Vec3> = rocks.iter().map(|&(p, centre, _)| pose_of(&items, p, centre).translation).collect();
        let at: Vec<usize> = if b.every {
            (0..n).collect()
        } else {
            let fallen = |k: usize| !rocks[k].2;
            let pick = match b.target {
                Some(t) => (0..n).filter(|&k| fallen(k)).min_by(|&x, &y| {
                    let d = |k: usize| approx_hypot(spots[k].x - t[0], spots[k].z - t[2]);
                    d(x).total_cmp(&d(y))
                }),
                None => (0..n).find(|&k| fallen(k)).map(|k| (level.last_rock + k as i32 + 1).rem_euclid(n as i32) as usize),
            };
            level.last_rock = pick.map_or(-1, |k| k as i32);
            pick.into_iter().collect()
        };
        for k in at {
            let (placement, _, _) = rocks[k];
            let spot = spots[k];
            if let Some(model) = &b.effect {
                let e = model.spawn(Transform::from_translation(spot), &mut commands);
                commands.entity(e).insert((OneShot(b.life), LevelEntity));
            }
            blasts.write(CritterBlast {
                owner: b.critter,
                at: spot,
                kind: b.kind,
                damage: b.damage,
                radius: b.radius,
                life: b.life,
                heroes: true,
                monsters: false,
                items: false,
                follow: None,
                steady: false,
                cone: None,
            });
            sounds.write_batch(b.sounds.iter().cloned());
            if !b.every {
                let wait = (f32::from(b.frames) - 1.0).max(0.0) / 30.0;
                level.rock_timers.retain(|(p, _)| *p != placement);
                level.rock_timers.push((placement, wait));
                info!("the boss throws down the rock at placement {placement}: made in {wait:.2} s");
            }
        }
    }
}

/// Statues the heroes walked into wake (the item touch handler's placed
/// monster case); a placement whose critter was made goes.
fn touch_statues(mut commands: Commands, level: Option<ResMut<CritterLevel>>, items: Option<ResMut<LevelItems>>) {
    let (Some(mut level), Some(mut items)) = (level, items) else { return };
    for placement in items.take_woken() {
        if let Some(s) = level.statues.iter_mut().find(|s| s.placement == placement && !s.waking && !s.done) {
            info!("the hero walks into the statue at placement {placement}: it wakes");
            s.waking = true;
        }
    }
    for placement in std::mem::take(&mut level.made) {
        items.free(placement, &mut commands);
    }
}

/// A dead golem, gargoyle or general lets go of the powerup it held where
/// it fell; a gargoyle holding none leaves a gargoyle piece of its kind.
/// Either way the general's and the gargoyle's hint follows (the game's
/// critter drop).
fn drop_items(
    mut commands: Commands,
    level: Option<ResMut<CritterLevel>>,
    items: Option<ResMut<LevelItems>>,
    (population, contents): (Option<Res<LevelPopulation>>, Option<Res<ContentModels>>),
    mut models: Query<&mut Transform>,
    (mut hints, mut loot): (MessageWriter<ShowHint>, MessageWriter<BossLoot>),
) {
    let (Some(mut level), Some(mut items)) = (level, items) else { return };
    loot.write_batch(std::mem::take(&mut level.loot));
    for d in std::mem::take(&mut level.drops) {
        let dropped = if let Some(h) = d.held {
            if let Some(m) = items.drop_held(h, d.at, DROP_DELAY)
                && let Ok(mut t) = models.get_mut(m)
            {
                t.translation = Vec3::from(d.at);
            }
            info!("a critter drops item {h} at {:?}", d.at);
            true
        } else if d.class == class::GARGOYLE
            && let Some(ty) = population.as_ref().and_then(|p| {
                p.population.item_types.iter().find(|t| t.name.eq_ignore_ascii_case(&level.gargoyle_piece)).cloned()
            })
        {
            let name = ty.name.clone();
            let placement = items.release(ty, d.at, rotation_matrix([0.0; 3]), None, DROP_DELAY);
            if let Some(models) = contents.as_deref() {
                models.spawn(&name, Transform::from_translation(Vec3::from(d.at)), placement, &mut commands);
            }
            info!("the gargoyle leaves {name} at {:?}", d.at);
            true
        } else {
            false
        };
        if dropped {
            match d.class {
                class::GENERAL => {
                    hints.write(ShowHint::all(Hint::GeneralsCarryItems));
                }
                class::GARGOYLE => {
                    hints.write(ShowHint::all(Hint::DefeatGargoyles));
                }
                _ => {}
            }
        }
    }
}

/// Wakes statues: from the level's wake triggers (the nearest statue
/// within [`WAKE_REACH`] of each), and with `GDL_WAKE_STATUES` when the
/// hero comes that close; once its spot is on screen and within
/// [`PLACED_RANGE`] of a hero, a woken statue plays ACTIVE, then its
/// critter takes its place (the general, with no statue, comes at once).
fn wake_statues(
    level: &mut CritterLevel,
    mechanics: Option<ResMut<Mechanics>>,
    population: Option<&LevelPopulation>,
    heroes: &[Hero],
    view: &crate::monsters::Views,
    statues: &mut Query<&mut Animator, With<StatueModel>>,
    commands: &mut Commands,
) {
    if let (Some(mut mech), Some(pop)) = (mechanics, population) {
        for trigger in std::mem::take(&mut mech.woken) {
            let Some(at) = pop.population.placements.get(trigger).map(|p| p.position) else { continue };
            let nearest = level
                .statues
                .iter_mut()
                .filter(|s| !s.waking && !s.done)
                .map(|s| (horizontal(at, s.position) - s.radius, s))
                .filter(|(d, _)| *d < WAKE_REACH)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, s)) = nearest {
                info!("trigger {trigger} wakes the statue at placement {}", s.placement);
                s.waking = true;
            }
        }
    }
    if let Some(range) = std::env::var("GDL_WAKE_STATUES").ok().and_then(|v| v.parse::<f32>().ok()) {
        for s in level.statues.iter_mut().filter(|s| !s.waking && !s.done) {
            if heroes.iter().any(|h| distance(h.feet, s.position) < range) {
                info!("the hero wakes the statue at placement {} (GDL_WAKE_STATUES)", s.placement);
                s.waking = true;
            }
        }
    }
    let mut ready = Vec::new();
    for (i, s) in level.statues.iter_mut().enumerate() {
        if !s.waking || s.done {
            continue;
        }
        let seen = on_screen(view, s.position, PLACED_SCREEN_RADIUS * s.radius)
            && heroes.iter().any(|h| distance(h.feet, s.position) <= PLACED_RANGE);
        let started = s.entity.and_then(|e| statues.get(e).ok()).is_some_and(|a| a.action_name() == "ACTIVE");
        if !seen && !started {
            continue;
        }
        let finished = match s.entity.and_then(|e| statues.get_mut(e).ok()) {
            Some(mut a) => {
                if a.action_name() != "ACTIVE" && a.frame == 0.0 && a.play_named("ACTIVE") {
                    false
                } else {
                    a.action_name() != "ACTIVE" || a.finished()
                }
            }
            None => true,
        };
        if finished {
            s.done = true;
            if let Some(e) = s.entity.take() {
                commands.entity(e).try_despawn();
            }
            ready.push(i);
        }
    }
    for i in ready {
        let (enemy, position, yaw) = (level.statues[i].enemy, level.statues[i].position, level.statues[i].yaw);
        let Some(kind) = level.kinds.get(&enemy).cloned() else { continue };
        if let Some(e) = spawn_critter(level, &kind, position, yaw, commands) {
            info!("critter {} awakes at {position:?} ({e:?})", kind.file.desc.name);
            if let Some(h) = level.statues[i].holds {
                level.held.insert(e, h);
            }
            level.made.push(level.statues[i].placement);
        }
    }
}

/// The bits of `TYPE` the update reads, copied so the critter can be
/// borrowed mutably meanwhile.
#[derive(Clone, Copy)]
struct TypeInfo {
    class: i16,
    target: Condition,
    center: [f32; 3],
    radius: f32,
    height: f32,
    hover: f32,
    leash: f32,
    wake_distance: f32,
    flags: u32,
}

fn type_info(file: &CritterFile, ty: usize) -> TypeInfo {
    let t = &file.types[ty];
    TypeInfo {
        class: file.desc.class,
        target: t.target,
        center: t.center,
        radius: t.radius,
        height: t.height,
        hover: t.hover,
        leash: t.leash,
        wake_distance: t.wake_distance,
        flags: t.flags,
    }
}

/// The blows taken turn into knockback (not for bosses): the push × a
/// factor by the blows' kinds (less for the golem), capped.
fn knockback(c: &mut Critter) {
    let class = c.class();
    if class == class::BOSS {
        return;
    }
    let mut factor = if c.hit_points <= 0.0 {
        KNOCK_DEAD
    } else if c.kinds & 0x10140 != 0 {
        KNOCK_HEAVY
    } else if c.kinds & KIND_KNOCKDOWN != 0 {
        KNOCK_KNOCKDOWN
    } else if c.kinds & KIND_STRONG != 0 {
        KNOCK_STRONG
    } else {
        0.0
    };
    if class == class::GOLEM {
        factor -= KNOCK_GOLEM;
    }
    if factor > 0.0 {
        c.knock = add(c.knock, scale(c.push, factor));
        let speed = length(c.knock);
        if speed > KNOCK_MAX {
            c.knock = scale(c.knock, KNOCK_MAX / speed);
        }
        c.push = [0.0; 3];
    }
}

/// Its centre: the root plus the type's centre offset, turned with it.
fn centre_of(c: &Critter, ty: &TypeInfo) -> [f32; 3] {
    centre_at(c.position, c.yaw, ty.center)
}

/// A critter's centre: its root plus its type's centre offset, turned
/// with it (the game's `+0x5C`).
fn centre_at(position: [f32; 3], yaw: f32, o: [f32; 3]) -> [f32; 3] {
    let (s, co) = yaw.dsin_cos();
    [position[0] + o[0] * co + o[2] * s, position[1] + o[1], position[2] - o[0] * s + o[2] * co]
}

/// Whether the critter's anger is outside a condition's range.
fn anger_fails(anger: f32, cond: &Condition) -> bool {
    anger < cond.min_anger || (cond.min_anger < cond.max_anger && cond.max_anger <= anger)
}

/// The game's target score for a point: the horizontal distance over the
/// cosine between the (turned) facing and the direction, or twice the
/// distance when that's 60° or more off; a failed condition scores 1e21 or
/// more.
fn score(c: &Critter, cond: &Condition, centre: [f32; 3], point: [f32; 3]) -> (f32, f32, [f32; 2]) {
    let (dx, dy, dz) = (point[0] - centre[0], point[1] - centre[1], point[2] - centre[2]);
    let distance = (dx * dx + dz * dz).sqrt();
    let dir = if distance > 0.0 { [dx / distance, dz / distance] } else { [0.0, 1.0] };
    if anger_fails(c.anger, cond) {
        return (1.2e21, distance, dir);
    }
    if distance < cond.min_distance {
        return (1.01e21, distance, dir);
    }
    if cond.max_distance > 0.0 && cond.max_distance < distance {
        return (1.02e21, distance, dir);
    }
    if cond.max_height > 0.0 && cond.max_height < dy.abs() {
        return (1.03e21, distance, dir);
    }
    let h = c.yaw - cond.angle;
    let cos = h.dsin() * dir[0] + h.dcos() * dir[1];
    if cos < cond.min_cos {
        return (1.1e21, distance, dir);
    }
    let s = if cos <= 0.5 { distance * 2.0 } else { distance / cos.abs() };
    (s, distance, dir)
}

/// The players it tracks by its type's target condition, best first. A
/// golem keeps the best whatever it scores; a boss up to four that pass
/// (weighted 1: the game weighs damage dealt against taken, which only
/// matters with several players). Players a critter hit in the last
/// quarter second count a thousand times worse. Only bosses track an
/// invisible hero.
fn track(c: &mut Critter, ty: &TypeInfo, centre: [f32; 3], heroes: &[Hero], level: &CritterLevel) {
    let mut found: Vec<Tracked> = Vec::new();
    for h in heroes.iter().filter(|h| !h.hidden || ty.class == class::BOSS) {
        let (mut s, distance, direction) = score(c, &ty.target, centre, h.feet);
        if level.guard.get(&h.entity).is_some_and(|&until| level.clock < until) {
            s *= RECENTLY_HIT;
        }
        if ty.class == class::BOSS && s >= REJECTED {
            continue;
        }
        let centre = [h.feet[0], h.feet[1] + h.half, h.feet[2]];
        found.push(Tracked { player: h.entity, distance, direction, position: h.feet, score: s, weight: 1.0, centre });
    }
    found.sort_by(|a, b| a.score.total_cmp(&b.score));
    found.truncate(if ty.class == class::BOSS { MAX_TARGETS } else { 1 });
    c.tracked = found;
}

/// The tracked target that best meets `cond` (with `fallback`, the best one
/// anyway).
fn best_target(c: &Critter, cond: &Condition, fallback: bool) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for t in &c.tracked {
        let s = if anger_fails(c.anger, cond) {
            1.2e21
        } else if t.distance < cond.min_distance {
            1.01e21
        } else if cond.max_distance > 0.0 && cond.max_distance < t.distance {
            1.02e21
        } else {
            let h = c.yaw - cond.angle;
            if h.dsin() * t.direction[0] + h.dcos() * t.direction[1] < cond.min_cos { 1.1e21 } else { t.distance * t.weight }
        };
        if best.is_none_or(|(b, _)| s < b) {
            best = Some((s, t.player));
        }
    }
    match best {
        Some((s, p)) if s < REJECTED || fallback => Some(p),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Find {
    /// Only a move off cooldown.
    Ready,
    /// The one nearest to ready.
    Nearest,
}

/// The game's move lookup by kind: skips disabled moves; the one whose
/// cooldown runs out soonest (only ready ones for [`Find::Ready`]).
fn find(c: &Critter, k: i32, mode: Find, now: f32) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (i, m) in c.moves().iter().enumerate() {
        if m.flags & 4 != 0 || m.kind != k {
            continue;
        }
        let left = if m.cooldown <= 0.0 { 0.0 } else { c.ends[i] + m.cooldown - now };
        if (left <= 0.0 || mode == Find::Nearest) && best.is_none_or(|(b, _)| left < b) {
            best = Some((left, i));
        }
    }
    match best {
        Some((_, i)) => Some(i),
        None if mode == Find::Nearest && k != kind::READY => {
            warn!("critter can't find move kind {k:#x}");
            find(c, kind::READY, Find::Nearest, now)
        }
        None => None,
    }
}

fn ready(c: &Critter, i: usize, now: f32) -> bool {
    let m = &c.moves()[i];
    m.cooldown <= 0.0 || c.ends[i] + m.cooldown <= now
}

/// The moves forced on it: INIT when new (a sleeping boss keeps playing
/// it), START after INIT, DEATH when dying, a move's follow-up; else, by
/// the boss intro's state, READY in 1–2, ROAR in 3, READY for the chimera
/// in 3–5, and otherwise a boss's READY after START; then reactions to the
/// blows taken.
fn forced(c: &mut Critter, ty: &TypeInfo, now: f32, intro: i32, boss_type: i32, roar_at: f32) {
    let cur = c.current.map(|i| c.moves()[i].clone());
    let chimera_waits = (intro::ROAR..=intro::AFTER).contains(&intro) && boss_type == CHIMERA;
    let next = match (&cur, c.state) {
        (None, _) | (_, CritterState::New) => find(c, kind::INIT, Find::Nearest, now),
        (Some(m), _) if m.kind == kind::INIT => find(c, kind::START, Find::Ready, now),
        (_, CritterState::Dying) => find(c, kind::DEATH, Find::Nearest, now),
        (Some(m), _) if m.next >= 0 => Some(m.next as usize),
        _ if (intro::START..=intro::WAIT).contains(&intro) => find(c, kind::READY, Find::Nearest, now),
        (Some(m), _) if intro == intro::ROAR && m.kind != kind::ROAR => find(c, kind::ROAR, Find::Nearest, now),
        _ if chimera_waits => find(c, kind::READY, Find::Ready, now),
        (Some(m), _) if m.kind == kind::START && ty.class == class::BOSS => find(c, kind::READY, Find::Ready, now),
        _ => None,
    };
    c.next = next;
    reactions(c, now, roar_at);
    if c.next.is_some() {
        // A forced move ends a pattern, the parts' too.
        c.pattern = None;
        for p in &mut c.parts {
            p.pattern = None;
        }
    }
}

/// Reactions to the blows taken, when nothing else is forced: a knockdown
/// or knockback, a roar after enough damage, a flinch.
fn reactions(c: &mut Critter, now: f32, roar_at: f32) {
    if c.next.is_none() && c.kinds & 0x120 != 0 {
        if c.kinds & KIND_HEAVY != 0 {
            c.next = find(c, kind::KNOCKDOWN, Find::Ready, now);
        }
        if c.next.is_none() {
            c.next = find(c, kind::KNOCKBACK, Find::Ready, now);
        }
    }
    if c.next.is_none() && c.damage_taken >= roar_at {
        c.next = find(c, kind::ROAR, Find::Ready, now);
    }
    if c.next.is_none() && c.kinds & KIND_STRONG != 0 {
        c.next = find(c, kind::FLINCH, Find::Ready, now);
    }
    c.kinds &= !KIND_REACTIONS;
}

/// Whether a move's node (and its follow-up's) are there.
fn has_nodes(c: &Critter, i: usize) -> bool {
    let body = c.body();
    let m = &c.moves()[i];
    body.nodes[i].is_some() && (m.next < 0 || body.nodes.get(m.next as usize).copied().flatten().is_some())
}

/// A block when the target it would block is attacking.
fn choose_block(c: &mut Critter, now: f32, heroes: &[Hero]) {
    for i in 0..c.moves().len() {
        let m = c.moves()[i].clone();
        if m.kind != kind::BLOCK || m.flags & 4 != 0 || (m.flags & 0x10 != 0 && !has_nodes(c, i)) || !ready(c, i, now) {
            continue;
        }
        if let Some(t) = best_target(c, &m.condition, false)
            && heroes.iter().any(|h| h.entity == t && h.attacking)
        {
            c.next = Some(i);
        }
    }
}

/// The next step of a running pattern; else the least recently used
/// pattern or attack whose condition finds a target.
fn choose_attack(c: &mut Critter, now: f32) {
    let patterns = c.kind.file.type_patterns(c.ty).to_vec();
    if let Some((p, step)) = c.pattern
        && let Some(&m) = patterns[p].moves.get(step + 1)
        && m >= 0
    {
        c.next = Some(m as usize);
        return;
    }
    let all_parts = c.parts.iter().all(|p| p.state != CritterState::Dying);
    let mut best_time = 999_999.0f32;
    let mut pattern_pick: Option<usize> = None;
    for (i, p) in patterns.iter().enumerate() {
        if Some(i) == c.pattern.map(|p| p.0)
            || p.flags & 0x1000 != 0
            || (p.flags & 2 != 0 && !all_parts)
            || c.pattern_starts[i] + p.cooldown > now
        {
            continue;
        }
        let Some(t) = best_target(c, &p.condition, false) else { continue };
        if c.pattern_starts[i] < best_time {
            best_time = c.pattern_starts[i];
            pattern_pick = Some(i);
            c.pick = Some(t);
        }
    }
    let mut chosen: Option<usize> = None;
    for i in 0..c.moves().len() {
        let m = c.moves()[i].clone();
        if Some(i) == c.current || !m.is_attack() || m.flags & 4 != 0 || (m.flags & 2 != 0 && !all_parts) {
            continue;
        }
        if m.flags & 0x10 != 0 && !has_nodes(c, i) {
            continue;
        }
        if !ready(c, i, now) {
            continue;
        }
        let Some(t) = best_target(c, &m.condition, false) else { continue };
        if c.ends[i] < best_time {
            best_time = c.ends[i];
            chosen = Some(i);
            pattern_pick = None;
            c.pick = Some(t);
        } else if pattern_pick.is_none() && chosen.is_none_or(|b| transition(&c.moves()[b], &m) > 1) {
            chosen = Some(i);
            c.pick = Some(t);
        }
    }
    match pattern_pick {
        Some(p) => {
            c.chosen_pattern = Some(p);
            c.next = usize::try_from(patterns[p].moves[0]).ok();
        }
        None => c.next = chosen,
    }
}

/// The movement move that scores its target best.
fn choose_movement(c: &mut Critter, centre: [f32; 3], now: f32) {
    let Some(t) = c.tracked.first().copied() else { return };
    let mut best = REJECTED;
    for i in 0..c.moves().len() {
        let m = &c.moves()[i];
        if !m.is_movement() || m.flags & 4 != 0 || (m.kind == kind::GO_TO_POINT) || !ready(c, i, now) {
            continue;
        }
        let (s, _, _) = score(c, &m.condition, centre, t.position);
        if s < best {
            best = s;
            c.next = Some(i);
        }
    }
}

/// How a move may give way to the next (0 when its animation ends and the
/// move differs, 1 when it ends, 2 at once), by the playing move's
/// transition and the two priorities.
fn transition(cur: &CritterMove, next: &CritterMove) -> u8 {
    let (a, b) = (cur.priority, next.priority);
    match cur.transition {
        0 => 0,
        0x14 => {
            if (b & !0xFF) <= (a & !0xFF) {
                1
            } else {
                2
            }
        }
        0x3C => {
            if b < a {
                1
            } else {
                2
            }
        }
        0x50 => {
            if b < 1 {
                1
            } else {
                2
            }
        }
        0x5A => 2,
        _ => {
            if a < b {
                2
            } else {
                1
            }
        }
    }
}

/// Switches to the chosen move the way the transition allows (not before
/// a boss's hold on its move runs out, unless the new one outranks
/// everything), and starts its animation; a switch records when the move
/// will end (its cooldown runs from then), and steps or starts a pattern.
/// Leaving START (without a follow-up) moves the boss intro 1 → 2, leaving
/// ROAR 3 → 4.
fn switch(c: &mut Critter, now: f32, animator: Option<&mut Animator>, intro: &mut i32, steps_pattern: bool) {
    let cur = c.current.map(|i| c.moves()[i].clone());
    let next = c.next;
    let (target, mode) = match (&cur, next) {
        (_, None) => (c.current, 0u8),
        (None, Some(n)) => (Some(n), 3),
        (Some(m), Some(n)) => {
            let nm = &c.moves()[n];
            if Some(n) != c.current && nm.priority >= 0xF00 && m.transition != 0 {
                (Some(n), 3)
            } else if c.hold_until > now {
                (c.current, 0)
            } else {
                (Some(n), transition(m, nm))
            }
        }
    };
    let mode = if target != c.current && mode == 0 && c.hold_until <= now && c.clock.ended { 1 } else { mode };
    let Some(t) = target else { return };
    let action = c.body().actions[t];
    let differs = action != c.clock.action || c.current.is_none();
    let go = match mode {
        0 => c.clock.ended && differs,
        1 => c.clock.ended,
        2 => c.clock.ended || differs,
        _ => true,
    };
    if !go {
        if next.is_none() && c.clock.ended {
            c.current = None;
        }
        return;
    }
    if let Some(m) = &cur {
        if m.kind == kind::ROAR && *intro == intro::ROAR {
            *intro = intro::ROARED;
        } else if m.kind == kind::START && m.next < 0 && *intro == intro::START {
            *intro = intro::WAIT;
        }
    }
    let clip = c.body().clips.get(action).copied().unwrap_or((1, 30, false));
    c.clock.start(action, clip);
    if let Some(a) = animator {
        a.play(action);
    }
    c.switched = true;
    c.current = Some(t);
    c.hold_until = 0.0;
    // Patterns: a new one starts; a running one steps on (or ends).
    if let Some(p) = c.chosen_pattern.take() {
        c.pattern_starts[p] = now;
        c.pattern = Some((p, 0));
    } else if let Some((p, step)) = c.pattern {
        // A part following its body's pattern doesn't step it itself.
        if steps_pattern {
            let patterns = c.kind.file.type_patterns(c.ty);
            let step = step + 1;
            c.pattern = patterns[p].moves.get(step).filter(|&&m| m >= 0 && m as usize == t).map(|_| (p, step));
        }
    } else {
        c.ends[t] = now + FRAME_TIME * (f32::from(clip.0) - 2.0);
    }
}

/// Which of the move's two blows land this frame (bits 1, 2): sweeps land
/// every frame of their windows, the rest once from their frames.
fn blow_bits(m: &CritterMove, frame: i32, done: u8) -> u8 {
    if m.hit_frames[0] < 0 {
        return 0;
    }
    let mut bits = 0;
    match m.kind {
        // A grab: blow 1 every frame of its window, blow 2 once.
        kind::GRAB => {
            if m.hit_frames[0] <= frame && frame <= i32::from(m.hit_ends[0]) {
                bits |= 1;
            }
            if m.hit_frames[1] >= 0 && done & 2 == 0 && m.hit_frames[1] <= frame {
                bits |= 2;
            }
        }
        kind::SWEEP | kind::SWEEP_2 | kind::SWEEP_3 => {
            if m.hit_frames[0] <= frame && frame <= i32::from(m.hit_ends[0]) {
                bits |= 1;
            }
            if m.hit_frames[1] >= 0 && m.hit_frames[1] <= frame && frame <= i32::from(m.hit_ends[1]) {
                bits |= 2;
            }
        }
        _ => {
            if done & 1 == 0 && m.hit_frames[0] <= frame {
                bits |= 1;
            }
            if m.hit_frames[1] >= 0 && done & 2 == 0 && m.hit_frames[1] <= frame {
                bits |= 2;
            }
        }
    }
    bits
}

/// A blow, by its `DAMG` kind:
///
/// - 0: a sphere on the move's node, swept from where it was last tick,
///   against each player's cylinder;
/// - 1, 2, 8: a missile from the node (its offset in the critter's space),
///   aimed with an arc at its target (1: flag 1) or along its facing, as
///   fast as the critter's anger picks from the speed range, turned by the
///   blow's yaw and spread;
/// - 3: a ring on the ground: players within its radius (stand-in for the
///   game's damaging effect, at once);
/// - 4: a breath cone along the node's forward axis (turned by its yaw
///   and pitch): players between its reach and length, within its
///   thickness;
/// - 9: a boss's loot (`loot.rs`).
///
/// Players a critter hit can't be hit again for a quarter second.
#[allow(clippy::too_many_arguments)]
fn deal(
    c: &mut Critter,
    me: Entity,
    d: &CritterDamage,
    (slot, first): (usize, bool),
    (node, node_bone): (Affine3A, Option<Entity>),
    heroes: &[Hero],
    level: &mut CritterLevel,
    blows: &mut Vec<Blow>,
    to_play: &mut Vec<PlaySoundAt>,
    commands: &mut Commands,
) {
    let mut damage = d.damage * level.damage_scale;
    // The boss's loot (its DEATH's blow): its forward, tilted by the
    // blow's pitch, at the blow's speed, spread acos(param) either side,
    // from the move's node (`loot.rs`).
    if d.kind == LOOT {
        if first {
            let (s, co) = c.yaw.dsin_cos();
            let velocity = turn_dir(Vec3::new(s, 0.0, co), d.yaw, d.pitch) * d.speed[0];
            let at = Vec3::from(c.node_at.unwrap_or(c.position));
            let spread = d.param.clamp(-1.0, 1.0).dacos();
            level.loot.push(BossLoot { at, velocity, spread, boss: level.boss_type, realm: level.realm_id });
            level.events.push("loot");
        }
        return;
    }
    // The blow's effect starts on its first frame (a grab's, the safe
    // rocks' at each rock): its sounds.
    if first
        && !matches!(d.kind, 5..=7)
        && let Ok(e) = usize::try_from(d.effects[0])
    {
        level.shake |= effect_sounds(c, e, level.realm, to_play);
    }
    if d.kind == 1 {
        if first {
            launch(c, me, d, damage, level, commands);
            level.events.push("missile");
        }
        return;
    }
    // A grab: blow 1 holds a hero, blow 2 throws it.
    if d.kind == GRAB_BLOW {
        grab(c, me, d, slot, damage, (node, node_bone), heroes, level, to_play, commands);
        return;
    }
    // Still effects doing their damage where they're set down.
    if matches!(d.kind, 2 | 3 | 8) {
        if first {
            set_down(c, me, d, damage, (node, node_bone), heroes, level, commands);
        }
        return;
    }
    // A sphere's or a cone's own effect; a sphere's slot hurts the first
    // hero it touches.
    if first && matches!(d.kind, 0 | 4) {
        show_effect(c, me, d, (node, node_bone), level, commands);
        if d.kind == 0 {
            touch_slot(c, me, d, damage, (node, node_bone), level, commands);
        }
    }
    // At the safe rocks (`rock_blows`); the game makes none with no rocks
    // gathered.
    if matches!(d.kind, 5 | 6) {
        if first {
            let target = c.move_target.and_then(|p| heroes.iter().find(|h| h.entity == p)).map(|h| h.feet);
            let e = usize::try_from(d.effects[0]).ok();
            let (frames, rate) = e.and_then(|e| c.kind.effect_clips.get(&e)).copied().unwrap_or((0, 0));
            let mut sounds = Vec::new();
            if let Some(e) = e {
                level.shake |= effect_sounds(c, e, level.realm, &mut sounds);
            }
            level.rock_blows.push(RockBlow {
                critter: me,
                every: d.kind == 5,
                target,
                damage,
                radius: d.radius,
                kind: d.blow,
                effect: e.and_then(|e| c.kind.effects.get(&e)).cloned(),
                frames,
                life: effect_life(frames, rate),
                sounds,
            });
            level.events.push("rocks");
        }
        return;
    }
    // Shrunk, all but bosses deal half (their missiles' damage isn't
    // traced).
    if c.class() != class::BOSS && level.enemy_scale < 1.0 {
        damage *= SHRUNK_DEAL;
    }
    let offset = node.matrix3 * Vec3::from(d.offset);
    let at = Vec3::from(c.node_at.unwrap_or(c.position)) + offset;
    let was = Vec3::from(c.node_was.unwrap_or(c.position)) + offset;
    // The breath's direction: the node's forward axis, turned.
    let forward = turn_dir(Vec3::from(node.matrix3.z_axis).normalize_or_zero(), d.yaw, d.pitch);
    for h in heroes {
        if level.guard.get(&h.entity).is_some_and(|&until| level.clock < until) {
            continue;
        }
        let centre = Vec3::from(h.feet) + Vec3::Y * h.half;
        let hit = match d.kind {
            0 => cylinder_hit(was, at, centre, h.radius + d.radius, h.half + d.radius).is_some(),
            4 => {
                let v = centre - at;
                let across = Vec2::new(v.x, v.z).length();
                across >= d.min_range
                    && across <= d.radius
                    && cylinder_hit(at, at + forward * d.radius, centre, h.radius + d.size, h.half + d.size).is_some()
            }
            _ => false,
        };
        if !hit {
            continue;
        }
        let towards = Vec3::new(h.feet[0] - c.position[0], 1.0, h.feet[2] - c.position[2]).normalize_or_zero();
        let push = 0.5 * ((at - was) + towards);
        // A sphere or a cone that does damage starts the blow's hit effect
        // at the hero's feet (its sounds), standing in for the hero's own
        // hit look.
        let mut kind_bits = d.blow;
        if matches!(d.kind, 0 | 4)
            && damage > 0.0
            && let Ok(e) = usize::try_from(d.effects[1])
        {
            level.shake |= effect_sounds(c, e, level.realm, to_play);
            kind_bits |= crate::combat::hit_kind::NO_HIT_LOOK;
        }
        blows.push((h.entity, damage, kind_bits, push.to_array()));
        level.guard.insert(h.entity, level.clock + f64::from(HIT_GUARD));
        c.blows_dealt += 1;
        debug!("critter {me:?} blow kind {} lands for {damage:.1}", d.kind);
    }
    if first && !matches!(d.kind, 0 | 4) {
        debug!("critter {me:?}: damage kind {} not done yet", d.kind);
    }
}

/// A grab's blow kind.
const GRAB_BLOW: i16 = 7;
/// The throw's push is the body's forward with this much down, as a unit,
/// × the blow's speed.
const THROW_DOWN: f32 = -0.1;
/// The hero's top point above its feet (`+0x838`): a held hero hangs that
/// far below the node's point.
const HERO_TOP: f32 = 4.4;

/// A grab (`DAMG` kind 7; `docs/critters.md`, "7 — grab"): blow 1 (every
/// frame of its window), while it holds nobody, finds the nearest hero its
/// sphere reaches (no damage, no guard) and holds it on the move's node at
/// the blow's offset, the hero's top there — the hit record's effect and
/// sounds on it; blow 2 throws the hero it holds: the blow's damage (half
/// for all but bosses while the enemies are shrunk), kind `| 0x8050`, a
/// push along the body's forward tilted down by 0.1, × the blow's speed,
/// and the hero guarded a quarter second.
#[allow(clippy::too_many_arguments)]
fn grab(
    c: &mut Critter,
    me: Entity,
    d: &CritterDamage,
    slot: usize,
    mut damage: f32,
    (node, node_bone): (Affine3A, Option<Entity>),
    heroes: &[Hero],
    level: &mut CritterLevel,
    to_play: &mut Vec<PlaySoundAt>,
    commands: &mut Commands,
) {
    let hit_record = usize::try_from(d.effects[1]).ok();
    if slot == 0 {
        if c.held.is_some() {
            return;
        }
        let offset = node.matrix3 * Vec3::from(d.offset);
        let at = Vec3::from(c.node_at.unwrap_or(c.position)) + offset;
        let was = Vec3::from(c.node_was.unwrap_or(c.position)) + offset;
        let found = heroes
            .iter()
            .filter(|h| {
                let centre = Vec3::from(h.feet) + Vec3::Y * h.half;
                cylinder_hit(was, at, centre, h.radius + d.radius, h.half + d.radius).is_some()
            })
            .min_by(|a, b| {
                let d = |h: &Hero| (Vec3::from(h.feet) + Vec3::Y * h.half).distance(at);
                d(a).total_cmp(&d(b))
            });
        let Some(h) = found else { return };
        c.held = Some(h.entity);
        level.grabs.push(GrabHero {
            hero: h.entity,
            node: node_bone,
            offset: Vec3::from(d.offset) - Vec3::Y * HERO_TOP,
            at: at - Vec3::Y * HERO_TOP,
        });
        if let Some(e) = hit_record {
            level.shake |= effect_sounds(c, e, level.realm, to_play);
            spawn_effect(c, e, Anchor::On(h.entity, Vec3::ZERO), Affine3A::IDENTITY, record_life(c, e), level, commands);
        }
        info!("critter {me:?} grabs the hero");
        level.events.push("grab");
        return;
    }
    let Some(hero) = c.held.take() else { return };
    if c.class() != class::BOSS && level.enemy_scale < 1.0 {
        damage *= SHRUNK_DEAL;
    }
    // The throw's kind (the blow's | 0x8050) makes the hero fall
    // (`player.rs`); its damage lands later, plain.
    let (s, co) = c.yaw.dsin_cos();
    let push = Vec3::new(s, THROW_DOWN, co).normalize_or_zero() * d.speed[0];
    level.throws.push(ThrowHero { hero, damage, push });
    level.guard.insert(hero, level.clock + f64::from(HIT_GUARD));
    info!("critter {me:?} throws the hero: {damage:.0} damage, push {push:?}");
    level.events.push("throw");
}

/// Where a critter's effect goes: on an entity (its root or a node), at a
/// point in its space, or left at a point in the world.
#[derive(Clone, Copy, Debug)]
enum Anchor {
    On(Entity, Vec3),
    At(Vec3),
}

impl Anchor {
    /// Where it is now, the entity's pose being `pose`.
    fn point(self, pose: impl Fn(Entity) -> Affine3A) -> Vec3 {
        match self {
            Anchor::On(e, off) => pose(e).transform_point3(off),
            Anchor::At(p) => p,
        }
    }
}

/// Where a blow's effect record puts its effect (the game's effect start,
/// `docs/critters.md` "What a DAMG does"): record flag 1 or 0x800 on the
/// critter's root at the record's offset; 0x80 at the critter's home (its
/// spawn point) plus the offset, 0x40 at the move's node (the offset in
/// its space) plus the blow's place — both left there; else a blow on the
/// node (kinds 0, 2, 3, 4) goes on the node at the blow's offset plus the
/// record's, and a blow at the target (kind 8) at the target's feet plus
/// the blow's offset turned with the critter, plus the record's. (Kind 8
/// with 0x40 adds the node's place to the target's, as decoded.)
fn anchor(c: &Critter, me: Entity, d: &CritterDamage, record: &gdl_formats::critter::CritterSound, (node, node_bone): (Affine3A, Option<Entity>), target: Option<Vec3>) -> Anchor {
    let off = Vec3::from(record.offset);
    let blow = Vec3::from(d.offset);
    // Kind 8's place is the target's plus the blow's offset turned with
    // the critter; the others' the blow's offset as it is.
    let place = if d.kind == 8 {
        target.unwrap_or(Vec3::from(c.position)) + Quat::from_rotation_y(c.yaw) * blow
    } else {
        blow
    };
    if record.flags & SFXX_ON_ROOT != 0 {
        Anchor::On(me, off)
    } else if record.flags & SFXX_AT_SPAWN != 0 {
        Anchor::At(Vec3::from(c.home) + off)
    } else if record.flags & SFXX_AT_NODE != 0 {
        Anchor::At(node.transform_point3(off) + place)
    } else if d.kind == 8 {
        Anchor::At(place + off)
    } else {
        match node_bone {
            Some(b) => Anchor::On(b, place + off),
            None => Anchor::On(me, place + off),
        }
    }
}

/// Shows a blow's effect (its effect record's model at the record's size)
/// where the record puts it, for `life` seconds — from the effect table's
/// bank when the critter's folder doesn't hold it.
fn spawn_effect(c: &Critter, e: usize, at: Anchor, pose: Affine3A, life: f32, level: &mut CritterLevel, commands: &mut Commands) {
    let size = c.kind.file.sounds.get(e).map_or(1.0, |r| r.size);
    let Some(model) = c.kind.effects.get(&e) else {
        if let Some(r) = c.kind.file.sounds.get(e).filter(|r| !r.effect.is_empty()) {
            let (_, rotation, _) = pose.to_scale_rotation_translation();
            level.bank_effects.push(BankEffect {
                name: r.effect.clone(),
                at: match at {
                    Anchor::At(p) => p,
                    Anchor::On(..) => pose.translation.into(),
                },
                on: match at {
                    Anchor::On(who, off) => Some((who, off)),
                    Anchor::At(_) => None,
                },
                rotation,
                scale: size,
                life,
            });
        }
        return;
    };
    let local = |off: Vec3| Transform::from_translation(off).with_scale(Vec3::splat(size));
    let fx = match at {
        Anchor::On(parent, off) => {
            let fx = model.spawn(local(off), commands);
            commands.entity(fx).insert(ChildOf(parent));
            fx
        }
        Anchor::At(p) => {
            let (_, rotation, _) = pose.to_scale_rotation_translation();
            model.spawn(local(p).with_rotation(rotation), commands)
        }
    };
    commands.entity(fx).insert((OneShot(life), LevelEntity));
}

/// An effect record's life: its own, else its clip's (30 frames with
/// none).
fn record_life(c: &Critter, e: usize) -> f32 {
    let record = c.kind.file.sounds.get(e);
    match record.map(|r| r.life) {
        Some(life) if life > 0.0 => life,
        _ => c.kind.effect_clips.get(&e).map_or(1.0, |&(f, rate)| effect_life(f, rate)),
    }
}

/// A still effect doing its damage where it's set down (kinds 2, 3 and 8:
/// the drider's and the wraith's attached attacks, the stomp rings, the
/// djinn's and the lich's at their target): the blow's effect (none, or one
/// the table doesn't hold, and nothing happens) where its record puts it,
/// and a blast out to `DAMG +0x0C` there over the effect's life —
/// following the critter or its node when the effect is attached — on the
/// heroes (not with the blow's flag 0x1000) and, for all but bosses, the
/// monsters (never itself). Kinds 3 and 8 grow as any blast does; kind 2
/// (its slot's flags `0x30`) holds its whole radius at full damage, each
/// target spared a second at most.
#[allow(clippy::too_many_arguments)]
fn set_down(c: &Critter, me: Entity, d: &CritterDamage, damage: f32, node: (Affine3A, Option<Entity>), heroes: &[Hero], level: &mut CritterLevel, commands: &mut Commands) {
    let Some((e, record)) = usize::try_from(d.effects[0]).ok().filter(|e| c.kind.effect_clips.contains_key(e)).and_then(|e| Some((e, c.kind.file.sounds.get(e)?))) else {
        debug!("critter {me:?}: a still effect with no effect the table holds: nothing");
        return;
    };
    let target = c.move_target.and_then(|p| heroes.iter().find(|h| h.entity == p)).map(|h| Vec3::from(h.feet));
    let at = anchor(c, me, d, record, node, target);
    let life = record_life(c, e);
    let pose = node.0;
    spawn_effect(c, e, at, pose, life, level, commands);
    let root = Affine3A::from_rotation_translation(Quat::from_rotation_y(c.yaw), Vec3::from(c.position));
    let here = at.point(|who| if who == me { root } else { pose });
    level.area_blasts.push(CritterBlast {
        owner: me,
        at: here,
        follow: match at {
            Anchor::On(who, off) => Some((who, off)),
            Anchor::At(_) => None,
        },
        kind: d.blow,
        damage,
        radius: d.radius,
        life,
        heroes: d.flags & NO_PLAYERS == 0,
        monsters: c.class() != class::BOSS,
        items: false,
        steady: d.kind == HELD_ATTACK,
        cone: area_cone(d, record.flags, at, c.yaw),
    });
    info!("critter {me:?} sets down {} (kind {}): {damage:.0} out to {:.0} over {life:.2} s at {here:?}", record.effect, d.kind, d.radius);
    level.events.push("effect");
}

/// An area's cone (its blow's `DAMG +0x18` above −1: the cosine it hits
/// within) and the slot's facing: the blow's yaw `+0x14` from its anchor's
/// facing when it's held on the root or a node (in that space), from the
/// critter's when set down at the node's point (record flag 0x40: the slot
/// takes the critter's rotation), else from the world's +Z (at the spawn
/// point, or at the target).
fn area_cone(d: &CritterDamage, record_flags: u32, at: Anchor, yaw: f32) -> Option<(f32, Quat)> {
    if d.param <= -1.0 {
        return None;
    }
    let from = match at {
        Anchor::On(..) => 0.0,
        Anchor::At(_) if record_flags & (SFXX_ON_ROOT | SFXX_AT_SPAWN) == 0 && record_flags & SFXX_AT_NODE != 0 => yaw,
        Anchor::At(_) => 0.0,
    };
    Some((d.param, Quat::from_rotation_y(from + d.yaw)))
}

/// A sphere's or a cone's own effect (kinds 0 and 4): shown where its
/// record puts it for its life. A cone's slot does no damage (the game
/// gives it none, and no flags to hit anything with); a sphere's touches
/// ([`touch_slot`]).
fn show_effect(c: &Critter, me: Entity, d: &CritterDamage, node: (Affine3A, Option<Entity>), level: &mut CritterLevel, commands: &mut Commands) {
    let Some((e, record)) = usize::try_from(d.effects[0]).ok().and_then(|e| Some((e, c.kind.file.sounds.get(e)?))) else { return };
    let at = anchor(c, me, d, record, node, None);
    spawn_effect(c, e, at, node.0, record_life(c, e), level, commands);
}

/// How far a blow's own effect slot touches heroes from where it's held: a
/// sphere's (kind 0) with an effect record, its `DAMG +0x08` × the
/// critter's scale. A cone's (kind 4) carries no damage; the other kinds'
/// slots are missiles or areas.
fn touch_radius(d: &CritterDamage, scale: f32) -> Option<f32> {
    (d.kind == 0 && d.effects[0] >= 0).then_some(d.size * scale)
}

/// A blow's hit record (`DAMG +0x42`): where its effect slot stops, the
/// record's sound, and its effect for that clip, bursting out to `DAMG
/// +0x0C` (no effect, no burst).
fn hit_record(c: &Critter, d: &CritterDamage, realm: char) -> CritterStop {
    let hit = usize::try_from(d.effects[1]).ok().and_then(|h| c.kind.file.sounds.get(h).map(|r| (h, r)));
    hit.map_or_else(CritterStop::default, |(h, r)| {
        let clip = c.kind.effect_clips.get(&h).map(|&(f, rt)| f32::from(f) / clip_fps(rt));
        CritterStop {
            sound: (!r.sound.is_empty()).then(|| r.sound.replace("%c", &realm.to_string())),
            effect: c.kind.effects.get(&h).cloned(),
            life: clip.unwrap_or(0.0),
            blast: if clip.is_some() { d.radius } else { 0.0 },
        }
    })
}

/// A sphere blow's own effect slot (kind 0 with an effect record: the
/// lich's axe and chain; `docs/critters.md`, "What a `DAMG` does"): a still
/// slot where the record puts its effect — held on the move's node, at the
/// blow's offset — for the record's life, carrying the blow's damage and
/// kind: the first hero (not with the blow's flag 0x1000) whose cylinder
/// comes within its own radius (`DAMG +0x08`) takes the damage through the
/// critter guard, and the slot stops there (a guarded hero stops it too),
/// playing its hit record. It doesn't touch the level or items. With no
/// effect the table holds there's no slot.
fn touch_slot(c: &Critter, me: Entity, d: &CritterDamage, damage: f32, node: (Affine3A, Option<Entity>), level: &mut CritterLevel, commands: &mut Commands) {
    let Some((e, record)) = usize::try_from(d.effects[0])
        .ok()
        .filter(|e| c.kind.effect_clips.contains_key(e))
        .and_then(|e| Some((e, c.kind.file.sounds.get(e)?)))
    else {
        return;
    };
    let Some(radius) = touch_radius(d, level.enemy_scale) else { return };
    let at = anchor(c, me, d, record, node, None);
    let root = Affine3A::from_rotation_translation(Quat::from_rotation_y(c.yaw), Vec3::from(c.position));
    let here = at.point(|who| if who == me { root } else { node.0 });
    let life = record_life(c, e);
    let stop = hit_record(c, d, level.realm);
    spawn_critter_missile(
        commands,
        CritterMissile {
            model: None,
            critter: me,
            start: here,
            velocity: Vec3::ZERO,
            gravity: 0.0,
            radius,
            damage,
            kind: d.blow,
            scale: record.size,
            hits_players: d.flags & NO_PLAYERS == 0,
            hits_level: false,
            stop,
            anchor: match at {
                Anchor::On(who, off) => Some((who, off)),
                Anchor::At(_) => None,
            },
            life: Some(life),
        },
    );
    info!("critter {me:?}'s {} slot: {damage:.0} to the first hero within {radius:.1} for {life:.2} s at {here:?}", record.effect);
    level.events.push("touch");
}

/// Turns a direction by `yaw` about the vertical and tilts it by `pitch`
/// about the horizontal axis across it.
fn turn_dir(dir: Vec3, yaw: f32, pitch: f32) -> Vec3 {
    let d = Quat::from_rotation_y(yaw) * dir;
    let across = Vec3::new(d.z, 0.0, -d.x).normalize_or_zero();
    if across == Vec3::ZERO { d } else { Quat::from_axis_angle(across, pitch) * d }
}

/// The launch direction that brings a missile of `speed` under `gravity`
/// from `from` to `to` on the flatter arc (straight at it when it can't
/// reach).
fn lob_toward(from: Vec3, to: Vec3, speed: f32, gravity: f32) -> Vec3 {
    let d = to - from;
    let flat = Vec2::new(d.x, d.z);
    let h = flat.length();
    if gravity <= 0.0 || h < 1e-3 || speed <= 0.0 {
        return d.normalize_or_zero();
    }
    let s2 = speed * speed;
    let disc = s2 * s2 - gravity * (gravity * h * h + 2.0 * d.y * s2);
    if disc < 0.0 {
        return d.normalize_or_zero();
    }
    let angle = ((s2 - disc.sqrt()) / (gravity * h)).datan();
    let f = flat / h;
    Vec3::new(f.x * angle.dcos(), angle.dsin(), f.y * angle.dcos())
}

/// A missile blow (kind 1): the blow's effect — its `SFXX` record; with
/// none, or one the effect table doesn't hold, nothing flies — leaves the
/// move's node (its offset turned with the critter) as fast as the
/// critter's anger picks from the speed range, aimed as the blow's flags
/// say. It hits with its own radius (`DAMG +0x08`) and is drawn at the
/// record's size; the blow's flag 0x40 takes it through walls and items,
/// 0x1000 past the heroes. Where it stops, its hit record plays: its
/// sound, and its effect for that clip, bursting out to `DAMG +0x0C` (none
/// without the effect).
fn launch(c: &Critter, me: Entity, d: &CritterDamage, damage: f32, level: &mut CritterLevel, commands: &mut Commands) {
    let effect = usize::try_from(d.effects[0]).ok().and_then(|e| Some((e, c.kind.file.sounds.get(e)?, *c.kind.effect_clips.get(&e)?)));
    let Some((e, record, _)) = effect else {
        debug!("critter {me:?}: a missile blow with no effect the table holds: nothing flies");
        return;
    };
    let (s, co) = c.yaw.dsin_cos();
    let o = d.offset;
    let offset = Vec3::new(o[0] * co + o[2] * s, o[1], -o[0] * s + o[2] * co);
    let start = Vec3::from(c.node_at.unwrap_or(c.position)) + offset;
    let t = ((c.anger.clamp(ANGER_BASE, ANGER_CAP) - ANGER_BASE) * SPEED_SPAN).min(1.0);
    let speed = d.speed[0] + t * (d.speed[1] - d.speed[0]);
    if speed <= 0.0 {
        debug!("critter {me:?}: a still effect (not done)");
        return;
    }
    let forward = Vec3::new(s, 0.0, co);
    let target = c.move_target.and_then(|p| c.tracked.iter().find(|t| t.player == p));
    let mut dir = match target {
        Some(t) if d.flags & 1 != 0 => {
            let to = Vec3::from(t.centre);
            if d.flags & 8 == 0 { lob_toward(start, to, speed, d.gravity) } else { (to - start).normalize_or_zero() }
        }
        _ if d.flags & 4 != 0 => forward,
        _ => Vec3::new(forward.x, UNAIMED_DROP, forward.z).normalize_or_zero(),
    };
    if d.kind == 1 {
        let spread = if d.spread > 0.0 { (level.random() - 0.5) * d.spread } else { 0.0 };
        dir = turn_dir(dir, d.yaw + spread, if d.flags & 8 != 0 { d.pitch } else { 0.0 });
    }
    let model = c.kind.effects.get(&e).map(|m| m.as_ref());
    let stop = hit_record(c, d, level.realm);
    info!(
        "critter {me:?} launches a missile: {damage:.0} damage at {speed:.0}/s from {start:?}, radius {:.2}, burst {:.1}",
        d.size, stop.blast
    );
    let e = spawn_critter_missile(
        commands,
        CritterMissile {
            model,
            critter: me,
            start,
            velocity: dir * speed,
            gravity: d.gravity,
            radius: d.size,
            damage,
            kind: d.blow,
            scale: record.size,
            hits_players: d.flags & NO_PLAYERS == 0,
            hits_level: d.flags & NO_LEVEL == 0,
            stop,
            anchor: None,
            life: None,
        },
    );
    // Stand-in glow (half its burst radius): some effect models take their
    // textures from the effects system, which doesn't supply them yet.
    let (mesh, material) = level.glow.clone();
    let glow = Transform::from_scale(Vec3::splat(0.5 * d.radius.max(1.0) / record.size.max(0.01)));
    commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), glow, ChildOf(e)));
}

/// Walks at the move's speed (× the level's monster speed) in its
/// direction, plus the knockback, on the level's collision; stops short of
/// players.
fn walk(c: &mut Critter, m: &CritterMove, ty: &TypeInfo, collision: &LevelCollision, heroes: &[Hero], speed_scale: f32) {
    let s = m.speed * speed_scale * DT;
    let (dx, dz) = move_direction(c.yaw, m.kind, s);
    let mut v = [dx + c.knock[0] * DT, c.knock[1] * DT, dz + c.knock[2] * DT];
    for (i, k) in c.knock.iter_mut().enumerate() {
        *k *= KNOCK_DECAY;
        if k.abs() < KNOCK_STOP {
            *k = 0.0;
        }
        if i == 1 && *k > 0.0 {
            *k = (*k - KNOCK_FALL * DT).max(0.0);
        }
    }
    // Players stop it.
    let from = [c.position[0], c.position[1], c.position[2]];
    let to = [from[0] + v[0], from[1], from[2] + v[2]];
    for h in heroes {
        let reach = ty.radius + h.radius;
        let (ax, az) = (to[0] - h.feet[0], to[2] - h.feet[2]);
        let (bx, bz) = (from[0] - h.feet[0], from[2] - h.feet[2]);
        if ax * ax + az * az < reach * reach && ax * ax + az * az < bx * bx + bz * bz {
            v[0] = 0.0;
            v[2] = 0.0;
        }
    }
    // Walls from its centre, then the floor at the leading edge.
    let feet_y = c.position[1] - ty.hover;
    let start = [c.position[0], feet_y + 2.0, c.position[2]];
    if (v[0] != 0.0 || v[2] != 0.0)
        && let Some(hit) = collision.wall(start, add(start, [v[0], 0.0, v[2]]), ty.radius)
        && collision.nodes[hit.node].flags & node_flags::NO_PUSH == 0
    {
        let mut d = [v[0], 0.0, v[2]];
        if push_out(ty.radius, start, &mut d, hit.point, hit.normal) {
            d = [0.0; 3];
        }
        v[0] = d[0];
        v[2] = d[2];
    }
    let probe = |at: [f32; 3]| collision.floor_probe(at, ty.height, -ty.height - 3.0, 1.0, 2);
    let len = (v[0] * v[0] + v[2] * v[2]).sqrt();
    if len > 0.0 {
        let edge = [start[0] + v[0] / len * (ty.radius + len), feet_y, start[2] + v[2] / len * (ty.radius + len)];
        let mut ok = false;
        if let Some(hit) = probe(edge) {
            let rise = (hit.point[1] - c.floor).abs();
            if rise <= 2.0 * (ty.radius + len) {
                ok = true;
                c.floor = hit.point[1];
                c.ground_node = Some(hit.node);
                if rise > 0.1 * len {
                    match probe([from[0] + v[0], feet_y, from[2] + v[2]]) {
                        Some(h) => {
                            c.floor = h.point[1];
                            c.ground_node = Some(h.node);
                        }
                        None => ok = false,
                    }
                }
            }
        }
        if !ok {
            v[0] = 0.0;
            v[2] = 0.0;
        }
    } else if let Some(h) = probe([from[0], feet_y, from[2]]) {
        c.floor = h.point[1];
        c.ground_node = Some(h.node);
    }
    let dy = (c.floor - feet_y).max(-MAX_DROP * DT) + v[1].max(0.0);
    c.position = [from[0] + v[0], c.position[1] + dy, from[2] + v[2]];
}

/// A move's ground step for a critter facing `yaw`: forward, back, to
/// either side or diagonally.
fn move_direction(yaw: f32, k: i32, s: f32) -> (f32, f32) {
    let f = [yaw.dsin(), yaw.dcos()];
    match k {
        kind::BACK => (-s * f[0], -s * f[1]),
        kind::WALK_RIGHT => (s * f[1], -s * f[0]),
        kind::WALK_LEFT => (-s * f[1], s * f[0]),
        kind::WALK_DIAGONAL => (-s * f[1] + s * f[0], s * f[1] + s * f[0]),
        _ => (s * f[0], s * f[1]),
    }
}

/// A boss walks without the level's collision, kept within its leash of
/// home (a box with the type's flag), at its hover height.
fn walk_leashed(c: &mut Critter, m: &CritterMove, ty: &TypeInfo, speed_scale: f32) {
    if ty.leash <= 0.0 {
        return;
    }
    let (dx, dz) = move_direction(c.yaw, m.kind, m.speed * speed_scale * DT);
    let mut off = [c.position[0] + dx - c.home[0], c.position[2] + dz - c.home[2]];
    let dist = (off[0] * off[0] + off[1] * off[1]).sqrt();
    if ty.flags & TYPE_BOX_LEASH != 0 {
        off = [off[0].clamp(-ty.leash, ty.leash), off[1].clamp(-ty.leash, ty.leash)];
    } else if dist > ty.leash {
        off = [off[0] * ty.leash / dist, off[1] * ty.leash / dist];
    }
    c.position = [c.home[0] + off[0], c.position[1], c.home[2] + off[1]];
}

/// Turns toward the move's target at the move's rate (toward home for
/// moves flagged so); without the free-turning flag, no further than the
/// type allows from the way it was made facing.
fn turn(c: &mut Critter, m: &CritterMove, heroes: &[Hero]) {
    let ty = &c.kind.file.types[c.ty];
    let goal = if m.flags & 0x20 != 0 {
        Some(c.home_yaw)
    } else {
        c.move_target.and_then(|t| heroes.iter().find(|h| h.entity == t)).map(|h| {
            let g = (h.feet[0] - c.position[0]).datan2(h.feet[2] - c.position[2]);
            if ty.flags & TYPE_FREE_TURN != 0 {
                g
            } else {
                let d = locomotion::wrap(g - c.home_yaw).clamp(-ty.max_turn, ty.max_turn);
                c.home_yaw + d
            }
        })
    };
    let Some(goal) = goal else { return };
    let rate = m.turn * DT * if c.stunned > 0.0 { STUNNED_TURN } else { 1.0 };
    let d = locomotion::wrap(goal - c.yaw).clamp(-rate, rate);
    c.yaw = locomotion::wrap(c.yaw + d);
}

/// Places the critters between ticks, drawn at the enemies' scale (the
/// shrink power's).
fn interpolate(between: Res<Between>, enemies: Res<EnemyScale>, mut critters: Query<(&Critter, &mut Transform)>) {
    let t = between.0;
    for (c, mut transform) in &mut critters {
        let (p0, f0) = c.previous;
        transform.translation = Vec3::from(p0).lerp(Vec3::from(c.position), t);
        transform.rotation = Quat::from_rotation_y(f0 + locomotion::wrap(c.yaw - f0) * t);
        transform.scale = Vec3::splat(enemies.0);
    }
}

fn horizontal(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dz) = (a[0] - b[0], a[2] - b[2]);
    (dx * dx + dz * dz).sqrt()
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    length([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(kind: i32, priority: i32, transition: i16, hit_frames: [i32; 2], hit_ends: [i16; 2]) -> CritterMove {
        CritterMove {
            kind,
            flags: 0,
            priority,
            name: String::new(),
            anim: String::new(),
            node: String::new(),
            hit_frames,
            damage: [-1, -1],
            hit_ends,
            next: -1,
            transition,
            sounds: [(-1, -1); 2],
            condition: Condition::default(),
            cooldown: 0.0,
            speed: 0.0,
            turn: 0.0,
            hold: 0.0,
        }
    }

    #[test]
    fn transitions_follow_the_priorities() {
        let walk = mv(kind::WALK, 0x210, 0x14, [-1, -1], [-1, -1]);
        let attack = mv(kind::SWEEP, 0x220, 0x14, [6, -1], [7, -1]);
        let knockback = mv(kind::KNOCKBACK, 0xD00, 0x14, [-1, -1], [-1, -1]);
        let start = mv(kind::START, 0xFFF, 0, [-1, -1], [-1, -1]);
        // Same priority band: wait for the animation to end.
        assert_eq!(transition(&walk, &attack), 1);
        // A higher band cuts in at once.
        assert_eq!(transition(&walk, &knockback), 2);
        // START gives way only when it's done.
        assert_eq!(transition(&start, &walk), 0);
    }

    #[test]
    fn sweeps_land_every_frame_of_their_window_others_once() {
        let sweep = mv(kind::SWEEP, 0x220, 0x14, [6, -1], [7, -1]);
        assert_eq!(blow_bits(&sweep, 5, 0), 0);
        assert_eq!(blow_bits(&sweep, 6, 0), 1);
        assert_eq!(blow_bits(&sweep, 7, 1), 1);
        assert_eq!(blow_bits(&sweep, 8, 1), 0);
        let stomp = mv(0x82, 0x220, 0x14, [10, -1], [-1, -1]);
        assert_eq!(blow_bits(&stomp, 12, 0), 1);
        assert_eq!(blow_bits(&stomp, 13, 1), 0);
        let idle = mv(kind::READY, 0x200, 0x14, [-1, -1], [-1, -1]);
        assert_eq!(blow_bits(&idle, 3, 0), 0);
    }

    #[test]
    fn the_clock_ends_clips_and_loops_them() {
        // Four frames at rate 30 (30 a second): over half a frame after the
        // last one comes up.
        let mut c = Clock { action: 0, frame: 0.0, frames: 1, rate: 30, loops: false, ended: true };
        c.start(1, (4, 30, false));
        assert!(!c.ended);
        for _ in 0..3 {
            c.advance(DT);
        }
        assert!(!c.ended && c.frame == 3.0);
        c.advance(DT);
        assert!(c.ended && c.frame == 3.5);
        // Rate 60 is 15 frames a second: four frames take seven ticks.
        c.start(1, (4, 60, false));
        let mut ticks = 0;
        while !c.ended {
            c.advance(DT);
            ticks += 1;
        }
        assert_eq!(ticks, 7);
        // A loop has ended on the tick it starts over.
        c.start(2, (4, 30, true));
        for _ in 0..3 {
            c.advance(DT);
        }
        assert!(!c.ended);
        c.advance(DT);
        assert!(c.ended && c.frame < 1.0);
        c.advance(DT);
        assert!(!c.ended);
    }

    #[test]
    fn lobs_land_on_the_target() {
        let (from, to) = (Vec3::ZERO, Vec3::new(30.0, -10.0, 0.0));
        let (speed, gravity) = (40.0, 30.0);
        let v = lob_toward(from, to, speed, gravity) * speed;
        // Where it is when it has come 30 across.
        let t = 30.0 / v.x;
        let y = v.y * t - 0.5 * gravity * t * t;
        assert!((y - to.y).abs() < 1e-3, "{y}");
        // Too far to reach: straight at it.
        assert_eq!(lob_toward(from, Vec3::new(1000.0, 0.0, 0.0), 10.0, 30.0), Vec3::X);
    }

    fn level_with(boss_type: i32, state: i32) -> CritterLevel {
        CritterLevel {
            players: 1,
            kinds: HashMap::new(),
            statues: Vec::new(),
            now: 0.0,
            realm: 'B',
            hit_point_scale: 1.0,
            speed_scale: 1.0,
            damage_scale: 1.0,
            enemy_scale: 1.0,
            time_stopped: false,
            guard: HashMap::new(),
            clock: 0.0,
            boss: None,
            boss_dead: false,
            boss_type,
            realm_id: 2,
            intro: state,
            intro_timer: 0.0,
            legendary_used: false,
            light: Darkening::default(),
            boss_spot: [0.0; 3],
            end: EndModels::default(),
            victory: None,
            events: Vec::new(),
            glow: (Handle::default(), Handle::default()),
            rng: 1,
            held: HashMap::new(),
            drops: Vec::new(),
            gargoyle_piece: String::new(),
            made: Vec::new(),
            loot: Vec::new(),
            rock_blows: Vec::new(),
            rock_timers: Vec::new(),
            last_rock: -1,
            area_blasts: Vec::new(),
            shake: false,
            bank_effects: Vec::new(),
            grabs: Vec::new(),
            throws: Vec::new(),
            releases: Vec::new(),
            meters: meter::Meters::default(),
        }
    }

    /// Ticks the level's side of the intro until the state changes (or a
    /// minute passes): the seconds it took.
    fn until_change(l: &mut CritterLevel) -> f32 {
        let (from, start) = (l.intro, l.now);
        while l.intro == from && l.now - start < 60.0 {
            l.now += DT;
            update_intro(l);
        }
        l.now - start
    }

    #[test]
    fn the_intro_waits_then_roars_in_the_dark() {
        let mut l = level_with(DRAGON, intro::WAIT);
        let waited = until_change(&mut l);
        assert_eq!(l.intro, intro::ROAR);
        // The timer is set on the first tick: 3 s and a tick or two.
        assert!(waited > INTRO_WAIT && waited < INTRO_WAIT + 2.5 * DT, "{waited}");
        assert!((l.light_offset() - DARKEN_TO).abs() < 1e-5);
        let mut chimera = level_with(CHIMERA, intro::WAIT);
        let waited = until_change(&mut chimera);
        assert!(waited > INTRO_WAIT_SHORT && waited < INTRO_WAIT_SHORT + 2.5 * DT, "{waited}");
    }

    #[test]
    fn after_the_roar_some_bosses_settle_into_the_fight() {
        let mut l = level_with(DJINN, intro::ROARED);
        until_change(&mut l);
        assert_eq!(l.intro, intro::AFTER);
        let settled = until_change(&mut l);
        assert_eq!(l.intro, intro::FIGHT);
        assert!((settled - INTRO_AFTER).abs() < 0.05, "{settled}");
        // The dragon stays in 5, its light back to normal, until it dies.
        let mut l = level_with(DRAGON, intro::ROARED);
        until_change(&mut l);
        until_change(&mut l);
        assert_eq!(l.intro, intro::AFTER);
        assert_eq!(l.light_offset(), 0.0);
        l.boss_dead = true;
        until_change(&mut l);
        assert_eq!(l.intro, intro::OVER);
    }

    #[test]
    fn missiles_cut_some_intros_short() {
        assert_eq!(missile_reaction(intro::WAIT, DRAGON), Some((intro::ROARED, DRAGON_FREEZE, 0.0)));
        assert_eq!(missile_reaction(intro::ROAR, DJINN), Some((intro::ROARED, 0.0, DJINN_STUN)));
        assert_eq!(missile_reaction(intro::ROAR, PBOSS), Some((intro::ROAR, 0.0, PBOSS_STUN)));
        assert_eq!(missile_reaction(intro::AFTER, CHIMERA), Some((intro::FIGHT, 0.0, 0.0)));
        assert_eq!(missile_reaction(intro::AFTER, DRAGON), None);
        assert_eq!(missile_reaction(intro::START, CHIMERA), None);
    }

    #[test]
    fn the_light_darkens_fast_and_comes_back_slowly() {
        let mut d = Darkening::default();
        let mut now = 0.0;
        for _ in 0..4 {
            now += DT;
            d.ask(now, DARKEN_HOLD, DARKEN_TO);
            d.step(now);
        }
        assert!((d.offset - DARKEN_TO).abs() < 1e-5, "{d:?}");
        let mut ticks = 0;
        while d.offset < 0.0 && ticks < 100 {
            now += DT;
            d.step(now);
            ticks += 1;
        }
        // At most 0.05 a tick back up.
        assert!((16..30).contains(&ticks), "{ticks}");
    }

    #[test]
    fn the_wizard_counts_the_realms_runestones() {
        // Realm B has stone 3; realm G stones 7 and 8.
        assert_eq!(runes_held(2, 0), 0);
        assert_eq!(runes_held(2, 1 << 3), 2);
        assert_eq!(runes_held(7, 1 << 7), 1);
        assert_eq!(runes_held(7, (1 << 7) | (1 << 8) | 1), 3);
        assert_eq!(second_message(DRAGON, runes_held(7, 1 << 8), false), Some("RUNE_PHRASE1"));
        assert_eq!(second_message(SKORNE, 0, true), Some("SKORNE1_RUNE_YES"));
        assert_eq!(second_message(GARM, 0, false), None);
        assert_eq!(wizard_speech(DRAGON, 'B', 0).as_deref(), Some("S_DEFEATVOXB"));
        assert_eq!(wizard_speech(DRAGON, 'B', 3).as_deref(), Some("S_RUNEVOX1B"));
        assert_eq!(wizard_speech(PBOSS, 'K', 3).as_deref(), Some("S_RUNEVOX2K"));
        assert_eq!(wizard_speech(GARM, 'H', 1), None);
    }

    #[test]
    fn the_wizard_stands_between_the_boss_and_the_heroes() {
        let at = wizard_spot([0.0, 50.0, -20.0], &[[10.0, 30.0, 0.0]]);
        assert_eq!(at, [5.0, 30.0, -10.0]);
        // Without heroes, at the boss's spot.
        assert_eq!(wizard_spot([1.0, 2.0, 3.0], &[]), [1.0, 2.0, 3.0]);
        // A page of 60 letters types in 105 ticks (and a little, for its
        // end) and stays a second.
        let t = message_box::caption_seconds(&["x".repeat(60)]);
        assert!((t - (106.0 / 30.0 + 1.0)).abs() < 1e-4, "{t}");
    }

    /// Which sphere blows leave a slot that hurts the first hero it touches,
    /// and how far it reaches (real data).
    #[test]
    fn the_lichs_sphere_blows_leave_touching_slots() {
        let root_dir = std::env::var("GAUNTLET_ASSET_ROOT").unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let read = |name: &str| std::fs::read(std::path::Path::new(&root_dir).join("CRITTER").join(name)).ok().map(|b| CritterFile::parse(&b).unwrap());
        let (Some(lich), Some(dragon)) = (read("LICH.WAD"), read("DRAGON.WAD")) else {
            eprintln!("skipping: no critter files");
            return;
        };
        // The lich's flaming axe (AXEF, 5 out to 4) and its chain's second
        // blow (10 out to 16, its own sphere's radius 0): the only sphere
        // blows on the disc with an effect record.
        assert_eq!(touch_radius(&lich.damage[3], 1.0), Some(4.0));
        assert_eq!(touch_radius(&lich.damage[7], 1.0), Some(16.0));
        assert_eq!(touch_radius(&lich.damage[7], 0.5), Some(8.0));
        assert_eq!((lich.damage[3].damage, lich.damage[7].damage, lich.damage[7].radius), (5.0, 10.0, 0.0));
        // Its plain axe has none; the dragon's breaths (cones, with NULLFX)
        // and its fireball (a missile) don't touch.
        assert_eq!(touch_radius(&lich.damage[2], 1.0), None);
        for d in &dragon.damage {
            assert_eq!(touch_radius(d, 1.0), None, "kind {}", d.kind);
        }
    }

    /// The chimera's body and heads share their wounds (real data).
    #[test]
    fn the_chimeras_heads_and_body_share_their_wounds() {
        let root_dir = std::env::var("GAUNTLET_ASSET_ROOT").unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let path = std::path::Path::new(&root_dir).join("CRITTER/CHIMERA.WAD");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("skipping: no {path:?}");
            return;
        };
        let file = CritterFile::parse(&bytes).unwrap();
        let kind = Arc::new(CritterKind {
            file,
            bodies: Vec::new(),
            statue: None,
            effects: HashMap::new(),
            effect_clips: HashMap::new(),
            folder: String::new(),
            solid_meter: None,
        });
        let level = level_with(CHIMERA, intro::NONE);
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let mut owners = Vec::new();
        let mut c = new_critter(&level, &kind, 0, [0.0; 3], 0.0, root, None, &mut owners, &mut commands);
        for (k, ty) in [1, 2, 3].into_iter().enumerate() {
            c.parts.push(new_critter(&level, &kind, ty, [0.0; 3], 0.0, root, Some(k), &mut owners, &mut commands));
        }
        c.sphere_owner = owners;
        c.state = CritterState::Active;
        for p in &mut c.parts {
            p.state = CritterState::Active;
        }
        // Five body spheres, then one per head.
        assert_eq!(c.sphere_owner.len(), 8);
        assert_eq!(c.sphere_owner[5], (Some(0), 0));
        let mut sounds = Vec::new();
        let hp = |c: &Critter| (c.hit_points, c.parts.iter().map(|p| p.hit_points).collect::<Vec<_>>());

        // A blow on the eagle's head comes off the body too.
        let (body, heads) = hp(&c);
        c.take_hit(100.0, 0, [0.0; 3], Some(5), None, false, None, &mut sounds);
        let (body2, heads2) = hp(&c);
        let dealt = heads[0] - heads2[0];
        assert!(dealt > 0.0 && (body - body2 - dealt).abs() < 1e-3, "{dealt} {body} {body2}");
        assert_eq!(heads[1..], heads2[1..]);

        // A blow on the body: half of it over the three heads.
        c.take_hit(90.0, 0, [0.0; 3], Some(0), None, false, None, &mut sounds);
        let (body3, heads3) = hp(&c);
        let dealt = body2 - body3;
        for k in 0..3 {
            assert!((heads2[k] - heads3[k] - 0.5 * dealt / 3.0).abs() < 1e-3);
        }

        // The killing blow on a head doesn't reach the body.
        c.take_hit(1.0e6, 0, [0.0; 3], Some(5), None, false, None, &mut sounds);
        assert_eq!(c.parts[0].state, CritterState::Dying);
        assert_eq!(c.hit_points, body3);

        // The body's death takes the heads with it.
        c.take_hit(1.0e6, 0, [0.0; 3], Some(0), None, false, None, &mut sounds);
        assert_eq!(c.state, CritterState::Dying);
        assert!(c.parts.iter().all(|p| p.state == CritterState::Dying));
    }

    #[test]
    fn turned_directions_keep_their_length() {
        let d = turn_dir(Vec3::Z, std::f32::consts::FRAC_PI_2, 0.0);
        assert!((d - Vec3::X).length() < 1e-5, "{d}");
        let d = turn_dir(Vec3::Z, 0.0, 0.3);
        assert!((d.length() - 1.0).abs() < 1e-5 && d.y.abs() > 0.2);
    }
}
