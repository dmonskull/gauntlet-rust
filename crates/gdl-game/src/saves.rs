//! Saved characters. The game keeps one "Gauntlet Save Data" file on the
//! memory card in slot A holding every character's record
//! (`docs/frontend.md`, "Saving"); here it's `characters.ron` in the
//! platform's data folder (`GDL_SAVE_DIR` overrides it). The tower's
//! Manage Character → Save writes the hero's record into it, and New/Load
//! → Load (or Manage Character → Load) brings a character back.
//!
//! A record holds what a character keeps between games: name, class and
//! colour, level and experience, health, gold, keys, potions, runestones,
//! the realms beaten, the quest's progress and the secret characters
//! unlocked. Stand-ins: the memory card
//! screens aren't drawn (the characters are listed in the player's column),
//! and experience isn't kept per class as the game's record does.

use std::path::PathBuf;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::player_state::PlayerState;
use crate::quest::Quest;

/// One saved character.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedCharacter {
    pub name: String,
    /// Three-letter class code (`WAR`) and colour (`YEL`).
    pub class: String,
    pub variant: String,
    pub level: u32,
    pub experience: u32,
    pub health: f32,
    pub gold: u32,
    pub keys: u32,
    pub potions: Vec<i32>,
    pub runestones: Vec<i32>,
    pub realms_beaten: u32,
    pub quest: Quest,
    /// Stat points bought in the shop.
    #[serde(default)]
    pub bought: crate::player_state::StatBonus,
    /// The final stats' totals.
    #[serde(default)]
    pub kills: u32,
    #[serde(default)]
    pub generators: u32,
    #[serde(default)]
    pub gold_found: u32,
    #[serde(default)]
    pub play_fields: u64,
    /// The secret characters unlocked (a bit per class from the ninth).
    #[serde(default)]
    pub secret_characters: u16,
}

impl SavedCharacter {
    /// The record of the hero playing now.
    pub fn of(name: &str, class: &str, variant: &str, state: &PlayerState) -> Self {
        Self {
            name: name.to_string(),
            class: class.to_string(),
            variant: variant.to_string(),
            level: state.level,
            experience: state.experience,
            health: state.health,
            gold: state.gold,
            keys: state.keys,
            potions: state.potions.clone(),
            runestones: state.runestones.clone(),
            realms_beaten: state.realms_beaten,
            quest: state.quest.clone(),
            bought: state.bought,
            kills: state.kills,
            generators: state.generators,
            gold_found: state.gold_found,
            play_fields: state.play_fields,
            secret_characters: state.secret_characters,
        }
    }

    /// Puts the record on a fresh hero of its class.
    pub fn apply(&self, state: &mut PlayerState) {
        state.level = self.level.max(1);
        state.experience = self.experience;
        state.gold = self.gold;
        state.keys = self.keys;
        state.potions = self.potions.clone();
        state.runestones = self.runestones.clone();
        state.realms_beaten = self.realms_beaten;
        state.quest = self.quest.clone();
        state.bought = self.bought;
        state.kills = self.kills;
        state.generators = self.generators;
        state.gold_found = self.gold_found;
        state.play_fields = self.play_fields;
        state.secret_characters = self.secret_characters;
        state.health = if self.health > 0.0 { self.health.min(state.max_health()) } else { state.max_health() };
    }

    /// Its line in the load list.
    pub fn label(&self) -> String {
        format!("{} Lv {}", self.name.replace('_', " "), self.level)
    }
}

/// The save file's contents.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveFile {
    pub characters: Vec<SavedCharacter>,
}

impl SaveFile {
    /// Adds the character, or replaces the one with its name.
    pub fn put(&mut self, c: SavedCharacter) {
        match self.characters.iter_mut().find(|k| k.name == c.name) {
            Some(k) => *k = c,
            None => self.characters.push(c),
        }
    }
}

/// The saved characters.
#[derive(Resource, Default)]
pub struct Saves {
    pub file: SaveFile,
}

impl Saves {
    pub fn read() -> Self {
        let file = match std::fs::read_to_string(path()) {
            Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
                warn!("{}: {e}", path().display());
                SaveFile::default()
            }),
            Err(_) => SaveFile::default(),
        };
        info!("{} saved characters in {}", file.characters.len(), path().display());
        Self { file }
    }

    /// Saves the character into the file, and the file to disk.
    pub fn save(&mut self, c: SavedCharacter) -> Result<(), String> {
        self.file.put(c);
        let text = ron::ser::to_string_pretty(&self.file, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
        let path = path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Where the save file lives: `GDL_SAVE_DIR`, else the platform's data
/// folder.
pub fn path() -> PathBuf {
    let dir = match std::env::var_os("GDL_SAVE_DIR") {
        Some(d) => PathBuf::from(d),
        None => data_dir().join("GauntletDarkLegacyRust"),
    };
    dir.join("characters.ron")
}

fn data_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home)
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
    }
}

pub struct SavesPlugin;

impl Plugin for SavesPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Saves::read());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_character_survives_the_round_trip() {
        let mut state = PlayerState::default();
        state.level = 7;
        state.gold = 1234;
        state.quest.crystals[1] = 12;
        state.quest.finish_level(7, 0);
        state.runestones = vec![0, 3];
        let saved = SavedCharacter::of("ANNA", "VAL", "BLU", &state);
        let mut file = SaveFile::default();
        file.put(saved.clone());
        let text = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default()).unwrap();
        let back: SaveFile = ron::from_str(&text).unwrap();
        assert_eq!(back, file);
        let mut fresh = PlayerState::default();
        back.characters[0].apply(&mut fresh);
        assert_eq!((fresh.level, fresh.gold, fresh.quest.crystals[1]), (7, 1234, 12));
        assert_eq!(fresh.runestones, vec![0, 3]);
        // Saving again under the same name replaces it.
        file.put(SavedCharacter { gold: 5, ..saved });
        assert_eq!(file.characters.len(), 1);
        assert_eq!(file.characters[0].gold, 5);
    }
}
