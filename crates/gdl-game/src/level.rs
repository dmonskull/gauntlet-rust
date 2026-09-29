//! Validates every level at boot and loads level data on demand.

use gdl_formats::{ModelFile, WorldFile, texture};
use gdl_install::GameInstall;

use bevy::prelude::*;

pub struct LevelSummary {
    pub name: String,
    pub objects: usize,
    pub triangles: usize,
    pub textures: usize,
    /// Textures using format selectors the game's own tables don't cover.
    pub unsupported_textures: usize,
}

/// A level's parsed data, ready to turn into meshes.
pub struct LevelData {
    pub name: String,
    pub model: ModelFile,
    pub textures: Vec<u8>,
    /// Model placements: (object index into `model.objects`, world position).
    pub placements: Vec<(usize, [f32; 3])>,
}

#[derive(Resource)]
pub struct LoadedGame {
    pub install: GameInstall,
    pub levels: Vec<LevelSummary>,
    /// Levels whose data failed to parse, with the reason.
    pub failures: Vec<(String, String)>,
    /// Index into `levels` of the level currently shown.
    pub current: usize,
}

impl LoadedGame {
    /// Parses every level fully (geometry and textures) so a broken level
    /// is reported up front rather than when someone walks into it.
    pub fn load(mut install: GameInstall, wanted_level: Option<&str>) -> Result<Self, String> {
        let names = install.levels.clone();
        if names.is_empty() {
            return Err("The game data has no levels.".into());
        }

        let mut levels = Vec::new();
        let mut failures = Vec::new();
        for name in &names {
            match load_level(&mut install, name).map(|data| summarize(&data)) {
                Ok(summary) => levels.push(summary),
                Err(why) => failures.push((name.clone(), why)),
            }
        }
        if levels.is_empty() {
            return Err(format!("No level could be loaded. First failure: {:?}", failures.first()));
        }

        let current = match wanted_level {
            Some(want) => levels
                .iter()
                .position(|l| l.name.eq_ignore_ascii_case(want))
                .ok_or_else(|| match failures.iter().find(|(n, _)| n.eq_ignore_ascii_case(want)) {
                    Some((n, why)) => format!("Level {n} failed to load: {why}"),
                    None => format!("No level named '{want}'. Levels: {}", names.join(", ")),
                })?,
            None => levels.iter().position(|l| l.name == "levelA1").unwrap_or(0),
        };

        Ok(Self { install, levels, failures, current })
    }

    pub fn current_name(&self) -> &str {
        &self.levels[self.current].name
    }

    pub fn load_current(&mut self) -> Result<LevelData, String> {
        let name = self.levels[self.current].name.clone();
        load_level(&mut self.install, &name)
    }

    pub fn summary_line(&self) -> String {
        let triangles: usize = self.levels.iter().map(|l| l.triangles).sum();
        let textures: usize = self.levels.iter().map(|l| l.textures).sum();
        format!(
            "Loaded {}/{} levels: {} triangles, {} textures",
            self.levels.len(),
            self.levels.len() + self.failures.len(),
            triangles,
            textures
        )
    }
}

pub fn load_level(install: &mut GameInstall, name: &str) -> Result<LevelData, String> {
    let objects = install
        .read(&format!("LEVELS/{name}/objects.ngc"))
        .map_err(|e| e.to_string())?;
    let textures = install
        .read(&format!("LEVELS/{name}/textures.ngc"))
        .map_err(|e| e.to_string())?;
    let world_file = install
        .read(&format!("LEVELS/{name}/WORLDS.PS2"))
        .map_err(|e| e.to_string())?;
    let model = ModelFile::parse(&objects).map_err(|e| format!("objects.ngc: {e}"))?;
    let world = WorldFile::parse(&world_file).map_err(|e| format!("WORLDS.PS2: {e}"))?;
    let positions = world.world_positions().map_err(|e| format!("WORLDS.PS2: {e}"))?;

    // Nodes find their model by name, like the game does.
    let by_name: std::collections::HashMap<&str, usize> =
        model.objects.iter().enumerate().map(|(i, o)| (o.name.as_str(), i)).collect();
    let placements = world
        .nodes
        .iter()
        .zip(positions)
        .filter(|(n, _)| n.has_model)
        .filter_map(|(n, p)| Some((*by_name.get(n.name.as_str())?, p?)))
        .collect();

    Ok(LevelData { name: name.to_string(), model, textures, placements })
}

fn summarize(level: &LevelData) -> LevelSummary {
    let mut textures = 0;
    let mut unsupported_textures = 0;
    for b in level.model.bindings.iter().filter(|b| b.is_textured()) {
        match texture::decode(&level.textures, b) {
            Ok(_) => textures += 1,
            Err(_) => unsupported_textures += 1,
        }
    }
    LevelSummary {
        name: level.name.clone(),
        objects: level.placements.len(),
        triangles: level
            .placements
            .iter()
            .flat_map(|&(i, _)| &level.model.objects[i].submeshes)
            .map(|s| s.triangles.len())
            .sum(),
        textures,
        unsupported_textures,
    }
}
