//! Player options, saved beside the remembered game path
//! (`gdl-artifacts/options.txt`, one `key=value` per line) and applied live,
//! so a settings menu only has to change [`GameOptions`]. `GDL_MUTE=1`
//! silences one run (tests) without touching the saved volumes.

use bevy::audio::{AudioSink, AudioSinkPlayback, GlobalVolume, Volume};
use bevy::prelude::*;

use crate::bootstrap::artifacts_dir;

pub struct OptionsPlugin;

impl Plugin for OptionsPlugin {
    fn build(&self, app: &mut App) {
        let options = GameOptions::load();
        let mute = Mute(std::env::var("GDL_MUTE").is_ok_and(|v| !v.is_empty() && v != "0"));
        app.insert_resource(GlobalVolume::new(Volume::Linear(mute.master(&options))))
            .insert_resource(options)
            .insert_resource(mute)
            .add_systems(Update, (volume_keys, apply_options.run_if(resource_changed::<GameOptions>)).chain());
    }
}

/// `GDL_MUTE`: this run plays at no volume; the saved options stay.
#[derive(Resource, Clone, Copy)]
pub struct Mute(pub bool);

impl Mute {
    /// The master volume sounds actually play at.
    pub fn master(self, options: &GameOptions) -> f32 {
        if self.0 { 0.0 } else { options.master_volume }
    }
}

/// Volumes are linear, 0 (silent) to 1 (the game's own level).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct GameOptions {
    pub master_volume: f32,
    pub music_volume: f32,
    pub effects_volume: f32,
}

impl Default for GameOptions {
    fn default() -> Self {
        // The game's mix is loud at full scale; start at a quarter.
        Self { master_volume: 0.25, music_volume: 1.0, effects_volume: 1.0 }
    }
}

/// What a playing sound is, for its category volume.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SoundKind {
    Music,
    Effect,
}

impl GameOptions {
    fn file() -> std::path::PathBuf {
        artifacts_dir().join("options.txt")
    }

    pub fn load() -> Self {
        let mut o = Self::default();
        let Ok(text) = std::fs::read_to_string(Self::file()) else { return o };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let Ok(v) = v.trim().parse::<f32>() else { continue };
            let v = v.clamp(0.0, 1.0);
            match k.trim() {
                "master_volume" => o.master_volume = v,
                "music_volume" => o.music_volume = v,
                "effects_volume" => o.effects_volume = v,
                _ => {}
            }
        }
        o
    }

    pub fn save(&self) {
        let body = format!(
            "master_volume={}\nmusic_volume={}\neffects_volume={}\n",
            self.master_volume, self.music_volume, self.effects_volume
        );
        let file = Self::file();
        let result = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&file, body));
        if let Err(e) = result {
            warn!("couldn't save options to {}: {e}", file.display());
        }
    }

    /// The volume a new sound of `kind` plays at, before the master volume
    /// (Bevy's global volume multiplies that in when it starts).
    pub fn category(&self, kind: SoundKind) -> Volume {
        Volume::Linear(match kind {
            SoundKind::Music => self.music_volume,
            SoundKind::Effect => self.effects_volume,
        })
    }
}

/// `-` / `=` turn the master volume down / up by 5% (until the settings
/// menu exists).
fn volume_keys(keys: Res<ButtonInput<KeyCode>>, mut options: ResMut<GameOptions>) {
    let step = if keys.just_pressed(KeyCode::Minus) {
        -0.05
    } else if keys.just_pressed(KeyCode::Equal) {
        0.05
    } else {
        return;
    };
    options.master_volume = ((options.master_volume + step) * 20.0).round().clamp(0.0, 20.0) / 20.0;
    info!("master volume {:.0}%", options.master_volume * 100.0);
}

/// Saves the options and turns every playing sound to the new volume.
fn apply_options(
    options: Res<GameOptions>,
    mute: Res<Mute>,
    mut global: ResMut<GlobalVolume>,
    mut sinks: Query<(&mut AudioSink, &SoundKind)>,
) {
    options.save();
    let master = mute.master(&options);
    global.volume = Volume::Linear(master);
    for (mut sink, kind) in &mut sinks {
        let Volume::Linear(v) = options.category(*kind) else { continue };
        sink.set_volume(Volume::Linear(master * v));
    }
}
