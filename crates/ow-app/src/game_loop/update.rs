//! Phase update logic — per-frame state ticking for each game phase.

use tracing::{info, warn};

use ow_audio::sfx::{CombatSound, SfxManager};
use ow_core::game_state::{GamePhase, MissionPhase};

use super::input::advance_initiative;
use super::{log_combat, CombatLogKind, GameLoop, PhaseHandler};

/// Tick the current phase's update logic.
pub(crate) fn update_phase(game: &mut GameLoop, delta_ms: u32, sfx: &mut SfxManager) {
    // Snapshot the phase discriminant to avoid borrowing game.phase_handler
    // across the update call.
    enum UpdateRoute {
        Travel,
        Combat,
        Debrief,
        Other,
    }

    let route = match &game.phase_handler {
        PhaseHandler::Travel { .. } => UpdateRoute::Travel,
        PhaseHandler::Combat(_) => UpdateRoute::Combat,
        PhaseHandler::Debrief { .. } => UpdateRoute::Debrief,
        _ => UpdateRoute::Other,
    };

    match route {
        UpdateRoute::Travel => update_travel(game, delta_ms),
        UpdateRoute::Combat => update_combat(game, delta_ms, sfx),
        UpdateRoute::Debrief => {
            // Tick the accountant animation timer so sprite frames cycle
            // on the video phone during the debrief screen.
            if let PhaseHandler::Debrief {
                anim_elapsed_ms, ..
            } = &mut game.phase_handler
            {
                *anim_elapsed_ms = anim_elapsed_ms.saturating_add(delta_ms);
            }
        }
        UpdateRoute::Other => {
            // Office, Deployment, Extraction, Paused:
            // No per-frame update logic (purely input-driven).
        }
    }
}

/// Travel phase update: auto-advance to Mission(Deployment) after a brief delay.
fn update_travel(game: &mut GameLoop, delta_ms: u32) {
    const TRAVEL_DURATION_MS: u32 = 2000;

    let should_transition = match &mut game.phase_handler {
        PhaseHandler::Travel { elapsed_ms } => {
            *elapsed_ms += delta_ms;
            *elapsed_ms >= TRAVEL_DURATION_MS
        }
        _ => false,
    };

    if should_transition {
        info!("Travel complete -- transitioning to Mission Deployment");
        game.game_state
            .set_phase(GamePhase::Mission(MissionPhase::Deployment));
        game.phase_handler = PhaseHandler::Deployment { selected_unit: 0 };
    }
}

/// Combat phase update: process AI turns and check victory/defeat conditions.
///
/// When it's an enemy's turn, the AI picks and executes one action per frame.
/// This gives a visible cadence to enemy actions and keeps the frame rate smooth.
fn update_combat(game: &mut GameLoop, _delta_ms: u32, sfx: &mut SfxManager) {
    // -- AI turn processing --
    let ai_acting = match &game.phase_handler {
        PhaseHandler::Combat(c) => c.ai_acting,
        _ => return,
    };

    if ai_acting {
        let current_id = match &game.phase_handler {
            PhaseHandler::Combat(c) => c.initiative_order.get(c.current_initiative_idx).copied(),
            _ => None,
        };

        if let Some(id) = current_id {
            let is_player = game.game_state.team.iter().any(|m| m.id == id);
            if !is_player {
                // AI decision: find the nearest player merc and shoot them.
                // If no merc in range, move toward the nearest one.
                //
                // We collect snapshot data (name, position, wsk) to avoid
                // holding borrows across log_combat / advance_initiative calls.
                let enemy_snapshot = game
                    .enemies
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| (e.name.clone(), e.current_hp, e.position, e.wsk));

                if let Some((enemy_name, enemy_hp, enemy_pos_opt, enemy_wsk)) = enemy_snapshot {
                    if enemy_hp == 0 {
                        // Dead enemy, skip turn
                        advance_initiative(game);
                    } else if let Some(enemy_pos) = enemy_pos_opt {
                        // Find nearest living player merc
                        let nearest_merc = game
                            .game_state
                            .team
                            .iter()
                            .filter(|m| m.is_alive() && m.position.is_some())
                            .min_by_key(|m| {
                                let mp = m.position.unwrap();
                                (mp.x - enemy_pos.x).abs() + (mp.y - enemy_pos.y).abs()
                            })
                            .map(|m| (m.id, m.name.clone(), m.position.unwrap()));

                        if let Some((target_id, target_name, tp)) = nearest_merc {
                            let dist = (tp.x - enemy_pos.x).abs() + (tp.y - enemy_pos.y).abs();

                            if dist <= 15 {
                                // In range — SHOOT!
                                use rand::Rng;
                                let mut rng = rand::thread_rng();
                                let hit_chance = (enemy_wsk as u32).min(80);
                                let roll: u32 = rng.gen_range(0..100);

                                // Enemy fires — play gunshot SFX regardless of hit/miss.
                                sfx.play(CombatSound::Rifle);

                                if roll < hit_chance {
                                    // AI damage stays uniform 3-15 for now —
                                    // enemy weapons in mission_setup are
                                    // stored as `Weapon_{idx}` placeholders
                                    // that don't resolve against ruleset.
                                    // Wiring `enemy_weapons[i] -> Weapon` is
                                    // a separate task (see HANDOFF).
                                    let damage = rng.gen_range(3..15);
                                    if let Some(merc) =
                                        game.game_state.team.iter_mut().find(|m| m.id == target_id)
                                    {
                                        merc.current_hp = merc.current_hp.saturating_sub(damage);
                                        info!(
                                            enemy = %enemy_name,
                                            target = %target_name,
                                            damage,
                                            remaining_hp = merc.current_hp,
                                            "Enemy HIT player merc!"
                                        );
                                        if merc.current_hp == 0 {
                                            sfx.play(CombatSound::Kill);
                                            log_combat(game,
                                                format!("{enemy_name} hits {target_name} for {damage} damage! {target_name} KILLED!"),
                                                CombatLogKind::Kill);
                                        } else {
                                            sfx.play(CombatSound::Hit);
                                            log_combat(game,
                                                format!("{enemy_name} hits {target_name} for {damage} damage!"),
                                                CombatLogKind::EnemyHit);
                                        }
                                    }
                                } else {
                                    info!(enemy = %enemy_name, "Enemy MISSED!");
                                    sfx.play(CombatSound::Miss);
                                    log_combat(
                                        game,
                                        format!("{enemy_name} misses {target_name}!"),
                                        CombatLogKind::Miss,
                                    );
                                }
                            } else {
                                // Too far — move toward the target
                                let dx = (tp.x - enemy_pos.x).signum() * 3;
                                let dy = (tp.y - enemy_pos.y).signum() * 3;
                                let new_pos = ow_core::merc::TilePos {
                                    x: enemy_pos.x + dx,
                                    y: enemy_pos.y + dy,
                                };
                                if let Some(e) = game.enemies.iter_mut().find(|e| e.id == id) {
                                    e.position = Some(new_pos);
                                }
                                log_combat(
                                    game,
                                    format!("{enemy_name} moves toward your team"),
                                    CombatLogKind::Info,
                                );
                            }
                        }
                        advance_initiative(game);
                    } else {
                        advance_initiative(game);
                    }
                } else {
                    advance_initiative(game);
                }
            } else {
                // Somehow landed on a player unit while AI is acting — hand back
                if let PhaseHandler::Combat(c) = &mut game.phase_handler {
                    c.ai_acting = false;
                    c.selected_unit_id = Some(id);
                }
            }
        } else {
            // Past the end of initiative order — reset
            if let PhaseHandler::Combat(c) = &mut game.phase_handler {
                c.ai_acting = false;
                c.current_initiative_idx = 0;
            }
        }
    }

    // -- Victory/defeat condition checks --

    // Defeat: all player mercs dead
    let all_dead =
        !game.game_state.team.is_empty() && game.game_state.team.iter().all(|m| !m.is_alive());

    if all_dead {
        warn!("All player mercs killed -- mission failed");
        log_combat(
            game,
            "ALL MERCS DOWN -- MISSION FAILED!".to_string(),
            CombatLogKind::Kill,
        );
        game.game_state.set_phase(GamePhase::Debrief);
        game.phase_handler = PhaseHandler::Debrief {
            success: false,
            anim_elapsed_ms: 0,
        };
        return;
    }

    // Victory: all enemies eliminated — transition to extraction then debrief.
    let all_enemies_dead =
        !game.enemies.is_empty() && game.enemies.iter().all(|e| e.current_hp == 0);

    if all_enemies_dead {
        info!("All enemies eliminated — MISSION COMPLETE!");
        log_combat(
            game,
            "All enemies eliminated -- MISSION COMPLETE!".to_string(),
            CombatLogKind::Kill,
        );
        // Credit the mission bonus to funds.
        if let Some(ref mission_ctx) = game.game_state.current_mission {
            info!(mission = %mission_ctx.name, "Mission successful, transitioning to debrief");
        }
        game.game_state.missions_completed += 1;

        // Credit bonus from the contract.
        // The advance was already credited when the contract was accepted.
        // Now add the completion bonus.
        // TODO: Look up actual bonus from ruleset (currently a flat value).
        let bonus = 200_000i64;
        game.game_state.funds += bonus;
        info!(
            bonus,
            total_funds = game.game_state.funds,
            "Mission bonus credited"
        );

        game.game_state.set_phase(GamePhase::Debrief);
        game.phase_handler = PhaseHandler::Debrief {
            success: true,
            anim_elapsed_ms: 0,
        };
    }
}
