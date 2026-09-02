//! # Loop Pump — per-frame orchestration glue
//!
//! This module holds the small per-frame policy functions that the run
//! loop calls between event handling and rendering. They are not
//! "business logic" in any deep sense — they're the "should the music
//! change?" and "should we load the mission map now?" decisions.
//! Pulled out of `mod.rs` so the run loop reads as a clean dispatcher
//! and these policies can grow without inflating the entry point.
//!
//! Each function takes the minimum state it needs and returns nothing;
//! the side effects are visible through the mutable borrows.

use std::path::Path;

use sdl2::render::TextureCreator;
use sdl2::video::WindowContext;
use tracing::{debug, info};

use ow_core::ruleset::Ruleset;

use super::asset_loader::{
    load_mission, load_palette_pcx, load_soldier_animation, mission_number_from_name,
};
use super::audio_handles::AudioHandles;
use super::mission::MissionData;
use super::music::{music_track_for_phase_with_mission, start_music, stop_music};
use super::soldier_anims::SoldierAnims;
use super::{GameLoop, PhaseHandler};

/// Compare the desired music track for the current phase to the one
/// currently loaded, and swap if they differ. Skip while paused.
/// Honours the `music_broken` latch — once MIDI failed to load (no
/// SoundFont), we stop retrying for the rest of the session.
pub fn maybe_transition_music(game: &GameLoop, audio: &mut AudioHandles, data_dir: &Path) {
    if !audio.audio_available {
        return;
    }
    // Pause: don't touch music at all.
    if matches!(game.phase_handler, PhaseHandler::Paused { .. }) {
        return;
    }

    let mission_num = mission_number_from_name(
        game.game_state
            .current_mission
            .as_ref()
            .map(|m| m.name.as_str()),
    );
    let wanted = music_track_for_phase_with_mission(&game.phase_handler, Some(mission_num));
    let need_change = match (&wanted, &audio.music_track) {
        (Some(w), Some(c)) => w.as_str() != c.as_str(),
        (Some(_), None) => true,
        (None, Some(_)) => true,
        (None, None) => false,
    };
    if !need_change {
        return;
    }

    stop_music();
    if let Some(track_name) = &wanted {
        let midi_dir = data_dir.join("WOW").join("MIDI");
        let handle = start_music(&midi_dir, track_name, &mut audio.music_broken);
        audio.music_track = if handle.is_some() {
            Some(track_name.clone())
        } else {
            None
        };
        audio._music_handle = handle;
    } else {
        audio._music_handle = None;
        audio.music_track = None;
    }
}

/// Load the mission map, tileset, OBJ sprites, soldier animation, and
/// enemy units the first time the player enters Deployment. Idempotent
/// — once `mission` is `Some`, this is a no-op for the rest of the
/// session. On success also creates one AnimController per merc and
/// re-centres the camera on the map.
pub fn maybe_load_mission<'a>(
    game: &mut GameLoop,
    mission: &mut Option<MissionData<'a>>,
    anims: &mut SoldierAnims<'a>,
    ruleset: &Ruleset,
    data_dir: &Path,
    tc: &'a TextureCreator<WindowContext>,
) {
    if !matches!(game.phase_handler, PhaseHandler::Deployment { .. }) {
        return;
    }
    if mission.is_some() {
        return;
    }

    let mission_n = mission_number_from_name(
        game.game_state
            .current_mission
            .as_ref()
            .map(|m| m.name.as_str()),
    );
    let team_ids: Vec<u32> = game.game_state.team.iter().map(|m| m.id).collect();

    let data = match load_mission(data_dir, tc, ruleset, mission_n, &team_ids) {
        Ok(d) => d,
        Err(e) => {
            warn_or_info(e, data_dir);
            return;
        }
    };

    place_camera_on_map(game, &data.map);

    // Soldier animation loads after the mission (uses the same
    // palette source). AnimControllers are spawned once the team is
    // on the field; we just populate textures + anim_set here.
    let pic_dir = data_dir.join("WOW").join("PIC");
    if let Some(pal) = load_palette_pcx(&pic_dir) {
        load_soldier_animation(data_dir, tc, &pal, anims);
        anims.spawn_controllers(&game.game_state.team);
    } else {
        debug!("No palette available; soldier animations skipped");
    }

    *mission = Some(data);
}

/// Map a `LoadError` to either `warn!` (real failure) or `info!` (just
/// "no data file" — common on a fresh dev box). Distinguishes "data
/// missing" from "data corrupt" so a test run without a game
/// installation doesn't fill the log with warnings.
fn warn_or_info(err: super::asset_loader::LoadError, data_dir: &Path) {
    use super::asset_loader::LoadError;
    match &err {
        LoadError::MapNotFound(p) => {
            info!(
                path = %p.display(),
                "no mission map available (data dir: {}); staying on Office",
                data_dir.display()
            );
        }
        _ => tracing::warn!(error = %err, "mission load failed"),
    }
}

/// Centre the camera on the loaded map. Uses the MAP's stored camera
/// position if it has one, otherwise the map midpoint. The exe stores
/// camera Y using 64px row spacing, but we render at 32px
/// (half-height for diamond interlocking), so the Y value is halved.
fn place_camera_on_map(game: &mut GameLoop, map: &ow_data::map_loader::GameMap) {
    let mid_x = if map.header.camera_x != 0 {
        map.header.camera_x as f32
    } else {
        (map.width() as f32 / 2.0) * 128.0
    };
    let mid_y = if map.header.camera_y != 0 {
        (map.header.camera_y as f32) / 2.0
    } else {
        (map.height() as f32 / 2.0) * 32.0
    };
    game.camera.x = mid_x - (game.window_width as f32 / 2.0);
    game.camera.y = mid_y - (game.window_height as f32 / 2.0);
}
