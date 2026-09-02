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

use super::mission::MissionData;
use super::office_layout::{
    click_to_office, contracts_row, equipment_row, hire_mercs_row, hotspot_at,
};
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

/// Route input events to the active phase handler. Phases that need
/// mission data (Deployment, Combat, Debrief) get the `&mut MissionData`;
/// phases that don't (Office, Paused, Travel, Extraction) ignore it.
pub(crate) fn handle_phase_input(
    game: &mut GameLoop,
    mission: Option<&mut MissionData>,
    event: &Event,
    ruleset: &Ruleset,
    sfx: &mut SfxManager,
    voice: &mut Option<VoicePlayer>,
) {
    match &game.phase_handler {
        PhaseHandler::Paused { .. } => handle_pause_input(game, event),
        PhaseHandler::Office { .. } => handle_office_input(game, event, ruleset, voice),
        PhaseHandler::Travel { .. } => {}
        PhaseHandler::Extraction => handle_extraction_input(game, event),
        PhaseHandler::Deployment { .. } => {
            if let Some(m) = mission {
                handle_deployment_input(game, m, event);
            }
        }
        PhaseHandler::Combat(_) => {
            if let Some(m) = mission {
                handle_combat_input(game, m, event, ruleset, sfx, voice);
            }
        }
        PhaseHandler::Debrief { .. } => {
            if let Some(m) = mission {
                handle_debrief_input(game, m, event);
            }
        }
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
/// The function is a thin dispatcher: it pulls the current sub-phase
/// out of the phase handler and forwards the event to a per-sub-phase
/// input helper. The hotspot → sub-phase mapping (Overview click on
/// phone → HireMercs, etc.) is in `handle_office_overview_click`.
fn handle_office_input(
    game: &mut GameLoop,
    event: &Event,
    ruleset: &Ruleset,
    voice: &mut Option<VoicePlayer>,
) {
    let current_sub = match &game.phase_handler {
        PhaseHandler::Office { sub_phase } => *sub_phase,
        _ => return,
    };

    match event {
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            let (ww, wh) = (game.window_width, game.window_height);
            let click = ScreenPos {
                x: *x as f32,
                y: *y as f32,
            };
            handle_office_subphase_click(game, ruleset, voice, current_sub, click, ww, wh);
        }
        Event::KeyDown {
            keycode: Some(key), ..
        } => {
            handle_office_keyboard(game, ruleset, *key, current_sub);
        }
        _ => {}
    }
}

/// Map a click in the office to a per-sub-phase action. Sub-phases
/// with clickable lists (HireMercs, Equipment, Contracts) get their
/// own row-tap handler. The Overview sub-phase gets the icon-hotspot
/// dispatcher. Intel and Training have no clickable content.
fn handle_office_subphase_click(
    game: &mut GameLoop,
    ruleset: &Ruleset,
    voice: &mut Option<VoicePlayer>,
    current_sub: OfficePhase,
    click: ScreenPos,
    ww: u32,
    wh: u32,
) {
    match current_sub {
        OfficePhase::Overview => handle_office_overview_click(game, click, ww, wh),
        OfficePhase::HireMercs => {
            handle_office_hire_mercs_click(game, ruleset, voice, click, ww, wh)
        }
        OfficePhase::Equipment => handle_office_equipment_click(game, ruleset, click, ww, wh),
        OfficePhase::Contracts => handle_office_contracts_click(game, ruleset, click, ww, wh),
        OfficePhase::Intel | OfficePhase::Training => {}
    }
}

/// Overview: project the click to 640x480 office space, log it, and
/// dispatch to the hotspot's target sub-phase.
fn handle_office_overview_click(game: &mut GameLoop, click: ScreenPos, ww: u32, wh: u32) {
    let (sx, sy) = click_to_office(click, ww, wh);
    info!(
        window_x = click.x as i32,
        window_y = click.y as i32,
        game_x = sx,
        game_y = sy,
        "Office click"
    );

    if let Some(h) = hotspot_at(click, ww, wh) {
        info!(action = h.label, "Office click");
        game.game_state.set_phase(GamePhase::Office(h.target));
        game.phase_handler = PhaseHandler::Office {
            sub_phase: h.target,
        };
    }
}

/// HireMercs: a click in the list area toggles hire/fire for the
/// merc at that row. The list is sorted by rating (descending) to
/// match the render order.
fn handle_office_hire_mercs_click(
    game: &mut GameLoop,
    ruleset: &Ruleset,
    voice: &mut Option<VoicePlayer>,
    click: ScreenPos,
    ww: u32,
    wh: u32,
) {
    let (_sx, sy) = click_to_office(click, ww, wh);
    let Some(row) = hire_mercs_row(sy) else {
        return;
    };

    let mut sorted_mercs: Vec<_> = ruleset.mercs.values().collect();
    sorted_mercs.sort_by(|a, b| b.rating.cmp(&a.rating));

    let Some(merc) = sorted_mercs.get(row) else {
        return;
    };
    let already_hired = game.game_state.team.iter().any(|m| m.name == merc.name);

    if already_hired {
        // Fire the merc — no refund, like the original.
        game.game_state.team.retain(|m| m.name != merc.name);
        info!(name = %merc.name, "Fired mercenary");
    } else if merc.avail == 1 {
        // Hire — check team size and funds.
        if game.game_state.team.len() >= 8 {
            warn!("Team full (max 8 mercs)");
        } else if game.game_state.funds < merc.fee_hire as i64 {
            warn!(
                name = %merc.name, cost = merc.fee_hire, funds = game.game_state.funds,
                "Cannot afford to hire"
            );
        } else {
            game.game_state.funds -= merc.fee_hire as i64;
            let id = game.game_state.team.len() as u32 + 1;
            let active = ow_core::merc::ActiveMerc::from_data(id, merc);
            info!(
                name = %merc.name, cost = merc.fee_hire,
                remaining_funds = game.game_state.funds, "Hired mercenary"
            );
            game.game_state.team.push(active);
            // Play the merc's voice line on hire (greeting/intro clip).
            if let Some(vp) = voice.as_mut() {
                vp.play(&merc.name);
            }
        }
    } else {
        info!(name = %merc.name, "Merc unavailable for hire");
    }
}

/// Equipment: a click in the weapon list leases the weapon at that row
/// to the first unarmed merc.
fn handle_office_equipment_click(
    game: &mut GameLoop,
    ruleset: &Ruleset,
    click: ScreenPos,
    ww: u32,
    wh: u32,
) {
    let (_sx, sy) = click_to_office(click, ww, wh);
    let Some(row) = equipment_row(sy) else { return };

    let mut sorted_weapons: Vec<_> = ruleset.weapons.values().collect();
    sorted_weapons.sort_by_key(|w| w.weapon_type);

    let Some(weapon) = sorted_weapons.get(row) else {
        return;
    };
    let unarmed_idx = game
        .game_state
        .team
        .iter()
        .position(|m| m.inventory.is_empty());

    let Some(idx) = unarmed_idx else {
        warn!(weapon = %weapon.name, "No unarmed mercs to assign weapon to");
        return;
    };

    if game.game_state.funds < weapon.cost as i64 {
        warn!(
            weapon = %weapon.name, cost = weapon.cost,
            funds = game.game_state.funds, "Cannot afford weapon lease"
        );
        return;
    }

    game.game_state.funds -= weapon.cost as i64;
    let merc_name = game.game_state.team[idx].name.clone();
    game.game_state.team[idx]
        .inventory
        .push(ow_core::merc::InventoryItem {
            name: weapon.name.clone(),
            encumbrance: weapon.encumbrance,
        });
    info!(
        weapon = %weapon.name, cost = weapon.cost,
        merc = %merc_name, remaining_funds = game.game_state.funds,
        "Leased weapon to merc"
    );
}

/// Contracts: a click in the contract list accepts the mission at that
/// row. The accepted mission gets the advance credited immediately.
fn handle_office_contracts_click(
    game: &mut GameLoop,
    ruleset: &Ruleset,
    click: ScreenPos,
    ww: u32,
    wh: u32,
) {
    let (_sx, sy) = click_to_office(click, ww, wh);
    let has_accepted = game.game_state.current_mission.is_some();
    let Some(row) = contracts_row(sy, has_accepted) else {
        return;
    };

    let mut mission_ids: Vec<_> = ruleset.missions.keys().collect();
    mission_ids.sort();
    let Some(mid) = mission_ids.get(row) else {
        return;
    };
    let Some(mission) = ruleset.missions.get(*mid) else {
        return;
    };

    let already_accepted = game
        .game_state
        .current_mission
        .as_ref()
        .map(|m| m.name == **mid)
        .unwrap_or(false);

    if already_accepted {
        info!(mission = %mid, "Contract already accepted");
    } else {
        // Switching contracts — no refund on old advance.
        let advance = mission.contract.advance;
        game.game_state.funds += advance as i64;
        game.game_state.current_mission = Some(ow_core::game_state::MissionContext {
            name: mid.to_string(),
            weather: ow_core::weather::Weather::Clear,
            combat: None,
            turn_number: 0,
        });
        info!(
            mission = %mid, advance = advance, funds = game.game_state.funds,
            "Contract accepted!"
        );
    }
}

/// Keyboard handler for the office phase. ESC → Overview, Num1-5 →
/// sub-phases, U → unequip-all, B → begin mission.
fn handle_office_keyboard(
    game: &mut GameLoop,
    ruleset: &Ruleset,
    key: Keycode,
    current_sub: OfficePhase,
) {
    let new_sub = match key {
        // ESC returns to the overview (office desk scene). Don't go to
        // overview if we're already there (that would trigger pause).
        Keycode::Escape => {
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
            return_unequip_all(game, ruleset);
            None
        }
        Keycode::B => return begin_mission(game),
        _ => None,
    };

    if let Some(sub) = new_sub {
        debug!(sub_phase = ?sub, "Office sub-phase switch");
        game.game_state.set_phase(GamePhase::Office(sub));
        game.phase_handler = PhaseHandler::Office { sub_phase: sub };
    }
}

/// Drain every merc's inventory, refunding weapons that have a
/// known cost in the ruleset. Items without a known cost are
/// returned but the refund is logged as zero.
fn return_unequip_all(game: &mut GameLoop, ruleset: &Ruleset) {
    let mut total_refund: i64 = 0;
    for merc in &mut game.game_state.team {
        for item in merc.inventory.drain(..) {
            if let Some(weapon) = ruleset.weapons.values().find(|w| w.name == item.name) {
                total_refund += weapon.cost as i64;
                info!(
                    weapon = %item.name, refund = weapon.cost,
                    merc = %merc.name, "Returned leased weapon"
                );
            } else {
                info!(
                    item = %item.name, merc = %merc.name,
                    "Returned item (no cost lookup)"
                );
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
}

/// Validate the preconditions for starting a mission and, if they
/// hold, transition the phase handler to Travel. Returns the team size
/// for logging on success.
fn begin_mission(game: &mut GameLoop) {
    if game.game_state.team.is_empty() {
        warn!("Cannot begin mission: no mercs hired");
        return;
    }
    if game.game_state.current_mission.is_none() {
        warn!("Cannot begin mission: no contract accepted");
        return;
    }
    info!(
        team_size = game.game_state.team.len(),
        mission = %game.game_state.current_mission.as_ref().unwrap().name,
        "Beginning mission"
    );
    game.game_state.set_phase(GamePhase::Travel);
    game.phase_handler = PhaseHandler::Travel { elapsed_ms: 0 };
}

// ---------------------------------------------------------------------------
// Deployment input
// ---------------------------------------------------------------------------

/// Handle input during the deployment phase.
///
/// - WASD / Arrows: scroll camera
/// - +/- / Mouse wheel: zoom
/// - Tab: cycle through mercs to place
/// - Left click: place selected merc on the clicked tile
/// - Enter: confirm deployment, transition to combat
fn handle_deployment_input(game: &mut GameLoop, mission: &mut MissionData, event: &Event) {
    match event {
        Event::KeyDown {
            keycode: Some(key), ..
        } if game.camera.scroll_for_key(*key, 32.0) => {}

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

        Event::MouseWheel { y, .. } => {
            if *y > 0 {
                game.camera.zoom_in();
            } else if *y < 0 {
                game.camera.zoom_out();
            }
        }

        Event::KeyDown {
            keycode: Some(Keycode::Tab),
            ..
        } => {
            handle_deployment_tab(game);
        }

        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            handle_deployment_click(game, mission, *x, *y);
        }

        Event::KeyDown {
            keycode: Some(Keycode::Return),
            ..
        } => {
            handle_deployment_confirm(game, mission);
        }

        _ => {}
    }
}

/// Tab: advance the deployment cursor to the next merc on the team.
fn handle_deployment_tab(game: &mut GameLoop) {
    let team_len = game.game_state.team.len();
    if team_len == 0 {
        return;
    }
    if let PhaseHandler::Deployment { selected_unit } = &mut game.phase_handler {
        *selected_unit = (*selected_unit + 1) % team_len;
        debug!(
            selected = *selected_unit,
            name = %game.game_state.team[*selected_unit].name,
            "Deployment: selected next merc"
        );
    }
}

/// Left click: project the click to a tile, place the currently-
/// selected merc on that tile, and auto-advance to the next unplaced
/// merc.
fn handle_deployment_click(game: &mut GameLoop, mission: &MissionData, x: i32, y: i32) {
    let screen = ScreenPos {
        x: x as f32,
        y: y as f32,
    };
    let world = game.camera.screen_to_world(screen);
    // Mission iso is the active config during deployment — the
    // input handler is only called when a mission is loaded, so
    // there's no fallback to compute.
    let tile = mission.iso.screen_to_tile(world);
    let core_tile = ow_core::merc::TilePos {
        x: tile.x,
        y: tile.y,
    };

    let selected = match &game.phase_handler {
        PhaseHandler::Deployment { selected_unit } => *selected_unit,
        _ => return,
    };
    let team_len = game.game_state.team.len();
    if selected >= team_len {
        return;
    }
    info!(
        name = %game.game_state.team[selected].name,
        tile_x = tile.x,
        tile_y = tile.y,
        "Deployment: placed merc"
    );
    game.game_state.team[selected].position = Some(core_tile);

    if let PhaseHandler::Deployment { selected_unit } = &mut game.phase_handler {
        *selected_unit = (*selected_unit + 1) % team_len;
    }
}

/// Enter: confirm deployment and transition to combat. Builds the
/// initiative order from the placed, living player units interleaved
/// with the enemies (core WoW mechanic: not I-go-you-go, all units
/// mixed by initiative).
fn handle_deployment_confirm(game: &mut GameLoop, mission: &MissionData) {
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

    let init_order = build_initiative_order(&game.game_state.team, &mission.enemies);
    let first_id = init_order.first().copied();

    game.phase_handler = PhaseHandler::Combat(CombatHandler {
        initiative_order: init_order,
        current_initiative_idx: 0,
        selected_unit_id: first_id,
        ai_acting: false,
        tab_cycle_index: 0,
    });
}

/// Build the initiative order: all placed, living player mercs,
/// followed by all living enemies with positions. Order within each
/// faction is team/enemy iteration order — the real initiative
/// sort by EXP+WIL is a future round.
fn build_initiative_order(
    team: &[ow_core::merc::ActiveMerc],
    enemies: &[ow_core::mission_setup::EnemyUnit],
) -> Vec<MercId> {
    let mut order: Vec<MercId> = Vec::new();
    for merc in team {
        if merc.position.is_some() && merc.is_alive() {
            order.push(merc.id);
        }
    }
    for enemy in enemies {
        if enemy.current_hp > 0 && enemy.position.is_some() {
            order.push(enemy.id);
        }
    }
    order
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
    mission: &mut MissionData,
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

        // Tab: cycle through living player units and play their
        // voice line as audio feedback.
        Event::KeyDown {
            keycode: Some(Keycode::Tab),
            ..
        } => {
            handle_combat_tab(game, voice);
        }

        // E: end current unit's turn
        Event::KeyDown {
            keycode: Some(Keycode::E),
            ..
        } => {
            handle_combat_end_turn(game);
        }

        // Mouse click: move or shoot depending on what occupies the target tile
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            x,
            y,
            ..
        } => {
            handle_combat_mouse_click(game, mission, ruleset, sfx, *x, *y);
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

/// Tab: cycle to the next living merc with a position. Each Tab plays
/// the selected merc's voice line so the player gets audio feedback
/// on who they just tabbed to.
fn handle_combat_tab(game: &mut GameLoop, voice: &mut Option<VoicePlayer>) {
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
        let sel_id = living[c.tab_cycle_index];
        if let Some(vp) = voice.as_mut() {
            if let Some(merc) = game.game_state.team.iter().find(|m| m.id == sel_id) {
                vp.play(&merc.name);
            }
        }
    }
}

/// E: end the currently-selected unit's turn.
fn handle_combat_end_turn(game: &mut GameLoop) {
    let selected = match &game.phase_handler {
        PhaseHandler::Combat(c) => c.selected_unit_id,
        _ => None,
    };
    if let Some(unit_id) = selected {
        info!(unit_id, "Player ended unit's turn");
        advance_initiative(game);
    }
}

/// Left-click in the combat phase: project the click to a tile and
/// resolve it as a shot (if an enemy is in the 2-tile radius) or a
/// move (otherwise).
fn handle_combat_mouse_click(
    game: &mut GameLoop,
    mission: &mut MissionData,
    ruleset: &Ruleset,
    sfx: &mut SfxManager,
    x: i32,
    y: i32,
) {
    let selected = match &game.phase_handler {
        PhaseHandler::Combat(c) => c.selected_unit_id,
        _ => None,
    };
    let Some(unit_id) = selected else { return };
    let screen = ScreenPos {
        x: x as f32,
        y: y as f32,
    };
    let world = game.camera.screen_to_world(screen);
    let iso = &mission.iso;
    let tile = iso.screen_to_tile(world);
    let target_tile = ow_core::merc::TilePos {
        x: tile.x,
        y: tile.y,
    };
    resolve_combat_click(game, mission, unit_id, target_tile, ruleset, sfx);
}

/// Resolve a combat click: either shoot an enemy in the 2-tile radius
/// around the click, or move the selected unit to the click. Owns all
/// the per-attack logic (weapon lookup, hit/miss roll, damage jitter,
/// AP deduction, SFX, combat log) so `handle_combat_input` stays a
/// pure event dispatcher.
fn resolve_combat_click(
    game: &mut GameLoop,
    mission: &mut MissionData,
    unit_id: MercId,
    target_tile: ow_core::merc::TilePos,
    ruleset: &Ruleset,
    sfx: &mut SfxManager,
) {
    // Click within 2 tiles of an enemy = target them. Otherwise, the
    // click is a move order.
    let enemy_idx = mission.enemies.iter().position(|e| {
        e.current_hp > 0
            && e.position
                .map(|p| (p.x - target_tile.x).abs() <= 2 && (p.y - target_tile.y).abs() <= 2)
                .unwrap_or(false)
    });

    if let Some(eidx) = enemy_idx {
        resolve_shot(game, mission, unit_id, eidx, ruleset, sfx);
    } else {
        resolve_move(game, unit_id, target_tile);
    }
}

/// Resolve a player attack on the enemy at `enemy_idx`. Looks up the
/// attacker's weapon, rolls a hit/miss with the standard skill+range
/// formula, applies the damage, deducts AP, plays SFX, and logs the
/// result.
fn resolve_shot(
    game: &mut GameLoop,
    mission: &mut MissionData,
    unit_id: MercId,
    enemy_idx: usize,
    ruleset: &Ruleset,
    sfx: &mut SfxManager,
) {
    let attacker = game.game_state.team.iter().find(|m| m.id == unit_id);
    let attacker_name = attacker
        .map(|m| m.name.clone())
        .unwrap_or_else(|| format!("Unit_{unit_id}"));
    let wsk = attacker.map(|m| m.wsk).unwrap_or(50);
    let attacker_pos = attacker.and_then(|m| m.position);

    // Resolve weapon stats from inventory. The equipment screen pushes
    // weapons in order, so the first match is the primary. Unarmed
    // mercs get fist-equivalent stats so nothing breaks if equipment
    // hookup misfires.
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
        None => (8, 15, 8),
    };
    let weapon_name = weapon.map(|w| w.name.as_str()).unwrap_or("(fists)");

    // Range check: Manhattan distance > weapon range = miss.
    let target_pos = mission.enemies[enemy_idx].position;
    let range_tiles = match (attacker_pos, target_pos) {
        (Some(a), Some(t)) => ((a.x - t.x).abs() + (a.y - t.y).abs()) as u32,
        _ => 0,
    };
    let out_of_range = range_tiles > weapon_range;

    // Hit chance: capped at 95 (always a chance to miss), halved at
    // the edge of range, zero when out of range.
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let hit_roll: u32 = rng.gen_range(0..100);
    let mut hit_chance = (wsk as u32).min(95);
    if range_tiles > weapon_range / 2 {
        hit_chance /= 2;
    }
    if out_of_range {
        hit_chance = 0;
    }

    // Roll, apply, log. We collect the log message first so we can call
    // `log_combat` outside the mutable enemy borrow.
    let log_msg: (String, CombatLogKind);
    let enemy = &mut mission.enemies[enemy_idx];
    if hit_roll < hit_chance {
        // Damage from `damage_class` with ±25% jitter. ow-core's full
        // `resolve_attack` adds penetration vs armor — out of scope
        // here until hit_table + armor are plumbed through.
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
        // Deduct AP on hit.
        if let Some(merc) = game.game_state.team.iter_mut().find(|m| m.id == unit_id) {
            merc.current_ap = merc.current_ap.saturating_sub(weapon_ap);
        }
        log_msg = if enemy.current_hp == 0 {
            info!(target = %enemy.name, "Enemy KILLED!");
            (
                format!(
                    "{attacker_name} hits {ename} for {damage} damage! {ename} KILLED!",
                    ename = enemy.name
                ),
                CombatLogKind::Kill,
            )
        } else {
            (
                format!("{attacker_name} hits {} for {damage} damage!", enemy.name),
                CombatLogKind::PlayerHit,
            )
        };
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
        log_msg = if out_of_range {
            (
                format!(
                    "{attacker_name}: {} out of range ({range_tiles} > {weapon_range})",
                    mission.enemies[enemy_idx].name
                ),
                CombatLogKind::Miss,
            )
        } else {
            (
                format!("{attacker_name} misses {}!", enemy.name),
                CombatLogKind::Miss,
            )
        };
        // Misses burn AP only if the shot was in range; out-of-range
        // clicks are free.
        if !out_of_range {
            if let Some(merc) = game.game_state.team.iter_mut().find(|m| m.id == unit_id) {
                merc.current_ap = merc.current_ap.saturating_sub(weapon_ap);
            }
        }
    }

    // SFX: gunshot on every attempt, then layer kill or miss on top.
    sfx.play(CombatSound::Pistol);
    match log_msg.1 {
        CombatLogKind::Kill => sfx.play(CombatSound::Kill),
        CombatLogKind::Miss => sfx.play(CombatSound::Miss),
        _ => {} // Hit uses just the gunshot
    }
    log_combat(game, log_msg.0, log_msg.1);
}

/// Resolve a move order: teleport the unit to the target tile, deduct
/// AP (2 per Manhattan-distance tile, capped at current AP). Out-of-
/// range clicks aren't possible here because this only fires when no
/// enemy is in the 2-tile radius.
fn resolve_move(game: &mut GameLoop, unit_id: MercId, target_tile: ow_core::merc::TilePos) {
    if let Some(merc) = game.game_state.team.iter_mut().find(|m| m.id == unit_id) {
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
fn handle_debrief_input(game: &mut GameLoop, mission: &mut MissionData, event: &Event) {
    if let Event::KeyDown {
        keycode: Some(Keycode::Return),
        ..
    } = event
    {
        info!("Debrief acknowledged -- returning to Office");
        // Clear mission state for next contract.
        game.game_state.current_mission = None;
        mission.enemies.clear();
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
/// Delegates to `Camera::scroll_for_key`; the local helper exists so
/// call sites can be terse.
fn apply_camera_scroll(camera: &mut Camera, key: Keycode) {
    camera.scroll_for_key(key, 32.0);
}
