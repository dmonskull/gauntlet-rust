//! Gauntlet: Dark Legacy runtime. Point it at your own copy of the game —
//! disc image, extracted folder or main.dol — and it finds, validates and
//! loads the rest. No game assets ship with this program.

mod audio;
mod autoshot;
mod bootstrap;
mod camera;
mod camera_rig;
mod character;
mod collision_debug;
mod exits;
mod hints;
mod hud;
mod items;
mod level;
mod level_material;
mod locomotion;
mod model_mesh;
mod play_camera;
mod player;
mod player_state;
mod population;
mod status_hud;
mod viewer;
mod world;

use bevy::prelude::*;

use bootstrap::Args;
use level::LoadedGame;

fn main() {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(message) => {
            println!("{message}");
            return;
        }
    };

    let install = match bootstrap::resolve(&args) {
        Ok(install) => install,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    println!(
        "Game: {} ({}) from {} {}",
        install.title.as_deref().unwrap_or("Gauntlet: Dark Legacy"),
        install.game_id.as_deref().unwrap_or("id unknown"),
        install.source_kind(),
        install.origin.display()
    );
    if let Some(warning) = &install.warning {
        eprintln!("warning: {warning}");
    }

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window { title: "Gauntlet: Dark Legacy".into(), ..default() }),
        ..default()
    }))
    .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
    .add_plugins((
        level_material::LevelMaterialPlugin,
        camera::CameraPlugin,
        character::CharacterPlugin,
        autoshot::AutoShotPlugin,
    ));

    if args.viewer {
        let viewer = viewer::Viewer::new(
            install,
            args.character.as_deref(),
            args.monster.as_deref(),
            args.variant.as_deref(),
            args.action.as_deref(),
        );
        match viewer {
            Ok(v) => app.insert_resource(v).add_plugins(viewer::ViewerPlugin),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        };
    } else {
        let game = match LoadedGame::load(install, args.level.as_deref()) {
            Ok(game) => game,
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        };
        println!("{}; starting in {}", game.summary_line(), game.current_name());
        for (name, why) in &game.failures {
            eprintln!("warning: level {name} failed to load: {why}");
        }
        let choice = player::PlayerChoice {
            class: args.character.as_deref().unwrap_or("WAR").to_ascii_uppercase(),
            variant: args.variant.as_deref().unwrap_or("BLU").to_ascii_uppercase(),
        };
        app.insert_resource(game)
            .insert_resource(choice)
            .add_plugins((world::WorldPlugin, hud::HudPlugin, player::PlayerPlugin, play_camera::PlayCameraPlugin, audio::GameAudioPlugin, population::PopulationPlugin, collision_debug::CollisionDebugPlugin))
            .add_plugins((
                player_state::PlayerStatePlugin,
                items::ItemsPlugin,
                exits::ExitsPlugin,
                hints::HintsPlugin,
                status_hud::StatusHudPlugin,
            ));
    }
    app.run();
}
