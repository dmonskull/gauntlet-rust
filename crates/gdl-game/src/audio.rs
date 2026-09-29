//! The game's own music and sound effects, decoded from the user's copy.
//!
//! Level music is picked the way the game does it (see
//! `docs/audio-format.md`): the level's world-data audio record names a
//! stream; a level starts on track 0 and plays its parts in order, the last
//! one looping. Sound effects are played by catalog name through
//! [`PlaySound`]; `N` steps through the current level's bank.
//!
//! Anything missing or undecodable is logged and skipped — audio never
//! stops the game from running.

use std::collections::HashMap;
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
            .add_message::<LoopSound>()
            .init_resource::<AudioStatus>()
            .add_systems(Startup, load_audio_tables)
            .add_systems(Update, (level_music, audio_keys, play_sounds, loop_sounds).chain());
    }
}

/// Plays a sound effect by its catalog name (`S_WARN`).
#[derive(Message)]
pub struct PlaySound(pub String);

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
        match build_sound(&mut game, &mut tables, name) {
            Ok(effect) => {
                commands.spawn((
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
}
