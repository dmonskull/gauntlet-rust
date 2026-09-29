//! Loads and validates every level's model data at boot, and holds the game
//! install for later on-demand reads.

use std::io::Cursor;

use bevy::prelude::*;
use gdl_formats::{MaterialBinding, ModelHeader};
use gdl_install::GameInstall;

pub struct LevelSummary {
    pub name: String,
    pub objects: u32,
    pub bindings: usize,
    pub textured: usize,
    pub texture_bytes: usize,
}

#[derive(Resource)]
pub struct LoadedGame {
    pub install: GameInstall,
    pub levels: Vec<LevelSummary>,
    /// Levels whose data failed to parse, with the reason.
    pub failures: Vec<(String, String)>,
    pub current_level: String,
}

impl LoadedGame {
    pub fn load(mut install: GameInstall, wanted_level: Option<&str>) -> Result<Self, String> {
        let names = install.levels.clone();
        if names.is_empty() {
            return Err("The game data has no levels.".into());
        }

        let current_level = match wanted_level {
            Some(want) => names
                .iter()
                .find(|n| n.eq_ignore_ascii_case(want))
                .cloned()
                .ok_or_else(|| format!("No level named '{want}'. Levels: {}", names.join(", ")))?,
            None => names
                .iter()
                .find(|n| n.eq_ignore_ascii_case("levelA1"))
                .unwrap_or(&names[0])
                .clone(),
        };

        let mut levels = Vec::new();
        let mut failures = Vec::new();
        for name in &names {
            match load_level(&mut install, name) {
                Ok(summary) => levels.push(summary),
                Err(why) => failures.push((name.clone(), why)),
            }
        }
        if failures.iter().any(|(n, _)| *n == current_level) {
            let why = &failures.iter().find(|(n, _)| *n == current_level).unwrap().1;
            return Err(format!("Level {current_level} failed to load: {why}"));
        }

        Ok(Self { install, levels, failures, current_level })
    }

    pub fn summary_line(&self) -> String {
        let bindings: usize = self.levels.iter().map(|l| l.bindings).sum();
        format!(
            "Loaded {}/{} levels ({} material bindings); starting in {}",
            self.levels.len(),
            self.levels.len() + self.failures.len(),
            bindings,
            self.current_level
        )
    }
}

fn load_level(install: &mut GameInstall, name: &str) -> Result<LevelSummary, String> {
    let objects = install
        .read(&format!("LEVELS/{name}/objects.ngc"))
        .map_err(|e| e.to_string())?;
    let texture_bytes = install
        .read(&format!("LEVELS/{name}/textures.ngc"))
        .map_err(|e| e.to_string())?
        .len();

    let mut cursor = Cursor::new(&objects);
    let header = ModelHeader::read_from(&mut cursor).map_err(|e| e.to_string())?;
    let bindings = MaterialBinding::read_all_from(&mut cursor, &header).map_err(|e| e.to_string())?;

    let mut textured = 0;
    for (i, b) in bindings.iter().enumerate().filter(|(_, b)| b.is_textured()) {
        if b.texture_offset as usize >= texture_bytes {
            return Err(format!("binding {i} points past the end of textures.ngc"));
        }
        textured += 1;
    }

    Ok(LevelSummary {
        name: name.to_string(),
        objects: header.num_objects,
        bindings: bindings.len(),
        textured,
        texture_bytes,
    })
}

pub fn spawn_boot_screen(mut commands: Commands, game: Res<LoadedGame>) {
    commands.spawn(Camera2d);

    let install = &game.install;
    let mut text = format!(
        "{}  [{}]\n{}: {}\n\n{}\n",
        install.title.as_deref().unwrap_or("Gauntlet: Dark Legacy"),
        install.game_id.as_deref().unwrap_or("id unknown"),
        install.source_kind(),
        install.origin.display(),
        game.summary_line(),
    );
    if let Some(level) = game.levels.iter().find(|l| l.name == game.current_level) {
        text += &format!(
            "\n{}: {} objects, {} material bindings ({} textured), {} KiB of textures\n",
            level.name,
            level.objects,
            level.bindings,
            level.textured,
            level.texture_bytes / 1024
        );
    }
    for (name, why) in &game.failures {
        text += &format!("\nfailed: {name}: {why}");
    }
    // Bevy's built-in font is ASCII-only; keep on-screen text ASCII.
    text += "\n\nLevel geometry isn't reverse engineered yet - see docs/INDEX.md.";

    commands.spawn((
        Text::new(text),
        TextFont { font_size: 18.0, ..default() },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(24.0),
            left: Val::Px(24.0),
            ..default()
        },
    ));
}
