//! The game's own music and sound effects, decoded from the user's copy.
//!
//! Level music is picked the way the game does it (see
//! `docs/audio-format.md`): the level's world-data audio record names a
//! stream; a level starts on track 0 and plays its parts in order, the last
//! one looping. Sound effects are played by catalog name through
//! [`PlaySound`] (centred, at the call's own volume) or [`PlaySoundAt`]
//! (the game's positional calls: a requested volume, a pan from where the
//! sound is relative to the camera, and a fade with the distance from the
//! heroes); `N` steps through the current level's bank.
//!
//! Voice lines wait their turn in the game's two voice queues
//! ([`QueueVoice`], [`VoiceQueues`]; `docs/frontend.md`, "The voice
//! queues"): the heroes' own lines in one, the announcer's and the
//! wizards' in the other.
//!
//! Anything missing or undecodable is logged and skipped — audio never
//! stops the game from running.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use bevy::audio::{AddAudioSource, Decodable, Source};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use gdl_formats::audio::{AdsSamples, AdsStream, AudioCatalog, SoundBank};
use gdl_formats::{LevelAudio, WorldData};

use crate::frontend::Frontend;
use crate::level::LoadedGame;
use crate::options::{GameOptions, SoundKind};
use crate::play_camera::PlayCamera;
use crate::player::Player;
use crate::player_state::PlayerState;
use crate::world::CurrentLevelStats;

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<MusicTrack>()
            .add_audio_source::<SoundEffect>()
            .add_message::<PlaySound>()
            .add_message::<PlaySoundAt>()
            .add_message::<StopSound>()
            .add_message::<LoopSound>()
            .add_message::<QueueVoice>()
            .init_resource::<AudioStatus>()
            .init_resource::<VoiceQueues>()
            .add_systems(Startup, load_audio_tables)
            .add_systems(
                Update,
                (level_music, audio_keys, step_voices, play_sounds, play_sounds_at, stop_sounds, loop_sounds).chain(),
            );
    }
}

/// Plays a sound effect by catalog name the way the game's own calls do
/// (`docs/audio-format.md`, "Positional sounds"): at `volume`, the call's
/// requested volume ([`CALL_VOLUME`] plays it at its own, `0xE0` louder);
/// panned by where `at` lies from the camera's focus (none: centred); and
/// with `fade`, quieter the further `at` is from the nearest hero in play —
/// full within 20, silent from 70, not played at all then. The pan and the
/// fade are taken once, as it starts.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct PlaySoundAt {
    pub name: String,
    pub at: Option<Vec3>,
    pub volume: u8,
    pub fade: bool,
}

impl PlaySoundAt {
    /// Panned by where it is.
    pub fn panned(name: impl Into<String>, at: Vec3, volume: u8) -> Self {
        Self { name: name.into(), at: Some(at), volume, fade: false }
    }

    /// Panned by where it is and faded by its distance from the heroes.
    pub fn faded(name: impl Into<String>, at: Vec3, volume: u8) -> Self {
        Self { name: name.into(), at: Some(at), volume, fade: true }
    }

    /// Centred, at a requested volume.
    pub fn centred(name: impl Into<String>, volume: u8) -> Self {
        Self { name: name.into(), at: None, volume, fade: false }
    }
}

/// The requested volume that plays a call at its own volume.
pub const CALL_VOLUME: u8 = 127;
/// The pan dead ahead of the camera's focus (and of a sound with no place).
pub const CENTRE_PAN: i32 = 127;
/// Offsets from the focus pan fully to a side from this far.
const PAN_REACH: f32 = 20.0;
/// The fade: full volume, less 1/50 a unit, capped at 1.
const FADE_START: f32 = 1.4;
const FADE_PER_UNIT: f32 = 1.0 / 50.0;
/// The distance the fade takes with no hero in play.
const NO_HERO: f32 = 1000.0;

/// The positional pan (`docs/audio-format.md`, "Positional sounds"),
/// `-256..=255`: with `o` the level offset of `at` from the camera's
/// `focus`, 127.5 + 127.5 × (`o`'s direction · `right`) × min(|`o`| / 20,
/// 1), truncated — 0 full left, 127 dead ahead, 255 full right — and
/// negated when `o` points away from the way the camera faces (behind the
/// focus). `right` is the camera's right, level and of unit length.
pub fn pan(at: Vec3, focus: Vec3, right: Vec3) -> i32 {
    let offset = Vec3::new(at.x - focus.x, 0.0, at.z - focus.z);
    let length = offset.length();
    let unit = if length > 0.0 { offset / length } else { offset };
    let reach = (length / PAN_REACH).min(1.0);
    let mut p = (127.5 * unit.dot(right) * reach + 127.5) as i32;
    if right.x * unit.z < right.z * unit.x {
        p = -p;
    }
    p.clamp(-256, 255)
}

/// The distance fade: 1.4 − `distance` / 50, clamped to 0–1.
pub fn fade(distance: f32) -> f32 {
    (FADE_START - distance * FADE_PER_UNIT).clamp(0.0, 1.0)
}

/// The sound driver's mixer settings for a pan (the pan as an angle, 512
/// to the turn): the side pan, 0 left … 127 right (|pan| / 2), and the
/// surround pan, 0 behind … 127 in front.
fn mix(pan: i32) -> (u32, u32) {
    let side = |shift: i32| ((0x100 - ((pan + shift) & 0x1ff)).unsigned_abs() >> 1).min(127);
    (side(0x100), side(0x180))
}

/// The mixer's pan table, tenths of a dB: a channel `k` steps away from
/// its own side (0 … 127) gets 10·log10((127 − k) / 127) — constant power,
/// −3 dB each at the centre — and −90.4 dB at 127.
fn pan_db(k: u32) -> i32 {
    if k >= 127 {
        return -904;
    }
    (100.0 * (f64::from(127 - k) / 127.0).log10()).round() as i32
}

/// The left and right gains of a pan: the mixer's side pan through its
/// table, relative to the centre's so a centred sound plays as an unpanned
/// one. The surround pan (front or behind) needs a surround decoder and
/// isn't applied here.
pub fn stereo_gains(pan: i32) -> [f32; 2] {
    let (side, _surround) = mix(pan);
    let amp = |db: i32| 10f32.powf(db as f32 / 200.0);
    let centre = amp(pan_db(mix(CENTRE_PAN).0));
    [amp(pan_db(side)) / centre, amp(pan_db(127 - side)) / centre]
}

/// Where the positional sounds are heard from: the camera's focus (the
/// target of its view) and its right, level.
fn ear(camera: &PlayCamera) -> (Vec3, Vec3) {
    let (eye, target) = camera.view();
    let (eye, target) = (Vec3::from(eye), Vec3::from(target));
    let ahead = Vec3::new(target.x - eye.x, 0.0, target.z - eye.z);
    let ahead = if ahead.length_squared() > 1e-8 {
        ahead.normalize()
    } else {
        Vec3::new(camera.yaw().sin(), 0.0, camera.yaw().cos())
    };
    (target, Vec3::new(ahead.z, 0.0, -ahead.x))
}

/// The game's two voice queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceQueue {
    /// The heroes' own lines (eating, hurt cries).
    Heroes = 0,
    /// The announcer's lines — hints, the tower's unlocks — and the tower's
    /// and the bosses' wizards' speeches.
    Announcer = 1,
}

/// Queues a voice line (or a sentence of them) by catalog name: it plays
/// once the lines queued before it are done. The first line is dropped when
/// what is queued ahead would keep it waiting more than `most_wait`
/// seconds (never, for `None`), or when the queue is full; the lines after
/// it wait as long as it takes, and go with it when it's dropped.
#[derive(Message, Clone, Debug)]
pub struct QueueVoice {
    pub queue: VoiceQueue,
    pub lines: Vec<String>,
    pub most_wait: Option<f32>,
    /// Refused once a boss level's end has begun (the announcer's lines;
    /// the wizards' aren't).
    pub gated: bool,
}

impl QueueVoice {
    /// An announcer line.
    pub fn announcer(line: impl Into<String>, most_wait: f32) -> Self {
        Self { queue: VoiceQueue::Announcer, lines: vec![line.into()], most_wait: Some(most_wait), gated: false }
    }

    /// A hero's own line: dropped when it would wait more than a second.
    pub fn hero(line: impl Into<String>) -> Self {
        Self { queue: VoiceQueue::Heroes, lines: vec![line.into()], most_wait: Some(HERO_MOST_WAIT), gated: false }
    }

    /// A line said right after the last.
    pub fn then(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    /// Refused once a boss level's end has begun.
    pub fn gated(mut self) -> Self {
        self.gated = true;
        self
    }
}

/// The heroes' lines' longest wait, seconds.
const HERO_MOST_WAIT: f32 = 1.0;
/// Lines a queue holds.
const QUEUE_LINES: usize = 16;
/// The voice queues count fields, 60 a second of real time.
const FIELDS_PER_SECOND: f32 = 60.0;

/// A queued line: its sound and how long it holds the queue, in fields.
#[derive(Debug)]
struct Line {
    name: String,
    fields: f32,
}

#[derive(Default)]
struct Queue {
    lines: VecDeque<Line>,
    /// When the first line is done, once it has started.
    ends: Option<f32>,
}

/// The voice queues: each plays its first line when that line's turn
/// comes and takes it off `length` fields later, the length being the
/// sound's catalog length. They run on real time (the game's field
/// counter is a clock), so they go on under the message box and menus.
#[derive(Resource, Default)]
pub struct VoiceQueues {
    queues: [Queue; 2],
    /// Fields of real time.
    now: f32,
    /// A boss level's end has begun: the announcer's lines are refused.
    closed: bool,
}

impl VoiceQueues {
    /// Whether either queue holds a line (playing or waiting). A level
    /// doesn't end while one does.
    pub fn busy(&self) -> bool {
        self.queues.iter().any(|q| !q.lines.is_empty())
    }

    /// Refuses the announcer's lines from now until the next level starts
    /// (a boss level's end, from the wizard's appearance).
    pub fn close_announcer(&mut self) {
        self.closed = true;
    }

    /// Appends a line `fields` long; `false` when it's dropped: the queue
    /// is full, or what's ahead of it runs more than `most_wait` seconds.
    fn append(&mut self, queue: VoiceQueue, name: &str, fields: f32, most_wait: Option<f32>) -> bool {
        let now = self.now;
        let q = &mut self.queues[queue as usize];
        if q.lines.len() >= QUEUE_LINES {
            return false;
        }
        // When it would start: after the first line (from now, if that
        // hasn't started) and all the others.
        let starts = match q.lines.front() {
            None => now,
            Some(first) => q.ends.unwrap_or(now + first.fields) + q.lines.iter().skip(1).map(|l| l.fields).sum::<f32>(),
        };
        if most_wait.is_some_and(|w| w * FIELDS_PER_SECOND < starts - now) {
            return false;
        }
        q.lines.push_back(Line { name: name.into(), fields });
        true
    }

    /// One step at field `now`: a queue whose first line hasn't started
    /// starts it; one whose first line is done takes it off (the next
    /// starts on the following step). The lines to play now.
    fn step(&mut self, now: f32) -> Vec<String> {
        self.now = now;
        let mut start = Vec::new();
        for q in &mut self.queues {
            let Some(first) = q.lines.front() else { continue };
            match q.ends {
                None => {
                    q.ends = Some(now + first.fields);
                    start.push(first.name.clone());
                }
                Some(ends) if ends <= now => {
                    q.lines.pop_front();
                    q.ends = None;
                }
                Some(_) => {}
            }
        }
        start
    }
}

/// Queues the lines asked for and steps the queues, starting the lines
/// whose turn has come.
fn step_voices(
    real: Res<Time<Real>>,
    stats: Option<Res<CurrentLevelStats>>,
    tables: Res<AudioTables>,
    mut voices: ResMut<VoiceQueues>,
    mut requests: MessageReader<QueueVoice>,
    mut play: MessageWriter<PlaySound>,
) {
    // A new level opens the announcer's queue again.
    if stats.is_some_and(|s| s.is_changed()) {
        voices.closed = false;
    }
    let now = voices.now + real.delta_secs() * FIELDS_PER_SECOND;
    voices.now = now;
    for QueueVoice { queue, lines, most_wait, gated } in requests.read() {
        if *gated && voices.closed {
            info!("voice {lines:?} refused: the level's end has begun");
            continue;
        }
        for (i, name) in lines.iter().enumerate() {
            // A line lasts its sound's catalog length (a looping sound's is
            // negative: it's taken off at once); a sound the catalog
            // doesn't have, none.
            let length = tables.catalog.as_ref().and_then(|c| c.find_sound(name)).map_or(0.0, |s| s.length);
            let wait = if i == 0 { *most_wait } else { None };
            if !voices.append(*queue, name, length * FIELDS_PER_SECOND, wait) {
                info!("voice {name} dropped: the {queue:?} queue is too long");
                break;
            }
        }
    }
    for name in voices.step(now) {
        info!("voice line {name} starts");
        play.write(PlaySound(name));
    }
}

/// Plays a sound effect by its catalog name (`S_WARN`).
#[derive(Message)]
pub struct PlaySound(pub String);

/// Stops every sound effect of that name still playing (a message box
/// cutting its voice line short, `docs/frontend.md`).
#[derive(Message)]
pub struct StopSound(pub String);

/// A playing sound effect's name (it's gone once the sound ends).
#[derive(Component)]
pub struct EffectName(pub String);

/// Starts (`Some`) or stops (`None`) the looping sound on channel `key`:
/// one loop per channel, left alone when asked for the one it's playing
/// (a lift's rumble while it moves).
#[derive(Message)]
pub struct LoopSound {
    pub key: &'static str,
    pub name: Option<String>,
}

#[derive(Component)]
struct LoopChannel(&'static str, String);

/// What's playing, for the HUD.
#[derive(Resource, Default)]
pub struct AudioStatus {
    pub music: String,
    pub last_sound: String,
    pub muted: bool,
}

/// The catalog and each level's audio record, read once at startup.
#[derive(Resource, Default)]
struct AudioTables {
    catalog: Option<AudioCatalog>,
    /// Lower-case level folder (`levela1`) → its audio record.
    levels: HashMap<String, LevelAudio>,
    banks: HashMap<usize, Arc<SoundBank>>,
    /// Next sound `N` plays from the current level's bank.
    next_sound: usize,
}

#[derive(Component)]
struct LevelMusic;

fn load_audio_tables(mut commands: Commands, mut game: ResMut<LoadedGame>) {
    let mut tables = AudioTables::default();
    match game.install.read("AUDIO/AUDATPS2.ROM").map_err(|e| e.to_string()).and_then(|b| {
        AudioCatalog::parse(&b).map_err(|e| e.to_string())
    }) {
        Ok(catalog) => tables.catalog = Some(catalog),
        Err(e) => warn!("no sound catalog, sound effects disabled: {e}"),
    }
    let wads: Vec<String> = game
        .install
        .files()
        .iter()
        .filter(|f| {
            let f = f.to_ascii_uppercase();
            f.starts_with("WDATA/") && f.ends_with(".WAD")
        })
        .cloned()
        .collect();
    for path in wads {
        let parsed = game.install.read(&path).map_err(|e| e.to_string()).and_then(|b| {
            WorldData::parse(&b).map_err(|e| e.to_string())
        });
        match parsed {
            Ok(world) => {
                for level in &world.levels {
                    tables.levels.insert(level.folder().to_ascii_lowercase(), world.audio[level.audio].clone());
                }
            }
            Err(e) => warn!("{path}: {e}"),
        }
    }
    info!("audio: {} levels with music", tables.levels.len());
    commands.insert_resource(tables);
}

/// Starts the new level's music whenever the level changes.
#[allow(clippy::too_many_arguments)]
fn level_music(
    mut commands: Commands,
    stats: Option<Res<CurrentLevelStats>>,
    mut game: ResMut<LoadedGame>,
    mut tables: ResMut<AudioTables>,
    mut status: ResMut<AudioStatus>,
    mut tracks: ResMut<Assets<MusicTrack>>,
    playing: Query<Entity, With<LevelMusic>>,
    options: Res<GameOptions>,
) {
    let Some(stats) = stats else { return };
    if !stats.is_changed() {
        return;
    }
    for e in &playing {
        commands.entity(e).despawn();
    }
    tables.next_sound = 0;
    status.music = match tables.levels.get(&stats.name.to_ascii_lowercase()) {
        None => "none for this level".into(),
        Some(audio) => match load_track(&mut game, audio, 0) {
            Ok((track, name)) => {
                commands.spawn((
                    LevelMusic,
                    SoundKind::Music,
                    AudioPlayer(tracks.add(track)),
                    PlaybackSettings {
                        muted: status.muted,
                        volume: options.category(SoundKind::Music),
                        ..PlaybackSettings::DESPAWN
                    },
                ));
                name
            }
            Err(e) => {
                warn!("level {} music: {e}", stats.name);
                format!("failed: {e}")
            }
        },
    };
}

/// Reads and parses every part of one track of a level's music.
fn load_track(game: &mut LoadedGame, audio: &LevelAudio, track: usize) -> Result<(MusicTrack, String), String> {
    let mut parts = Vec::new();
    let mut names = Vec::new();
    for part in 0..audio.part_count(track) {
        let path = audio.stream_path(track, part);
        let bytes = game.install.read(&path).map_err(|e| format!("{path}: {e}"))?;
        let stream = AdsStream::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        if stream.num_samples == 0 {
            return Err(format!("{path} is empty"));
        }
        names.push(path.trim_start_matches("STREAMS/").to_string());
        parts.push(Arc::new(stream));
    }
    Ok((MusicTrack { parts }, names.join(" then ")))
}

fn audio_keys(
    keys: Res<ButtonInput<KeyCode>>,
    stats: Option<Res<CurrentLevelStats>>,
    mut tables: ResMut<AudioTables>,
    mut status: ResMut<AudioStatus>,
    mut sinks: Query<&mut AudioSink, With<LevelMusic>>,
    mut play: MessageWriter<PlaySound>,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        status.muted = !status.muted;
        for mut sink in &mut sinks {
            if status.muted { sink.mute() } else { sink.unmute() }
        }
    }
    if keys.just_pressed(KeyCode::KeyN) {
        let Some(stats) = stats else { return };
        let level_bank = tables.levels.get(&stats.name.to_ascii_lowercase()).map(|a| a.bank.clone());
        let Some(catalog) = &tables.catalog else { return };
        let bank = level_bank.and_then(|b| catalog.find_bank(&b)).or_else(|| catalog.find_bank("COMMON"));
        let Some(bank) = bank else { return };
        let sounds = catalog.bank_sounds(bank);
        if sounds.is_empty() {
            return;
        }
        let name = sounds[tables.next_sound % sounds.len()].name.clone();
        tables.next_sound += 1;
        play.write(PlaySound(name));
    }
}

/// What starting a sound effect takes.
#[derive(SystemParam)]
struct Effects<'w, 's> {
    commands: Commands<'w, 's>,
    game: ResMut<'w, LoadedGame>,
    tables: ResMut<'w, AudioTables>,
    status: ResMut<'w, AudioStatus>,
    assets: ResMut<'w, Assets<SoundEffect>>,
    options: Res<'w, GameOptions>,
}

impl Effects<'_, '_> {
    /// Builds and starts a sound effect: centred at its call's volume, or
    /// as a positional call asks.
    fn start(&mut self, name: &str, look: Option<EffectLook>) {
        let started = std::time::Instant::now();
        let built = build_sound(&mut self.game, &mut self.tables, name);
        let took = started.elapsed();
        if took.as_millis() > 2 {
            debug!("sound {name} took {took:?} to build");
        }
        match built {
            Ok(mut effect) => {
                if let Some(look) = look {
                    effect.gain *= f32::from(look.volume) / f32::from(CALL_VOLUME);
                    effect.stereo = look.stereo;
                }
                self.commands.spawn((
                    EffectName(name.to_string()),
                    SoundKind::Effect,
                    AudioPlayer(self.assets.add(effect)),
                    PlaybackSettings { volume: self.options.category(SoundKind::Effect), ..PlaybackSettings::DESPAWN },
                ));
                self.status.last_sound = name.to_string();
            }
            Err(e) => {
                warn!("sound {name}: {e}");
                self.status.last_sound = format!("{name} failed: {e}");
            }
        }
    }
}

fn play_sounds(mut requests: MessageReader<PlaySound>, mut effects: Effects) {
    for PlaySound(name) in requests.read() {
        effects.start(name, None);
    }
}

/// The game's positional calls: the fade (skipping what it silences) and
/// the pan from the heroes and the camera as the sound starts.
fn play_sounds_at(
    mut requests: MessageReader<PlaySoundAt>,
    mut effects: Effects,
    camera: Option<Res<PlayCamera>>,
    heroes: Query<&Player>,
    state: Option<Res<PlayerState>>,
    frontend: Option<Res<Frontend>>,
) {
    if requests.is_empty() {
        return;
    }
    let ear = camera.as_deref().map(ear);
    // The heroes in play (not dead, not out of the level): their feet.
    let in_play = state.is_some_and(|s| s.alive) && !frontend.is_some_and(|f| f.hero_out());
    let feet: Vec<Vec3> = if in_play { heroes.iter().map(|p| Vec3::from(p.mover.position)).collect() } else { Vec::new() };
    for r in requests.read() {
        let volume = match r.at.filter(|_| r.fade) {
            Some(at) => {
                let nearest = feet.iter().map(|f| f.distance(at)).fold(NO_HERO, f32::min);
                let f = fade(nearest);
                if f <= 0.0 {
                    debug!("sound {} faded out ({nearest:.0} from the heroes)", r.name);
                    continue;
                }
                (f32::from(r.volume) * f) as u8
            }
            None => r.volume,
        };
        let pan = match (r.at, ear) {
            (Some(at), Some((focus, right))) => pan(at, focus, right),
            _ => CENTRE_PAN,
        };
        let look = EffectLook { volume, stereo: (pan != CENTRE_PAN).then(|| stereo_gains(pan)) };
        debug!("sound {} at {:?}: volume {volume}, pan {pan}", r.name, r.at);
        effects.start(&r.name, Some(look));
    }
}

/// How a positional call plays: its requested volume and its left and
/// right gains (none: centred).
#[derive(Clone, Copy, Debug, PartialEq)]
struct EffectLook {
    volume: u8,
    stereo: Option<[f32; 2]>,
}

fn stop_sounds(mut commands: Commands, mut requests: MessageReader<StopSound>, playing: Query<(Entity, &EffectName)>) {
    for StopSound(name) in requests.read() {
        for (e, _) in playing.iter().filter(|(_, n)| &n.0 == name) {
            commands.entity(e).try_despawn();
        }
    }
}

fn loop_sounds(
    mut commands: Commands,
    mut requests: MessageReader<LoopSound>,
    mut game: ResMut<LoadedGame>,
    mut tables: ResMut<AudioTables>,
    mut effects: ResMut<Assets<SoundEffect>>,
    options: Res<GameOptions>,
    playing: Query<(Entity, &LoopChannel)>,
) {
    for LoopSound { key, name } in requests.read() {
        let current = playing.iter().find(|(_, c)| c.0 == *key);
        if current.is_some_and(|(_, c)| Some(&c.1) == name.as_ref()) {
            continue;
        }
        if let Some((e, _)) = current {
            commands.entity(e).try_despawn();
        }
        let Some(name) = name else { continue };
        match build_sound(&mut game, &mut tables, name) {
            Ok(effect) => {
                commands.spawn((
                    LoopChannel(key, name.clone()),
                    SoundKind::Effect,
                    AudioPlayer(effects.add(effect)),
                    PlaybackSettings { volume: options.category(SoundKind::Effect), ..PlaybackSettings::DESPAWN },
                ));
            }
            Err(e) => warn!("sound {name}: {e}"),
        }
    }
}

/// Decodes a catalog sound's call into PCM segments.
fn build_sound(game: &mut LoadedGame, tables: &mut AudioTables, name: &str) -> Result<SoundEffect, String> {
    let catalog = tables.catalog.as_ref().ok_or("no sound catalog")?;
    let sound = catalog.find_sound(name).ok_or("no such sound")?;
    let (bank_index, call) = (sound.bank, sound.call);
    let bank = match tables.banks.get(&bank_index) {
        Some(bank) => bank.clone(),
        None => {
            let path = catalog.banks[bank_index].path();
            let bytes = game.install.read(&path).map_err(|e| format!("{path}: {e}"))?;
            let bank = Arc::new(SoundBank::parse(&bytes).map_err(|e| format!("{path}: {e}"))?);
            tables.banks.insert(bank_index, bank.clone());
            bank
        }
    };
    let call = bank.calls.get(call).ok_or("call missing from bank")?;
    let sequence = call.sequence().map_err(|e| e.to_string())?;
    let segment = |i: &usize| {
        let s = &bank.samples[*i];
        Segment { sample_rate: s.sample_rate, pcm: s.decode().into() }
    };
    let mut segments: Vec<Segment> = sequence.intro.iter().map(segment).collect();
    let loop_from = (!sequence.looped.is_empty()).then_some(segments.len());
    segments.extend(sequence.looped.iter().map(segment));
    segments.retain(|s| !s.pcm.is_empty() && s.sample_rate > 0);
    if segments.is_empty() {
        return Err("call has no audio".into());
    }
    let loop_from = loop_from.filter(|&l| l < segments.len());
    // Volume is 0..127 of full scale.
    let gain = call.volume.min(127) as f32 / 127.0;
    Ok(SoundEffect { segments: segments.into(), loop_from, gain, stereo: None })
}

/// A level's music: its parts in order, the last one looping forever.
#[derive(Asset, TypePath)]
pub struct MusicTrack {
    parts: Vec<Arc<AdsStream>>,
}

impl Decodable for MusicTrack {
    type DecoderItem = i16;
    type Decoder = MusicDecoder;

    fn decoder(&self) -> MusicDecoder {
        MusicDecoder { parts: self.parts.clone(), part: 0, samples: AdsStream::samples(self.parts[0].clone()) }
    }
}

pub struct MusicDecoder {
    parts: Vec<Arc<AdsStream>>,
    part: usize,
    samples: AdsSamples<Arc<AdsStream>>,
}

impl Iterator for MusicDecoder {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let sample = self.samples.next()?;
        // Move to the next part (or back to the start of the last one) as
        // soon as this one is used up, so `current_frame_len` is never 0.
        if self.samples.remaining() == 0 {
            self.part = (self.part + 1).min(self.parts.len() - 1);
            self.samples = AdsStream::samples(self.parts[self.part].clone());
        }
        Some(sample)
    }
}

impl Source for MusicDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.samples.remaining())
    }
    fn channels(&self) -> u16 {
        self.samples.stream().channel_count() as u16
    }
    fn sample_rate(&self) -> u32 {
        self.samples.stream().sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct Segment {
    sample_rate: u32,
    pcm: Arc<[i16]>,
}

/// A decoded sound-effect call: mono segments played in order, optionally
/// looping from one of them — centred (one channel), or panned (two, with
/// these left and right gains). Samples are floats, so a call asked for
/// louder than its own volume isn't clipped here.
#[derive(Asset, TypePath)]
pub struct SoundEffect {
    segments: Arc<[Segment]>,
    loop_from: Option<usize>,
    gain: f32,
    stereo: Option<[f32; 2]>,
}

impl Decodable for SoundEffect {
    type DecoderItem = f32;
    type Decoder = SoundEffectDecoder;

    fn decoder(&self) -> SoundEffectDecoder {
        SoundEffectDecoder {
            segments: self.segments.clone(),
            loop_from: self.loop_from,
            gain: self.gain,
            stereo: self.stereo,
            segment: 0,
            pos: 0,
            right_next: false,
        }
    }
}

pub struct SoundEffectDecoder {
    segments: Arc<[Segment]>,
    loop_from: Option<usize>,
    gain: f32,
    stereo: Option<[f32; 2]>,
    segment: usize,
    pos: usize,
    /// Panned: the current sample's right channel comes next.
    right_next: bool,
}

impl SoundEffectDecoder {
    fn advance(&mut self) {
        let Some(seg) = self.segments.get(self.segment) else { return };
        self.pos += 1;
        if self.pos == seg.pcm.len() {
            self.pos = 0;
            self.segment += 1;
            if self.segment == self.segments.len()
                && let Some(l) = self.loop_from
            {
                self.segment = l;
            }
        }
    }
}

impl Iterator for SoundEffectDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let seg = self.segments.get(self.segment)?;
        let sample = f32::from(seg.pcm[self.pos]) / 32768.0 * self.gain;
        let out = match self.stereo {
            Some([left, _]) if !self.right_next => {
                self.right_next = true;
                return Some(sample * left);
            }
            Some([_, right]) => {
                self.right_next = false;
                sample * right
            }
            None => sample,
        };
        self.advance();
        Some(out)
    }
}

impl Source for SoundEffectDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        let left = self.segments.get(self.segment).map_or(0, |s| s.pcm.len() - self.pos);
        Some(match self.stereo {
            Some(_) => left * 2 - usize::from(self.right_next),
            None => left,
        })
    }
    fn channels(&self) -> u16 {
        if self.stereo.is_some() { 2 } else { 1 }
    }
    fn sample_rate(&self) -> u32 {
        self.segments.get(self.segment).map_or(22050, |s| s.sample_rate)
    }
    fn total_duration(&self) -> Option<Duration> {
        if self.loop_from.is_some() {
            return None;
        }
        let secs: f64 = self.segments.iter().map(|s| s.pcm.len() as f64 / s.sample_rate as f64).sum();
        Some(Duration::from_secs_f64(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(lens: &[(u32, usize)], loop_from: Option<usize>) -> SoundEffect {
        let segments: Vec<Segment> = lens
            .iter()
            .enumerate()
            .map(|(i, &(rate, n))| Segment { sample_rate: rate, pcm: vec![(i as i16 + 1) * 8192; n].into() })
            .collect();
        SoundEffect { segments: segments.into(), loop_from, gain: 1.0, stereo: None }
    }

    /// Segment n's samples come out as (n + 1) / 4 of full scale.
    fn quarters(samples: impl Iterator<Item = f32>) -> Vec<f32> {
        samples.map(|s| s * 4.0).collect()
    }

    #[test]
    fn effect_plays_segments_in_order_then_stops() {
        let mut d = effect(&[(12000, 2), (18000, 3)], None).decoder();
        assert_eq!((d.sample_rate(), d.current_frame_len(), d.channels()), (12000, Some(2), 1));
        assert_eq!(quarters(d.by_ref().take(2)), [1.0, 1.0]);
        assert_eq!((d.sample_rate(), d.current_frame_len()), (18000, Some(3)));
        assert_eq!(quarters(d), [2.0, 2.0, 2.0]);
    }

    #[test]
    fn effect_loops_from_its_loop_segment() {
        let d = effect(&[(12000, 1), (12000, 2)], Some(1)).decoder();
        assert_eq!(quarters(d.take(7)), [1.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn a_panned_effect_plays_both_channels_by_their_gains() {
        let mut e = effect(&[(12000, 2), (18000, 1)], None);
        e.stereo = Some([0.5, 2.0]);
        e.gain = 2.0;
        let mut d = e.decoder();
        assert_eq!((d.channels(), d.current_frame_len()), (2, Some(4)));
        assert_eq!(quarters(d.by_ref().take(1)), [1.0]);
        // Mid-frame: the right channel is still to come, at the same rate.
        assert_eq!((d.sample_rate(), d.current_frame_len()), (12000, Some(3)));
        assert_eq!(quarters(d.by_ref().take(3)), [4.0, 1.0, 4.0]);
        assert_eq!((d.sample_rate(), d.current_frame_len()), (18000, Some(2)));
        assert_eq!(quarters(d), [2.0, 8.0]);
    }

    #[test]
    fn the_fade_by_distance() {
        assert_eq!(fade(0.0), 1.0);
        assert_eq!(fade(20.0), 1.0);
        assert!((fade(45.0) - 0.5).abs() < 1e-6);
        assert_eq!(fade(70.0), 0.0);
        assert_eq!(fade(NO_HERO), 0.0);
    }

    #[test]
    fn the_pan_from_the_camera() {
        let (focus, right) = (Vec3::new(10.0, 0.0, 5.0), Vec3::X);
        // The camera faces +Z here (right = (ahead.z, 0, -ahead.x)).
        let at = |x: f32, z: f32| pan(focus + Vec3::new(x, 3.0, z), focus, right);
        assert_eq!(at(0.0, 0.0), 127);
        assert_eq!(at(0.0, 30.0), 127);
        assert_eq!(at(30.0, 0.0), 255);
        assert_eq!(at(-30.0, 0.0), 0);
        assert_eq!(at(21.0, 21.0), 217);
        // Within 20 of the focus the pan narrows.
        assert_eq!(at(10.0, 0.0), 191);
        // Behind the focus: negated.
        assert_eq!(at(0.0, -30.0), -127);
        assert_eq!(at(21.0, -21.0), -217);
    }

    #[test]
    fn the_mixer_pans_by_angle() {
        // Side pan |pan| / 2; surround pan 127 ahead, 0 behind.
        assert_eq!(mix(127), (63, 127));
        assert_eq!(mix(0), (0, 64));
        assert_eq!(mix(255), (127, 64));
        assert_eq!(mix(-127), (63, 0));
        assert_eq!(mix(-217), (108, 44));
        assert_eq!(mix(-256), (127, 64));
    }

    /// The mixer's table, as the game has it (spot checks).
    #[test]
    fn the_pan_table() {
        let table: Vec<i32> = (0..128).map(pan_db).collect();
        assert_eq!(table[..8], [0, 0, -1, -1, -1, -2, -2, -2]);
        assert_eq!(table[60..68], [-28, -28, -29, -30, -30, -31, -32, -33]);
        assert_eq!(table[122..], [-140, -150, -163, -180, -210, -904]);
    }

    #[test]
    fn stereo_gains_keep_the_power() {
        let [l, r] = stereo_gains(CENTRE_PAN);
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 0.01, "{l} {r}");
        let [l, r] = stereo_gains(255);
        assert!(l < 0.001 && (r - std::f32::consts::SQRT_2).abs() < 0.01, "{l} {r}");
        let [l, r] = stereo_gains(0);
        assert!((l - std::f32::consts::SQRT_2).abs() < 0.01 && r < 0.001, "{l} {r}");
        // Behind the camera it pans by the same side pan.
        assert_eq!(stereo_gains(-255), stereo_gains(255));
        for p in [-200, -50, 0, 40, 127, 200, 255] {
            let [l, r] = stereo_gains(p);
            assert!((l * l + r * r - 2.0).abs() < 0.1, "{p}: {l} {r}");
        }
    }

    #[test]
    fn voice_lines_play_one_after_another() {
        use VoiceQueue::Announcer;
        let mut v = VoiceQueues::default();
        assert!(v.append(Announcer, "A", 60.0, None));
        assert!(v.append(Announcer, "B", 30.0, None));
        assert!(v.busy());
        assert_eq!(v.step(0.0), ["A"]);
        assert!(v.step(59.0).is_empty());
        // A is taken off at its end; B starts on the next step.
        assert!(v.step(60.0).is_empty());
        assert_eq!(v.step(61.0), ["B"]);
        assert!(v.step(91.0).is_empty());
        assert!(!v.busy());
    }

    #[test]
    fn voice_queues_run_side_by_side() {
        let mut v = VoiceQueues::default();
        v.append(VoiceQueue::Heroes, "H", 10.0, None);
        v.append(VoiceQueue::Announcer, "A", 10.0, None);
        assert_eq!(v.step(0.0), ["H", "A"]);
    }

    #[test]
    fn voice_line_dropped_when_it_would_wait_too_long() {
        use VoiceQueue::Announcer;
        let mut v = VoiceQueues::default();
        v.append(Announcer, "A", 120.0, None);
        // 2 s ahead of it: a half-second line is dropped, a 10 s one isn't.
        assert!(!v.append(Announcer, "B", 30.0, Some(0.5)));
        assert!(v.append(Announcer, "C", 30.0, Some(10.0)));
        // Once A has played a second: its last second and C's half wait.
        v.step(0.0);
        v.now = 60.0;
        assert!(!v.append(Announcer, "D", 30.0, Some(1.4)));
        assert!(v.append(Announcer, "E", 30.0, Some(1.5)));
        // Nothing ahead: never dropped.
        let mut empty = VoiceQueues::default();
        assert!(empty.append(Announcer, "F", 30.0, Some(0.0)));
    }

    #[test]
    fn voice_queue_holds_sixteen_lines() {
        let mut v = VoiceQueues::default();
        for _ in 0..QUEUE_LINES {
            assert!(v.append(VoiceQueue::Heroes, "L", 1.0, None));
        }
        assert!(!v.append(VoiceQueue::Heroes, "L", 1.0, None));
    }
}
