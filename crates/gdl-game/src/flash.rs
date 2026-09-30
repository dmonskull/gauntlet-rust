//! Hit flashes (`docs/rendering.md`, "Texture overrides"): a struck body is
//! drawn for two of its updates through `AAAWHITE` as a texture over it —
//! the rasterized colour × that texture × 2, so it glows near white in the
//! shape of its lit self. Monsters flash on a blow they live through,
//! heroes on one of more than a point, critters (and their parts) on the
//! blows that reach past their hit spheres. A struck obstacle instead
//! shows `AAAWHITE` in place of its root object's texture, and its whole
//! model skips the lightmap, for one update.
//!
//! `AAAWHITE` is one colour (an 8×8 texture of 189, 231, 189 in every bank
//! that has it), so the flash rides on the meshes' [`MeshTag`] —
//! `level.wgsl` reads the colour and what to do with it from the tag —
//! instead of on copies of their materials.
//!
//! The same slot on a hero shows the invulnerability power-ups' chrome
//! (`CHROMESILVER`, `CHROMEGOLD`): a real texture in place of every part's
//! own, so that one draws through copies of the body's materials
//! (`fade.rs::BodyLook`).

use bevy::mesh::MeshTag;
use bevy::prelude::*;
use gdl_formats::ModelFile;

use crate::character::Animator;
use crate::level::LoadedGame;

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlashColours>().add_systems(Update, load_powerups_white);
    }
}

/// The tag bits `level.wgsl` reads: the colour (0xRRGGBB) drawn over the
/// object, in place of its texture, and the lightmap stage skipped.
pub const TAG_OVER: u32 = 1 << 24;
pub const TAG_REPLACE: u32 = 1 << 25;
pub const TAG_NO_LIGHTMAP: u32 = 1 << 26;

/// `AAAWHITE`'s colour (0xRRGGBB): the shared powerups bank's, which the
/// game flashes bodies with, and the level's own, for its obstacles (none
/// where the bank hasn't one: `levelT4`). And the powerups bank's chrome
/// textures, silver and gold.
#[derive(Resource, Default)]
pub struct FlashColours {
    pub powerups: Option<u32>,
    pub level: Option<u32>,
    pub chrome: [Option<Handle<Image>>; 2],
    tried: bool,
}

impl FlashColours {
    /// A body's flash tag, when the colour's known.
    pub fn body(&self) -> Option<u32> {
        self.powerups.map(|c| c | TAG_OVER)
    }
}

/// `AAAWHITE`'s colour in a bank, when it has one and it is one colour.
pub fn white_of(model: &ModelFile, textures: &[u8]) -> Option<u32> {
    let name = model.texture_names.iter().find(|t| t.name == "AAAWHITE")?;
    let binding = model.bindings.get(usize::from(name.binding))?;
    let image = gdl_formats::texture::decode(textures, binding).ok()?;
    let (texels, _) = image.pixels.as_chunks::<4>();
    let first = *texels.first()?;
    if texels.iter().any(|t| *t != first) {
        warn!("AAAWHITE isn't one colour; hit flashes are off");
        return None;
    }
    Some(u32::from(first[0]) << 16 | u32::from(first[1]) << 8 | u32::from(first[2]))
}

/// The chrome textures' names, silver then gold.
const CHROME: [&str; 2] = ["CHROMESILVER", "CHROMEGOLD"];

/// Reads the powerups bank's `AAAWHITE` and chrome once the game data is
/// there.
fn load_powerups_white(
    mut colours: ResMut<FlashColours>,
    game: Option<ResMut<LoadedGame>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut game) = game else { return };
    if colours.tried {
        return;
    }
    colours.tried = true;
    let install = &mut game.install;
    let bank = (|| {
        let model = ModelFile::parse(&install.read("POWERUPS/objects.ngc").ok()?).ok()?;
        Some((model, install.read("POWERUPS/textures.ngc").ok()?))
    })();
    let Some((model, textures)) = bank else {
        warn!("hit flashes: no powerups bank");
        return;
    };
    colours.powerups = white_of(&model, &textures);
    match colours.powerups {
        Some(c) => info!("hit flashes: AAAWHITE is {c:06x}"),
        None => warn!("hit flashes: the powerups bank has no AAAWHITE"),
    }
    let mut cache = crate::model_mesh::TextureCache::new(&model, &textures);
    for (slot, name) in colours.chrome.iter_mut().zip(CHROME) {
        let binding = model.texture_names.iter().find(|t| t.name == name).map(|t| t.binding);
        *slot = binding.and_then(|b| cache.get(b, &mut images)).map(|(image, _)| image);
        if slot.is_none() {
            warn!("the powerups bank has no {name}");
        }
    }
}

/// How many of its owner's updates a flash shows for.
pub const FLASH_UPDATES: u8 = 2;

/// A body's hit flash: once started it shows for the owner's next two
/// updates (the game's timed texture effect with `AAAWHITE`, a step of 1
/// to an end of 1, repeated once; a critter's flash count of 2).
#[derive(Clone, Copy, Debug, Default)]
pub struct Flash {
    left: u8,
    on: bool,
}

impl Flash {
    pub fn start(&mut self) {
        self.left = FLASH_UPDATES;
    }

    /// Its owner's update: shown while any are left. True when it came on
    /// or went off.
    pub fn step(&mut self) -> bool {
        let on = self.left > 0;
        self.left = self.left.saturating_sub(1);
        std::mem::replace(&mut self.on, on) != on
    }

    /// Off at once (a death texture takes over). True when it was on.
    pub fn stop(&mut self) -> bool {
        self.left = 0;
        std::mem::replace(&mut self.on, false)
    }

    pub fn on(&self) -> bool {
        self.on
    }
}

/// Sets the tag of a body's meshes on the nodes `nodes` picks.
pub fn tag_body(
    animator: &Animator,
    nodes: impl Fn(usize) -> bool,
    tag: u32,
    tags: &mut Query<&mut MeshTag>,
    commands: &mut Commands,
) {
    for &(node, e) in animator.meshes() {
        if nodes(node) {
            set_tag(e, tag, tags, commands);
        }
    }
}

/// Sets one mesh's tag (inserting it the first time).
pub fn set_tag(e: Entity, tag: u32, tags: &mut Query<&mut MeshTag>, commands: &mut Commands) {
    match tags.get_mut(e) {
        Ok(mut t) => {
            if t.0 != tag {
                t.0 = tag;
            }
        }
        Err(_) if tag != 0 => {
            commands.entity(e).try_insert(MeshTag(tag));
        }
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_shows_for_two_updates() {
        let mut f = Flash::default();
        f.start();
        assert!(f.step() && f.on());
        assert!(!f.step() && f.on());
        assert!(f.step() && !f.on());
        assert!(!f.step() && !f.on());
        // Struck again while it shows: two more from then.
        f.start();
        f.step();
        f.start();
        f.step();
        f.step();
        assert!(f.on());
        f.step();
        assert!(!f.on());
    }

    #[test]
    fn aaawhite_is_one_colour() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("POWERUPS");
        let (Ok(model), Ok(textures)) = (std::fs::read(dir.join("objects.ngc")), std::fs::read(dir.join("textures.ngc")))
        else {
            return;
        };
        let model = ModelFile::parse(&model).unwrap();
        assert_eq!(white_of(&model, &textures), Some(0xBDE7BD));
    }
}
