//! `WDATA/*.WAD` — per-realm world data. Parsed so far: each level's name,
//! camera record and audio record; the camera records; the audio records
//! (sound bank + music stream). The container is `chunk.rs`.
//!
//! Confirmed against the world-data loader and the level-music code in
//! `main.dol` (see `docs/audio-format.md`). Little-endian.
//!
//! ```text
//! 0x00  u32 directory offset, u32 chunk count
//! dir   16 bytes per chunk: u32 tag ('WRLD', 'LEVL', 'AUDS', …),
//!       u32 offset, u32 record count, u32
//! LEVL  0x10C bytes per level: +0x08 name ("A1" → LEVELS/levelA1),
//!       +0x58 i16 camera record, +0x5A i16 audio record
//! CAMS  0x6C bytes per record: +0x00 i16 mode, +0x08 f32 pitch limit,
//!       +0x0C/+0x18 target bounds min/max (used when +0x24 byte is set,
//!       else the level's own bounds inset by 8, raised by 4),
//!       +0x2C/+0x30 f32 near/far distance
//! AUDS  0x3C bytes per record: +0x00 bank name[16], +0x18 stream name[16],
//!       +0x28 i16 track count, +0x2C 8 × i16 parts per track
//! ```

use thiserror::Error;

use crate::chunk::{ChunkError, ChunkFile};

const LEVEL_LEN: usize = 0x10C;
const AUDIO_LEN: usize = 0x3C;
const CAMERA_LEN: usize = 0x6C;
const MAX_TRACKS: usize = 8;

#[derive(Debug, Error)]
pub enum WorldDataError {
    #[error(transparent)]
    Chunk(#[from] ChunkError),
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
    #[error("no {0} chunk")]
    MissingChunk(&'static str),
    #[error("level {0} uses audio record {1}, but there are {2}")]
    BadAudioIndex(String, i16, usize),
    #[error("level {0} uses camera record {1}, but there are {2}")]
    BadCameraIndex(String, i16, usize),
    #[error("audio record {0} has {1} tracks (at most 8)")]
    BadTrackCount(usize, i16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldLevel {
    /// `A1` for `LEVELS/levelA1`.
    pub name: String,
    /// Index into [`WorldData::audio`].
    pub audio: usize,
    /// Index into [`WorldData::cameras`].
    pub camera: usize,
}

impl WorldLevel {
    pub fn folder(&self) -> String {
        format!("level{}", self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelAudio {
    /// Sound bank the level loads, by catalog name (`CASTLE`, `PYRAMID`).
    pub bank: String,
    /// Stream name stem (`castle1`).
    pub stream: String,
    /// Music tracks; gameplay switches between them, a level starts on 0.
    pub tracks: usize,
    /// Parts per track: played in order, the last one looping.
    pub parts: [u16; MAX_TRACKS],
}

impl LevelAudio {
    pub fn part_count(&self, track: usize) -> usize {
        self.parts.get(track).map_or(1, |&p| p.max(1) as usize)
    }

    /// `STREAMS/<name>.ads` for a track and part, named the way the game
    /// builds it: a track letter when there's more than one track, and a
    /// `_<n>` suffix when the track has more than one part.
    pub fn stream_path(&self, track: usize, part: usize) -> String {
        let mut name = self.stream.clone();
        if self.tracks != 1 {
            name.push((b'a' + track as u8) as char);
        }
        if self.parts.get(track).is_some_and(|&p| p >= 2) {
            name.push_str(&format!("_{}", part + 1));
        }
        format!("STREAMS/{name}.ads")
    }
}

/// How the camera behaves in a level (`CAMS`).
#[derive(Debug, Clone, PartialEq)]
pub struct LevelCamera {
    /// Camera mode; 0 in every retail level.
    pub mode: i16,
    /// With several players, the camera looks down at least this steeply
    /// (radians).
    pub pitch_limit: f32,
    /// Box the camera's target is kept in, when the record sets one;
    /// otherwise the game derives it from the level bounds
    /// ([`LevelCamera::target_bounds`]).
    pub bounds: Option<([f32; 3], [f32; 3])>,
    /// Distance from the target: `near` alone for one player, up to `far`
    /// to fit several.
    pub near: f32,
    pub far: f32,
}

impl LevelCamera {
    /// The target box: the record's own, or the level bounds pulled in by 8
    /// on X/Z and with 4 more headroom, as the game does at level start.
    pub fn target_bounds(&self, level_min: [f32; 3], level_max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        self.bounds.unwrap_or((
            [level_min[0] + 8.0, level_min[1], level_min[2] + 8.0],
            [level_max[0] - 8.0, level_max[1] + 4.0, level_max[2] - 8.0],
        ))
    }
}

#[derive(Debug, Clone)]
pub struct WorldData {
    pub levels: Vec<WorldLevel>,
    pub audio: Vec<LevelAudio>,
    pub cameras: Vec<LevelCamera>,
}

impl WorldData {
    pub fn parse(file: &[u8]) -> Result<Self, WorldDataError> {
        let chunks = ChunkFile::parse(file)?;
        let records = |tag: &'static str, size: usize| {
            let c = chunks.get(tag).ok_or(WorldDataError::MissingChunk(tag))?;
            chunks.records(tag, size).ok_or(WorldDataError::Truncated(
                c.offset as usize,
                c.offset as usize + c.count as usize * size,
            ))
        };

        let audio = records("AUDS", AUDIO_LEN)?
            .enumerate()
            .map(|(i, a)| {
                let tracks = le_u16(a, 0x28) as i16;
                if !(1..=MAX_TRACKS as i16).contains(&tracks) {
                    return Err(WorldDataError::BadTrackCount(i, tracks));
                }
                Ok(LevelAudio {
                    bank: cstr(&a[0x00..0x10]),
                    stream: cstr(&a[0x18..0x28]),
                    tracks: tracks as usize,
                    parts: std::array::from_fn(|t| le_u16(a, 0x2C + t * 2)),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let cameras: Vec<LevelCamera> = records("CAMS", CAMERA_LEN)?
            .map(|c| LevelCamera {
                mode: le_u16(c, 0) as i16,
                pitch_limit: le_f32(c, 0x08),
                bounds: (c[0x24] != 0).then(|| (vec3(c, 0x0C), vec3(c, 0x18))),
                near: le_f32(c, 0x2C),
                far: le_f32(c, 0x30),
            })
            .collect();

        let levels = records("LEVL", LEVEL_LEN)?
            .map(|l| {
                let name = cstr(&l[0x08..0x18]);
                // The game clamps a negative index to record 0.
                let index = le_u16(l, 0x5A) as i16;
                let audio_index = index.max(0) as usize;
                if audio_index >= audio.len() {
                    return Err(WorldDataError::BadAudioIndex(name, index, audio.len()));
                }
                // Likewise for the camera record.
                let index = le_u16(l, 0x58) as i16;
                let camera = index.max(0) as usize;
                if camera >= cameras.len() {
                    return Err(WorldDataError::BadCameraIndex(name, index, cameras.len()));
                }
                Ok(WorldLevel { name, audio: audio_index, camera })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { levels, audio, cameras })
    }

    /// The level with folder name `levelA1` (case-insensitive).
    pub fn level(&self, folder: &str) -> Option<&WorldLevel> {
        self.levels.iter().find(|l| l.folder().eq_ignore_ascii_case(folder))
    }
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_f32(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn vec3(b: &[u8], at: usize) -> [f32; 3] {
    [le_f32(b, at), le_f32(b, at + 4), le_f32(b, at + 8)]
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(stream: &str, tracks: usize, parts: &[u16]) -> LevelAudio {
        let mut p = [1u16; MAX_TRACKS];
        p[..parts.len()].copy_from_slice(parts);
        LevelAudio { bank: "CASTLE".into(), stream: stream.into(), tracks, parts: p }
    }

    #[test]
    fn stream_names_follow_the_games_pattern() {
        assert_eq!(audio("castle1", 1, &[1]).stream_path(0, 0), "STREAMS/castle1.ads");
        assert_eq!(audio("castle2", 2, &[1, 1]).stream_path(1, 0), "STREAMS/castle2b.ads");
        assert_eq!(audio("castle6", 1, &[2]).stream_path(0, 1), "STREAMS/castle6_2.ads");
        assert_eq!(audio("desert2", 3, &[1, 2, 1]).stream_path(1, 0), "STREAMS/desert2b_1.ads");
        assert_eq!(audio("forest5", 2, &[1, 2, 0]).part_count(2), 1);
    }

    #[test]
    fn parses_directory_levels_and_audio() {
        // header, one AUDS record, one LEVL record, directory.
        let mut f = vec![0u8; 8];
        let auds_at = f.len();
        let mut a = [0u8; AUDIO_LEN];
        a[..6].copy_from_slice(b"CASTLE");
        a[0x18..0x1F].copy_from_slice(b"castle6");
        a[0x28] = 1;
        a[0x2C] = 2;
        f.extend_from_slice(&a);
        let cams_at = f.len();
        let mut c = [0u8; CAMERA_LEN];
        c[0x2C..0x30].copy_from_slice(&24f32.to_le_bytes());
        c[0x30..0x34].copy_from_slice(&32f32.to_le_bytes());
        f.extend_from_slice(&c);
        let levl_at = f.len();
        let mut l = [0u8; LEVEL_LEN];
        l[8..10].copy_from_slice(b"A6");
        l[0x5A..0x5C].copy_from_slice(&(-1i16).to_le_bytes());
        f.extend_from_slice(&l);
        let dir_at = f.len();
        for (tag, at) in [(b"AUDS", auds_at), (b"CAMS", cams_at), (b"LEVL", levl_at)] {
            f.extend_from_slice(&u32::from_be_bytes(*tag).to_le_bytes());
            f.extend_from_slice(&(at as u32).to_le_bytes());
            f.extend_from_slice(&1u32.to_le_bytes());
            f.extend_from_slice(&[0; 4]);
        }
        f[0..4].copy_from_slice(&(dir_at as u32).to_le_bytes());
        f[4..8].copy_from_slice(&3u32.to_le_bytes());

        let w = WorldData::parse(&f).unwrap();
        assert_eq!(w.cameras[0].near, 24.0);
        assert_eq!(w.cameras[0].bounds, None);
        assert_eq!(w.cameras[0].target_bounds([-100.0; 3], [100.0; 3]), ([-92.0, -100.0, -92.0], [92.0, 104.0, 92.0]));
        let level = w.level("LEVELA6").unwrap();
        assert_eq!(level.audio, 0); // -1 clamps to 0
        assert_eq!(w.audio[0].stream_path(0, 1), "STREAMS/castle6_2.ads");
        assert!(w.level("levelA1").is_none());
    }

    /// Every realm's camera records are sane and every level has one.
    #[test]
    fn every_real_realm_has_level_cameras() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(entries) = std::fs::read_dir(std::path::Path::new(&root).join("WDATA")) else {
            eprintln!("skipping: no WDATA folder");
            return;
        };
        let mut levels = 0;
        for p in entries.flatten().map(|e| e.path()) {
            let w = WorldData::parse(&std::fs::read(&p).unwrap()).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            for c in &w.cameras {
                assert_eq!(c.mode, 0, "{p:?}");
                assert!(c.near > 0.0 && c.near <= c.far && c.far < 100.0, "{p:?} {c:?}");
                assert!((0.0..=std::f32::consts::FRAC_PI_2).contains(&c.pitch_limit), "{p:?} {c:?}");
                if let Some((lo, hi)) = c.bounds {
                    assert!((0..3).all(|i| lo[i] < hi[i]), "{p:?} {c:?}");
                }
            }
            levels += w.levels.len();
        }
        eprintln!("{levels} levels with camera records");
        assert!(levels > 60);
    }
}
