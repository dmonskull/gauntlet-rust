//! Decodes a music stream or a sound bank from your own copy of the game to
//! `.wav`, and prints numbers that tell a correct decode from a broken one.
//!
//! ```text
//! cargo run -p gdl-formats --example audio_dump -- STREAMS/CASTLE1.ads out.wav
//! cargo run -p gdl-formats --example audio_dump -- AUDIO/CASTLE.VBK out_dir/
//! ```
//!
//! For each output: duration, RMS level, zero-crossing rate and the
//! *roughness* — mean |sample-to-sample step| / RMS. Decoded music and
//! effects are smooth (roughness well under 0.5); a wrong decode is close
//! to white noise (roughness ≈ 1.1 or more). The *edge ratio* compares the
//! step across ADPCM frame boundaries (every 14 samples) with the step
//! inside frames: ≈ 1 when channels and frames are laid out right, clearly
//! above 1 when frames are decoded with the wrong history (wrong interleave).

use std::path::{Path, PathBuf};

use gdl_formats::audio::{AdsStream, SoundBank};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: audio_dump <file.ads> <out.wav> | <file.VBK> <out_dir>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(input).unwrap_or_else(|e| fail(&format!("{input}: {e}")));
    if bytes.starts_with(b"dhSS") {
        let stream = AdsStream::parse(&bytes).unwrap_or_else(|e| fail(&format!("{input}: {e}")));
        let pcm = stream.decode();
        let channels = stream.channel_count() as u16;
        write_wav(Path::new(output), stream.sample_rate, channels, &pcm);
        report(output, stream.sample_rate, channels, &pcm);
    } else {
        let bank = SoundBank::parse(&bytes).unwrap_or_else(|e| fail(&format!("{input}: {e}")));
        let dir = PathBuf::from(output);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| fail(&format!("{output}: {e}")));
        println!("{} calls, {} samples", bank.calls.len(), bank.samples.len());
        for (i, s) in bank.samples.iter().enumerate() {
            let pcm = s.decode();
            let path = dir.join(format!("{i:03}_{}.wav", s.name));
            write_wav(&path, s.sample_rate, 1, &pcm);
            report(&path.display().to_string(), s.sample_rate, 1, &pcm);
        }
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn report(name: &str, rate: u32, channels: u16, pcm: &[i16]) {
    let ch = channels as usize;
    let frames = pcm.len() / ch;
    let seconds = frames as f64 / rate as f64;
    let (mut sum_sq, mut step, mut crossings) = (0f64, 0f64, 0usize);
    let (mut edge, mut edges, mut inner, mut inners) = (0f64, 0usize, 0f64, 0usize);
    for c in 0..ch {
        let mut prev: Option<i16> = None;
        for (i, &s) in pcm.iter().skip(c).step_by(ch).enumerate() {
            sum_sq += (s as f64).powi(2);
            if let Some(p) = prev {
                let d = (s as f64 - p as f64).abs();
                step += d;
                if i % 14 == 0 {
                    (edge, edges) = (edge + d, edges + 1);
                } else {
                    (inner, inners) = (inner + d, inners + 1);
                }
                crossings += ((p < 0) != (s < 0)) as usize;
            }
            prev = Some(s);
        }
    }
    let n = pcm.len().max(1) as f64;
    let rms = (sum_sq / n).sqrt();
    let roughness = if rms > 0.0 { step / n / rms } else { 0.0 };
    let zcr = crossings as f64 / seconds.max(1e-9) / ch as f64;
    let edge_ratio = (edge / edges.max(1) as f64) / (inner / inners.max(1) as f64).max(1e-9);
    println!(
        "{name}: {seconds:.2}s @ {rate} Hz × {channels}, rms {rms:.0}, \
         zero crossings {zcr:.0}/s, roughness {roughness:.3}, edge ratio {edge_ratio:.2}"
    );
}

fn write_wav(path: &Path, rate: u32, channels: u16, pcm: &[i16]) {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
}
