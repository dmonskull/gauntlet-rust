//! Gauntlet: Dark Legacy runtime. Point it at your own copy of the game —
//! disc image, extracted folder or main.dol — and it finds, validates and
//! loads the rest. No game assets ship with this program.

mod actions;
mod audio;
mod autoshot;
mod billboard;
mod breakables;
mod bootstrap;
mod camera;
mod camera_rig;
mod character;
mod collision_debug;
mod combat;
mod critters;
mod damage;
mod deaths;
mod effects;
mod exits;
mod font;
mod frontend;
mod game_hud;
mod generators;
mod hazards;
mod hints;
mod hud;
mod items;
mod level;
mod level_material;
mod locomotion;
mod mechanics;
mod model_mesh;
mod options;
mod monsters;
mod particles;
mod play_camera;
mod player;
mod player_state;
mod texanim;
mod population;
mod projectiles;
mod status_hud;
mod viewer;
mod world;

use bevy::prelude::*;

use bootstrap::Args;
use level::LoadedGame;

/// With `GDL_FPS`, names every frame over 20 ms (hitches).
fn log_slow_frames(time: Res<Time<Real>>, mut frame: Local<u64>) {
    *frame += 1;
    let dt = time.delta_secs() * 1000.0;
    if *frame > 60 && dt > 20.0 {
        info!("slow frame {}: {dt:.1} ms", *frame);
    }
}

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
        billboard::BillboardPlugin,
        options::OptionsPlugin,
        texanim::TexAnimPlugin,
        particles::ParticlesPlugin,
        autoshot::AutoShotPlugin,
    ));

    // GDL_FPS=1 logs frame rate and frame time every second.
    if std::env::var("GDL_FPS").is_ok_and(|v| !v.is_empty() && v != "0") {
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::diagnostic::LogDiagnosticsPlugin::default(),
        ))
        .add_systems(Last, log_slow_frames);
    }

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
        // The front end runs unless the command line picked a level or a
        // hero; a new game starts in the tower hub.
        let skip_menus = args.level.is_some() || args.character.is_some();
        let first_level = args.level.as_deref().or((!skip_menus).then_some(frontend::TOWER));
        let game = match LoadedGame::load(install, first_level) {
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
            .add_plugins((world::WorldPlugin, hud::HudPlugin, player::PlayerPlugin, combat::CombatPlugin, damage::DamagePlugin, play_camera::PlayCameraPlugin, audio::GameAudioPlugin, population::PopulationPlugin, collision_debug::CollisionDebugPlugin))
            .add_plugins((monsters::MonstersPlugin, projectiles::ProjectilesPlugin, critters::CrittersPlugin, effects::EffectsPlugin, deaths::DeathsPlugin))
            .add_plugins((
                player_state::PlayerStatePlugin,
                items::ItemsPlugin,
                mechanics::MechanicsPlugin,
                hazards::HazardsPlugin,
                breakables::BreakablesPlugin,
                exits::ExitsPlugin,
                hints::HintsPlugin,
                status_hud::StatusHudPlugin,
            ))
            .add_plugins((font::Screen2dPlugin, frontend::FrontendPlugin { skip: skip_menus }, game_hud::GameHudPlugin));
    }
    app.run();
}
