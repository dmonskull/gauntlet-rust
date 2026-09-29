//! `STREAMS/*.ads` — streamed music: a small header, one DSP-ADPCM header
//! per channel, then the channels' ADPCM data interleaved in fixed blocks.
//!
//! Confirmed against the stream player in `main.dol` (header check, header
//! read, per-channel de-interleave into the voice buffers; see
//! `docs/audio-format.md`). Big-endian, like the banks.
//!
//! ```text
//! 0x00  "dhSS"
//! 0x04  u32 0x18
//! 0x08  u32 codec: 0x20 = DSP-ADPCM (every retail stream)
//! 0x0C  u32 sample rate
//! 0x10  u32 channels (1 or 2)
//! 0x14  u32 interleave in bytes (0x20 on every retail stream)
//! 0x18  u32, u32 (0xFFFFFFFF)
//! 0x20  "dbSS"
//! 0x24  u32 data size in bytes (all channels)
//! 0x28  channels × 0x60-byte DSP-ADPCM header
//!       data: interleave bytes of channel 0, of channel 1, …, repeating
//! ```

use std::borrow::Borrow;

use super::dsp::{self, AdpcmDecoder, DspHeader};
use super::{AudioError, be_u32, slice};

pub const HEADER_MAGIC: &[u8; 4] = b"dhSS";
pub const DATA_MAGIC: &[u8; 4] = b"dbSS";
pub const CODEC_DSP_ADPCM: u32 = 0x20;
const FIXED_HEADER_LEN: usize = 0x28;

#[derive(Debug, Clone)]
pub struct AdsStream {
    pub sample_rate: u32,
    /// One DSP header per channel.
    pub channels: Vec<DspHeader>,
    pub interleave: usize,
    /// Samples per channel.
    pub num_samples: usize,
    /// Interleaved ADPCM data.
    data: Vec<u8>,
}

impl AdsStream {
    pub fn parse(file: &[u8]) -> Result<Self, AudioError> {
        let h = slice(file, 0, FIXED_HEADER_LEN)?;
        if &h[0..4] != HEADER_MAGIC {
            return Err(AudioError::BadMagic("dhSS"));
        }
        if &h[0x20..0x24] != DATA_MAGIC {
            return Err(AudioError::BadMagic("dbSS"));
        }
        let codec = be_u32(h, 0x08);
        if codec != CODEC_DSP_ADPCM {
            return Err(AudioError::UnsupportedCodec(codec));
        }
        let sample_rate = be_u32(h, 0x0C);
        let channel_count = be_u32(h, 0x10) as usize;
        if !(1..=2).contains(&channel_count) {
            return Err(AudioError::Invalid("stream channel count isn't 1 or 2"));
        }
        let interleave = be_u32(h, 0x14) as usize;
        if interleave == 0 || !interleave.is_multiple_of(dsp::FRAME_BYTES) {
            return Err(AudioError::Invalid("stream interleave isn't a whole number of frames"));
        }
        let data_size = be_u32(h, 0x24) as usize;

        let channels = (0..channel_count)
            .map(|c| DspHeader::parse(slice(file, FIXED_HEADER_LEN + c * dsp::HEADER_LEN, dsp::HEADER_LEN)?))
            .collect::<Result<Vec<_>, _>>()?;
        let data_start = FIXED_HEADER_LEN + channel_count * dsp::HEADER_LEN;
        // The player tolerates a stream shorter than its header claims (it
        // just hits end of file); do the same.
        let data = file.get(data_start..).unwrap_or_default();
        let data = data[..data.len().min(data_size)].to_vec();

        // Samples each channel can actually produce: whole frames present,
        // capped by the headers' counts.
        // A partial last block fills channel 0 first, so the last channel
        // has the least.
        let blocks = data.len() / (interleave * channel_count);
        let tail = data.len() % (interleave * channel_count);
        let last_tail = tail.saturating_sub((channel_count - 1) * interleave).min(interleave);
        let per_channel_bytes = blocks * interleave + last_tail / dsp::FRAME_BYTES * dsp::FRAME_BYTES;
        let present = per_channel_bytes / dsp::FRAME_BYTES * dsp::SAMPLES_PER_FRAME;
        let declared = channels.iter().map(|c| c.num_samples as usize).min().unwrap_or(0);
        Ok(Self { sample_rate, channels, interleave, num_samples: declared.min(present), data })
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn duration_secs(&self) -> f64 {
        self.num_samples as f64 / self.sample_rate.max(1) as f64
    }

    /// Frame `frame` of channel `channel`.
    fn frame(&self, channel: usize, frame: usize) -> &[u8; dsp::FRAME_BYTES] {
        let offset = frame * dsp::FRAME_BYTES;
        let (block, within) = (offset / self.interleave, offset % self.interleave);
        let at = (block * self.channel_count() + channel) * self.interleave + within;
        self.data[at..at + dsp::FRAME_BYTES].try_into().unwrap()
    }

    /// Interleaved PCM, decoded as it's iterated. Takes `&AdsStream` or an
    /// owning handle such as `Arc<AdsStream>`.
    pub fn samples<S: Borrow<AdsStream>>(stream: S) -> AdsSamples<S> {
        let decoders = stream.borrow().channels.iter().map(DspHeader::decoder).collect();
        AdsSamples { stream, decoders, frame: 0, buffer: [0; FRAME_OUT], len: 0, pos: 0, emitted: 0 }
    }

    /// The whole stream as interleaved PCM.
    pub fn decode(&self) -> Vec<i16> {
        Self::samples(self).collect()
    }
}

const FRAME_OUT: usize = 2 * dsp::SAMPLES_PER_FRAME;

/// Interleaved PCM from an [`AdsStream`], one ADPCM frame at a time.
pub struct AdsSamples<S: Borrow<AdsStream>> {
    stream: S,
    decoders: Vec<AdpcmDecoder>,
    frame: usize,
    /// One frame of interleaved output (no allocation on the audio thread).
    buffer: [i16; FRAME_OUT],
    len: usize,
    pos: usize,
    /// Per-channel samples handed out so far.
    emitted: usize,
}

impl<S: Borrow<AdsStream>> AdsSamples<S> {
    pub fn stream(&self) -> &AdsStream {
        self.stream.borrow()
    }

    /// Interleaved samples still to come.
    pub fn remaining(&self) -> usize {
        let s = self.stream.borrow();
        (s.num_samples - self.emitted) * s.channel_count() + (self.len - self.pos)
    }

    fn refill(&mut self) -> bool {
        let s = self.stream.borrow();
        if self.emitted >= s.num_samples {
            return false;
        }
        let n = (s.num_samples - self.emitted).min(dsp::SAMPLES_PER_FRAME);
        let channels = s.channel_count();
        let mut decoded = [[0i16; dsp::SAMPLES_PER_FRAME]; 2];
        for (c, d) in self.decoders.iter_mut().enumerate() {
            decoded[c] = d.decode_frame(s.frame(c, self.frame));
        }
        for i in 0..n {
            for (c, channel) in decoded[..channels].iter().enumerate() {
                self.buffer[i * channels + c] = channel[i];
            }
        }
        self.len = n * channels;
        self.pos = 0;
        self.frame += 1;
        self.emitted += n;
        true
    }
}

impl<S: Borrow<AdsStream>> Iterator for AdsSamples<S> {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        if self.pos == self.len && !self.refill() {
            return None;
        }
        self.pos += 1;
        Some(self.buffer[self.pos - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream whose channel `c` holds constant `value[c]` via predictor 1
    /// (c1 = 2048: repeat the previous sample) and a nonzero first sample.
    fn synthetic(channels: usize, frames_per_channel: usize, interleave: usize) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(HEADER_MAGIC);
        for v in [0x18, CODEC_DSP_ADPCM, 24000, channels as u32, interleave as u32, !0, !0] {
            f.extend_from_slice(&v.to_be_bytes());
        }
        f.extend_from_slice(DATA_MAGIC);
        f.extend_from_slice(&[0; 4]); // data size, patched below
        for c in 0..channels {
            let mut h = [0u8; dsp::HEADER_LEN];
            h[0..4].copy_from_slice(&((frames_per_channel * 14) as u32).to_be_bytes());
            h[0x1C + 4..0x1C + 6].copy_from_slice(&2048i16.to_be_bytes()); // predictor 1 c1
            h[0x40..0x42].copy_from_slice(&(100 * (c as i16 + 1)).to_be_bytes()); // hist1
            f.extend_from_slice(&h);
        }
        // Like the retail files, each channel's last block is padded out to
        // the interleave.
        let frames_per_block = interleave / 8;
        for block in 0..frames_per_channel.div_ceil(frames_per_block) {
            for _ in 0..channels {
                for fr in 0..frames_per_block {
                    let used = block * frames_per_block + fr < frames_per_channel;
                    f.extend_from_slice(&[if used { 0x10 } else { 0 }, 0, 0, 0, 0, 0, 0, 0]);
                }
            }
        }
        let data_len = (f.len() - FIXED_HEADER_LEN - channels * dsp::HEADER_LEN) as u32;
        f[0x24..0x28].copy_from_slice(&data_len.to_be_bytes());
        f
    }

    #[test]
    fn deinterleaves_channels() {
        let s = AdsStream::parse(&synthetic(2, 9, 0x20)).unwrap();
        assert_eq!((s.channel_count(), s.num_samples, s.sample_rate), (2, 9 * 14, 24000));
        let pcm = s.decode();
        assert_eq!(pcm.len(), 2 * 9 * 14);
        assert!(pcm.chunks(2).all(|f| f == [100, 200]));
    }

    #[test]
    fn iterator_reports_what_is_left() {
        let s = AdsStream::parse(&synthetic(1, 3, 0x20)).unwrap();
        let mut it = AdsStream::samples(&s);
        assert_eq!(it.remaining(), 42);
        it.next();
        assert_eq!(it.remaining(), 41);
        assert_eq!(it.count(), 41);
    }

    #[test]
    fn short_file_decodes_what_is_there() {
        let mut f = synthetic(1, 4, 0x20);
        f.truncate(f.len() - 8);
        let s = AdsStream::parse(&f).unwrap();
        assert_eq!(s.num_samples, 3 * 14);
    }

    #[test]
    fn rejects_other_codecs() {
        let mut f = synthetic(1, 1, 0x20);
        f[0x0B] = 0x10;
        assert!(matches!(AdsStream::parse(&f), Err(AudioError::UnsupportedCodec(0x10))));
    }
}
