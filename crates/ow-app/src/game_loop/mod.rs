//! # Game Loop — SDL2 state-machine event loop
//!
//! Implements the OXCE-style phase-driven game loop. Each [`GamePhase`] has its
//! own `handle_input` / `update` / `render` cycle, driven by the top-level
//! `run_game_loop` function.
//!
//! ## State Machine
//!
//! ```text
//! Office (overview)
//!   → HireMercs  (1)   select/deselect mercs from roster
//!   → Equipment  (2)   buy/sell gear
//!   → Intel      (3)   read reports
//!   → Contracts  (4)   view/accept contracts
//!   → Training   (5)   train mercs between missions
//!   → Begin Mission (B) → Travel
//! Travel → auto-transition → Mission(Deployment)
//! Mission
//!   → Deployment   place mercs on start tiles
//!   → Combat       initiative turns, move/shoot/AI
//!   → Extraction   mission complete, reach exit
//! Debrief → show results → Enter → back to Office
//! ```
//!
//! ## Frame Timing
//!
//! The loop targets 60 fps with delta-time tracking. Delta is capped at 33 ms
//! (floor of 30 fps) to prevent physics/animation explosions on hitches.

mod asset_loader;
mod audio_handles;
mod audio_init;
mod auto_screenshot;
mod combat_log;
mod dev_hotkeys;
mod input;
mod loop_pump;
mod mission;
mod music;
mod office_layout;
mod render;
mod screenshot;
mod soldier_anims;
mod update;

use asset_loader::{load_debrief_sprites, load_office_texture};
use audio_init::init_audio;
use input::{handle_escape, handle_phase_input};
use loop_pump::{maybe_load_mission, maybe_transition_music};
use mission::MissionData;
use render::render_phase;
use soldier_anims::SoldierAnims;
use update::update_phase;

pub(crate) use auto_screenshot::AutoScreenshot;
pub(crate) use combat_log::{log_combat, CombatLogEntry, CombatLogKind};
pub(crate) use dev_hotkeys::handle_dev_hotkeys;
pub(crate) use screenshot::{save_screenshot, write_canvas_as_bmp};

use std::time::Instant;

use anyhow::Result;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;
use sdl2::render::Canvas;
use sdl2::video::Window;
use tracing::{debug, info};

use ow_core::game_state::{GamePhase, GameState, MissionPhase, OfficePhase};
use ow_core::merc::MercId;

use ow_core::ruleset::Ruleset;
use ow_render::camera::Camera;
use ow_render::text::TextRenderer;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Target frame duration for 60 fps (16.67 ms).
const TARGET_FRAME_MS: u32 = 16;

/// Maximum delta time in milliseconds. Frames longer than this are clamped
/// to prevent animation/physics blow-ups during hitches or debugger pauses.
const MAX_DELTA_MS: u32 = 33;

/// Window width at startup.
pub(crate) const WINDOW_WIDTH: u32 = 1280;
/// Window height at startup.
pub(crate) const WINDOW_HEIGHT: u32 = 720;

// ---------------------------------------------------------------------------
// Phase-specific state
// ---------------------------------------------------------------------------

/// Per-phase handler state. Each variant carries the mutable state that only
/// matters while that phase is active — released on phase transition.
#[derive(Debug)]
pub enum PhaseHandler {
    /// Office phase — tracks which sub-phase (overview, hiring, etc.) is shown.
    Office { sub_phase: OfficePhase },

    /// Travel screen — purely cosmetic, auto-advances after a short delay.
    Travel {
        /// Accumulated time in this phase (ms). Auto-transitions to mission
        /// after a brief "traveling..." display.
        elapsed_ms: u32,
    },

    /// Deployment — player places mercs on start tiles before combat begins.
    Deployment {
        /// Index into `player_units` of the currently selected merc for placement.
        selected_unit: usize,
    },

    /// Active turn-based combat.
    Combat(CombatHandler),

    /// Extraction — objectives complete, move to exit zone.
    Extraction,

    /// Post-mission debrief showing results.
    /// The accountant calls on the video phone to deliver the financial report.
    Debrief {
        /// True if the mission was a success.
        success: bool,
        /// Accumulated time in this phase (ms). Drives the accountant sprite
        /// animation cycling on the video phone.
        anim_elapsed_ms: u32,
    },

    /// Pause overlay — remembers the phase we paused from.
    Paused {
        /// The phase handler we were in before pausing.
        previous: Box<PhaseHandler>,
    },
}

/// Combat-specific state tracked across turns.
#[derive(Debug)]
pub struct CombatHandler {
    /// Initiative-sorted list of unit IDs for this round.
    /// Contains both player and enemy unit IDs.
    pub initiative_order: Vec<MercId>,
    /// Index into `initiative_order` for the currently acting unit.
    pub current_initiative_idx: usize,
    /// Currently selected player unit (for UI highlighting / input).
    pub selected_unit_id: Option<MercId>,
    /// True when the AI is processing enemy turns (blocks player input).
    pub ai_acting: bool,
    /// Index for Tab-cycling through player units.
    pub tab_cycle_index: usize,
}

// ---------------------------------------------------------------------------
// GameLoop — the top-level struct
// ---------------------------------------------------------------------------

/// Top-level game loop state, tying together game state, camera, and
/// phase-specific handling.
///
/// Mission-scoped resources (map data, tile renderers, soldier textures,
/// enemy units) and audio state live in [`MissionAssets`] and
/// [`AudioHandles`] respectively — see those structs and the Phase B
/// refactor plan in `docs/refactor-game-loop.md`.
pub struct GameLoop {
    /// The campaign game state (phase, team, funds, mission context, etc.).
    pub game_state: GameState,
    /// Isometric camera controlling the viewport.
    pub camera: Camera,
    /// Phase-specific handler with per-phase mutable state.
    pub phase_handler: PhaseHandler,
    /// Current window dimensions (updated on resize).
    pub window_width: u32,
    pub window_height: u32,
    /// Combat message log (max 8 entries, newest at bottom). Color-coded by type.
    pub combat_log: Vec<CombatLogEntry>,
}

impl GameLoop {
    /// Create a new game loop from an initialized game state.
    pub fn new(game_state: GameState) -> Self {
        let phase_handler = phase_handler_for(&game_state.phase);

        Self {
            game_state,
            camera: Camera::new(WINDOW_WIDTH, WINDOW_HEIGHT),
            phase_handler,
            window_width: WINDOW_WIDTH,
            window_height: WINDOW_HEIGHT,
            combat_log: Vec::new(),
        }
    }
}

/// Build the appropriate `PhaseHandler` for a given `GamePhase`.
fn phase_handler_for(phase: &GamePhase) -> PhaseHandler {
    match phase {
        GamePhase::Office(sub) => PhaseHandler::Office { sub_phase: *sub },
        GamePhase::Travel => PhaseHandler::Travel { elapsed_ms: 0 },
        GamePhase::Mission(MissionPhase::Deployment) => {
            PhaseHandler::Deployment { selected_unit: 0 }
        }
        GamePhase::Mission(MissionPhase::Combat) => PhaseHandler::Combat(CombatHandler {
            initiative_order: Vec::new(),
            current_initiative_idx: 0,
            selected_unit_id: None,
            ai_acting: false,
            tab_cycle_index: 0,
        }),
        GamePhase::Mission(MissionPhase::Extraction) => PhaseHandler::Extraction,
        GamePhase::Debrief => PhaseHandler::Debrief {
            success: true,
            anim_elapsed_ms: 0,
        },
    }
}

use music::stop_music;

// ---------------------------------------------------------------------------
// Color palette for placeholder rendering
// ---------------------------------------------------------------------------

/// Background colors for each phase — used for placeholder rendering before
/// real art assets are wired up.
fn phase_background_color(handler: &PhaseHandler) -> Color {
    match handler {
        PhaseHandler::Office { sub_phase } => match sub_phase {
            OfficePhase::Overview => Color::RGB(30, 40, 60),
            OfficePhase::HireMercs => Color::RGB(40, 60, 40),
            OfficePhase::Equipment => Color::RGB(60, 50, 30),
            OfficePhase::Intel => Color::RGB(40, 40, 60),
            OfficePhase::Contracts => Color::RGB(50, 35, 35),
            OfficePhase::Training => Color::RGB(35, 55, 55),
        },
        PhaseHandler::Travel { .. } => Color::RGB(20, 20, 40),
        // Black background — the original game uses a black back buffer.
        // Diamond tile corners are transparent and show this color.
        PhaseHandler::Deployment { .. } => Color::RGB(0, 0, 0),
        PhaseHandler::Combat(_) => Color::RGB(10, 10, 10),
        PhaseHandler::Extraction => Color::RGB(40, 50, 30),
        PhaseHandler::Debrief { success, .. } => {
            if *success {
                Color::RGB(20, 50, 20)
            } else {
                Color::RGB(60, 20, 20)
            }
        }
        PhaseHandler::Paused { .. } => Color::RGB(30, 30, 30),
    }
}

/// Human-readable label for the current phase.
fn phase_label(handler: &PhaseHandler) -> &'static str {
    match handler {
        PhaseHandler::Office { sub_phase } => match sub_phase {
            OfficePhase::Overview => "OFFICE - Overview",
            OfficePhase::HireMercs => "OFFICE - Hire Mercs",
            OfficePhase::Equipment => "OFFICE - Equipment",
            OfficePhase::Intel => "OFFICE - Intel",
            OfficePhase::Contracts => "OFFICE - Contracts",
            OfficePhase::Training => "OFFICE - Training",
        },
        PhaseHandler::Travel { .. } => "TRAVELING...",
        PhaseHandler::Deployment { .. } => "MISSION - Deployment",
        PhaseHandler::Combat(_) => "MISSION - Combat",
        PhaseHandler::Extraction => "MISSION - Extraction",
        PhaseHandler::Debrief { success, .. } => {
            if *success {
                "DEBRIEF - Mission Complete!"
            } else {
                "DEBRIEF - Mission Failed"
            }
        }
        PhaseHandler::Paused { .. } => "PAUSED",
    }
}

// ===========================================================================
// run_game_loop — the main entry point
// ===========================================================================

/// Run the SDL2 game loop until the player quits.
///
/// This is the beating heart of the engine. It drives the per-phase
/// update/render cycle at 60 fps with delta-time tracking.
///
/// # Parameters
/// - `canvas`: SDL2 window canvas for rendering.
/// - `event_pump`: Initialized SDL2 event pump (created by the caller, since
///   the intro cutscenes already created one before handing off to us).
/// - `game_state`: Pre-initialized campaign state (from main.rs).
///
/// # Returns
/// `Ok(())` on clean exit, `Err` on SDL2 or fatal engine errors.
///
/// # Architecture
///
/// The body is split (Phase A + B of the refactor plan in
/// `docs/refactor-game-loop.md`) into five cohesive sections:
///
/// 1. **One-time setup** — text renderer, audio device, SFX/voice
///    players, office texture, debrief sprite sheets, the
///    `Option<MissionData<'a>>` mission slot, the [`SoldierAnims`]
///    bundle, the [`AudioHandles`] bundle.
/// 2. **The per-frame loop** — event poll → update → animation tick →
///    music transition → mission-load guard → render → present →
///    sleep. Each step is a single helper call.
/// 3. **Per-frame helpers** — animation dispatch, audio mixing,
///    mission-resource loading. Each lives in its own module
///    (see `soldier_anims`, `asset_loader`, `audio_init`,
///    `loop_pump`).
/// 4. **Cleanup** — drop the music handle and voice player before
///    closing the mixer (cached Chunks must be freed while the device
///    is open).
pub fn run_game_loop_with_pump(
    mut canvas: Canvas<Window>,
    mut event_pump: sdl2::EventPump,
    game_state: GameState,
    ruleset: Ruleset,
    data_dir: &std::path::Path,
) -> Result<()> {
    info!(phase = ?game_state.phase, "Starting game loop");

    let mut game = GameLoop::new(game_state);
    let texture_creator = canvas.texture_creator();
    // SDL2_ttf is initialised once and lives for the whole loop. The
    // TextRenderer below borrows from it; both must outlive every render
    // call (cheap — they live on the run-loop stack until exit).
    let ttf_context =
        sdl2::ttf::init().map_err(|e| anyhow::anyhow!("SDL2_ttf init failed: {e}"))?;
    let text_renderer = TextRenderer::new(&ttf_context, None)
        .map_err(|e| anyhow::anyhow!("Font loading failed: {e}"))?;
    let mut audio = init_audio(data_dir, &game.phase_handler);
    let office_texture = load_office_texture(data_dir, &texture_creator);
    let (acct_textures, phone_textures) = load_debrief_sprites(data_dir, &texture_creator);
    let mut mission: Option<MissionData<'_>> = None;
    let mut anims = SoldierAnims::empty();

    let mut last_frame = Instant::now();
    let mut running = true;
    let mut auto_screenshot = AutoScreenshot::from_env();

    // -----------------------------------------------------------------------
    // Main loop: poll events -> update -> render -> present -> sleep
    // -----------------------------------------------------------------------
    while running {
        let now = Instant::now();
        let raw_delta_ms = now.duration_since(last_frame).as_millis() as u32;
        let delta_ms = raw_delta_ms.min(MAX_DELTA_MS);
        last_frame = now;

        // -- Event handling --
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => {
                    info!("Quit event received");
                    running = false;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => {
                    running = handle_escape(&mut game);
                }
                Event::Window {
                    win_event: sdl2::event::WindowEvent::Resized(w, h),
                    ..
                } => {
                    game.window_width = w as u32;
                    game.window_height = h as u32;
                    debug!(width = w, height = h, "Window resized");
                }
                Event::KeyDown {
                    keycode: Some(Keycode::F12),
                    ..
                } => {
                    save_screenshot(&canvas);
                }
                // F1-F5 + M (cheat/dev). `handle_dev_hotkeys` is a no-op on
                // non-keydown events and on unknown keycodes.
                Event::KeyDown { .. } => {
                    handle_dev_hotkeys(&mut game, mission.as_mut(), &event);
                }
                // Delegate all other input to the current phase handler
                _ => {
                    handle_phase_input(
                        &mut game,
                        mission.as_mut(),
                        &event,
                        &ruleset,
                        &mut audio.sfx_manager,
                        &mut audio.voice_player,
                    );
                }
            }
        }

        if !running {
            break;
        }

        // -- Update --
        update_phase(
            &mut game,
            mission.as_mut(),
            delta_ms,
            &mut audio.sfx_manager,
        );

        // -- Animation state-machine --
        // Diffs each merc's position/hp/ap against the previous frame's
        // snapshot, dispatches Walk / ShootStand / Die transitions,
        // and ticks the controllers so the chosen action's frames advance.
        anims.tick(&game.game_state.team, delta_ms);

        // -- Music transitions on phase change --
        maybe_transition_music(&game, &mut audio, data_dir);

        // -- Lazy-load mission map the first time we enter Deployment --
        maybe_load_mission(
            &mut game,
            &mut mission,
            &mut anims,
            &ruleset,
            data_dir,
            &texture_creator,
        );

        // -- Window dimensions every frame (handles fullscreen, DPI changes,
        // and resize events we might miss). Cheap call, prevents coordinate
        // bugs. --
        let (cw, ch) = canvas.window().size();
        game.window_width = cw;
        game.window_height = ch;

        // -- Render --
        let bg = phase_background_color(&game.phase_handler);
        canvas.set_draw_color(bg);
        canvas.clear();

        render_phase(
            &game,
            mission.as_ref(),
            &anims,
            &mut canvas,
            &text_renderer,
            &texture_creator,
            &ruleset,
            &office_texture,
            &acct_textures,
            &phone_textures,
        );

        let label = phase_label(&game.phase_handler);
        canvas
            .window_mut()
            .set_title(&format!("Open Wages \u{2014} {label}"))
            .ok();

        canvas.present();

        if let Some(auto_ss) = auto_screenshot.as_mut() {
            auto_ss.tick(&canvas, label);
        }

        // -- Frame pacing --
        let frame_elapsed = now.elapsed().as_millis() as u32;
        if frame_elapsed < TARGET_FRAME_MS {
            std::thread::sleep(std::time::Duration::from_millis(
                (TARGET_FRAME_MS - frame_elapsed) as u64,
            ));
        }
    }

    // -- Cleanup --
    // Drop music handle and voice player before closing the audio device
    // — cached Chunks must be freed while the mixer is still open.
    drop(audio._music_handle);
    drop(audio.voice_player);
    if audio.audio_available {
        stop_music();
        sdl2::mixer::close_audio();
        debug!("SDL2_mixer audio closed");
    }

    info!("Game loop exited cleanly");
    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_handler_round_trip() {
        let office = phase_handler_for(&GamePhase::Office(OfficePhase::Overview));
        assert!(matches!(office, PhaseHandler::Office { .. }));

        let travel = phase_handler_for(&GamePhase::Travel);
        assert!(matches!(travel, PhaseHandler::Travel { elapsed_ms: 0 }));

        let deploy = phase_handler_for(&GamePhase::Mission(MissionPhase::Deployment));
        assert!(matches!(
            deploy,
            PhaseHandler::Deployment { selected_unit: 0 }
        ));

        let combat = phase_handler_for(&GamePhase::Mission(MissionPhase::Combat));
        assert!(matches!(combat, PhaseHandler::Combat(_)));

        let extract = phase_handler_for(&GamePhase::Mission(MissionPhase::Extraction));
        assert!(matches!(extract, PhaseHandler::Extraction));

        let debrief = phase_handler_for(&GamePhase::Debrief);
        assert!(matches!(
            debrief,
            PhaseHandler::Debrief { success: true, .. }
        ));
    }

    #[test]
    fn game_loop_initializes_in_office() {
        let state = GameState::new(500_000);
        let game = GameLoop::new(state);
        assert!(matches!(game.phase_handler, PhaseHandler::Office { .. }));
        assert_eq!(game.camera.viewport_width, WINDOW_WIDTH);
        assert_eq!(game.camera.viewport_height, WINDOW_HEIGHT);
    }

    #[test]
    fn phase_labels_are_unique() {
        let handlers = [
            PhaseHandler::Office {
                sub_phase: OfficePhase::Overview,
            },
            PhaseHandler::Travel { elapsed_ms: 0 },
            PhaseHandler::Deployment { selected_unit: 0 },
            PhaseHandler::Combat(CombatHandler {
                initiative_order: vec![],
                current_initiative_idx: 0,
                selected_unit_id: None,
                ai_acting: false,
                tab_cycle_index: 0,
            }),
            PhaseHandler::Extraction,
            PhaseHandler::Debrief {
                success: true,
                anim_elapsed_ms: 0,
            },
            PhaseHandler::Debrief {
                success: false,
                anim_elapsed_ms: 0,
            },
        ];

        let labels: Vec<&str> = handlers.iter().map(phase_label).collect();
        // Verify non-debrief labels are all distinct
        for i in 0..5 {
            for j in (i + 1)..5 {
                assert_ne!(labels[i], labels[j], "duplicate label at {i} and {j}");
            }
        }
    }

    #[test]
    fn phase_colors_are_distinct() {
        let handlers = [
            PhaseHandler::Office {
                sub_phase: OfficePhase::Overview,
            },
            PhaseHandler::Travel { elapsed_ms: 0 },
            PhaseHandler::Combat(CombatHandler {
                initiative_order: vec![],
                current_initiative_idx: 0,
                selected_unit_id: None,
                ai_acting: false,
                tab_cycle_index: 0,
            }),
            PhaseHandler::Debrief {
                success: true,
                anim_elapsed_ms: 0,
            },
            PhaseHandler::Debrief {
                success: false,
                anim_elapsed_ms: 0,
            },
        ];

        let colors: Vec<Color> = handlers.iter().map(phase_background_color).collect();
        // Success and failure debrief must have different colors
        assert_ne!(colors[3], colors[4]);
    }
}

// ---------------------------------------------------------------------------
// Screenshot — F12 saves the current frame to disk as BMP. See `screenshot.rs`.
// ---------------------------------------------------------------------------
