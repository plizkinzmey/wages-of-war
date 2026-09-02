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

mod input;
mod music;
mod render;
mod update;

use input::{handle_escape, handle_phase_input};
use render::render_phase;
use update::update_phase;

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::mixer::Music;
use sdl2::pixels::Color;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use tracing::{debug, info, trace, warn};

use ow_audio::sfx::SfxManager;
use ow_audio::voice::VoicePlayer;
use ow_core::game_state::{GamePhase, GameState, MissionPhase, OfficePhase};
use ow_core::merc::MercId;

use ow_core::ruleset::Ruleset;
use ow_render::camera::Camera;
use ow_render::iso_math::IsoConfig;
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
pub struct GameLoop {
    /// The campaign game state (phase, team, funds, mission context, etc.).
    pub game_state: GameState,
    /// Isometric camera controlling the viewport.
    pub camera: Camera,
    /// Isometric projection configuration (tile dimensions, origin).
    pub iso_config: IsoConfig,
    /// Phase-specific handler with per-phase mutable state.
    pub phase_handler: PhaseHandler,
    /// Current window dimensions (updated on resize).
    pub window_width: u32,
    pub window_height: u32,
    /// Mission-specific IsoConfig (set when map loads, uses actual tile dimensions).
    pub mission_iso: Option<IsoConfig>,
    /// Enemy units for the current mission.
    pub enemies: Vec<ow_core::mission_setup::EnemyUnit>,
    /// Combat message log (max 8 entries, newest at bottom). Color-coded by type.
    pub combat_log: Vec<CombatLogEntry>,
    /// Persistent latch: once SDL2_mixer fails to load a MIDI file (no
    /// SoundFont), we stop retrying for the rest of the session. This
    /// prevents stderr spam from the MIDI loader on every frame.
    pub music_broken: bool,
}

/// Maximum number of combat log entries displayed on screen.
const COMBAT_LOG_MAX: usize = 8;

/// A single entry in the combat message log, with color-coding info.
#[derive(Debug, Clone)]
pub struct CombatLogEntry {
    /// The message text to display.
    pub text: String,
    /// The category determines the display color.
    pub kind: CombatLogKind,
}

/// Color categories for combat log entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatLogKind {
    /// Player hit on enemy — green.
    PlayerHit,
    /// Enemy hit on player merc — red.
    EnemyHit,
    /// Any miss — gray.
    Miss,
    /// A unit was killed — yellow.
    Kill,
    /// Informational (movement, round changes) — white.
    Info,
}

impl CombatLogKind {
    /// Return the SDL2 color for this log category.
    fn color(self) -> Color {
        match self {
            CombatLogKind::PlayerHit => Color::RGB(80, 220, 80),
            CombatLogKind::EnemyHit => Color::RGB(220, 60, 60),
            CombatLogKind::Miss => Color::RGB(160, 160, 160),
            CombatLogKind::Kill => Color::RGB(255, 220, 50),
            CombatLogKind::Info => Color::RGB(200, 200, 200),
        }
    }
}

/// Push a message to the combat log, trimming to [`COMBAT_LOG_MAX`] entries.
fn log_combat(game: &mut GameLoop, msg: String, kind: CombatLogKind) {
    debug!(combat_log = %msg, "Combat log entry");
    game.combat_log.push(CombatLogEntry { text: msg, kind });
    if game.combat_log.len() > COMBAT_LOG_MAX {
        let excess = game.combat_log.len() - COMBAT_LOG_MAX;
        game.combat_log.drain(..excess);
    }
}

impl GameLoop {
    /// Create a new game loop from an initialized game state.
    pub fn new(game_state: GameState) -> Self {
        let phase_handler = phase_handler_for(&game_state.phase);

        Self {
            game_state,
            camera: Camera::new(WINDOW_WIDTH, WINDOW_HEIGHT),
            iso_config: IsoConfig {
                tile_width: 64.0,
                tile_height: 32.0,
                origin_x: (WINDOW_WIDTH as f32) / 2.0,
                origin_y: 64.0,
            },
            phase_handler,
            window_width: WINDOW_WIDTH,
            window_height: WINDOW_HEIGHT,
            mission_iso: None,
            enemies: Vec::new(),
            combat_log: Vec::new(),
            music_broken: false,
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

use music::{music_track_for_phase, music_track_for_phase_with_mission, start_music, stop_music};

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
pub fn run_game_loop_with_pump(
    mut canvas: Canvas<Window>,
    mut event_pump: sdl2::EventPump,
    game_state: GameState,
    ruleset: Ruleset,
    data_dir: &std::path::Path,
) -> Result<()> {
    info!(phase = ?game_state.phase, "Starting game loop");

    let mut game = GameLoop::new(game_state);

    // Initialize text rendering — loads a system font for UI text.
    let ttf_context =
        sdl2::ttf::init().map_err(|e| anyhow::anyhow!("SDL2_ttf init failed: {e}"))?;
    let text_renderer = TextRenderer::new(&ttf_context, None)
        .map_err(|e| anyhow::anyhow!("Font loading failed: {e}"))?;
    let texture_creator = canvas.texture_creator();

    // -----------------------------------------------------------------------
    // MIDI music via SDL2_mixer
    // -----------------------------------------------------------------------
    let midi_dir = data_dir.join("WOW").join("MIDI");
    let audio_available = match sdl2::mixer::open_audio(44100, sdl2::mixer::AUDIO_S16LSB, 2, 1024) {
        Ok(()) => {
            info!("SDL2_mixer audio device opened (44100 Hz, S16LSB, stereo)");
            true
        }
        Err(e) => {
            warn!(error = %e, "SDL2_mixer failed to open audio -- continuing without music");
            false
        }
    };

    // Start initial music for whatever phase we launched into.
    let mut current_music_track: Option<String> = None;
    let mut _music_handle: Option<Music> = if audio_available {
        let track = music_track_for_phase(&game.phase_handler);
        if let Some(name) = track {
            let handle = start_music(&midi_dir, name, &mut game.music_broken);
            if handle.is_some() {
                current_music_track = Some(name.to_string());
            }
            handle
        } else {
            None
        }
    } else {
        None
    };

    // -----------------------------------------------------------------------
    // Combat SFX — pre-load WAV files from WOW/SND/ as mixer Chunks.
    // Channels 2–7 are reserved for SFX; 0–1 stay free for voice/music.
    // -----------------------------------------------------------------------
    let snd_dir = data_dir.join("WOW").join("SND");
    let mut sfx_manager = SfxManager::new(&snd_dir, audio_available);

    // -----------------------------------------------------------------------
    // Voice line playback — on-demand WAV loading from WOW/WAV/.
    // Uses mixer channel 1 (separate from music and SFX channels 2-7).
    // Voice lines play when hiring a merc or selecting one in combat.
    // -----------------------------------------------------------------------
    let wav_dir = data_dir.join("WOW").join("WAV");
    let mut voice_player: Option<VoicePlayer> = if audio_available {
        Some(VoicePlayer::new(wav_dir))
    } else {
        debug!("Voice player disabled (no audio device)");
        None
    };

    // Load the office background image — OFFICE.PCX is the main HQ screen.
    // The original game renders this as a 640x480 scene with clickable objects
    // (phone, fax, filing cabinet, pizza, etc.) overlaid on the background.
    let office_texture = {
        // OFFICE.PCX is the base layer of the office scene. The original engine
        // composites OBJ sprites on top for the interactive objects (phone, fax, etc.).
        // OFFPIC2.PCX is a pre-composited version with all objects baked in.
        // We use OFFPIC2 for now; proper compositing comes later.
        let pcx_path = data_dir.join("WOW").join("PIC").join("OFFPIC2.PCX");
        match ow_render::pcx::load_pcx(&pcx_path) {
            Ok(img) => {
                info!(
                    width = img.width,
                    height = img.height,
                    "Office background loaded"
                );
                match ow_render::pcx::pcx_to_texture(&img, &texture_creator) {
                    Ok(tex) => Some(tex),
                    Err(e) => {
                        warn!("Failed to create office texture: {e}");
                        None
                    }
                }
            }
            Err(e) => {
                warn!("Failed to load OFFICE.PCX: {e}");
                None
            }
        }
    };

    // -----------------------------------------------------------------------
    // Debrief screen sprites -- accountant + video phone
    // -----------------------------------------------------------------------
    // ACCT.OBJ contains the accountant character sprites (animated on the
    // video phone during the post-mission financial debrief). PHONSPR.OBJ
    // contains the phone scene background frames. Both use the same FLC
    // sprite container format as tilesets and OBJ files.
    //
    // We load all frames at startup and convert them to SDL2 textures so
    // the debrief renderer can just index into them by frame number.
    let (acct_textures, phone_textures) = {
        // Use OFFPIC2.PCX palette -- it is the closest match to the game's
        // master VGA palette and we already loaded it for the office scene.
        let pic_dir = data_dir.join("WOW").join("PIC");
        let palette = {
            let offpic = pic_dir.join("OFFPIC2.PCX");
            match ow_render::palette::load_pcx_palette(&offpic) {
                Ok(pal) => Some(pal),
                Err(e) => {
                    warn!("Failed to load palette for debrief sprites: {e}");
                    None
                }
            }
        };

        let spr_dir = data_dir.join("WOW").join("SPR");

        /// Decode all frames from a sprite sheet into RGBA SDL2 textures.
        /// Returns an empty vec if loading fails -- the renderer will fall
        /// back to the placeholder debrief display.
        fn load_sprite_textures<'a>(
            path: &Path,
            palette: &Option<ow_render::palette::Palette256>,
            tc: &'a TextureCreator<WindowContext>,
        ) -> Vec<Texture<'a>> {
            let pal = match palette {
                Some(p) => p,
                None => {
                    warn!(path = %path.display(),
                          "no palette available -- skipping sprite load");
                    return Vec::new();
                }
            };

            let sheet = match ow_data::sprite::parse_sprite_file(path) {
                Ok(s) => {
                    info!(
                        path = %path.display(),
                        frames = s.file_header.sprite_count,
                        "debrief sprite sheet loaded"
                    );
                    s
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e,
                          "failed to parse debrief sprite sheet");
                    return Vec::new();
                }
            };

            let mut textures = Vec::with_capacity(sheet.frames.len());
            for (i, frame) in sheet.frames.iter().enumerate() {
                let fw = frame.header.width as u32;
                let fh = frame.header.height as u32;
                if fw == 0 || fh == 0 {
                    // Some sprite sheets have empty placeholder frames.
                    trace!(frame = i, "skipping zero-size sprite frame");
                    continue;
                }
                match ow_data::sprite::decode_rle(
                    &frame.compressed_data,
                    frame.header.width,
                    frame.header.height,
                    i,
                ) {
                    Ok(pixels) => {
                        // Brightness boost of 1.5 to compensate for CRT->LCD gamma.
                        let rgba =
                            ow_render::palette::apply_palette_with_brightness(&pixels, pal, 1.5);
                        match tc.create_texture_static(
                            sdl2::pixels::PixelFormatEnum::RGBA32,
                            fw,
                            fh,
                        ) {
                            Ok(mut tex) => {
                                tex.set_blend_mode(sdl2::render::BlendMode::Blend);
                                if let Err(e) = tex.update(None, &rgba, (fw * 4) as usize) {
                                    warn!(frame = i, error = %e,
                                          "failed to upload sprite texture");
                                } else {
                                    textures.push(tex);
                                }
                            }
                            Err(e) => {
                                warn!(frame = i, error = %e,
                                      "failed to create sprite texture");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(frame = i, error = %e, "RLE decode failed for sprite frame");
                    }
                }
            }
            info!(
                path = %path.display(),
                decoded = textures.len(),
                total = sheet.frames.len(),
                "debrief sprite textures ready"
            );
            textures
        }

        let acct_path = spr_dir.join("ACCT.OBJ");
        let phone_path = spr_dir.join("PHONSPR.OBJ");

        let acct = load_sprite_textures(&acct_path, &palette, &texture_creator);
        let phone = load_sprite_textures(&phone_path, &palette, &texture_creator);
        (acct, phone)
    };

    // -- Mission map resources (loaded when entering deployment) --
    // These are Option because they don't exist until a mission starts.
    let mut tile_renderer: Option<ow_render::tile_renderer::TileMapRenderer> = None;
    let mut obj_renderer: Option<ow_render::tile_renderer::TileMapRenderer> = None;
    let mut loaded_map: Option<ow_data::map_loader::GameMap> = None;
    let mut mission_iso_config: Option<IsoConfig> = None;

    // Soldier animation system: all frames from ANIM/JUNGSLD.DAT decoded into
    // textures, indexed by the AnimController's current_frame_index().
    // The COR file maps (action, direction, weapon) → frame ranges.
    let mut soldier_textures: Vec<Option<Texture>> = Vec::new();
    let mut soldier_anim_set: Option<ow_data::animation::AnimationSet> = None;
    // Per-merc animation controllers, keyed by merc index in the team.
    let mut soldier_anims: Vec<ow_render::anim_controller::AnimController> = Vec::new();

    // Per-merc snapshot of state from the previous frame, in lockstep with
    // `game.game_state.team`. Drives the animation state-watcher below: when
    // a merc's position / hp / ap differs from its snapshot, we transition
    // its AnimController accordingly. Tuple is (id, position, hp, ap).
    let mut prev_merc_states: Vec<(MercId, Option<ow_core::merc::TilePos>, u32, u32)> = Vec::new();
    // Frames remaining before Walk auto-reverts to Idle. Tracks per-merc by
    // index. Set on each Walk transition; counted down each frame.
    let mut walk_grace_remaining: Vec<u32> = Vec::new();
    // Backwards compat — kept as fallback if full animation loading fails.
    let soldier_texture: Option<Texture> = None;

    // Enemy units generated from mission data. Stored here so they persist
    // across the deployment and combat phases.
    let mut enemy_units: Vec<ow_core::mission_setup::EnemyUnit> = Vec::new();

    let mut last_frame = Instant::now();
    let mut running = true;
    let mut _screenshot_count = 0u32;

    // -----------------------------------------------------------------------
    // Dev auto-screenshot — env-gated. When `OW_AUTO_SCREENSHOT_MS` is set
    // to a positive integer, the loop drops a BMP into
    // `dev-screenshots/run-<unix-ts>/` every N ms, with a phase tag in the
    // filename so a thousand frames are still searchable. This exists so an
    // analysis session can see what actually rendered during a play session
    // without anyone hand-pressing F12 every few seconds. Disabled by default;
    // pure dev tooling, gitignored output dir.
    // -----------------------------------------------------------------------
    let auto_ss_interval_ms: Option<u128> = std::env::var("OW_AUTO_SCREENSHOT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n: &u128| n > 0);
    let auto_ss_dir: Option<std::path::PathBuf> = if let Some(ms) = auto_ss_interval_ms {
        let run_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dir = std::path::PathBuf::from(format!("dev-screenshots/run-{run_id}"));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!(?dir, "auto-screenshot mkdir failed: {e} — feature disabled");
            None
        } else {
            info!(interval_ms = ms, dir = %dir.display(), "Auto-screenshot enabled");
            Some(dir)
        }
    } else {
        None
    };
    let mut last_auto_ss = Instant::now();
    let mut auto_ss_count = 0u32;

    // -----------------------------------------------------------------------
    // Main loop: poll events -> update -> render -> present -> sleep
    // -----------------------------------------------------------------------
    while running {
        // -- Delta time calculation --
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

                // ESC toggles pause overlay (or quits from pause)
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => {
                    running = handle_escape(&mut game);
                }

                // Track window resizes so click coordinates scale correctly.
                Event::Window {
                    win_event: sdl2::event::WindowEvent::Resized(w, h),
                    ..
                } => {
                    game.window_width = w as u32;
                    game.window_height = h as u32;
                    debug!(width = w, height = h, "Window resized");
                }

                // F12 saves a screenshot to disk.
                Event::KeyDown {
                    keycode: Some(Keycode::F12),
                    ..
                } => {
                    save_screenshot(&canvas);
                }

                // ======= DEV HOTKEYS =======
                // F1: Skip to debrief (win current mission instantly)
                Event::KeyDown {
                    keycode: Some(Keycode::F1),
                    ..
                } => {
                    info!("[DEV] F1: Force win → Debrief");
                    game.game_state.set_phase(GamePhase::Debrief);
                    game.phase_handler = PhaseHandler::Debrief {
                        success: true,
                        anim_elapsed_ms: 0,
                    };
                }

                // F2: Skip to office (abort mission, go home)
                Event::KeyDown {
                    keycode: Some(Keycode::F2),
                    ..
                } => {
                    info!("[DEV] F2: Force → Office");
                    game.game_state.set_phase(GamePhase::Office(
                        ow_core::game_state::OfficePhase::Overview,
                    ));
                    game.phase_handler = PhaseHandler::Office {
                        sub_phase: ow_core::game_state::OfficePhase::Overview,
                    };
                }

                // F3: Skip to deployment (start mission 1 with current team)
                Event::KeyDown {
                    keycode: Some(Keycode::F3),
                    ..
                } => {
                    if game.game_state.team.is_empty() {
                        info!("[DEV] F3: Can't deploy — no mercs hired");
                    } else {
                        info!("[DEV] F3: Force → Deployment (mission 1)");
                        if game.game_state.current_mission.is_none() {
                            game.game_state.current_mission =
                                Some(ow_core::game_state::MissionContext {
                                    name: "MSSN01".to_string(),
                                    weather: ow_core::weather::Weather::Clear,
                                    combat: None,
                                    turn_number: 0,
                                });
                        }
                        game.game_state.set_phase(GamePhase::Mission(
                            ow_core::game_state::MissionPhase::Deployment,
                        ));
                        game.phase_handler = PhaseHandler::Deployment { selected_unit: 0 };
                    }
                }

                // F4: Kill all enemies (instant win condition)
                Event::KeyDown {
                    keycode: Some(Keycode::F4),
                    ..
                } => {
                    info!("[DEV] F4: Kill all enemies");
                    game.enemies.clear();
                }

                // F5: Add $500k funds
                Event::KeyDown {
                    keycode: Some(Keycode::F5),
                    ..
                } => {
                    game.game_state.funds += 500_000;
                    info!("[DEV] F5: +$500k → funds={}", game.game_state.funds);
                }

                // M: Toggle music mute
                Event::KeyDown {
                    keycode: Some(Keycode::M),
                    ..
                } => {
                    if sdl2::mixer::Music::get_volume() > 0 {
                        sdl2::mixer::Music::set_volume(0);
                        info!("[DEV] M: Music muted");
                    } else {
                        sdl2::mixer::Music::set_volume(64);
                        info!("[DEV] M: Music unmuted");
                    }
                }

                // Delegate all other input to the current phase handler
                _ => {
                    handle_phase_input(
                        &mut game,
                        &event,
                        &ruleset,
                        &mut sfx_manager,
                        &mut voice_player,
                    );
                }
            }
        }

        if !running {
            break;
        }

        // -- Update --
        update_phase(&mut game, delta_ms, &mut sfx_manager);

        // Drive soldier animations from merc state changes. The
        // AnimController already plays the chosen action; this block
        // decides WHEN to switch by diffing each merc's position / hp / ap
        // against the previous frame's snapshot. Walk fires on position
        // delta with an 8-way direction computed from the move vector.
        // ShootStand fires when AP drops without movement (i.e. the click
        // resolved as an attack). Die fires once on the alive→0-hp edge.
        // Without this watcher the controllers stay stuck on Idle, which
        // is the bug the handoff documented as "only idle plays."
        {
            use ow_render::anim_controller::{AnimAction, Direction};

            // Map a tile-delta to one of 8 cardinal/diagonal directions.
            // +y is south on the staggered isometric grid (row index grows
            // downward), so the dy sign maps directly to N/S.
            fn dir_from_delta(dx: i32, dy: i32) -> Direction {
                match (dx.signum(), dy.signum()) {
                    (0, -1) => Direction::N,
                    (1, -1) => Direction::NE,
                    (1, 0) => Direction::E,
                    (1, 1) => Direction::SE,
                    (0, 1) => Direction::S,
                    (-1, 1) => Direction::SW,
                    (-1, 0) => Direction::W,
                    (-1, -1) => Direction::NW,
                    _ => Direction::S,
                }
            }

            for (i, merc) in game.game_state.team.iter().enumerate() {
                let Some(ctrl) = soldier_anims.get_mut(i) else {
                    continue;
                };
                let prev = prev_merc_states.get(i).copied();
                let prev_pos = prev.and_then(|p| p.1);
                let prev_hp = prev.map(|p| p.2).unwrap_or(merc.current_hp);
                let prev_ap = prev.map(|p| p.3).unwrap_or(merc.current_ap);

                // Death edge: alive last frame, dead this frame. Set Die
                // and skip — the controller will hold the final death frame.
                if prev_hp > 0 && merc.current_hp == 0 {
                    ctrl.set_action(AnimAction::Die, Direction::S, 1);
                    continue;
                }
                if merc.current_hp == 0 {
                    continue;
                }

                // Movement: position changed since last frame.
                if let (Some(np), Some(pp)) = (merc.position, prev_pos) {
                    if np.x != pp.x || np.y != pp.y {
                        let dir = dir_from_delta(np.x - pp.x, np.y - pp.y);
                        ctrl.set_action(AnimAction::Walk, dir, 1);
                        continue;
                    }
                }

                // Shoot: AP dropped without a position change, i.e. the
                // player's click resolved as an attack. Direction here is
                // a default (S) — refining this requires plumbing the
                // target tile through to the watcher; out of scope for now.
                // The animation still plays and looks correct because the
                // sprite mostly faces the camera at S.
                if merc.current_ap < prev_ap {
                    ctrl.set_action(AnimAction::ShootStand, Direction::S, 1);
                    continue;
                }
            }

            // Auto-revert finishing one-shot animations (ShootStand/Hit/
            // Throw/Melee) back to Idle, and revert Walk after a brief
            // grace period since the in-game movement is teleport-based —
            // Walk only ever fires for one frame so the loop would
            // otherwise march in place forever.
            const WALK_GRACE_FRAMES: u32 = 24; // ~400ms at 60fps
            for (i, merc) in game.game_state.team.iter().enumerate() {
                let Some(ctrl) = soldier_anims.get_mut(i) else {
                    continue;
                };
                if merc.current_hp == 0 {
                    continue;
                }
                let cur_action = ctrl.state().map(|s| s.action);
                match cur_action {
                    Some(AnimAction::ShootStand)
                    | Some(AnimAction::ShootCrouch)
                    | Some(AnimAction::Hit)
                    | Some(AnimAction::Throw)
                    | Some(AnimAction::Melee)
                        if ctrl.is_finished() =>
                    {
                        ctrl.set_action(AnimAction::Idle, Direction::S, 1);
                        if let Some(slot) = walk_grace_remaining.get_mut(i) {
                            *slot = 0;
                        }
                    }
                    Some(AnimAction::Walk) => {
                        let slot = match walk_grace_remaining.get_mut(i) {
                            Some(s) => s,
                            None => continue,
                        };
                        *slot = slot.saturating_sub(1);
                        if *slot == 0 {
                            ctrl.set_action(AnimAction::Idle, Direction::S, 1);
                        }
                    }
                    _ => {}
                }
            }

            // Resize the walk-grace tracker to match team length, and reset
            // the counter for any merc whose Walk animation just (re)fired.
            walk_grace_remaining.resize(game.game_state.team.len(), 0);
            for (i, merc) in game.game_state.team.iter().enumerate() {
                let Some(prev) = prev_merc_states.get(i).copied() else {
                    continue;
                };
                if let (Some(np), Some(pp)) = (merc.position, prev.1) {
                    if np.x != pp.x || np.y != pp.y {
                        if let Some(slot) = walk_grace_remaining.get_mut(i) {
                            *slot = WALK_GRACE_FRAMES;
                        }
                    }
                }
            }

            // Snapshot for next frame's diff. Resized to match team length.
            prev_merc_states.clear();
            prev_merc_states.extend(
                game.game_state
                    .team
                    .iter()
                    .map(|m| (m.id, m.position, m.current_hp, m.current_ap)),
            );
        }

        // Tick soldier animation controllers so idle/walk/shoot frames advance.
        for ctrl in soldier_anims.iter_mut() {
            ctrl.update(delta_ms as f32);
        }

        // -- Music transitions on phase change --
        // Compare what we're currently playing to what the new phase wants.
        // If they differ, stop old music and start the new track.
        // Uses mission number for mission-phase track selection (WOWMIS01–09).
        if audio_available {
            let mission_num = game
                .game_state
                .current_mission
                .as_ref()
                .and_then(|m| m.name.strip_prefix("MSSN"))
                .and_then(|n| n.parse::<u32>().ok());
            let wanted = music_track_for_phase_with_mission(&game.phase_handler, mission_num);
            let need_change = match (&wanted, &current_music_track) {
                // Pause: don't touch music at all.
                _ if matches!(game.phase_handler, PhaseHandler::Paused { .. }) => false,
                (Some(w), Some(c)) => w.as_str() != c.as_str(),
                (Some(_), None) => true,
                (None, Some(_)) => true,
                (None, None) => false,
            };
            if need_change {
                stop_music();
                if let Some(track_name) = &wanted {
                    let handle = start_music(&midi_dir, track_name, &mut game.music_broken);
                    if handle.is_some() {
                        current_music_track = Some(track_name.clone());
                    } else {
                        current_music_track = None;
                    }
                    _music_handle = handle;
                } else {
                    _music_handle = None;
                    current_music_track = None;
                }
            }
        }

        // -- Load mission map when entering deployment for the first time --
        // We check if we just transitioned to Deployment and haven't loaded a map yet.
        if matches!(game.phase_handler, PhaseHandler::Deployment { .. }) && loaded_map.is_none() {
            // Determine which mission scenario to load from the accepted contract.
            let mission_num = game
                .game_state
                .current_mission
                .as_ref()
                .and_then(|m| m.name.strip_prefix("MSSN"))
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(1);

            info!(mission = mission_num, "Loading mission map for deployment");

            // Load MAP file from WOW/MAPS/SCEN{n}/
            // Try SCEN{n}.MAP first, then SCEN{n}A.MAP (the actual filename varies).
            let scen_dir = data_dir
                .join("WOW")
                .join("MAPS")
                .join(format!("SCEN{mission_num}"));
            let map_path = {
                let try1 = scen_dir.join(format!("SCEN{mission_num}.MAP"));
                let try2 = scen_dir.join(format!("SCEN{mission_num}A.MAP"));
                if try1.exists() {
                    try1
                } else {
                    try2
                }
            };

            match ow_data::map_loader::parse_map(&map_path) {
                Ok(map) => {
                    info!(width = map.width(), height = map.height(),
                          tileset = %map.asset_refs.tileset_path, "Map loaded");

                    // Load the TIL tileset referenced by the MAP's string table.
                    // The MAP references paths like "C:\WOW\SPR\SCEN1\TILSCN01.TIL".
                    // The TIL files live in WOW/SPR/SCEN{n}/, not WOW/MAPS/SCEN{n}/.
                    let til_name =
                        ow_data::map_loader::filename_from_build_path(&map.asset_refs.tileset_path);
                    let spr_scen_dir = data_dir
                        .join("WOW")
                        .join("SPR")
                        .join(format!("SCEN{mission_num}"));
                    let til_path = spr_scen_dir.join(til_name);
                    match ow_data::sprite::parse_sprite_file(&til_path) {
                        Ok(tileset) => {
                            info!(sprites = tileset.file_header.sprite_count, "Tileset loaded");

                            // Load the palette from a PCX in PIC/.
                            // TODO: The game uses a master VGA palette that differs from
                            // individual PCX palettes. For now we use OFFPIC2.PCX which
                            // has the closest match to the terrain colors.
                            let pic_dir = data_dir.join("WOW").join("PIC");
                            let pal_pcx = {
                                // Try OFFPIC2 first (office scene, closest to game palette)
                                let offpic = pic_dir.join("OFFPIC2.PCX");
                                if offpic.exists() {
                                    Some(offpic)
                                } else {
                                    std::fs::read_dir(&pic_dir).ok().and_then(|entries| {
                                        entries
                                            .flatten()
                                            .find(|e| {
                                                e.path().extension().map(|x| x.to_ascii_uppercase())
                                                    == Some("PCX".into())
                                            })
                                            .map(|e| e.path())
                                    })
                                }
                            };
                            if let Some(pcx_path) = pal_pcx {
                                match ow_render::palette::load_pcx_palette(&pcx_path) {
                                    Ok(pal) => {
                                        // Create tile renderer and load textures.
                                        let mut tr = ow_render::tile_renderer::TileMapRenderer::new(
                                            &texture_creator,
                                        );
                                        if let Err(e) = tr.load_tileset(&tileset, &pal) {
                                            warn!("Failed to load tileset textures: {e}");
                                        } else {
                                            let tw = tr.tile_pixel_width() as f32;
                                            let th = tr.tile_pixel_height() as f32;
                                            info!(
                                                tile_w = tw,
                                                tile_h = th,
                                                tiles = tr.tile_count(),
                                                "Tiles ready"
                                            );

                                            // Configure iso projection for the staggered grid.
                                            // Wages of War uses a staggered grid, NOT standard
                                            // diamond iso. Tile dimensions are 128x64 from the exe.
                                            // tile_width = 128 (full tile width, horizontal step)
                                            // tile_height = 64 (full tile height, vertical step)
                                            // Odd rows are offset +64px by tile_to_screen().
                                            let mis_iso = IsoConfig {
                                                tile_width: 128.0,
                                                tile_height: 64.0,
                                                origin_x: 0.0,
                                                origin_y: 0.0,
                                            };
                                            game.mission_iso = Some(IsoConfig {
                                                tile_width: 128.0,
                                                tile_height: 64.0,
                                                origin_x: 0.0,
                                                origin_y: 0.0,
                                            });
                                            mission_iso_config = Some(mis_iso);

                                            // Center the camera on the middle of the 140x72
                                            // staggered grid. Use the initial camera position
                                            // from the MAP file if available, otherwise center
                                            // on the map midpoint.
                                            // Camera position: use MAP's stored position if
                                            // available, otherwise center on the map.
                                            // Row spacing is half tile height (32px) for
                                            // interlocking diamonds.
                                            // The exe stores camera coords using 64px row
                                            // spacing, but we render with 32px (half-height
                                            // for diamond interlocking). Halve the Y value.
                                            let mid_x = if map.header.camera_x != 0 {
                                                map.header.camera_x as f32
                                            } else {
                                                (map.width() as f32 / 2.0) * 128.0
                                            };
                                            // Camera Y from MAP uses 64px row spacing but
                                            // we render at 32px (half-height). Halve it.
                                            let mid_y = if map.header.camera_y != 0 {
                                                (map.header.camera_y as f32) / 2.0
                                            } else {
                                                (map.height() as f32 / 2.0) * 32.0
                                            };
                                            game.camera.x =
                                                mid_x - (game.window_width as f32 / 2.0);
                                            game.camera.y =
                                                mid_y - (game.window_height as f32 / 2.0);
                                            tile_renderer = Some(tr);

                                            // Load the OBJ sprite sheet for map objects
                                            // (buildings, walls, fences, trees).
                                            // Same sprite container format as TIL, lives
                                            // in the same SPR/SCEN{n}/ directory.
                                            let obj_name =
                                                ow_data::map_loader::filename_from_build_path(
                                                    &map.asset_refs.object_sprite_path,
                                                );
                                            let obj_path = spr_scen_dir.join(obj_name);
                                            if obj_path.exists() {
                                                match ow_data::sprite::parse_sprite_file(&obj_path)
                                                {
                                                    Ok(obj_sheet) => {
                                                        info!(
                                                            sprites = obj_sheet.file_header.sprite_count,
                                                            path = %obj_path.display(),
                                                            "OBJ sprite sheet loaded"
                                                        );
                                                        let mut or = ow_render::tile_renderer::TileMapRenderer::new(&texture_creator);
                                                        if let Err(e) =
                                                            or.load_tileset(&obj_sheet, &pal)
                                                        {
                                                            warn!(
                                                                "Failed to load OBJ textures: {e}"
                                                            );
                                                        } else {
                                                            info!(
                                                                obj_tiles = or.tile_count(),
                                                                obj_w = or.tile_pixel_width(),
                                                                obj_h = or.tile_pixel_height(),
                                                                "OBJ textures ready"
                                                            );
                                                            obj_renderer = Some(or);
                                                        }
                                                    }
                                                    Err(e) => warn!(
                                                        "Failed to load OBJ sheet {obj_name}: {e}"
                                                    ),
                                                }
                                            } else {
                                                warn!(path = %obj_path.display(), "OBJ sprite file not found");
                                            }

                                            // Load soldier animation: COR index + DAT sprite frames.
                                            let anim_dir = data_dir.join("WOW").join("ANIM");
                                            let cor_path = anim_dir.join("JUNGSLD.COR");
                                            let sld_path = anim_dir.join("JUNGSLD.DAT");

                                            if cor_path.exists() {
                                                match ow_data::animation::parse_animation(&cor_path)
                                                {
                                                    Ok(anim_set) => {
                                                        info!(
                                                            entries = anim_set.entries.len(),
                                                            "COR animation index loaded"
                                                        );
                                                        soldier_anim_set = Some(anim_set);
                                                    }
                                                    Err(e) => {
                                                        warn!("Failed to parse JUNGSLD.COR: {e}")
                                                    }
                                                }
                                            }

                                            if sld_path.exists() {
                                                match ow_data::sprite::parse_sprite_file(&sld_path)
                                                {
                                                    Ok(sld_sheet) => {
                                                        let total = sld_sheet.frames.len();
                                                        let max_frames = total.min(2000);
                                                        info!(
                                                            total,
                                                            loading = max_frames,
                                                            "Decoding soldier frames"
                                                        );
                                                        soldier_textures.clear();
                                                        let mut decoded = 0u32;
                                                        for i in 0..max_frames {
                                                            let frame = &sld_sheet.frames[i];
                                                            let fw = frame.header.width as u32;
                                                            let fh = frame.header.height as u32;
                                                            if fw == 0 || fh == 0 {
                                                                soldier_textures.push(None);
                                                                continue;
                                                            }
                                                            let tex_opt = ow_data::sprite::decode_rle(
                                                                &frame.compressed_data, frame.header.width, frame.header.height, i,
                                                            ).ok().and_then(|pixels| {
                                                                let rgba = ow_render::palette::apply_palette_with_brightness(&pixels, &pal, 1.5);
                                                                let mut tex = texture_creator.create_texture_static(
                                                                    sdl2::pixels::PixelFormatEnum::RGBA32, fw, fh,
                                                                ).ok()?;
                                                                tex.set_blend_mode(sdl2::render::BlendMode::Blend);
                                                                tex.update(None, &rgba, (fw * 4) as usize).ok()?;
                                                                decoded += 1;
                                                                Some(tex)
                                                            });
                                                            soldier_textures.push(tex_opt);
                                                        }
                                                        info!(
                                                            decoded,
                                                            "Soldier animation frames ready"
                                                        );

                                                        // Create per-merc AnimControllers in idle pose.
                                                        if let Some(ref anim_set) = soldier_anim_set
                                                        {
                                                            soldier_anims.clear();
                                                            for _merc in &game.game_state.team {
                                                                let mut ctrl = ow_render::anim_controller::AnimController::new(anim_set.clone());
                                                                ctrl.set_action(
                                                                    ow_render::anim_controller::AnimAction::Idle,
                                                                    ow_render::anim_controller::Direction::S, 1,
                                                                );
                                                                soldier_anims.push(ctrl);
                                                            }
                                                            info!(
                                                                controllers = soldier_anims.len(),
                                                                "AnimControllers ready"
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        warn!("Failed to load JUNGSLD.DAT: {e}")
                                                    }
                                                }
                                            } else {
                                                warn!(path = %sld_path.display(), "JUNGSLD.DAT not found");
                                            }
                                        }
                                    }
                                    Err(e) => warn!("Palette error: {e}"),
                                }
                            }
                            // Generate enemy units from mission data.
                            let mission_key = format!("MSSN{mission_num:02}");
                            if let Some(mission_data) = ruleset.missions.get(&mission_key) {
                                let mut rng = rand::thread_rng();
                                // Generate enemies with random positions on the map.
                                let max_player_id =
                                    game.game_state.team.iter().map(|m| m.id).max().unwrap_or(0);
                                let mut next_id = max_player_id + 1000;

                                for (i, rating) in mission_data.enemy_ratings.iter().enumerate() {
                                    use rand::Rng;
                                    // Roll for presence
                                    let roll: u8 = rng.gen_range(0..100);
                                    if roll >= rating.presence_chance {
                                        continue;
                                    }
                                    // Generate enemy with a random position in the upper portion of the map.
                                    let ex: i32 = rng.gen_range(20..180);
                                    let ey: i32 = rng.gen_range(10..100);
                                    let default_weapon = ow_data::mission::EnemyWeapon {
                                        weapon1: -1,
                                        weapon2: -1,
                                        ammo1: 0,
                                        ammo2: 0,
                                        weapon3: -1,
                                        extra: 0,
                                    };
                                    let weapon = mission_data
                                        .enemy_weapons
                                        .get(i)
                                        .unwrap_or(&default_weapon);
                                    let mut enemy = ow_core::mission_setup::EnemyUnit::from_rating(
                                        next_id, rating, weapon,
                                    );
                                    enemy.position = Some(ow_core::merc::TilePos { x: ex, y: ey });
                                    next_id += 1;
                                    enemy_units.push(enemy);
                                }
                                game.enemies = enemy_units.clone();
                                info!(enemies = enemy_units.len(), "Enemies generated for mission");
                            }

                            loaded_map = Some(map);
                        }
                        Err(e) => warn!("Failed to load tileset {til_name}: {e}"),
                    }
                }
                Err(e) => warn!("Failed to load map {}: {e}", map_path.display()),
            }
        }

        // -- Update window dimensions every frame (handles fullscreen, DPI changes,
        // and resize events we might miss). Cheap call, prevents coordinate bugs. --
        let (cw, ch) = canvas.window().size();
        game.window_width = cw;
        game.window_height = ch;

        // -- Render --
        let bg = phase_background_color(&game.phase_handler);
        canvas.set_draw_color(bg);
        canvas.clear();

        render_phase(
            &game,
            &mut canvas,
            &text_renderer,
            &texture_creator,
            &ruleset,
            &office_texture,
            &tile_renderer,
            &obj_renderer,
            &loaded_map,
            &mission_iso_config,
            &soldier_texture,
            &acct_textures,
            &phone_textures,
            &soldier_textures,
            &soldier_anims,
        );

        // Title bar shows the current phase (placeholder for real UI)
        let label = phase_label(&game.phase_handler);
        canvas
            .window_mut()
            .set_title(&format!("Open Wages \u{2014} {label}"))
            .ok();

        canvas.present();

        // Dev auto-screenshot tick. Cheap when disabled (just two `if let`s).
        if let (Some(ms), Some(dir)) = (auto_ss_interval_ms, auto_ss_dir.as_ref()) {
            if now.duration_since(last_auto_ss).as_millis() >= ms {
                let phase = phase_label(&game.phase_handler)
                    .replace(' ', "_")
                    .replace('—', "-")
                    .replace('/', "_")
                    .to_lowercase();
                let path = dir.join(format!("ss_{:05}_{phase}.bmp", auto_ss_count));
                save_screenshot_to_path(&canvas, &path);
                auto_ss_count = auto_ss_count.saturating_add(1);
                last_auto_ss = now;
            }
        }

        // -- Frame pacing --
        // Sleep for remaining frame budget to hit ~60 fps.
        let frame_elapsed = now.elapsed().as_millis() as u32;
        if frame_elapsed < TARGET_FRAME_MS {
            std::thread::sleep(std::time::Duration::from_millis(
                (TARGET_FRAME_MS - frame_elapsed) as u64,
            ));
        }
    }

    // Clean up music before exit.
    drop(_music_handle);
    // Drop voice player before closing audio device — cached Chunks
    // must be freed while the mixer is still open.
    drop(voice_player);
    if audio_available {
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
// Screenshot — F12 saves the current frame to disk as BMP
// ---------------------------------------------------------------------------

/// Save the current canvas contents to a specific path. Used by the dev
/// auto-screenshot loop, which already controls the filename and doesn't
/// need the collision search the F12 path takes.
fn save_screenshot_to_path(canvas: &Canvas<Window>, path: &std::path::Path) {
    let (w, h) = canvas.output_size().unwrap_or((1280, 720));
    match canvas.read_pixels(None, sdl2::pixels::PixelFormatEnum::RGB24) {
        Ok(pixels) => {
            match sdl2::surface::Surface::from_data_pixelmasks(
                &mut pixels.clone(),
                w,
                h,
                w * 3,
                &sdl2::pixels::PixelMasks {
                    bpp: 24,
                    rmask: 0xFF0000,
                    gmask: 0x00FF00,
                    bmask: 0x0000FF,
                    amask: 0,
                },
            ) {
                Ok(surface) => {
                    if let Err(e) = surface.save_bmp(path) {
                        warn!(path = %path.display(), "auto-screenshot save_bmp: {e}");
                    }
                }
                Err(e) => warn!(path = %path.display(), "auto-screenshot surface: {e}"),
            }
        }
        Err(e) => warn!(path = %path.display(), "auto-screenshot read_pixels: {e}"),
    }
}

/// Save the current canvas contents to a BMP file.
/// Files are named screenshot_001.bmp, screenshot_002.bmp, etc.
fn save_screenshot(canvas: &Canvas<Window>) {
    // Find the next available screenshot number.
    let mut num = 1u32;
    loop {
        let path = format!("screenshot_{num:03}.bmp");
        if !std::path::Path::new(&path).exists() {
            // Read pixels from the canvas in its native format.
            let (w, h) = canvas.output_size().unwrap_or((1280, 720));
            match canvas.read_pixels(None, sdl2::pixels::PixelFormatEnum::RGB24) {
                Ok(pixels) => {
                    // RGB24 = 3 bytes per pixel, no alpha confusion.
                    match sdl2::surface::Surface::from_data_pixelmasks(
                        &mut pixels.clone(),
                        w,
                        h,
                        w * 3,
                        &sdl2::pixels::PixelMasks {
                            bpp: 24,
                            rmask: 0xFF0000,
                            gmask: 0x00FF00,
                            bmask: 0x0000FF,
                            amask: 0,
                        },
                    ) {
                        Ok(surface) => match surface.save_bmp(&path) {
                            Ok(()) => info!("Screenshot saved: {path}"),
                            Err(e) => warn!("Failed to save screenshot: {e}"),
                        },
                        Err(e) => warn!("Failed to create screenshot surface: {e}"),
                    }
                }
                Err(e) => warn!("Failed to read pixels for screenshot: {e}"),
            }
            break;
        }
        num += 1;
        if num > 999 {
            warn!("Too many screenshots (>999)");
            break;
        }
    }
}
