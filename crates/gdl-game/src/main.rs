//! Gauntlet: Dark Legacy runtime. Point it at your own copy of the game —
//! disc image, extracted folder or main.dol — and it finds, validates and
//! loads the rest. No game assets ship with this program.

mod autoshot;
mod bootstrap;
mod level;

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

    let game = match LoadedGame::load(install, args.level.as_deref()) {
        Ok(game) => game,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    println!("{}", game.summary_line());

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("Gauntlet: Dark Legacy — {}", game.current_level),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
        .add_plugins(autoshot::AutoShotPlugin)
        .insert_resource(game)
        .add_systems(Startup, level::spawn_boot_screen)
        .run();
}
