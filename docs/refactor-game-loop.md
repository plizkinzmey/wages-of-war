# Game Loop Atomic Refactoring Plan

Target: `crates/ow-app/src/game_loop/` (4 809 LOC across 5 files)

## Goal
Largest file 1747 → ~680 LOC. No function > 150 LOC. `run_game_loop_with_pump` becomes a readable outline.

## Phase A — Safe leaf extractions
Zero behavior change. Pure code motion + `use` adjustments.

1. `combat_log.rs` — `COMBAT_LOG_MAX`, `CombatLogEntry`, `CombatLogKind`, `color()`, `log_combat()` (~50 LOC)
2. `screenshot.rs` — `save_screenshot`, `save_screenshot_to_path` (~73 LOC)
3. `dev_hotkeys.rs` — F1-F5 + M keydown arms (~85 LOC)
4. `auto_screenshot.rs` — `auto_ss_*` state machine (~40 LOC)

`mod.rs`: 1551 → ~1100 LOC

## Phase B — Consolidate `run_game_loop` state
Introduce sibling structs to GameLoop, lift stack-local state into them.

5. `MissionAssets` struct (tile_renderer, obj_renderer, loaded_map, mission_iso, soldier_textures, soldier_anims, soldier_anim_set, prev_merc_states, walk_grace_remaining)
6. `AudioHandles` struct (music_track, music_handle, sfx_manager, voice_player, music_broken_latch)
7. `audio_init.rs::init_audio(data_dir) -> AudioHandles`
8. `asset_loader.rs::{load_office_texture, load_debrief_sprites, load_mission_map}`
9. `animation_watcher.rs::update_animations(anim_state, game, delta_ms)`

`run_game_loop_with_pump` body: ~1000 → ~150 LOC.

## Phase C — Split god-functions

10. `input/office.rs` — split `handle_office_input` (339 LOC) into mouse + hotspot + per-sub-view click + keyboard. Move `OFFICE_HOTSPOTS` const to shared module (eliminates duplicate hotspot table in `render.rs:171-207`).
11. `input/combat.rs` — split `handle_combat_input` (291 LOC). Extract `resolve_shoot(attacker, target_tile, weapon, ruleset, rng) -> ShotResult` so input parsing stays separate from combat resolution.
12. `update.rs` — split `update_combat` (193 LOC) into `process_ai_turn` + `check_victory_defeat` + `apply_mission_payout`.
13. `render/office.rs` — split `render_office` (482 LOC) into one fn per sub-phase + shared `render_office_chrome`.
14. `render/mission.rs` — split `render_mission_map` (657 LOC) into 9 passes: terrain, OBJ, overlays, walls, units, enemies, combat HUD, deployment HUD, minimap.
15. `render/debrief.rs` — extract `compute_financial_report` (lines 1484-1562) into `ow-core` as `compute_mission_payout(state, mission) -> FinancialReport`.

`render.rs`: 1747 → ~250 LOC. `input.rs`: 1131 → ~350 LOC.

## Phase D — Reorganize (after C settles)

16. `combat_state.rs` — `CombatHandler` + `advance_initiative`.
17. `phase.rs` — `PhaseHandler` + `phase_handler_for` + `phase_label`.
18. `ow-core` — `compute_mission_payout` (already moved in C15, just re-home here).
19. Delete `phase_background_color` (only used in tests now).

## Refrain from
- Splitting `update.rs` further (267 LOC, cohesive)
- Splitting `music.rs` (113 LOC, cohesive)
- Premature trait abstractions — split by phase, not by abstraction

## File Size After All Phases
| File | Before | After |
|---|---|---|
| mod.rs | 1551 | ~250 |
| input.rs | 1131 | ~350 |
| render.rs | 1747 | ~250 |
| update.rs | 267 | ~270 |
| music.rs | 113 | 113 |
| (new modules) | — | ~3 600 |
| **Total** | **4 809** | **~4 830** |

Same total LOC, but cohesive boundaries and small functions.
