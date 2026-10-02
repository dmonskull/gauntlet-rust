//! Between levels: the loading screen and the level's movie
//! (`docs/frontend.md`, "Loading screens and movies", has the game's
//! routines and numbers).
//!
//! Going to any level but the tower, the game shows its loading screen
//! (mode `0x400F`) while the level loads behind it: the realm's map
//! (`MAPS/level<code>`), the path to the level drawn a dash at a time, the
//! level's place glowing, then the map's "loading" picture fading in over
//! it with "Loading...". Without a map (the secret realm, the way back
//! from it) `TRANSITION_SCREEN` stands instead, for two seconds at least.
//! The screen ends once the level is in, the voices are done and the map
//! has had its time. Then the level's movie (`LEVL +0x34`; not on the way
//! back from the secret realm) plays — Start or A skips it — and the level
//! starts, its music with it (held until then here, `audio.rs`).
//!
//! The world's level change waits two frames, as the game's load step
//! does (`r13-0x77d8`), so the screen is up before the load stalls.
//! Online every machine would have to wait for the others' screens: the
//! level changes there as before (stand-in). `GDL_SKIP_INTRO=1` (testing)
//! changes levels at once.
//!
//! Not done: the narration the screen queues (two announcer lines from
//! the level's audio record), the dash sound (by realm) and the level's
//! sound bank loading at the screen's step 3.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bevy::asset::RenderAssetUsages;
use bevy::audio::{AddAudioSource, Decodable, Source};
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use gdl_formats::LevelMap;
use gdl_formats::font::FONT32;
use gdl_formats::movie::{Movie, MvdvDecoder};

use crate::audio::{AudioStatus, VoiceQueues};
use crate::font::{Draw2d, Flush2d, GameFonts, UiImage, UiTextures};
use crate::frontend::Frontend;
use crate::level::LoadedGame;
use crate::options::{GameOptions, SoundKind};
use crate::population::LevelPopulation;
use crate::world::ChangeLevel;

/// Frames the level change waits for the screen (`r13-0x77d8`).
const LOAD_WAIT: u32 = 2;
/// The least time the screen stands, fields (`0x78`),
/// and what it waits again while the map is still being drawn (`0x3C`).
const LEAST: f32 = 120.0;
const AGAIN: f32 = 60.0;
/// Fields between the path's dashes.
const DASH_FIELDS: f32 = 30.0;
/// After the path: the glow pulses this long (`r13-0x7FA8`), then fades
/// out as the "loading" picture fades in, 4 a field; the screen can end
/// this long after that began (`r13-0x7FA4`).
const PULSE_FIELDS: f32 = 210.0;
const FADE_STEP: f32 = 4.0;
const AFTER_FADE: f32 = 180.0;
/// The glow's pulse: up over 60 fields (`r13-0x7FB0`), down over 60, 10
/// at rest (`r13-0x7FAC`).
const PULSE_RAMP: f32 = 60.0;
const PULSE_REST: f32 = 10.0;
/// Where the map's four pictures go (as the title's).
const TILES: [(f32, f32); 4] = [(0.0, 0.0), (256.0, 0.0), (0.0, 256.0), (256.0, 256.0)];
/// "Loading..." on the map (`0x154`, `0x140`, scale 1).
const LOADING_AT: (f32, f32) = (340.0, 320.0);
/// The transition screen's size (`0x200` × `0x140`).
const TRANSITION_SIZE: (f32, f32) = (512.0, 320.0);
/// A loading stall shouldn't skip the map's dashes.
const MOST_FIELDS: f32 = 4.0;

pub struct LevelIntroPlugin;

impl Plugin for LevelIntroPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<MovieSound>()
            .init_resource::<LevelIntro>()
            .init_resource::<LevelIntros>()
            .add_systems(Update, step.after(crate::exits::change_level_to).after(crate::frontend::run))
            .add_systems(PostUpdate, draw.before(Flush2d));
    }
}

/// Each level's loading map and movie, by folder (lowercase), from the
/// realm WADs' level records (`exits.rs` reads them).
#[derive(Resource, Default, Clone, Debug)]
pub struct LevelIntros(pub HashMap<String, IntroInfo>);

#[derive(Clone, Debug)]
pub struct IntroInfo {
    /// The level record's name (`A1`): the map pictures' code.
    pub code: String,
    pub map: Option<LevelMap>,
    pub movie: Option<String>,
}

/// The loading screen or movie under way.
#[derive(Resource, Default)]
pub struct LevelIntro {
    phase: Phase,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Loading(Box<Loading>),
    Movie(Box<Playing>),
}

impl LevelIntro {
    /// Whether a loading screen or movie is up (play waits; the level's
    /// music too).
    pub fn active(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// Going to `level` (the world's `change` made once the screen is up):
    /// its screen, then its movie — none of it on the way back from the
    /// secret realm but the plain screen.
    pub fn begin(&mut self, level: &str, info: Option<&IntroInfo>, change: isize, coming_back: bool) {
        let info = info.cloned().unwrap_or(IntroInfo { code: String::new(), map: None, movie: None });
        info!(
            "loading screen for {level}: {}, movie {}",
            if coming_back || info.map.is_none() { "the transition screen".to_string() } else { format!("map {}", info.code) },
            info.movie.as_deref().filter(|_| !coming_back).unwrap_or("none")
        );
        self.phase = Phase::Loading(Box::new(Loading {
            level: level.to_string(),
            change: Some(change),
            wait: LOAD_WAIT,
            map: info.map.filter(|_| !coming_back),
            movie: info.movie.filter(|_| !coming_back),
            code: info.code,
            textures: None,
            fields: 0.0,
            since_path: 0.0,
            dashes: 0,
            hold: LEAST,
            map_done: true,
            glow: 255.0,
            cover: 255.0,
            loading_text: false,
            loaded: false,
            t: 0.0,
        }));
    }
}

/// `GDL_SKIP_INTRO=1` (testing): no loading screens or movies.
pub fn skipped() -> bool {
    std::env::var("GDL_SKIP_INTRO").is_ok_and(|v| v == "1")
}

/// The loading screen (the game's counters by name).
struct Loading {
    level: String,
    /// The world's level change, made after `wait` frames.
    change: Option<isize>,
    wait: u32,
    code: String,
    /// None: the transition screen.
    map: Option<LevelMap>,
    movie: Option<String>,
    /// `MAPS/level<code>`, read as the screen first draws.
    textures: Option<UiTextures>,
    /// Fields since the screen began (`r13-0x77DC`).
    fields: f32,
    /// Fields since the path was complete (`r13-0x77D0`).
    since_path: f32,
    /// Dashes drawn so far.
    dashes: usize,
    /// Fields the screen still stands at least (`r13-0x77E0`).
    hold: f32,
    /// The map has had its time (the game's `bVar2`).
    map_done: bool,
    /// Transparencies, 0 solid to 255 gone: the glow's, and the "loading"
    /// picture's over the map.
    glow: f32,
    cover: f32,
    loading_text: bool,
    /// The level is in.
    loaded: bool,
    /// Fields for the text's pulse.
    t: f32,
}

impl Loading {
    /// One frame of the map (after the screen's step).
    fn animate(&mut self, fields: f32) {
        self.fields += fields;
        self.t += fields;
        let Some(map) = &self.map else { return };
        // A dash every 30 fields from the first frame.
        while self.dashes < map.dashes.len() && self.fields > DASH_FIELDS * self.dashes as f32 {
            self.dashes += 1;
        }
        if map.glow.is_none() || self.dashes < map.dashes.len() {
            self.hold = AGAIN;
            return;
        }
        self.since_path += fields;
        if self.since_path > PULSE_FIELDS {
            let k = self.since_path - PULSE_FIELDS;
            self.map_done = k >= AFTER_FADE;
            let fade = (k * FADE_STEP).clamp(0.0, 255.0);
            self.glow = fade;
            self.cover = 255.0 - fade;
            self.loading_text = fade >= 255.0;
            if !self.loading_text {
                self.hold = AGAIN;
            }
        } else {
            self.map_done = true;
            self.glow = 255.0 - pulse(self.since_path);
            self.hold = AGAIN;
        }
    }

    /// The screen is done: the level in, the voices quiet, the map's time
    /// over (what the screen's update returns).
    fn done(&self, voices_busy: bool) -> bool {
        self.loaded && !voices_busy && self.map_done && self.hold < 1.0
    }
}

/// The glow's pulse at `t` fields after the path: up from 4 to 250 over
/// 60 fields, back down over 60, then 10 at rest.
fn pulse(t: f32) -> f32 {
    let period = PULSE_REST + 2.0 * PULSE_RAMP;
    let mut x = (t as i32) % period as i32;
    if x > 2 * PULSE_RAMP as i32 {
        x = 0;
    } else if x > PULSE_RAMP as i32 {
        x = 2 * PULSE_RAMP as i32 - x;
    }
    ((PULSE_RAMP as i32 + x * 255 - 1) / PULSE_RAMP as i32).clamp(4, 250) as f32
}

/// A sprite's transparency (0 solid … 255 gone) as an alpha: the game's
/// `0x80 − t/2` of `0x80`.
fn alpha(transparency: f32) -> f32 {
    ((128.0 - transparency / 2.0) / 128.0).clamp(0.0, 1.0)
}

/// A movie playing.
struct Playing {
    movie: Movie,
    decoder: MvdvDecoder,
    image: UiImage,
    /// Frames decoded so far.
    decoded: usize,
    /// Seconds since it began.
    seconds: f32,
    audio: Option<Entity>,
    rgba: Vec<u8>,
}

/// What starting a movie needs.
#[derive(SystemParam)]
struct MovieParts<'w> {
    game: ResMut<'w, LoadedGame>,
    images: ResMut<'w, Assets<Image>>,
    sounds: ResMut<'w, Assets<MovieSound>>,
    status: Res<'w, AudioStatus>,
    options: Res<'w, GameOptions>,
}

#[allow(clippy::too_many_arguments)]
fn step(
    mut commands: Commands,
    mut intro: ResMut<LevelIntro>,
    mut fe: ResMut<Frontend>,
    real: Res<Time<Real>>,
    mut change: MessageWriter<ChangeLevel>,
    population: Option<Res<LevelPopulation>>,
    voices: Res<VoiceQueues>,
    mut parts: MovieParts,
) {
    let MovieParts { game, images, sounds, status, options } = &mut parts;
    let fields = real.delta_secs() * 60.0;
    let next = match &mut intro.phase {
        Phase::Idle => return,
        Phase::Loading(l) => {
            fe.show_intro();
            if l.wait > 0 {
                l.wait -= 1;
            } else if let Some(delta) = l.change.take() {
                change.write(ChangeLevel(delta));
            }
            if l.change.is_none() && population.as_ref().is_some_and(|p| p.level.eq_ignore_ascii_case(&l.level)) {
                l.loaded = true;
            }
            l.animate(fields.min(MOST_FIELDS));
            l.hold -= fields.min(MOST_FIELDS);
            if !l.done(voices.busy()) {
                return;
            }
            match l.movie.as_deref().and_then(|name| open_movie(name, game, images)) {
                Some(mut playing) => {
                    playing.audio = start_sound(&mut commands, &playing.movie, sounds, status, options);
                    Phase::Movie(Box::new(playing))
                }
                None => Phase::Idle,
            }
        }
        Phase::Movie(m) => {
            m.seconds += real.delta_secs();
            let want = ((m.seconds * m.movie.rate) as usize).min(m.movie.frame_count());
            // Frames depend on the ones before: decode each in turn (a few
            // at most a frame), showing the last.
            let mut fresh = false;
            for _ in 0..4 {
                if m.decoded >= want {
                    break;
                }
                if let Some(packet) = m.movie.frame(m.decoded)
                    && let Err(why) = m.decoder.decode(packet)
                {
                    warn!("movie frame {}: {why}", m.decoded);
                }
                m.decoded += 1;
                fresh = true;
            }
            if fresh {
                m.decoder.rgba(&mut m.rgba);
                if let Some(image) = images.get_mut(&m.image.handle) {
                    image.data = Some(m.rgba.clone());
                }
            }
            if !(fe.skip_pressed() || m.decoded >= m.movie.frame_count()) {
                return;
            }
            if let Some(e) = m.audio.take() {
                commands.entity(e).try_despawn();
            }
            info!("movie over ({} of {} frames)", m.decoded, m.movie.frame_count());
            Phase::Idle
        }
    };
    if matches!(next, Phase::Idle) {
        fe.end_intro();
    }
    intro.phase = next;
}

/// Reads and readies `VQMOVIES/<name>.avi`.
fn open_movie(name: &str, game: &mut LoadedGame, images: &mut Assets<Image>) -> Option<Playing> {
    let path = format!("VQMOVIES/{name}.avi");
    let movie = match game.install.read(&path).map_err(|e| e.to_string()).and_then(|b| Movie::parse(b).map_err(|e| e.to_string())) {
        Ok(m) => m,
        Err(why) => {
            warn!("movie {path}: {why}");
            return None;
        }
    };
    info!("movie {path}: {} frames at {} a second", movie.frame_count(), movie.rate);
    let (w, h) = (movie.width as u32, movie.height as u32);
    let mut image = Image::new_fill(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    let image = UiImage { handle: images.add(image), size: Vec2::new(w as f32, h as f32) };
    Some(Playing {
        decoder: MvdvDecoder::new(movie.width, movie.height),
        movie,
        image,
        decoded: 0,
        seconds: 0.0,
        audio: None,
        rgba: Vec::new(),
    })
}

/// Plays the movie's sound, at the music's volume.
fn start_sound(
    commands: &mut Commands,
    movie: &Movie,
    sounds: &mut Assets<MovieSound>,
    status: &AudioStatus,
    options: &GameOptions,
) -> Option<Entity> {
    if movie.sound.is_empty() {
        return None;
    }
    let pcm: Arc<[f32]> = match movie.bits {
        16 => movie.sound.as_chunks::<2>().0.iter().map(|&s| f32::from(i16::from_le_bytes(s)) / 32768.0).collect(),
        _ => movie.sound.iter().map(|&s| (f32::from(s) - 128.0) / 128.0).collect(),
    };
    let sound = MovieSound { pcm, rate: movie.sample_rate, channels: movie.channels.max(1) };
    Some(
        commands
            .spawn((
                SoundKind::Music,
                AudioPlayer(sounds.add(sound)),
                PlaybackSettings { muted: status.muted, volume: options.category(SoundKind::Music), ..PlaybackSettings::DESPAWN },
            ))
            .id(),
    )
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut intro: ResMut<LevelIntro>,
    mut draw: ResMut<Draw2d>,
    fonts: Option<Res<GameFonts>>,
    mut front: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut game: ResMut<LoadedGame>,
) {
    match &mut intro.phase {
        Phase::Idle => {}
        Phase::Movie(m) => draw.image(&m.image, 0.0, 0.0, 512.0, 384.0, Color::WHITE),
        Phase::Loading(l) => {
            let Some(map) = l.map.clone() else {
                if let Some(t) = front.as_deref_mut().and_then(|t| t.get("TRANSITION_SCREEN", &mut images)) {
                    draw.image(&t, 0.0, 0.0, TRANSITION_SIZE.0, TRANSITION_SIZE.1, Color::WHITE);
                }
                return;
            };
            let code = l.code.to_ascii_uppercase();
            let tex = l.textures.get_or_insert_with(|| UiTextures::load(&mut game.install, &[&format!("MAPS/level{code}")]));
            // Back to front, by the game's depths: the map (64100), the
            // glow (64080), the dashes (64060), the "loading" picture
            // (63950).
            for (i, &(x, y)) in TILES.iter().enumerate() {
                if let Some(t) = tex.get(&format!("MAP_{code}_{i:02}"), &mut images) {
                    draw.image(&t, x, y, t.size.x, t.size.y, Color::WHITE);
                }
            }
            if let Some([x, y]) = map.glow
                && l.glow < 255.0
                && let Some(t) = tex.get(&format!("MAP_{code}GLOW"), &mut images)
            {
                draw.image(&t, x.trunc(), y.trunc(), t.size.x, t.size.y, Color::srgba(1.0, 1.0, 1.0, alpha(l.glow)));
            }
            for (n, [x, y]) in map.dashes.iter().take(l.dashes).enumerate() {
                if let Some(t) = tex.get(&format!("DASH_{code}_{}", n + 1), &mut images) {
                    draw.image(&t, x.trunc(), y.trunc(), t.size.x, t.size.y, Color::WHITE);
                }
            }
            if l.cover < 255.0 {
                for (i, &(x, y)) in TILES.iter().enumerate() {
                    if let Some(t) = tex.get(&format!("LDMAP_{code}_{i:02}"), &mut images) {
                        draw.image(&t, x, y, t.size.x, t.size.y, Color::srgba(1.0, 1.0, 1.0, alpha(l.cover)));
                    }
                }
            }
            if l.loading_text && let Some(fonts) = fonts.as_deref() {
                let glow = crate::frontend::glow_colour();
                draw.shimmer(fonts, FONT32, 1.0, LOADING_AT.0, LOADING_AT.1, "Loading...", glow, crate::frontend::pulse(l.t));
            }
        }
    }
}

/// A movie's sound: its PCM as floats, at the file's rate.
#[derive(Asset, TypePath)]
pub struct MovieSound {
    pcm: Arc<[f32]>,
    rate: u32,
    channels: u16,
}

impl Decodable for MovieSound {
    type DecoderItem = f32;
    type Decoder = MovieSoundDecoder;

    fn decoder(&self) -> MovieSoundDecoder {
        MovieSoundDecoder { pcm: self.pcm.clone(), rate: self.rate, channels: self.channels, pos: 0 }
    }
}

pub struct MovieSoundDecoder {
    pcm: Arc<[f32]>,
    rate: u32,
    channels: u16,
    pos: usize,
}

impl Iterator for MovieSoundDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let s = self.pcm.get(self.pos).copied()?;
        self.pos += 1;
        Some(s)
    }
}

impl Source for MovieSoundDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.pcm.len() - self.pos)
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(self.pcm.len() as f64 / f64::from(self.channels) / f64::from(self.rate)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(dashes: usize) -> Loading {
        let mut intro = LevelIntro::default();
        let map = LevelMap { glow: Some([10.0, 20.0]), dashes: vec![[1.0, 1.0]; dashes] };
        intro.begin("levelX1", Some(&IntroInfo { code: "X1".into(), map: Some(map), movie: None }), 1, false);
        match intro.phase {
            Phase::Loading(l) => *l,
            _ => unreachable!(),
        }
    }

    #[test]
    fn the_glow_pulses_from_4_to_250_and_rests() {
        assert_eq!(pulse(0.0), 4.0);
        assert_eq!(pulse(60.0), 250.0);
        assert_eq!(pulse(120.0), 4.0);
        assert_eq!(pulse(125.0), 4.0);
        assert_eq!(pulse(130.0), 4.0);
        assert_eq!(pulse(30.0), 128.0);
    }

    #[test]
    fn the_map_draws_a_dash_every_30_fields_then_fades_and_ends_390_fields_after_the_path() {
        let mut l = screen(3);
        l.loaded = true;
        l.animate(1.0);
        assert_eq!(l.dashes, 1);
        for _ in 0..60 {
            l.animate(1.0);
            l.hold -= 1.0;
        }
        assert_eq!(l.dashes, 3, "a dash at fields > 0, > 30, > 60");
        assert!(!l.done(false));
        // The path was complete at field 61; the glow pulses for 210, then
        // the picture fades in over 64 and the screen stands to 180.
        let mut frames = 0;
        while !l.done(false) {
            l.animate(1.0);
            l.hold -= 1.0;
            frames += 1;
            assert!(frames < 1000);
        }
        assert!(l.loading_text);
        assert_eq!(l.cover, 0.0);
        assert!((l.since_path - (PULSE_FIELDS + AFTER_FADE)).abs() <= 1.0, "{}", l.since_path);
    }

    #[test]
    fn without_a_map_the_screen_stands_two_seconds_and_waits_for_the_voices() {
        let mut intro = LevelIntro::default();
        intro.begin("levelS1", None, 1, false);
        let Phase::Loading(mut l) = intro.phase else { unreachable!() };
        l.loaded = true;
        for _ in 0..119 {
            l.animate(1.0);
            l.hold -= 1.0;
        }
        assert!(!l.done(false));
        l.animate(1.0);
        l.hold -= 1.0;
        assert!(l.done(false));
        assert!(!l.done(true), "busy voices hold it");
    }

    #[test]
    fn the_way_back_from_the_secret_realm_has_no_map_or_movie() {
        let mut intro = LevelIntro::default();
        let info = IntroInfo { code: "C2".into(), map: Some(LevelMap { glow: Some([1.0, 1.0]), dashes: vec![] }), movie: Some("movieC2".into()) };
        intro.begin("levelC2", Some(&info), 1, true);
        let Phase::Loading(l) = intro.phase else { unreachable!() };
        assert!(l.map.is_none() && l.movie.is_none());
    }

    #[test]
    fn transparency_is_the_games_half_steps_of_0x80() {
        assert_eq!(alpha(0.0), 1.0);
        assert!(alpha(255.0) < 0.01);
        assert_eq!(alpha(128.0), 0.5);
    }
}
