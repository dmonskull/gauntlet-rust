//! Runs the audio parsers over every bank, stream and world-data file in
//! the user's own copy of the game. Skips cleanly when it isn't present.

use std::path::{Path, PathBuf};

use gdl_formats::WorldData;
use gdl_formats::audio::{AdsStream, AudioCatalog, SoundBank};

fn data_root() -> Option<PathBuf> {
    let root = PathBuf::from(
        std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into()),
    );
    if root.join("AUDIO").is_dir() {
        Some(root)
    } else {
        eprintln!("skipping: no game data at {root:?}");
        None
    }
}

fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)));
    v.sort();
    v
}

/// Case-insensitive lookup of a game path under the data root.
fn find(root: &Path, game_path: &str) -> Option<PathBuf> {
    let mut at = root.to_path_buf();
    for part in game_path.split('/') {
        at = std::fs::read_dir(&at)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(part)))?;
    }
    Some(at)
}

#[test]
fn every_bank_parses_and_decodes() {
    let Some(root) = data_root() else { return };
    let (mut banks, mut calls, mut samples, mut seconds, mut looping) = (0, 0, 0, 0.0, 0);
    for path in files_with_ext(&root.join("AUDIO"), "vbk") {
        let bank = SoundBank::parse(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        for (i, call) in bank.calls.iter().enumerate() {
            let seq = call.sequence().unwrap_or_else(|e| panic!("{path:?} call {i}: {e}"));
            assert!(!(seq.intro.is_empty() && seq.looped.is_empty()), "{path:?} call {i} plays nothing");
            looping += !seq.looped.is_empty() as usize;
            assert!(call.volume <= 0x7F, "{path:?} call {i} volume {}", call.volume);
        }
        for (i, s) in bank.samples.iter().enumerate() {
            assert_eq!(s.dsp.format, 0, "{path:?} sample {i} isn't ADPCM");
            assert_eq!(s.dsp.sample_rate, s.sample_rate, "{path:?} sample {i}: VAG and DSP rates differ");
            assert_eq!(s.num_samples(), s.dsp.num_samples as usize, "{path:?} sample {i} is short");
            let pcm = s.decode();
            assert_eq!(pcm.len(), s.num_samples());
            seconds += s.duration_secs();
        }
        banks += 1;
        calls += bank.calls.len();
        samples += bank.samples.len();
    }
    eprintln!("{banks} banks: {calls} calls ({looping} looping), {samples} samples, {seconds:.0}s of audio");
    assert!(banks == 0 || banks >= 60);
}

#[test]
fn every_stream_parses_and_decodes() {
    let Some(root) = data_root() else { return };
    let (mut streams, mut stereo, mut seconds) = (0, 0, 0.0);
    for path in files_with_ext(&root.join("STREAMS"), "ads") {
        let bytes = std::fs::read(&path).unwrap();
        let s = AdsStream::parse(&bytes).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        for c in &s.channels {
            assert_eq!(c.format, 0, "{path:?} isn't ADPCM");
            assert_eq!(c.sample_rate, s.sample_rate, "{path:?}: channel rate differs");
            // Every retail stream is complete: no channel is cut short.
            assert_eq!(c.num_samples as usize, s.num_samples, "{path:?} is short");
        }
        assert_eq!(AdsStream::samples(&s).count(), s.num_samples * s.channel_count(), "{path:?}");
        streams += 1;
        stereo += (s.channel_count() == 2) as usize;
        seconds += s.duration_secs();
    }
    eprintln!("{streams} streams ({stereo} stereo), {:.0} minutes of music", seconds / 60.0);
    assert!(streams == 0 || streams >= 100);
}

#[test]
fn catalog_matches_the_banks() {
    let Some(root) = data_root() else { return };
    let Some(rom) = find(&root, "AUDIO/AUDATPS2.ROM") else { return };
    let catalog = AudioCatalog::parse(&std::fs::read(rom).unwrap()).unwrap();
    let mut checked = 0;
    for (b, cb) in catalog.banks.iter().enumerate() {
        let Some(path) = find(&root, &cb.path()) else {
            panic!("catalog bank {} has no file {}", cb.name, cb.path());
        };
        let bank = SoundBank::parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(bank.calls.len(), cb.sound_count, "{}: calls vs catalog sounds", cb.name);
        for s in catalog.bank_sounds(b) {
            assert_eq!(s.bank, b, "{} is listed under bank {}", s.name, cb.name);
            assert!(s.call < bank.calls.len(), "{} plays call {}", s.name, s.call);
            // The length field is the call's samples played back to back
            // (which also confirms multi-step calls are sequences), or -1.
            let seq = bank.calls[s.call].sequence().unwrap();
            if seq.looped.is_empty() {
                let secs: f64 = seq.intro.iter().map(|&i| bank.samples[i].duration_secs()).sum();
                assert!((s.length as f64 - secs).abs() < 0.01, "{}: length {} vs {secs}", s.name, s.length);
            } else {
                assert!(s.length < 0.0, "{} loops but has length {}", s.name, s.length);
            }
        }
        checked += 1;
    }
    let warn = catalog.find_sound("S_WARN").expect("S_WARN");
    assert_eq!(catalog.banks[warn.bank].name, "COMMON");
    let modes: Vec<_> = catalog.modes.iter().map(|m| m.name.as_str()).collect();
    eprintln!("{checked} banks, {} sounds, modes {modes:?}", catalog.sounds.len());
}

#[test]
fn every_level_has_its_music_and_bank() {
    let Some(root) = data_root() else { return };
    let Some(rom) = find(&root, "AUDIO/AUDATPS2.ROM") else { return };
    let catalog = AudioCatalog::parse(&std::fs::read(rom).unwrap()).unwrap();
    let (mut levels, mut parts) = (0, 0);
    let mut missing = Vec::new();
    for path in files_with_ext(&root.join("WDATA"), "wad") {
        let world = WorldData::parse(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        for level in &world.levels {
            let audio = &world.audio[level.audio];
            assert!(catalog.find_bank(&audio.bank).is_some(), "{path:?} {}: no bank {}", level.name, audio.bank);
            for track in 0..audio.tracks {
                for part in 0..audio.part_count(track) {
                    let stream = audio.stream_path(track, part);
                    match find(&root, &stream) {
                        Some(_) => parts += 1,
                        None => missing.push(format!("{} {}: {stream}", path.display(), level.name)),
                    }
                }
            }
            levels += 1;
        }
    }
    eprintln!("{levels} levels, {parts} music stream parts present, {} missing", missing.len());
    // Only the leftover TEST realm points at streams that aren't on the
    // disc (its record describes an older dream1 split into parts).
    let retail_missing: Vec<_> = missing.iter().filter(|m| !m.contains("TEST.WAD")).collect();
    assert!(retail_missing.is_empty(), "missing music: {retail_missing:#?}");
}
