//! `WDATA/*.WAD` — per-realm world data. Only what the audio needs is
//! parsed so far: the chunk directory, each level's name and audio record,
//! and the audio records (sound bank + music stream).
//!
//! Confirmed against the world-data loader and the level-music code in
//! `main.dol` (see `docs/audio-format.md`). Little-endian.
//!
//! ```text
//! 0x00  u32 directory offset, u32 chunk count
//! dir   16 bytes per chunk: u32 tag ('WRLD', 'LEVL', 'AUDS', …),
//!       u32 offset, u32 record count, u32
//! LEVL  0x10C bytes per level: +0x08 name ("A1" → LEVELS/levelA1),
//!       +0x5A i16 audio record
//! AUDS  0x3C bytes per record: +0x00 bank name[16], +0x18 stream name[16],
//!       +0x28 i16 track count, +0x2C 8 × i16 parts per track
//! ```

use thiserror::Error;

const DIR_ENTRY_LEN: usize = 0x10;
const LEVEL_LEN: usize = 0x10C;
const AUDIO_LEN: usize = 0x3C;
const MAX_TRACKS: usize = 8;

#[derive(Debug, Error)]
pub enum WorldDataError {
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
    #[error("no {0} chunk")]
    MissingChunk(&'static str),
    #[error("level {0} uses audio record {1}, but there are {2}")]
    BadAudioIndex(String, i16, usize),
    #[error("audio record {0} has {1} tracks (at most 8)")]
    BadTrackCount(usize, i16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldLevel {
    /// `A1` for `LEVELS/levelA1`.
    pub name: String,
    /// Index into [`WorldData::audio`].
    pub audio: usize,
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

#[derive(Debug, Clone)]
pub struct WorldData {
    pub levels: Vec<WorldLevel>,
    pub audio: Vec<LevelAudio>,
}

impl WorldData {
    pub fn parse(file: &[u8]) -> Result<Self, WorldDataError> {
        let h = slice(file, 0, 8)?;
        let (dir_at, count) = (le_u32(h, 0) as usize, le_u32(h, 4) as usize);
        let dir = slice(file, dir_at, count.saturating_mul(DIR_ENTRY_LEN))?;
        let chunk = |tag: &'static str| -> Result<(usize, usize), WorldDataError> {
            let want = u32::from_be_bytes(tag.as_bytes().try_into().unwrap());
            dir.as_chunks::<DIR_ENTRY_LEN>().0.iter()
                .find(|e| le_u32(&e[..], 0) == want)
                .map(|e| (le_u32(e, 4) as usize, le_u32(e, 8) as usize))
                .ok_or(WorldDataError::MissingChunk(tag))
        };

        let (at, n) = chunk("AUDS")?;
        let audio = slice(file, at, n * AUDIO_LEN)?
            .as_chunks::<AUDIO_LEN>().0.iter()
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

        let (at, n) = chunk("LEVL")?;
        let levels = slice(file, at, n * LEVEL_LEN)?
            .as_chunks::<LEVEL_LEN>().0.iter()
            .map(|l| {
                let name = cstr(&l[0x08..0x18]);
                // The game clamps a negative index to record 0.
                let index = le_u16(l, 0x5A) as i16;
                let audio_index = index.max(0) as usize;
                if audio_index >= audio.len() {
                    return Err(WorldDataError::BadAudioIndex(name, index, audio.len()));
                }
                Ok(WorldLevel { name, audio: audio_index })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { levels, audio })
    }

    /// The level with folder name `levelA1` (case-insensitive).
    pub fn level(&self, folder: &str) -> Option<&WorldLevel> {
        self.levels.iter().find(|l| l.folder().eq_ignore_ascii_case(folder))
    }
}

fn slice(b: &[u8], at: usize, len: usize) -> Result<&[u8], WorldDataError> {
    let end = at.saturating_add(len);
    b.get(at..end).ok_or(WorldDataError::Truncated(at, end))
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
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
        let levl_at = f.len();
        let mut l = [0u8; LEVEL_LEN];
        l[8..10].copy_from_slice(b"A6");
        l[0x5A..0x5C].copy_from_slice(&(-1i16).to_le_bytes());
        f.extend_from_slice(&l);
        let dir_at = f.len();
        for (tag, at) in [(b"AUDS", auds_at), (b"LEVL", levl_at)] {
            f.extend_from_slice(&u32::from_be_bytes(*tag).to_le_bytes());
            f.extend_from_slice(&(at as u32).to_le_bytes());
            f.extend_from_slice(&1u32.to_le_bytes());
            f.extend_from_slice(&[0; 4]);
        }
        f[0..4].copy_from_slice(&(dir_at as u32).to_le_bytes());
        f[4..8].copy_from_slice(&2u32.to_le_bytes());

        let w = WorldData::parse(&f).unwrap();
        let level = w.level("LEVELA6").unwrap();
        assert_eq!(level.audio, 0); // -1 clamps to 0
        assert_eq!(w.audio[0].stream_path(0, 1), "STREAMS/castle6_2.ads");
        assert!(w.level("levelA1").is_none());
    }
}
