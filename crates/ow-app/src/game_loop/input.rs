//! Phase-specific input handling — routes SDL2 events to per-phase handlers.

use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::mouse::MouseButton;
use tracing::{debug, info, trace, warn};

use ow_audio::sfx::{CombatSound, SfxManager};
use ow_audio::voice::VoicePlayer;
use ow_core::game_state::{GamePhase, MissionPhase, OfficePhase};
use ow_core::merc::MercId;
use ow_core::ruleset::Ruleset;
use ow_render::camera::Camera;
use ow_render::iso_math::ScreenPos;

use super::{log_combat, CombatHandler, CombatLogKind, GameLoop, PhaseHandler};

// ===========================================================================
// Escape / Pause handling
// ===========================================================================

/// Handle the ESC key. Returns `false` if the game should quit.
pub(crate) fn handle_escape(game: &mut GameLoop) -> bool {
    match &game.phase_handler {
        // If we're in an office sub-screen (not Overview), ESC goes back to
        // the office desk. This is how the original game works — ESC closes
        // the current overlay and returns to the main office scene.
        PhaseHandler::Office { sub_phase } if *sub_phase != OfficePhase::Overview => {
            info!(from = ?sub_phase, "Returning to office overview");
            game.game_state
                .set_phase(GamePhase::Office(OfficePhase::Overview));
            game.phase_handler = PhaseHandler::Office {
                sub_phase: OfficePhase::Overview,
            };
            true
        }

        // From pause, ESC resumes (not quit — that was too aggressive).
        // Use the window X button or Alt+F4 to actually quit.
        PhaseHandler::Paused { .. } => {
            info!("Resuming from pause");
            let prev = std::mem::replace(
                &mut game.phase_handler,
                PhaseHandler::Office {
                    sub_phase: OfficePhase::Overview,
                },
            );
            if let PhaseHandler::Paused { previous } = prev {
                game.phase_handler = *previous;
            }
            true
        }

        // From the office overview or any other screen, ESC pauses.
        _ => {
            info!("Entering pause");
            let current = std::mem::replace(
                &mut game.phase_handler,
                PhaseHandler::Office {
                    sub_phase: OfficePhase::Overview,
                },
            );
            game.phase_handler = PhaseHandler::Paused {
                previous: Box::new(current),
            };
            true
        }
    }
}

// ===========================================================================
// Phase-specific input handling
// ===========================================================================

/// Route input events to the active phase handler.
///
/// To satisfy the borrow checker, each branch extracts any needed values from
/// `game.phase_handler` by copy/clone *before* passing `game` to sub-handlers.
/// Phase transitions replace `game.phase_handler` wholesale rather than
/// mutating through a partial borrow.
pub(crate) fn handle_phase_input(
    game: &mut GameLoop,
    event: &Event,
    ruleset: &Ruleset,
    sfx: &mut SfxManager,
    voice: &mut Option<VoicePlayer>,
) {
    // Take a snapshot of the current phase discriminant to route input.
    // We avoid borrowing game.phase_handler across the handler calls.
    enum Route {
        Paused,
        Office,
        Travel,
        Deployment,
        Combat,
        Extraction,
        Debrief,
    }

    let route = match &game.phase_handler {
        PhaseHandler::Paused { .. } => Route::Paused,
        PhaseHandler::Office { .. } => Route::Office,
        PhaseHandler::Travel { .. } => Route::Travel,
        PhaseHandler::Deployment { .. } => Route::Deployment,
        PhaseHandler::Combat(_) => Route::Combat,
        PhaseHandler::Extraction => Route::Extraction,
        PhaseHandler::Debrief { .. } => Route::Debrief,
    };

    match route {
        Route::Paused => handle_pause_input(game, event),
        Route::Office => handle_office_input(game, event, ruleset, voice),
        Route::Travel => { /* No player input during travel */ }
        Route::Deployment => handle_deployment_input(game, event),
        Route::Combat => handle_combat_input(game, event, ruleset, sfx, voice),
        Route::Extraction => handle_extraction_input(game, event),
        Route::Debrief => handle_debrief_input(game, event),
    }
}

// ---------------------------------------------------------------------------
// Pause input
// ---------------------------------------------------------------------------

/// While paused, Enter resumes.
fn handle_pause_input(game: &mut GameLoop, event: &Event) {
    if let Event::KeyDown {
        keycode: Some(Keycode::Return),
        ..
    } = event
    {
        info!("Resuming from pause");
        // Extract the previous handler from the Paused variant.
        let prev = match std::mem::replace(
            &mut game.phase_handler,
            PhaseHandler::Office {
                sub_phase: OfficePhase::Overview,
            },
        ) {
            PhaseHandler::Paused { previous } => *previous,
            other => other, // shouldn't happen, but be safe
        };
        game.phase_handler = prev;
    }
}

// ---------------------------------------------------------------------------
// Office input
// ---------------------------------------------------------------------------

/// Handle input while in the Office phase.
///
/// Number keys 1-6 switch between sub-phases:
///   1 = Overview, 2 = Hire Mercs, 3 = Equipment,
///   4 = Intel, 5 = Contracts, 6 = Training
///
/// 'B' begins a mission (transitions to Travel) if preconditions are met:
///   - At least one merc hired
///   - A contract accepted (placeholder: always allowed for now)
/// Map a mouse click on the office scene to a game action.
///
/// The original office screen is 640x480. We scale mouse coordinates from
/// the actual window size down to 640x480 space, then check which clickable
/// object the player hit. Each object on the desk maps to a game function:
///
/// - Filing cabinet (left side)  → View Files
/// - Fax machine (lower left)    → Contracts (Use Fax)
/// - Calculator (center desk)    → Calculator
/// - Pizza box (center-low desk) → Eat Pizza (easter egg)
/// - Phone (right side)          → Hire Mercs / Arm Mercs
/// - World map (wall, right)     → World Map / Intel
/// - Door (far right)            → Begin Mission
/// - Magazines (desk, left)      → Equipment catalog
fn handle_office_input(
    game: &mut GameLoop,
    event: &Event,
    ruleset: &Ruleset,
    voice: &mut Option<VoicePlayer>,
) {
    // Get current sub-phase.
    let current_sub = if let PhaseHandler::Office { sub_phase } = &game.phase_handler {
        *sub_phase
    } else {
        return;
    };
    // Helper: check if a point is inside a rect defined in 640x480 space.
    // We scale the mouse coordinates from window size to 640x480.
    // Scale mouse coords to the 640x480 game coordinate space.
    // On high-DPI displays, SDL2 mouse events use LOGICAL pixels
    // (window size), not physical pixels (canvas output size).
    // We use game.window_width/height (logical) for mouse mapping.
    let check_hit =
        |mx: i32, my: i32, x1: i32, y1: i32, x2: i32, y2: i32, ww: u32, wh: u32| -> bool {
            let sx = (mx as f32 * 640.0 / ww as f32) as i32;
            let sy = (my as f32 * 480.0 / wh as f32) as i32;
            sx >= x1 && sx <= x2 && sy >= y1 && sy <= y2
        };

    match event {
        // Mouse click on the office scene — check which object was clicked.
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            let (ww, wh) = (game.window_width, game.window_height);

            // --- HireMercs: clicking a merc row hires or fires them ---
            if current_sub == OfficePhase::HireMercs {
                // The merc list renders starting at y=85px (content_y=50 + header=35).
                // Each row is 16px tall. Match the render order: sorted by rating desc.
                let list_start_y = 85i32;
                let row_h = 16i32;
                let click_y = *y;

                if click_y >= list_start_y {
                    let row = ((click_y - list_start_y) / row_h) as usize;

                    // Build the same sorted merc list as the renderer.
                    let mut sorted_mercs: Vec<_> = ruleset.mercs.values().collect();
                    sorted_mercs.sort_by(|a, b| b.rating.cmp(&a.rating));

                    if let Some(merc) = sorted_mercs.get(row) {
                        let already_hired =
                            game.game_state.team.iter().any(|m| m.name == merc.name);

                        if already_hired {
                            // Fire the merc — remove from team (no refund, like the original).
                            game.game_state.team.retain(|m| m.name != merc.name);
                            info!(name = %merc.name, "Fired mercenary");
                        } else if merc.avail == 1 {
                            // Hire the merc — check funds and team size.
                            if game.game_state.team.len() >= 8 {
                                warn!("Team full (max 8 mercs)");
                            } else if game.game_state.funds < merc.fee_hire as i64 {
                                warn!(name = %merc.name, cost = merc.fee_hire, funds = game.game_state.funds,
                                      "Cannot afford to hire");
                            } else {
                                // Deduct funds and add to team.
                                game.game_state.funds -= merc.fee_hire as i64;
                                let id = game.game_state.team.len() as u32 + 1;
                                let active = ow_core::merc::ActiveMerc::from_data(id, merc);
                                info!(name = %merc.name, cost = merc.fee_hire,
                                      remaining_funds = game.game_state.funds, "Hired mercenary");
                                game.game_state.team.push(active);
                                // Play the mercs voice line on hire — the original game
                                // plays a greeting/intro clip when you add someone to your team.
                                if let Some(vp) = voice.as_mut() {
                                    vp.play(&merc.name);
                                }
                            }
                        } else {
                            info!(name = %merc.name, "Merc unavailable for hire");
                        }
                    }
                }
                return; // Don't fall through to office overview hotspots.
            }

            // --- Contracts: click a mission to accept/switch contracts ---
            if current_sub == OfficePhase::Contracts {
                // Contract list starts at y=107 (content_y=50 + header=35 + accepted_line=22).
                // If no contract is accepted yet, list starts at y=85.
                let has_accepted = game.game_state.current_mission.is_some();
                let list_start_y = if has_accepted { 107i32 } else { 85i32 };
                let row_h = 18i32;
                let click_y = *y;

                if click_y >= list_start_y {
                    let row = ((click_y - list_start_y) / row_h) as usize;

                    // Build sorted mission ID list (same order as render).
                    let mut mission_ids: Vec<_> = ruleset.missions.keys().collect();
                    mission_ids.sort();

                    if let Some(mid) = mission_ids.get(row) {
                        if let Some(mission) = ruleset.missions.get(*mid) {
                            // Accept this contract — credit the advance to funds.
                            let already_accepted = game
                                .game_state
                                .current_mission
                                .as_ref()
                                .map(|m| m.name == **mid)
                                .unwrap_or(false);

                            if already_accepted {
                                info!(mission = %mid, "Contract already accepted");
                            } else {
                                // If switching contracts, no refund on old advance.
                                let advance = mission.contract.advance;
                                game.game_state.funds += advance as i64;
                                game.game_state.current_mission =
                                    Some(ow_core::game_state::MissionContext {
                                        name: mid.to_string(),
                                        weather: ow_core::weather::Weather::Clear,
                                        combat: None,
                                        turn_number: 0,
                                    });
                                info!(mission = %mid, advance = advance,
                                      funds = game.game_state.funds, "Contract accepted!");
                            }
                        }
                    }
                }
                return;
            }

            // --- Equipment: clicking a weapon row leases it to the first unarmed merc ---
            if current_sub == OfficePhase::Equipment {
                // Weapon list starts at y=105 (content_y=50 + header=35 + section_header=20).
                // Each row is 14px tall. Match the render order: sorted by weapon_type name.
                let list_start_y = 105i32;
                let row_h = 14i32;
                let click_y = *y;

                if click_y >= list_start_y {
                    let row = ((click_y - list_start_y) / row_h) as usize;

                    // Build the same sorted weapon list as the renderer.
                    let mut sorted_weapons: Vec<_> = ruleset.weapons.values().collect();
                    sorted_weapons.sort_by_key(|w| format!("{:?}", w.weapon_type));

                    if let Some(weapon) = sorted_weapons.get(row) {
                        // Check if there's an unarmed merc to assign to.
                        let unarmed_idx = game
                            .game_state
                            .team
                            .iter()
                            .position(|m| m.inventory.is_empty());

                        if let Some(idx) = unarmed_idx {
                            // Check funds.
                            if game.game_state.funds < weapon.cost as i64 {
                                warn!(weapon = %weapon.name, cost = weapon.cost,
                                      funds = game.game_state.funds, "Cannot afford weapon lease");
                            } else {
                                // Deduct cost and assign weapon to the merc.
                                game.game_state.funds -= weapon.cost as i64;
                                let merc_name = game.game_state.team[idx].name.clone();
                                game.game_state.team[idx].inventory.push(
                                    ow_core::merc::InventoryItem {
                                        name: weapon.name.clone(),
                                        encumbrance: weapon.encumbrance,
                                    },
                                );
                                info!(weapon = %weapon.name, cost = weapon.cost,
                                      merc = %merc_name,
                                      remaining_funds = game.game_state.funds,
                                      "Leased weapon to merc");
                            }
                        } else {
                            warn!(weapon = %weapon.name, "No unarmed mercs to assign weapon to");
                        }
                    }
                }
                return;
            }

            // Check each clickable hotspot (640x480 coords from the original game).
            // Hotspots are checked in priority order — more specific areas first
            // to prevent overlap issues (e.g., phone vs world map).
            //
            // The office layout (from OFFPIC2.PCX):
            //   Top-left: window with desert view
            //   Left: filing cabinet (green, tall)
            //   Center: green desk pad with calculator, coffee mug
            //   Right: white telephone, desk lamp
            //   Far right wall: world map, fax machine on side table
            //   Background: door with "MERCS INC" glass, ceiling fan
            //   Bottom-left: magazines/catalogs on desk
            // Generous hotspots covering the full visual objects on OFFPIC2.
            // The original MAIN.BTN has tiny 22px icon buttons designed for
            // sprite overlays we don't render yet. These bigger rects match
            // what the player visually sees and can click comfortably.
            let sx = (*x as f32 * 640.0 / ww as f32) as i32;
            let sy = (*y as f32 * 480.0 / wh as f32) as i32;
            info!(
                window_x = x,
                window_y = y,
                game_x = sx,
                game_y = sy,
                "Office click"
            );

            // Coordinates measured from 640x480 grid overlay on OFFPIC2.PCX.
            let action = if check_hit(*x, *y, 400, 340, 520, 430, ww, wh) {
                // Phone (right side of desk) → Hire Mercenaries
                Some(("Hire Mercenaries", OfficePhase::HireMercs))
            } else if check_hit(*x, *y, 480, 230, 560, 310, ww, wh) {
                // Fax machine (on side table, far right) → Contracts
                Some(("Contracts (Fax)", OfficePhase::Contracts))
            } else if check_hit(*x, *y, 230, 330, 310, 380, ww, wh) {
                // Calculator (on green desk pad) → Training
                Some(("Training (Calculator)", OfficePhase::Training))
            } else if check_hit(*x, *y, 490, 50, 620, 190, ww, wh) {
                // World map (on wall, upper right) → Intel
                Some(("Mission Intel", OfficePhase::Intel))
            } else if check_hit(*x, *y, 70, 170, 130, 370, ww, wh) {
                // Filing cabinet (left wall) → View Files / Intel
                Some(("View Files (Cabinet)", OfficePhase::Intel))
            } else if check_hit(*x, *y, 100, 360, 220, 430, ww, wh) {
                // Magazines on desk (lower left) → Equipment
                Some(("Equipment (Magazines)", OfficePhase::Equipment))
            } else if check_hit(*x, *y, 240, 40, 370, 250, ww, wh) {
                // Door → Begin Mission (requires hired mercs AND accepted contract)
                if game.game_state.team.is_empty() {
                    warn!("Cannot begin mission: no mercs hired");
                    None
                } else if game.game_state.current_mission.is_none() {
                    warn!("Cannot begin mission: no contract accepted (click fax first)");
                    None
                } else {
                    info!(team_size = game.game_state.team.len(),
                          mission = %game.game_state.current_mission.as_ref().unwrap().name,
                          "Beginning mission");
                    game.game_state.set_phase(GamePhase::Travel);
                    game.phase_handler = PhaseHandler::Travel { elapsed_ms: 0 };
                    None
                }
            } else {
                None
            };

            if let Some((label, sub)) = action {
                info!(action = label, "Office click");
                game.game_state.set_phase(GamePhase::Office(sub));
                game.phase_handler = PhaseHandler::Office { sub_phase: sub };
            }
        }

        // Keyboard shortcuts still work as fallback.
        Event::KeyDown {
            keycode: Some(key), ..
        } => {
            let new_sub = match *key {
                // ESC returns to the overview (office desk scene).
                Keycode::Escape => {
                    // Only go to overview if we're in a sub-screen, not if we're
                    // already at overview (which would trigger the pause handler).
                    if let PhaseHandler::Office { sub_phase } = &game.phase_handler {
                        if *sub_phase != OfficePhase::Overview {
                            Some(OfficePhase::Overview)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                Keycode::Num1 => Some(OfficePhase::HireMercs),
                Keycode::Num2 => Some(OfficePhase::Equipment),
                Keycode::Num3 => Some(OfficePhase::Intel),
                Keycode::Num4 => Some(OfficePhase::Contracts),
                Keycode::Num5 => Some(OfficePhase::Training),
                Keycode::U if current_sub == OfficePhase::Equipment => {
                    // Unequip all weapons from all mercs, refunding lease costs.
                    let mut total_refund: i64 = 0;
                    for merc in &mut game.game_state.team {
                        for item in merc.inventory.drain(..) {
                            // Look up the weapon cost for refund.
                            if let Some(weapon) =
                                ruleset.weapons.values().find(|w| w.name == item.name)
                            {
                                total_refund += weapon.cost as i64;
                                info!(weapon = %item.name, refund = weapon.cost,
                                      merc = %merc.name, "Returned leased weapon");
                            } else {
                                info!(item = %item.name, merc = %merc.name,
                                      "Returned item (no cost lookup)");
                            }
                        }
                    }
                    if total_refund > 0 {
                        game.game_state.funds += total_refund;
                        info!(
                            total_refund,
                            funds = game.game_state.funds,
                            "All weapons returned — funds refunded"
                        );
                    } else {
                        info!("No weapons to return");
                    }
                    None
                }
                Keycode::B => {
                    if game.game_state.team.is_empty() {
                        warn!("Cannot begin mission: no mercs hired");
                        None
                    } else if game.game_state.current_mission.is_none() {
                        warn!("Cannot begin mission: no contract accepted");
                        None
                    } else {
                        info!(team_size = game.game_state.team.len(),
                              mission = %game.game_state.current_mission.as_ref().unwrap().name,
                              "Beginning mission");
                        game.game_state.set_phase(GamePhase::Travel);
                        game.phase_handler = PhaseHandler::Travel { elapsed_ms: 0 };
                        None
                    }
                }
                _ => None,
            };

            if let Some(sub) = new_sub {
                debug!(sub_phase = ?sub, "Office sub-phase switch");
                game.game_state.set_phase(GamePhase::Office(sub));
                game.phase_handler = PhaseHandler::Office { sub_phase: sub };
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Deployment input
// ---------------------------------------------------------------------------

/// Handle input during the deployment phase.
///
/// - Tab: cycle through mercs to place.
/// - Click: place selected merc on the clicked tile.
/// - Enter: confirm deployment, start combat.
/// - WASD: scroll camera.
fn handle_deployment_input(game: &mut GameLoop, event: &Event) {
    match event {
        // WASD / Arrow keys: scroll the camera around the map.
        Event::KeyDown {
            keycode: Some(key), ..
        } if matches!(
            *key,
            Keycode::W
                | Keycode::A
                | Keycode::S
                | Keycode::D
                | Keycode::Up
                | Keycode::Down
                | Keycode::Left
                | Keycode::Right
        ) =>
        {
            let speed = 32.0;
            match *key {
                Keycode::W | Keycode::Up => game.camera.scroll(0.0, -speed),
                Keycode::S | Keycode::Down => game.camera.scroll(0.0, speed),
                Keycode::A | Keycode::Left => game.camera.scroll(-speed, 0.0),
                Keycode::D | Keycode::Right => game.camera.scroll(speed, 0.0),
                _ => {}
            }
        }

        // +/- zoom
        Event::KeyDown {
            keycode: Some(Keycode::Equals),
            ..
        }
        | Event::KeyDown {
            keycode: Some(Keycode::Plus),
            ..
        } => {
            game.camera.zoom_in();
        }
        Event::KeyDown {
            keycode: Some(Keycode::Minus),
            ..
        } => {
            game.camera.zoom_out();
        }

        // Mouse wheel zoom
        Event::MouseWheel { y, .. } => {
            if *y > 0 {
                game.camera.zoom_in();
            } else if *y < 0 {
                game.camera.zoom_out();
            }
        }

        // Tab: cycle to next merc for placement
        Event::KeyDown {
            keycode: Some(Keycode::Tab),
            ..
        } => {
            let team_len = game.game_state.team.len();
            if team_len > 0 {
                if let PhaseHandler::Deployment { selected_unit } = &mut game.phase_handler {
                    *selected_unit = (*selected_unit + 1) % team_len;
                    debug!(
                        selected = *selected_unit,
                        name = %game.game_state.team[*selected_unit].name,
                        "Deployment: selected next merc"
                    );
                }
            }
        }

        // Click: place selected merc on the clicked tile
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            let screen = ScreenPos {
                x: *x as f32,
                y: *y as f32,
            };
            let world = game.camera.screen_to_world(screen);
            // Use mission iso config if available (actual tile dimensions),
            // fall back to default iso config.
            let iso = game.mission_iso.as_ref().unwrap_or(&game.iso_config);
            let tile = iso.screen_to_tile(world);
            let core_tile = ow_core::merc::TilePos {
                x: tile.x,
                y: tile.y,
            };

            // Read the selected index, place the merc, then advance
            let selected = match &game.phase_handler {
                PhaseHandler::Deployment { selected_unit } => *selected_unit,
                _ => return,
            };
            let team_len = game.game_state.team.len();
            if selected < team_len {
                info!(
                    name = %game.game_state.team[selected].name,
                    tile_x = tile.x,
                    tile_y = tile.y,
                    "Deployment: placed merc"
                );
                game.game_state.team[selected].position = Some(core_tile);

                // Auto-advance to next unplaced merc
                if let PhaseHandler::Deployment { selected_unit } = &mut game.phase_handler {
                    *selected_unit = (*selected_unit + 1) % team_len;
                }
            }
        }

        // Enter: confirm deployment, transition to combat
        Event::KeyDown {
            keycode: Some(Keycode::Return),
            ..
        } => {
            let placed = game
                .game_state
                .team
                .iter()
                .filter(|m| m.position.is_some())
                .count();
            let total = game.game_state.team.len();

            if placed == 0 {
                warn!("Cannot start combat: no mercs placed on the map");
                return;
            }

            info!(
                placed,
                total, "Deployment confirmed -- transitioning to Combat"
            );
            game.game_state
                .set_phase(GamePhase::Mission(MissionPhase::Combat));

            // Build initiative order from placed, living player units.
            // Build initiative order: interleave player mercs and enemies.
            // All units sorted by initiative (EXP + WIL) — highest first.
            // This is the core WoW mechanic: NOT I-go-you-go, but all
            // units mixed by initiative regardless of faction.
            let mut init_order: Vec<MercId> = Vec::new();
            for merc in &game.game_state.team {
                if merc.position.is_some() && merc.is_alive() {
                    init_order.push(merc.id);
                }
            }
            for enemy in &game.enemies {
                if enemy.current_hp > 0 && enemy.position.is_some() {
                    init_order.push(enemy.id);
                }
            }
            let first_id = init_order.first().copied();

            game.phase_handler = PhaseHandler::Combat(CombatHandler {
                initiative_order: init_order,
                current_initiative_idx: 0,
                selected_unit_id: first_id,
                ai_acting: false,
                tab_cycle_index: 0,
            });
        }

        // WASD camera scrolling
        Event::KeyDown {
            keycode: Some(key @ (Keycode::W | Keycode::A | Keycode::S | Keycode::D)),
            ..
        } => {
            apply_camera_scroll(&mut game.camera, *key);
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Combat input
// ---------------------------------------------------------------------------

/// Handle input during the combat phase.
///
/// - WASD: scroll camera
/// - Tab: cycle through player units
/// - Left click on tile: move selected unit (if reachable)
/// - Left click on enemy: shoot if in range and LOS
/// - 'E': end current unit's turn
/// - Mouse wheel: zoom in/out
///
/// When AI is acting, player input is blocked.
fn handle_combat_input(
    game: &mut GameLoop,
    event: &Event,
    ruleset: &Ruleset,
    sfx: &mut SfxManager,
    voice: &mut Option<VoicePlayer>,
) {
    // Check if the AI is acting — block player input if so.
    let ai_acting = match &game.phase_handler {
        PhaseHandler::Combat(c) => c.ai_acting,
        _ => return,
    };
    if ai_acting {
        trace!("Combat input blocked: AI is acting");
        return;
    }

    match event {
        // Camera scrolling
        Event::KeyDown {
            keycode: Some(key @ (Keycode::W | Keycode::A | Keycode::S | Keycode::D)),
            ..
        } => {
            apply_camera_scroll(&mut game.camera, *key);
        }

        // Tab: cycle through living player units
        Event::KeyDown {
            keycode: Some(Keycode::Tab),
            ..
        } => {
            let living: Vec<MercId> = game
                .game_state
                .team
                .iter()
                .filter(|m| m.is_alive() && m.position.is_some())
                .map(|m| m.id)
                .collect();

            if let PhaseHandler::Combat(c) = &mut game.phase_handler {
                if !living.is_empty() {
                    c.tab_cycle_index = (c.tab_cycle_index + 1) % living.len();
                    c.selected_unit_id = Some(living[c.tab_cycle_index]);
                    debug!(selected = ?c.selected_unit_id, "Tab-cycled to next player unit");
                }
                // Play a voice line for the newly-selected merc so the player
                // gets audio feedback on who they just tabbed to.
                let sel_id = living[c.tab_cycle_index];
                if let Some(vp) = voice.as_mut() {
                    if let Some(merc) = game.game_state.team.iter().find(|m| m.id == sel_id) {
                        vp.play(&merc.name);
                    }
                }
            }
        }

        // E: end current unit's turn
        Event::KeyDown {
            keycode: Some(Keycode::E),
            ..
        } => {
            let selected = match &game.phase_handler {
                PhaseHandler::Combat(c) => c.selected_unit_id,
                _ => None,
            };
            if let Some(unit_id) = selected {
                info!(unit_id, "Player ended unit's turn");
                advance_initiative(game);
            }
        }

        // Mouse click: move or shoot depending on what occupies the target tile
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            let selected = match &game.phase_handler {
                PhaseHandler::Combat(c) => c.selected_unit_id,
                _ => None,
            };
            if let Some(unit_id) = selected {
                let screen = ScreenPos {
                    x: *x as f32,
                    y: *y as f32,
                };
                let world = game.camera.screen_to_world(screen);
                let iso = game.mission_iso.as_ref().unwrap_or(&game.iso_config);
                let tile = iso.screen_to_tile(world);
                let target_tile = ow_core::merc::TilePos {
                    x: tile.x,
                    y: tile.y,
                };

                // Check if an enemy is at or near the clicked tile.
                // If so, shoot them. Otherwise, move there.
                let enemy_idx = game.enemies.iter().position(|e| {
                    e.current_hp > 0
                        && e.position
                            .map(|p| {
                                // Click within 2 tiles of an enemy = target them
                                (p.x - target_tile.x).abs() <= 2 && (p.y - target_tile.y).abs() <= 2
                            })
                            .unwrap_or(false)
                });

                if let Some(eidx) = enemy_idx {
                    // SHOOT — deal damage to the enemy.
                    //
                    // The previous version rolled `rng.gen_range(5..20)` for
                    // damage regardless of what the merc was carrying. Now we
                    // look up the equipped weapon from the merc's inventory
                    // (first matching entry in `ruleset.weapons` by name) and
                    // use its `damage_class`, `weapon_range`, and `ap_cost`.
                    // Falls back to the old constants when the merc is
                    // unarmed so nobody breaks if equipment hookup misfires.
                    let attacker = game.game_state.team.iter().find(|m| m.id == unit_id);
                    let attacker_name = attacker
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| format!("Unit_{unit_id}"));
                    let wsk = attacker.map(|m| m.wsk).unwrap_or(50);
                    let attacker_pos = attacker.and_then(|m| m.position);

                    // Resolve weapon stats from inventory. We accept any
                    // inventory item that matches a weapon name; in practice
                    // the equipment screen pushes weapons in order so the
                    // first match is the primary.
                    let weapon = attacker.and_then(|m| {
                        m.inventory
                            .iter()
                            .find_map(|it| ruleset.weapons.values().find(|w| w.name == it.name))
                    });
                    let (weapon_dmg_class, weapon_range, weapon_ap) = match weapon {
                        Some(w) => (
                            w.damage_class.max(1),
                            w.weapon_range.max(1),
                            w.ap_cost.max(1),
                        ),
                        None => (8, 15, 8), // unarmed-ish fallback
                    };
                    let weapon_name = weapon.map(|w| w.name.as_str()).unwrap_or("(fists)");

                    // Range check: Manhattan distance > weapon range = miss.
                    let target_pos = game.enemies[eidx].position;
                    let range_tiles = match (attacker_pos, target_pos) {
                        (Some(a), Some(t)) => ((a.x - t.x).abs() + (a.y - t.y).abs()) as u32,
                        _ => 0,
                    };
                    let out_of_range = range_tiles > weapon_range;

                    // Simple hit chance based on weapon skill, halved if the
                    // shot is at the edge of range. Capped at 95 so there's
                    // always a chance to miss. Forced miss when out of range.
                    use rand::Rng;
                    let mut rng = rand::thread_rng();
                    let hit_roll: u32 = rng.gen_range(0..100);
                    let mut hit_chance = (wsk as u32).min(95);
                    if range_tiles > weapon_range / 2 {
                        hit_chance = hit_chance / 2;
                    }
                    if out_of_range {
                        hit_chance = 0;
                    }

                    // Collect combat log message after resolving the shot so we
                    // can call log_combat outside the mutable enemy borrow.
                    let log_msg: (String, CombatLogKind);

                    let enemy = &mut game.enemies[eidx];
                    if hit_roll < hit_chance {
                        // Hit! Damage from the weapon's `damage_class`, with
                        // a small +/- 25% jitter to keep it from being purely
                        // deterministic. ow-core's full resolve_attack also
                        // applies penetration vs armor — out of scope here
                        // until we plumb hit_table + armor through.
                        let base = weapon_dmg_class;
                        let lo = (base * 3 / 4).max(1);
                        let hi = (base * 5 / 4).max(lo + 1);
                        let damage = rng.gen_range(lo..=hi);
                        let old_hp = enemy.current_hp;
                        enemy.current_hp = enemy.current_hp.saturating_sub(damage);
                        info!(
                            shooter = unit_id,
                            weapon = weapon_name,
                            range_tiles,
                            target = %enemy.name,
                            damage,
                            old_hp,
                            new_hp = enemy.current_hp,
                            "HIT! Damage dealt"
                        );

                        // Deduct AP for shooting (weapon's real cost now).
                        if let Some(merc) =
                            game.game_state.team.iter_mut().find(|m| m.id == unit_id)
                        {
                            merc.current_ap = merc.current_ap.saturating_sub(weapon_ap);
                        }

                        if enemy.current_hp == 0 {
                            info!(target = %enemy.name, "Enemy KILLED!");
                            log_msg = (
                                format!("{attacker_name} hits {ename} for {damage} damage! {ename} KILLED!",
                                        ename = enemy.name),
                                CombatLogKind::Kill,
                            );
                        } else {
                            log_msg = (
                                format!("{attacker_name} hits {} for {damage} damage!", enemy.name),
                                CombatLogKind::PlayerHit,
                            );
                        }
                    } else {
                        info!(
                            shooter = unit_id,
                            weapon = weapon_name,
                            range_tiles,
                            out_of_range,
                            target = %enemy.name,
                            roll = hit_roll,
                            needed = hit_chance,
                            "MISS!"
                        );
                        let miss_msg = if out_of_range {
                            format!(
                                "{attacker_name}: {} out of range ({range_tiles} > {weapon_range})",
                                enemy.name
                            )
                        } else {
                            format!("{attacker_name} misses {}!", enemy.name)
                        };
                        log_msg = (miss_msg, CombatLogKind::Miss);
                        // Misses still burn AP — but only if the shot was
                        // physically possible. Out-of-range clicks are
                        // free so the player isn't punished for clicking.
                        if !out_of_range {
                            if let Some(merc) =
                                game.game_state.team.iter_mut().find(|m| m.id == unit_id)
                            {
                                merc.current_ap = merc.current_ap.saturating_sub(weapon_ap);
                            }
                        }
                    }

                    // Play gunshot SFX first (always plays on a shot attempt),
                    // then layer a hit/kill sound on top if applicable.
                    sfx.play(CombatSound::Pistol);
                    match log_msg.1 {
                        CombatLogKind::Kill => sfx.play(CombatSound::Kill),
                        CombatLogKind::Miss => sfx.play(CombatSound::Miss),
                        _ => {} // Hit uses just the gunshot
                    }

                    // Push the combat log entry (outside the enemy borrow).
                    log_combat(game, log_msg.0, log_msg.1);
                } else {
                    // MOVE — teleport to the clicked tile, deduct AP.
                    if let Some(merc) = game.game_state.team.iter_mut().find(|m| m.id == unit_id) {
                        // Simple AP cost: 2 per tile (Manhattan distance).
                        let cost = if let Some(old_pos) = merc.position {
                            let dist = (old_pos.x - target_tile.x).unsigned_abs()
                                + (old_pos.y - target_tile.y).unsigned_abs();
                            (dist * 2).min(merc.current_ap)
                        } else {
                            2
                        };
                        merc.current_ap = merc.current_ap.saturating_sub(cost);
                        merc.position = Some(target_tile);
                        info!(
                            name = %merc.name,
                            ap_cost = cost,
                            remaining_ap = merc.current_ap,
                            "Unit moved"
                        );
                    }
                }
            }
        }

        // Mouse wheel: zoom
        Event::MouseWheel { y, .. } => {
            if *y > 0 {
                game.camera.zoom_in();
            } else if *y < 0 {
                game.camera.zoom_out();
            }
        }

        _ => {}
    }
}

/// Advance to the next unit in the initiative order.
///
/// If all player units have acted, trigger the AI turn for enemy units.
/// If all units (player + enemy) have acted, start a new round with AP resets.
pub(crate) fn advance_initiative(game: &mut GameLoop) {
    // Extract what we need, then mutate.
    let (order_len, mut next_idx) = match &game.phase_handler {
        PhaseHandler::Combat(c) => (c.initiative_order.len(), c.current_initiative_idx + 1),
        _ => return,
    };

    if next_idx >= order_len {
        // All units acted this round — start new round.
        info!("Round complete -- starting new round");
        log_combat(game, "--- New Round ---".to_string(), CombatLogKind::Info);
        next_idx = 0;

        // Reset AP for all player units
        for merc in &mut game.game_state.team {
            if merc.is_alive() {
                let base = merc.base_aps as u32;
                merc.current_ap = if merc.suppressed { base / 2 } else { base };
                merc.suppressed = false;
                trace!(name = %merc.name, ap = merc.current_ap, "AP reset for new round");
            }
        }
    }

    // Determine who acts next
    if let PhaseHandler::Combat(c) = &mut game.phase_handler {
        c.current_initiative_idx = next_idx;

        if let Some(&next_id) = c.initiative_order.get(next_idx) {
            let is_player = game.game_state.team.iter().any(|m| m.id == next_id);
            if is_player {
                c.selected_unit_id = Some(next_id);
                c.ai_acting = false;
                debug!(unit_id = next_id, "Player unit's turn");
            } else {
                c.ai_acting = true;
                debug!(unit_id = next_id, "Enemy unit's turn -- AI deciding");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Extraction input
// ---------------------------------------------------------------------------

/// Handle input during the extraction phase.
/// Press Enter to finish the mission and go to Debrief.
/// WASD scrolls the camera.
fn handle_extraction_input(game: &mut GameLoop, event: &Event) {
    match event {
        Event::KeyDown {
            keycode: Some(Keycode::Return),
            ..
        } => {
            info!("Extraction complete -- transitioning to Debrief");
            game.game_state.set_phase(GamePhase::Debrief);
            game.phase_handler = PhaseHandler::Debrief {
                success: true,
                anim_elapsed_ms: 0,
            };
        }
        Event::KeyDown {
            keycode: Some(key @ (Keycode::W | Keycode::A | Keycode::S | Keycode::D)),
            ..
        } => {
            apply_camera_scroll(&mut game.camera, *key);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Debrief input
// ---------------------------------------------------------------------------

/// Handle input during the debrief phase.
/// Press Enter to return to the Office.
fn handle_debrief_input(game: &mut GameLoop, event: &Event) {
    if let Event::KeyDown {
        keycode: Some(Keycode::Return),
        ..
    } = event
    {
        info!("Debrief acknowledged -- returning to Office");
        // Clear mission state for next contract.
        game.game_state.current_mission = None;
        game.enemies.clear();
        game.combat_log.clear();
        // Reset merc AP for next mission.
        for merc in &mut game.game_state.team {
            merc.reset_ap();
            merc.position = None;
        }
        game.game_state
            .set_phase(GamePhase::Office(OfficePhase::Overview));
        game.phase_handler = PhaseHandler::Office {
            sub_phase: OfficePhase::Overview,
        };
    }
}

// ---------------------------------------------------------------------------
// Camera scroll helper
// ---------------------------------------------------------------------------

/// Apply a single discrete camera scroll step for a WASD key press.
fn apply_camera_scroll(camera: &mut Camera, key: Keycode) {
    let step = 32.0;
    match key {
        Keycode::W => camera.scroll(0.0, -step),
        Keycode::A => camera.scroll(-step, 0.0),
        Keycode::S => camera.scroll(0.0, step),
        Keycode::D => camera.scroll(step, 0.0),
        _ => {}
    }
}
