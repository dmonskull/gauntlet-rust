//! The tower's own figures (`docs/items.md`, "The tower's wizard"). As
//! every tower level loads, the game makes the idle wizard — `GWIZ`, from
//! the tower's items bank — and stands him on the lookout whose parameter
//! is 0: the pedestal in front of the heroes' start on `levelL1`. He goes
//! through his first three actions in turn, each played to its end.

use bevy::prelude::*;
use gdl_formats::population::LocatorKind;

use crate::character::{Animator, CharacterModel};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::population::{self, LevelPopulation};
use crate::projectiles;
use crate::world::LevelEntity;

pub struct TowerPlugin;

impl Plugin for TowerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (place_wizard.run_if(resource_exists_and_changed::<LevelPopulation>), idle_wizard));
    }
}

/// The tower's realm.
const TOWER_REALM: u32 = 13;
/// His atree, in the tower's items bank.
const WIZARD_BANK: &str = "ITEMS/levelL";
const WIZARD_ATREE: &str = "GWIZ";
/// The lookout he stands on.
const WIZARD_LOOKOUT: u8 = 0;
/// He cycles through this many of his actions.
const IDLE_ACTIONS: usize = 3;

/// The idle wizard: the action he goes to next, and where his clip was a
/// frame ago (a looping clip is over when it starts round again).
#[derive(Component)]
struct TowerWizard {
    next: usize,
    last_frame: f32,
}

fn place_wizard(
    mut commands: Commands,
    population: Res<LevelPopulation>,
    mut game: ResMut<LoadedGame>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    if crate::quest::level_of(&population.level).is_none_or(|(realm, _)| realm != TOWER_REALM) {
        return;
    }
    let lookout = population
        .population
        .locators
        .iter()
        .find(|l| matches!(l.kind, LocatorKind::Lookout(_)) && l.param == WIZARD_LOOKOUT);
    let Some(lookout) = lookout else {
        warn!("{}: no lookout {WIZARD_LOOKOUT} for the wizard", population.level);
        return;
    };
    let Some(data) = projectiles::load_atree(&mut game, WIZARD_BANK, WIZARD_ATREE) else {
        warn!("{WIZARD_BANK}/{WIZARD_ATREE} didn't load");
        return;
    };
    let model = CharacterModel::build(&data, &mut meshes, &mut materials, &mut images);
    let transform = population::lookout_transform(lookout);
    let root = model.spawn(transform, &mut commands);
    commands.entity(root).insert((TowerWizard { next: 1, last_frame: 0.0 }, LevelEntity));
    info!("the wizard stands at {:?}", lookout.position);
}

/// Each of his actions plays out, then the next (0, 1, 2, 0…).
fn idle_wizard(mut wizards: Query<(&mut TowerWizard, &mut Animator)>) {
    for (mut w, mut animator) in &mut wizards {
        let looped = animator.frame < w.last_frame;
        w.last_frame = animator.frame;
        if animator.finished() || looped {
            let count = animator.clips.actions.len().clamp(1, IDLE_ACTIONS);
            let next = w.next % count;
            animator.play(next);
            w.next = next + 1;
            w.last_frame = 0.0;
        }
    }
}
