//! `VQMOVIES/*.avi` — the game's movies (the levels' openings, the
//! legendary items', the endings). AVI files holding MidiVid VQ video
//! (`MVDV`: 512 × 384, 30 frames a second) and unsigned 8-bit mono PCM
//! sound. The level records name theirs (`LEVL +0x34`, `docs/frontend.md`,
//! "Movies").
//!
//! The sound: the AVI's header says 8-bit PCM, but the game's movies carry
//! an ADS stream in it (`dhSS`/`dbSS`, DSP-ADPCM, as the music's:
//! [`crate::audio::AdsStream`]) — played as PCM it's only static.
//! [`Movie::audio`] decodes it.
//!
//! A video frame (as FFmpeg's `midivid` decoder reads it, which this
//! follows; `examples/moviecheck.rs` compares every frame with FFmpeg's):
//!
//! ```text
//! +0x00  8 bytes (unused), +0x08 u32 stored (0: the rest is LZSS-packed)
//! then   u16 vector count n, u16 key (non-zero: every block is coded)
//!        not key: u32 block count, a bit per 4 × 4 pixels (1: coded)
//!        n × 12 bytes: 2 × 2 YUV vectors (Y U V for the bottom-left,
//!        bottom-right, top-left, top-right pixel)
//!        n > 256: a ninth index bit per coded block
//!        a byte per coded 2 × 2 block, bottom row first
//! ```

use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum MovieError {
    #[error("not an AVI file")]
    NotAvi,
    #[error("the AVI has no video stream")]
    NoVideo,
    #[error("frame data runs out")]
    Truncated,
    #[error("a frame's vector index {0} is past its {1} vectors")]
    BadIndex(usize, usize),
    #[error("a packed frame copies from outside itself")]
    BadCopy,
}

/// A movie: its picture size and rate, where each video frame lies in the
/// file, and its sound.
pub struct Movie {
    pub width: usize,
    pub height: usize,
    /// Frames a second.
    pub rate: f32,
    data: Vec<u8>,
    frames: Vec<(usize, usize)>,
    /// The sound's samples (unsigned 8-bit), sample rate and channels.
    pub sound: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl Movie {
    pub fn parse(data: Vec<u8>) -> Result<Self, MovieError> {
        if data.get(0..4) != Some(b"RIFF") || data.get(8..12) != Some(b"AVI ") {
            return Err(MovieError::NotAvi);
        }
        let mut movie = Movie {
            width: 0,
            height: 0,
            rate: 30.0,
            data: Vec::new(),
            frames: Vec::new(),
            sound: Vec::new(),
            sample_rate: 22050,
            channels: 1,
            bits: 8,
        };
        // The stream headers in order: the video's then the sound's.
        let mut kind = *b"    ";
        let (mut video, mut audio, mut stream) = (None, None, 0usize);
        walk(&data, 12, data.len(), &mut |id, body, body_end| match id {
            b"avih" => {
                movie.width = u32_at(&data, body + 32).unwrap_or(0) as usize;
                movie.height = u32_at(&data, body + 36).unwrap_or(0) as usize;
            }
            b"strh" => {
                kind = data.get(body..body + 4).and_then(|k| k.try_into().ok()).unwrap_or(*b"    ");
                let scale = u32_at(&data, body + 20).unwrap_or(1).max(1);
                let rate = u32_at(&data, body + 24).unwrap_or(30);
                if &kind == b"vids" {
                    movie.rate = rate as f32 / scale as f32;
                    video = Some(stream);
                } else if &kind == b"auds" {
                    audio = Some(stream);
                }
                stream += 1;
            }
            b"strf" if &kind == b"auds" => {
                movie.channels = u16_at(&data, body + 2).unwrap_or(1);
                movie.sample_rate = u32_at(&data, body + 4).unwrap_or(22050);
                movie.bits = u16_at(&data, body + 14).unwrap_or(8);
            }
            _ => {
                // Stream chunks: `00dc` (video), `01wb` (sound).
                let number = std::str::from_utf8(&id[..2]).ok().and_then(|n| n.parse::<usize>().ok());
                match number {
                    Some(n) if Some(n) == video && (&id[2..] == b"dc" || &id[2..] == b"db") => movie.frames.push((body, body_end)),
                    Some(n) if Some(n) == audio && &id[2..] == b"wb" => movie.sound.extend_from_slice(&data[body..body_end]),
                    _ => {}
                }
            }
        });
        if video.is_none() || movie.width == 0 || movie.height == 0 {
            return Err(MovieError::NoVideo);
        }
        movie.data = data;
        Ok(movie)
    }

    /// How many video frames it has.
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Video frame `i`'s packet.
    pub fn frame(&self, i: usize) -> Option<&[u8]> {
        self.frames.get(i).map(|&(a, b)| &self.data[a..b])
    }

    /// The sound, decoded: 16-bit samples (channels interleaved), the rate
    /// and the channel count. An ADS stream in the sound chunks (every
    /// retail movie's) is decoded; anything else is taken as the PCM the
    /// header says.
    pub fn audio(&self) -> Option<(Vec<i16>, u32, u16)> {
        if self.sound.is_empty() {
            return None;
        }
        if self.sound.starts_with(crate::audio::stream::HEADER_MAGIC) {
            let stream = crate::audio::AdsStream::parse(&self.sound).ok()?;
            let channels = stream.channel_count() as u16;
            return Some((stream.decode(), stream.sample_rate, channels));
        }
        let pcm = match self.bits {
            16 => self.sound.as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b)).collect(),
            _ => self.sound.iter().map(|&b| (i16::from(b) - 128) << 8).collect(),
        };
        Some((pcm, self.sample_rate, self.channels.max(1)))
    }

    /// The sound chunks as a WAV file, as the header describes them.
    pub fn wav(&self) -> Vec<u8> {
        let block = u32::from(self.channels) * u32::from(self.bits / 8).max(1);
        let mut out = Vec::with_capacity(44 + self.sound.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + self.sound.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&(self.sample_rate * block).to_le_bytes());
        out.extend_from_slice(&(block as u16).to_le_bytes());
        out.extend_from_slice(&self.bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(self.sound.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.sound);
        out
    }
}

/// Calls `f` with each chunk's id and body range, going into `LIST`s
/// (past their 4-byte type); bodies are padded to an even length.
fn walk(data: &[u8], mut at: usize, end: usize, f: &mut impl FnMut(&[u8; 4], usize, usize)) {
    while at + 8 <= end {
        let id: [u8; 4] = data[at..at + 4].try_into().unwrap_or_default();
        let Some(size) = u32_at(data, at + 4).map(|s| s as usize) else { return };
        let body = at + 8;
        let body_end = body.saturating_add(size).min(end);
        if &id == b"LIST" {
            walk(data, body + 4, body_end, f);
        } else {
            f(&id, body, body_end);
        }
        at = (body.saturating_add(size) + 1) & !1;
    }
}

/// Decodes MidiVid VQ frames in order into Y, U and V planes (the picture
/// kept between frames: a frame that isn't a key frame codes only the
/// blocks that changed).
pub struct MvdvDecoder {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    skip: Vec<u8>,
    unpacked: Vec<u8>,
}

/// A reader that yields 0 past the end, as FFmpeg's byte reader does.
struct Bytes<'a> {
    b: &'a [u8],
    at: usize,
}

impl Bytes<'_> {
    fn left(&self) -> usize {
        self.b.len().saturating_sub(self.at)
    }
    fn byte(&mut self) -> u8 {
        let v = self.b.get(self.at).copied().unwrap_or(0);
        self.at = (self.at + 1).min(self.b.len());
        v
    }
    fn u16(&mut self) -> u16 {
        u16::from(self.byte()) | (u16::from(self.byte()) << 8)
    }
    fn u32(&mut self) -> u32 {
        u32::from(self.u16()) | (u32::from(self.u16()) << 16)
    }
}

/// Unpacks the game's LZSS: a 16-bit word of flags, then per flag (low bit
/// first) a literal byte or, set, two bytes: a 12-bit distance back and a
/// length of 3–18.
fn lzss(src: &[u8], dst: &mut Vec<u8>, size: usize) -> Result<usize, MovieError> {
    dst.clear();
    dst.resize(size, 0);
    let mut r = Bytes { b: src, at: 0 };
    let mut out = 0usize;
    while r.left() >= 3 {
        let mut op = r.u16();
        for _ in 0..16 {
            if op & 1 != 0 {
                let s0 = usize::from(r.byte());
                let s1 = usize::from(r.byte());
                let offset = ((s0 & 0xF0) << 4) | s1;
                let length = (s0 & 0xF) + 3;
                if out + length > size || offset > out {
                    return Err(MovieError::BadCopy);
                }
                if offset > 0 {
                    for j in 0..length {
                        dst[out + j] = dst[out + j - offset];
                    }
                }
                out += length;
            } else {
                if out >= size {
                    return Err(MovieError::BadCopy);
                }
                dst[out] = r.byte();
                out += 1;
            }
            op >>= 1;
        }
    }
    Ok(out)
}

impl MvdvDecoder {
    pub fn new(width: usize, height: usize) -> Self {
        let n = width * height;
        Self {
            width,
            height,
            y: vec![0; n],
            u: vec![0; n],
            v: vec![0; n],
            skip: vec![0; (width / 2) * (height / 2)],
            unpacked: Vec::new(),
        }
    }

    /// Decodes the next frame; whether it was a key frame.
    pub fn decode(&mut self, packet: &[u8]) -> Result<bool, MovieError> {
        if packet.len() <= 13 {
            return Err(MovieError::Truncated);
        }
        let stored = u32_at(packet, 8).ok_or(MovieError::Truncated)?;
        let mut unpacked = std::mem::take(&mut self.unpacked);
        let result = if stored == 0 {
            let n = lzss(&packet[12..], &mut unpacked, 16 * (packet.len() - 12))?;
            self.picture(&unpacked[..n])
        } else {
            self.picture(&packet[12..])
        };
        self.unpacked = unpacked;
        result
    }

    fn picture(&mut self, b: &[u8]) -> Result<bool, MovieError> {
        let (w, h) = (self.width, self.height);
        let mut r = Bytes { b, at: 0 };
        let vectors = usize::from(r.u16());
        let key = r.u16() != 0;
        let blocks = if key {
            (w / 2) * (h / 2)
        } else {
            let blocks = r.u32() as usize;
            let aligned = (w + 31) & !31;
            let mask_size = ((aligned >> 2) * (h >> 2)) >> 3;
            let padding = (aligned - w) >> 2;
            if r.left() < mask_size {
                return Err(MovieError::Truncated);
            }
            let mask = &b[r.at..r.at + mask_size];
            r.at += mask_size;
            let mut bit = 0usize;
            let line = w >> 1;
            for y in 0..h >> 2 {
                for x in 0..w >> 2 {
                    let set = mask.get(bit >> 3).is_some_and(|m| m & (0x80 >> (bit & 7)) != 0);
                    bit += 1;
                    let flag = u8::from(!set);
                    for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                        self.skip[(y * 2 + dy) * line + x * 2 + dx] = flag;
                    }
                }
                bit += padding;
            }
            blocks
        };
        if r.left() < vectors * 12 {
            return Err(MovieError::Truncated);
        }
        let vec = &b[r.at..r.at + vectors * 12];
        r.at += vectors * 12;
        let mut ninth = Bytes { b: &[], at: 0 };
        if vectors > 256 {
            let n = (blocks + if key { 0 } else { 7 }) / 8;
            if r.left() < n {
                return Err(MovieError::Truncated);
            }
            ninth = Bytes { b: &b[r.at..r.at + n], at: 0 };
            r.at += n;
        }
        let (mut bits, mut byte) = (0u32, 0u8);
        let mut skip = 0usize;
        for y in (0..=h - 2).rev().step_by(2) {
            for x in (0..w).step_by(2) {
                if !key {
                    let s = self.skip[skip];
                    skip += 1;
                    if s != 0 {
                        continue;
                    }
                }
                if r.left() == 0 {
                    return Err(MovieError::Truncated);
                }
                let idx = if vectors <= 256 {
                    usize::from(r.byte())
                } else {
                    if bits == 0 {
                        byte = ninth.byte();
                        bits = 8;
                    }
                    bits -= 1;
                    usize::from(r.byte()) | (usize::from((byte >> (7 - bits)) & 1) << 8)
                };
                if idx >= vectors {
                    return Err(MovieError::BadIndex(idx, vectors));
                }
                let q = &vec[idx * 12..idx * 12 + 12];
                let (top, below) = (y * w + x, (y + 1) * w + x);
                for (plane, c) in [(&mut self.y, 0), (&mut self.u, 1), (&mut self.v, 2)] {
                    plane[below] = q[c];
                    plane[below + 1] = q[3 + c];
                    plane[top] = q[6 + c];
                    plane[top + 1] = q[9 + c];
                }
            }
        }
        Ok(key)
    }

    /// The picture as RGBA (BT.601, studio range, as FFmpeg converts it).
    pub fn rgba(&self, out: &mut Vec<u8>) {
        out.resize(self.width * self.height * 4, 255);
        for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let y = (f32::from(self.y[i]) - 16.0) * 1.164;
            let u = f32::from(self.u[i]) - 128.0;
            let v = f32::from(self.v[i]) - 128.0;
            px[0] = (y + 1.596 * v).round().clamp(0.0, 255.0) as u8;
            px[1] = (y - 0.813 * v - 0.391 * u).round().clamp(0.0, 255.0) as u8;
            px[2] = (y + 2.018 * u).round().clamp(0.0, 255.0) as u8;
            px[3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzss_copies_literals_and_runs() {
        // Flags 0b0000_0000_0000_0010: a literal 'a', then 3 bytes from 1 back,
        // then 14 literals of 0 (the stream runs out: zeros).
        let src = [0x02, 0x00, b'a', 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut dst = Vec::new();
        let n = lzss(&src, &mut dst, 64).unwrap();
        assert_eq!(&dst[..4], b"aaaa");
        assert!(n >= 4);
        // A copy from before the start is refused.
        assert_eq!(lzss(&[0x01, 0x00, 0x00, 0x05, 0], &mut dst, 64), Err(MovieError::BadCopy));
    }

    #[test]
    fn a_key_frame_paints_every_block_bottom_row_first() {
        let (w, h) = (4, 2);
        let mut packet = vec![0u8; 8];
        packet.extend_from_slice(&1u32.to_le_bytes()); // stored
        packet.extend_from_slice(&2u16.to_le_bytes()); // two vectors
        packet.extend_from_slice(&1u16.to_le_bytes()); // key
        packet.extend((0..12).map(|i| i as u8)); // vector 0
        packet.extend((100..112).map(|i| i as u8)); // vector 1
        packet.extend([1, 0]); // the two blocks
        let mut d = MvdvDecoder::new(w, h);
        assert!(d.decode(&packet).unwrap());
        // Block (0,0) has vector 1: Y of bottom-left at row 1, top-left at row 0.
        assert_eq!((d.y[w], d.y[w + 1], d.y[0], d.y[1]), (100, 103, 106, 109));
        assert_eq!((d.y[w + 2], d.y[2]), (0, 6));
        assert_eq!((d.u[0], d.v[0]), (107, 108));
    }
}
