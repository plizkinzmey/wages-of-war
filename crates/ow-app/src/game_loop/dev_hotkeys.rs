//! # Dev hotkeys — F1-F5 + M
//!
//! Cheat/dev-only keybindings invoked from the main event loop. Keeps the
//! hotkey arm/dispatch out of `mod.rs` so the loop stays a thin dispatcher.
//!
//! - F1: Skip to debrief (force-win current mission).
//! - F2: Skip to office (abort mission, return home).
//! - F3: Skip to deployment of mission 1 (requires hired team).
//! - F4: Kill all enemies (instant win).
//! - F5: Add $500k funds.
//! - M: Toggle music mute (volume 0 ↔ 64).

use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use tracing::info;

use ow_core::game_state::{GamePhase, MissionContext, MissionPhase, OfficePhase};
use ow_core::weather::Weather;

use super::mission::MissionData;
use super::{GameLoop, PhaseHandler};

/// Handle the dev-only keypress arms. The event-loop dispatches here for
/// any `Event::KeyDown` whose keycode matches one of the dev hotkeys. F12
/// (screenshot) is handled by the caller because it needs the canvas.
pub(crate) fn handle_dev_hotkeys(
    game: &mut GameLoop,
    mission: Option<&mut MissionData>,
    event: &Event,
) {
    if let Event::KeyDown {
        keycode: Some(kc), ..
    } = event
    {
        match *kc {
            Keycode::F1 => dev_f1_force_win(game),
            Keycode::F2 => dev_f2_force_office(game),
            Keycode::F3 => dev_f3_force_deploy(game),
            Keycode::F4 => dev_f4_kill_enemies(mission),
            Keycode::F5 => dev_f5_add_funds(game),
            Keycode::M => dev_m_toggle_mute(),
            _ => {}
        }
    }
}

/// F1: Skip to debrief (win current mission instantly).
fn dev_f1_force_win(game: &mut GameLoop) {
    info!("[DEV] F1: Force win → Debrief");
    game.game_state.set_phase(GamePhase::Debrief);
    game.phase_handler = PhaseHandler::Debrief {
        success: true,
        anim_elapsed_ms: 0,
    };
}

/// F2: Skip to office (abort mission, go home).
fn dev_f2_force_office(game: &mut GameLoop) {
    info!("[DEV] F2: Force → Office");
    game.game_state
        .set_phase(GamePhase::Office(OfficePhase::Overview));
    game.phase_handler = PhaseHandler::Office {
        sub_phase: OfficePhase::Overview,
    };
}

/// F3: Skip to deployment (start mission 1 with current team).
fn dev_f3_force_deploy(game: &mut GameLoop) {
    if game.game_state.team.is_empty() {
        info!("[DEV] F3: Can't deploy — no mercs hired");
    } else {
        info!("[DEV] F3: Force → Deployment (mission 1)");
        if game.game_state.current_mission.is_none() {
            game.game_state.current_mission = Some(MissionContext {
                name: "MSSN01".to_string(),
                weather: Weather::Clear,
                combat: None,
                turn_number: 0,
            });
        }
        game.game_state
            .set_phase(GamePhase::Mission(MissionPhase::Deployment));
        game.phase_handler = PhaseHandler::Deployment { selected_unit: 0 };
    }
}

/// F4: Kill all enemies (instant win condition).
fn dev_f4_kill_enemies(mission: Option<&mut MissionData>) {
    info!("[DEV] F4: Kill all enemies");
    if let Some(m) = mission {
        m.enemies.clear();
    } else {
        tracing::info!("no mission loaded; F4 is a no-op");
    }
}

/// F5: Add $500k funds.
fn dev_f5_add_funds(game: &mut GameLoop) {
    game.game_state.funds += 500_000;
    info!("[DEV] F5: +$500k → funds={}", game.game_state.funds);
}

/// M: Toggle music mute.
fn dev_m_toggle_mute() {
    if sdl2::mixer::Music::get_volume() > 0 {
        sdl2::mixer::Music::set_volume(0);
        info!("[DEV] M: Music muted");
    } else {
        sdl2::mixer::Music::set_volume(64);
        info!("[DEV] M: Music unmuted");
    }
}
