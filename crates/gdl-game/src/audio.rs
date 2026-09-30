//! The game's own music and sound effects, decoded from the user's copy.
//!
//! Level music is picked the way the game does it (see
//! `docs/audio-format.md`): the level's world-data audio record names a
//! stream; a level starts on track 0 and plays its parts in order, the last
//! one looping. Sound effects are played by catalog name through
//! [`PlaySound`]; `N` steps through the current level's bank.
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
use bevy::prelude::*;
use gdl_formats::audio::{AdsSamples, AdsStream, AudioCatalog, SoundBank};
use gdl_formats::{LevelAudio, WorldData};

use crate::level::LoadedGame;
use crate::options::{GameOptions, SoundKind};
use crate::world::CurrentLevelStats;

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<MusicTrack>()
            .add_audio_source::<SoundEffect>()
            .add_message::<PlaySound>()
            .add_message::<StopSound>()
            .add_message::<LoopSound>()
            .add_message::<QueueVoice>()
            .init_resource::<AudioStatus>()
            .init_resource::<VoiceQueues>()
            .add_systems(Startup, load_audio_tables)
            .add_systems(
                Update,
                (level_music, audio_keys, step_voices, play_sounds, stop_sounds, loop_sounds).chain(),
            );
    }
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

fn play_sounds(
    mut commands: Commands,
    mut requests: MessageReader<PlaySound>,
    mut game: ResMut<LoadedGame>,
    mut tables: ResMut<AudioTables>,
    mut status: ResMut<AudioStatus>,
    mut effects: ResMut<Assets<SoundEffect>>,
    options: Res<GameOptions>,
) {
    for PlaySound(name) in requests.read() {
        let started = std::time::Instant::now();
        let built = build_sound(&mut game, &mut tables, name);
        let took = started.elapsed();
        if took.as_millis() > 2 {
            debug!("sound {name} took {took:?} to build");
        }
        match built {
            Ok(effect) => {
                commands.spawn((
                    EffectName(name.clone()),
                    SoundKind::Effect,
                    AudioPlayer(effects.add(effect)),
                    PlaybackSettings { volume: options.category(SoundKind::Effect), ..PlaybackSettings::DESPAWN },
                ));
                status.last_sound = name.clone();
            }
            Err(e) => {
                warn!("sound {name}: {e}");
                status.last_sound = format!("{name} failed: {e}");
            }
        }
    }
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
    Ok(SoundEffect { segments: segments.into(), loop_from, gain })
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
/// looping from one of them.
#[derive(Asset, TypePath)]
pub struct SoundEffect {
    segments: Arc<[Segment]>,
    loop_from: Option<usize>,
    gain: f32,
}

impl Decodable for SoundEffect {
    type DecoderItem = i16;
    type Decoder = SoundEffectDecoder;

    fn decoder(&self) -> SoundEffectDecoder {
        SoundEffectDecoder { segments: self.segments.clone(), loop_from: self.loop_from, gain: self.gain, segment: 0, pos: 0 }
    }
}

pub struct SoundEffectDecoder {
    segments: Arc<[Segment]>,
    loop_from: Option<usize>,
    gain: f32,
    segment: usize,
    pos: usize,
}

impl Iterator for SoundEffectDecoder {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let seg = self.segments.get(self.segment)?;
        let sample = seg.pcm[self.pos];
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
        Some((sample as f32 * self.gain) as i16)
    }
}

impl Source for SoundEffectDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.segments.get(self.segment).map_or(0, |s| s.pcm.len() - self.pos))
    }
    fn channels(&self) -> u16 {
        1
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
            .map(|(i, &(rate, n))| Segment { sample_rate: rate, pcm: vec![i as i16 + 1; n].into() })
            .collect();
        SoundEffect { segments: segments.into(), loop_from, gain: 1.0 }
    }

    #[test]
    fn effect_plays_segments_in_order_then_stops() {
        let mut d = effect(&[(12000, 2), (18000, 3)], None).decoder();
        assert_eq!((d.sample_rate(), d.current_frame_len()), (12000, Some(2)));
        let first: Vec<i16> = d.by_ref().take(2).collect();
        assert_eq!(first, [1, 1]);
        assert_eq!((d.sample_rate(), d.current_frame_len()), (18000, Some(3)));
        assert_eq!(d.collect::<Vec<_>>(), [2, 2, 2]);
    }

    #[test]
    fn effect_loops_from_its_loop_segment() {
        let d = effect(&[(12000, 1), (12000, 2)], Some(1)).decoder();
        assert_eq!(d.take(7).collect::<Vec<_>>(), [1, 2, 2, 2, 2, 2, 2]);
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
