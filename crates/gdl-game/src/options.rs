//! Player options, saved beside the remembered game path
//! (`gdl-artifacts/options.txt`, one `key=value` per line) and applied live,
//! so a settings menu only has to change [`GameOptions`]. `GDL_MUTE=1`
//! silences one run (tests) without touching the saved volumes.
//!
//! Besides the game's own (volumes, the control style, rumble, auto aim
//! and attack, the compass) they hold the PC settings the game doesn't
//! have: key, mouse and pad bindings (`controls.rs`), the window (full
//! screen, vsync) and the debugging aids.

use std::sync::atomic::{AtomicBool, Ordering};

use bevy::audio::{AudioSink, AudioSinkPlayback, GlobalVolume, Volume};
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode};

use crate::bootstrap::artifacts_dir;
use crate::controls::{self, Action, Bindings};
use crate::party::MAX_PLAYERS;

/// Six dB of output headroom for simultaneous effects and loud voice calls.
/// Applied once to both new and already playing sounds, after the user mix.
const OUTPUT_HEADROOM: f32 = 0.5;

pub struct OptionsPlugin;

impl Plugin for OptionsPlugin {
    fn build(&self, app: &mut App) {
        let options = GameOptions::load();
        let mute = Mute(std::env::var("GDL_MUTE").is_ok_and(|v| !v.is_empty() && v != "0"));
        DEV_KEYS.store(dev_keys_env() || options.dev_keys, Ordering::Relaxed);
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
        if self.0 { 0.0 } else { options.master_volume * OUTPUT_HEADROOM }
    }
}

/// Whether the developer keys work: the option, or `GDL_DEV_KEYS=1`.
static DEV_KEYS: AtomicBool = AtomicBool::new(false);

fn dev_keys_env() -> bool {
    std::env::var("GDL_DEV_KEYS").is_ok_and(|v| !v.is_empty() && v != "0")
}

pub fn dev_keys_on() -> bool {
    DEV_KEYS.load(Ordering::Relaxed)
}

/// Volumes are linear, 0 (silent) to 1 (the game's own level).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct GameOptions {
    pub master_volume: f32,
    pub music_volume: f32,
    pub effects_volume: f32,
    /// The game's Controls menu, for each player (the game keeps them per
    /// pad).
    pub players: [PlayerOptions; MAX_PLAYERS],
    /// The game's Compass menu (Show / Hide).
    pub compass: bool,
    pub bindings: Bindings,
    /// The window (not the game's): full screen and vsync.
    pub fullscreen: bool,
    pub vsync: bool,
    /// Debugging aids (not the game's): the developer keys, the status
    /// overlay, the frame rate, the collision overlay.
    pub dev_keys: bool,
    pub debug_overlay: bool,
    pub frame_rate: bool,
    pub collision: bool,
    /// Online (the host's choice): each player's camera follows their own
    /// hero; off, everyone shares the game's co-op camera over the level.
    pub online_cameras: bool,
}

impl Default for GameOptions {
    fn default() -> Self {
        Self {
            // The game's mix is loud at full scale; start at a quarter.
            master_volume: 0.25,
            music_volume: 1.0,
            effects_volume: 1.0,
            players: [PlayerOptions::default(); MAX_PLAYERS],
            compass: false,
            bindings: Bindings::default(),
            fullscreen: false,
            vsync: true,
            dev_keys: false,
            debug_overlay: false,
            frame_rate: false,
            collision: false,
            online_cameras: true,
        }
    }
}

/// A player's controls as the game's Controls menu sets them: the style
/// (`controls::SCHEME_NAMES`), the pad's rumble, attack aim (attacking in
/// place turns toward the target) and walk-into attack — on by default, as
/// the game's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerOptions {
    pub scheme: usize,
    pub rumble: bool,
    pub auto_aim: bool,
    pub auto_attack: bool,
    /// This player alone uses an eye-level view. Online this is local to their machine.
    pub first_person: bool,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        Self { scheme: 0, rumble: true, auto_aim: true, auto_attack: true, first_person: false }
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
            o.set(k.trim(), v.trim());
        }
        o
    }

    /// A player's controls.
    pub fn player(&self, slot: usize) -> PlayerOptions {
        self.players.get(slot).copied().unwrap_or_default()
    }

    /// Reads one saved `key=value`.
    fn set(&mut self, k: &str, v: &str) {
        let num = |v: &str| v.parse::<f32>().ok().filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0));
        let flag = |v: &str| match v {
            "1" | "on" | "true" => Some(true),
            "0" | "off" | "false" => Some(false),
            _ => None,
        };
        match k {
            "master_volume" => self.master_volume = num(v).unwrap_or(self.master_volume),
            "music_volume" => self.music_volume = num(v).unwrap_or(self.music_volume),
            "effects_volume" => self.effects_volume = num(v).unwrap_or(self.effects_volume),
            // Before each player had their own: player 1's.
            "scheme" | "rumble" | "auto_aim" | "auto_attack" => self.set(&format!("p1.{k}"), v),
            "compass" => self.compass = flag(v).unwrap_or(self.compass),
            "fullscreen" => self.fullscreen = flag(v).unwrap_or(self.fullscreen),
            "vsync" => self.vsync = flag(v).unwrap_or(self.vsync),
            "dev_keys" => self.dev_keys = flag(v).unwrap_or(self.dev_keys),
            "debug_overlay" => self.debug_overlay = flag(v).unwrap_or(self.debug_overlay),
            "frame_rate" => self.frame_rate = flag(v).unwrap_or(self.frame_rate),
            "collision" => self.collision = flag(v).unwrap_or(self.collision),
            "online_cameras" => self.online_cameras = flag(v).unwrap_or(self.online_cameras),
            _ => {
                if let Some((n, field)) = k.strip_prefix('p').and_then(|r| r.split_once('.'))
                    && let Some(o) = n.parse::<usize>().ok().and_then(|n| self.players.get_mut(n.checked_sub(1)?))
                {
                    match field {
                        "scheme" => o.scheme = v.parse::<usize>().ok().filter(|&s| s < controls::SCHEME_NAMES.len()).unwrap_or(0),
                        "rumble" => o.rumble = flag(v).unwrap_or(o.rumble),
                        "auto_aim" => o.auto_aim = flag(v).unwrap_or(o.auto_aim),
                        "auto_attack" => o.auto_attack = flag(v).unwrap_or(o.auto_attack),
                        "first_person" => o.first_person = flag(v).unwrap_or(o.first_person),
                        _ => {}
                    }
                } else if let Some(name) = k.strip_prefix("key.")
                    && let Some(action) = Action::ALL.into_iter().find(|a| a.key() == name)
                {
                    let inputs: Vec<_> = v.split(',').filter_map(|n| controls::input_from_name(n.trim())).collect();
                    if let Some(slot) = self.bindings.keys.iter_mut().find(|(a, _)| *a == action) {
                        slot.1 = inputs;
                    }
                } else if let Some(name) = k.strip_prefix("pad.")
                    && let Some(action) = Action::ALL.into_iter().find(|a| a.key() == name)
                    && let Some(b) = controls::pad_from_name(v)
                {
                    self.bindings.bind_pad(action, b);
                }
            }
        }
    }

    pub fn save(&self) {
        let flag = |b: bool| if b { "1" } else { "0" };
        let mut body = format!(
            "master_volume={}\nmusic_volume={}\neffects_volume={}\ncompass={}\nfullscreen={}\nvsync={}\ndev_keys={}\ndebug_overlay={}\nframe_rate={}\ncollision={}\nonline_cameras={}\n",
            self.master_volume,
            self.music_volume,
            self.effects_volume,
            flag(self.compass),
            flag(self.fullscreen),
            flag(self.vsync),
            flag(self.dev_keys),
            flag(self.debug_overlay),
            flag(self.frame_rate),
            flag(self.collision),
            flag(self.online_cameras),
        );
        for (i, o) in self.players.iter().enumerate() {
            let n = i + 1;
            body += &format!(
                "p{n}.scheme={}\np{n}.rumble={}\np{n}.auto_aim={}\np{n}.auto_attack={}\np{n}.first_person={}\n",
                o.scheme,
                flag(o.rumble),
                flag(o.auto_aim),
                flag(o.auto_attack),
                flag(o.first_person)
            );
        }
        for (action, inputs) in &self.bindings.keys {
            let names: Vec<&str> = inputs.iter().map(|&i| controls::input_name(i)).collect();
            body += &format!("key.{}={}\n", action.key(), names.join(","));
        }
        for &(action, b) in &self.bindings.pad {
            body += &format!("pad.{}={}\n", action.key(), controls::pad_name(b));
        }
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

/// `-` / `=` turn the master volume down / up by 5%.
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

/// Saves the options and applies them: every playing sound's volume, the
/// window's mode and vsync, the developer keys.
fn apply_options(
    options: Res<GameOptions>,
    mute: Res<Mute>,
    mut global: ResMut<GlobalVolume>,
    mut sinks: Query<(&mut AudioSink, &SoundKind)>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    options.save();
    let master = mute.master(&options);
    global.volume = Volume::Linear(master);
    for (mut sink, kind) in &mut sinks {
        let Volume::Linear(v) = options.category(*kind) else { continue };
        sink.set_volume(Volume::Linear(master * v));
    }
    DEV_KEYS.store(dev_keys_env() || options.dev_keys, Ordering::Relaxed);
    if let Ok(mut window) = windows.single_mut() {
        let mode = if options.fullscreen { WindowMode::BorderlessFullscreen(MonitorSelection::Current) } else { WindowMode::Windowed };
        if window.mode != mode {
            window.mode = mode;
        }
        let present = if options.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
        if window.present_mode != present {
            window.present_mode = present;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::Input;

    #[test]
    fn options_read_back_what_they_write() {
        let mut o = GameOptions { fullscreen: true, ..GameOptions::default() };
        o.players[2] = PlayerOptions { scheme: 1, rumble: false, first_person: true, ..PlayerOptions::default() };
        o.bindings.bind_key(Action::Magic, Input::Key(KeyCode::KeyK));
        o.bindings.bind_pad(Action::Combo, bevy::input::gamepad::GamepadButton::LeftTrigger);
        // What save writes, line by line, read into fresh options.
        let flag = |b: bool| if b { "1" } else { "0" };
        let mut lines = vec![
            format!("p3.scheme={}", o.players[2].scheme),
            format!("p3.rumble={}", flag(o.players[2].rumble)),
            format!("p3.first_person={}", flag(o.players[2].first_person)),
            format!("fullscreen={}", flag(o.fullscreen)),
            // An old file's player 1.
            "scheme=2".to_string(),
        ];
        for (action, inputs) in &o.bindings.keys {
            let names: Vec<&str> = inputs.iter().map(|&i| controls::input_name(i)).collect();
            lines.push(format!("key.{}={}", action.key(), names.join(",")));
        }
        for &(action, b) in &o.bindings.pad {
            lines.push(format!("pad.{}={}", action.key(), controls::pad_name(b)));
        }
        let mut back = GameOptions::default();
        for l in &lines {
            let (k, v) = l.split_once('=').unwrap();
            back.set(k, v);
        }
        assert_eq!((back.players[2].scheme, back.players[2].rumble, back.fullscreen), (1, false, true));
        assert!(back.players[2].first_person);
        assert!(!back.players[0].first_person);
        assert_eq!(back.players[0].scheme, 2);
        assert_eq!(back.players[1], PlayerOptions::default());
        assert_eq!(back.bindings, o.bindings);
    }
    #[test]
    fn audio_headroom_and_mute_apply_without_changing_the_saved_mix() {
        let mut o = GameOptions::default();
        assert_eq!(Mute(false).master(&o), 0.125);
        assert_eq!(Mute(true).master(&o), 0.0);
        o.set("master_volume", "NaN");
        assert_eq!(o.master_volume, 0.25);
        o.set("effects_volume", "2");
        assert_eq!(o.effects_volume, 1.0);
    }

}
