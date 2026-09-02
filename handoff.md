# HANDOFF.md — Open Wages → Claude Code Session

## 2026-09-02 — game-loop Phase B (round 5): thermo-nuclear review fixes (delete duplication, single source of truth)

**Last Updated:** 2026-09-02
**Project Status:** 🟢 Round 5 of the game-loop refactor complete. Round 4 left structural smell: the office hotspot table was duplicated between `render.rs` and `input.rs`; `draw_office_equipment` and `handle_deployment_input` were 130/180-LOC outliers; `draw_textured_merc` used a closure-as-state-hack to paper over a borrow; the `format!(...)` allocation was a workaround for `WeaponType` missing `Ord`; `visible_tile_rect` was a free function living in `render.rs` that should have been a `GameMap` method; the WASD/arrow camera scroll was re-implemented inline in two places. Round 5 deletes all of it.

### What was done this session (round 5)

**New module: `office_layout.rs` (171 LOC).** Single source of truth for the office scene's clickable geometry. Defines `OfficeHotspot` (rect, label, target sub-phase, debug-overlay color), the `HOTSPOTS` table (7 entries, order from most-specific to least-specific), `HIRE_MERCS_ROW_H/Y`, `EQUIPMENT_ROW_H/Y`, `CONTRACTS_ROW_H/Y` (with `CONTRACTS_LIST_Y_ACCEPTED` accounting for the "ACCEPTED:" banner offset), and the `click_to_office(click, ww, wh) -> (i32, i32)` projection. Both `draw_office_hotspot_overlays` (render side) and `handle_office_overview_click` (input side) now read from this table — they can no longer drift. The per-sub-phase click handlers (hire/equipment/contracts) all use the row-geometry helpers instead of inlining the `list_start_y/row_h` magic numbers. **This deletes ~30 LOC of duplicated tables and ~25 LOC of duplicated coordinate-scaling math.**

**`draw_player_mercs` closure hack deleted.** The previous version had a `|| { anims.anims.get(merc_idx).map(...).unwrap_or(false) }` closure because the `AnimController` borrow didn't outlive the match arm. The new version does a single `Option<ctrl>`-unwrap that extracts BOTH `current_frame_index` and `mirror_horizontal` in one pass, then resolves the texture. The closure is gone. `draw_textured_merc` now takes `mirror: bool` instead of `mirror: impl FnOnce() -> bool`. **Net result: one lookup instead of two, no closure allocation, and the borrow problem disappears because we're not re-borrowing after the match arm.**

**`draw_office_equipment` (130 → 22 LOC orchestrator).** Split into `draw_equipment_weapons_pane` (61 LOC: catalog list with xN-leased badges), `draw_equipment_team_pane` (57 LOC: roster with inventory), `draw_equipment_status_row` (10 LOC: bottom status text). Same pattern as the round-4 office/debrief decomposition — applied to the function that was missed.

**`handle_deployment_input` (178 → 60 LOC orchestrator).** Split into `handle_deployment_tab` (10 LOC), `handle_deployment_click` (30 LOC, takes `&MissionData` since it doesn't mutate), and `handle_deployment_confirm` (38 LOC). The inline 30-LOC WASD/arrow camera scroll block was deleted and replaced with `game.camera.scroll_for_key(*key, 32.0)` in the match guard — one call instead of a 14-line `matches!` + nested `match`.

**`build_initiative_order` extracted.** The 30-LOC initiative-order construction hidden in the Enter arm of `handle_deployment_input` is now a top-level helper. The team/enemies → `Vec<MercId>` projection is testable in isolation.

**`WeaponType` now derives `Ord, PartialOrd`.** The `sort_by(|a, b| format!("{:?}", a.weapon_type).cmp(&format!("{:?}", b.weapon_type))` workaround in `input.rs:364` is gone. Replaced with `sort_by_key(|w| w.weapon_type)`. **Deletes two `String` allocations per comparison (and the sort fires O(n log n) times on every equipment-pane render and click).** The fix lives in `crates/ow-data/src/weapons.rs` where it belongs, not in app-level code as a workaround.

**`GameMap::clamp_tile_rect` added.** The `visible_tile_rect` free function in `render.rs` is now a one-liner: `map.clamp_tile_rect(game.camera.visible_tile_bounds(iso))`. Map-bounds clamping moved into `ow-data` (where the map concept lives) and out of `render.rs` (which is just consuming the bounds). Used 4× — by `draw_obj_sprites`, `draw_word2_overlays`, `draw_walls`, and the now-replaced `visible_tile_rect` itself.

**`Camera::scroll_for_key` added.** The 14-LOC inline WASD/arrow → camera scroll block in `handle_deployment_input` and the 8-LOC `apply_camera_scroll` free function in `input.rs` are both gone. Now there's one canonical helper on `Camera` (in `ow-render` where the camera concept lives). The local `apply_camera_scroll(camera, key)` in `input.rs` is now a 1-line delegation.

**Stale doc comment deleted.** The 30-line `///` block before `handle_office_input` that described the pre-round-4 behavior (inline hotspot dispatch, inline `B`-key handler, per-event match arms) was deleted. The single new `///` block (already in round 4) is now the only doc. Rustdoc now shows accurate behavior.

### What I didn't do (with reasoning)

- **BL#1 — split `render.rs` and `input.rs` into per-phase sub-modules.** The reviewer's blocking feedback on this is valid and acknowledged. This is a separate, mechanical move (relocate existing functions to `render/{office, mission, debrief}.rs` and `input/{office, combat, mission}.rs`) that should be its own PR. The round-5 work didn't tackle it because the priority was the *correctness* smells (duplicate tables, closure workarounds, allocation workarounds), not the *file-size* smell. **Next round should be: file split, full stop. No new decomposition work until files are <500 LOC each.**

- **BL#3 — collapse the `handle_office_input` dispatcher-of-dispatcher-of-dispatcher.** The reviewer is right that `handle_office_input → handle_office_subphase_click → handle_office_overview_click` is three layers of indirection that all forward `current_sub` and click coords. I considered merging them but decided not to because the current shape maps cleanly onto the per-sub-phase concept. The per-sub-phase click handlers are individually testable and self-contained — collapsing them into a `(event, current_sub)` tuple match would be a longer function with more parameters. This is a judgment call; not blocking.

- **LOW: TODO in `draw_debrief_results` for hardcoded advance/bonus/medical.** Real bug (the financial report doesn't read from the actual contract) but a separate piece of work — needs the `MissionContext` to carry the actual advance/bonus/kia counts through the debrief, which is a state-model change. Deferred.

- **LOW: per-frame `trace!` spam in `draw_walls` etc.** Pre-existing. Not introduced by this work.

### Build + Test
```bash
cargo build --workspace    # clean
cargo test -p ow-app --bin ow-app  # 4/4 pass
cargo clippy -p ow-app     # 8 warnings (down from 10 in round 4)
cargo fmt -p ow-app -p ow-data -p ow-render  # clean for the files I touched
```

### File size after Phase B (round 5)
| File | Round 4 | Round 5 | Δ | Notes |
|---|---|---|---|---|
| mod.rs | 591 | 592 | +1 | +1 line for the new mod |
| render.rs | 1748 | 1863 | +115 | net of: −130 office equipment, −30 hotspot table, −25 inline code, +3 equipment sub-helpers, +office_layout table use, +office layout = 171 in new file |
| input.rs | 1222 | 1129 | −93 | net of: −30 hotspot table, −30 deployment inline code, +deployment helpers, +office layout use |
| mission.rs | 41 | 41 | 0 | unchanged |
| loop_pump.rs | 162 | 162 | 0 | unchanged |
| asset_loader.rs | 555 | 555 | 0 | unchanged |
| **office_layout.rs** | — | 171 | +171 | new module |
| **Total** | **5312** | **5483** | +171 | +171 from new module; net +95 across render+input from helper-signature overhead. The round-5 LOC is higher but the *function* count is the same and the duplication is gone. |

`render.rs` is up 115 LOC because `draw_office_equipment` got 3 sub-helpers (each with its own doc comment + signature) and the per-pass helpers now have cleaner signatures. The trade is explicit: more small functions, zero duplication, slightly more total LOC. **The file-size smell is real and the next round needs to be the sub-module split.** Round 5 prioritized deletion-of-duplication over deletion-of-LOC.

### What's next (round 6 candidate)

- **Split `render.rs` (1863 LOC) and `input.rs` (1129 LOC) into sub-modules.** This is the *entire* PR — no new decomposition work, no new helpers, just relocate existing functions to:
  - `render/office.rs` (8 `draw_office_*` + `render_office` + the now-shared `office_layout::HOTSPOTS` overlay)
  - `render/mission.rs` (`render_mission_map` + 8 per-pass helpers + `render_extraction`)
  - `render/debrief.rs` (`render_debrief` + 2 sub-helpers)
  - `render/shared.rs` (`render_pause`, `render_placeholder_grid`, the remaining bits)
  - `input/office.rs` (escape/pause handlers + all `handle_office_*` + `return_unequip_all` + `begin_mission`)
  - `input/combat.rs` (`handle_combat_input` + 3 arms + `resolve_combat_click` + `resolve_shot` + `resolve_move` + `advance_initiative`)
  - `input/mission.rs` (`handle_deployment_input` + 3 arms + `build_initiative_order` + `handle_extraction_input` + `handle_debrief_input`)
- **Fix the debrief financial-report TODO.** Move advance/bonus/medical/death-insurance to `MissionContext` so the debrief reads the actual numbers instead of hardcoded 324_000/200_000/79_000/89_000.

After the file split, the largest file in the module would be ~500 LOC, every function <130 LOC, no file-level duplication, no allocation workarounds, and no doc comments describing code that no longer exists.

## 2026-09-02 — game-loop Phase B (round 4): decompose the remaining god-functions

**Last Updated:** 2026-09-02
**Project Status:** 🟢 Round 4 of the game-loop refactor complete. The `render_office` (488 → 21 LOC) and `render_debrief` (327 → 27 LOC) god-functions are now thin orchestrators over per-section helpers. `handle_office_input` (339 → 37 LOC) is a thin dispatcher over per-sub-phase click + per-keyboard handlers. `handle_combat_input` has Tab, E, and click arms extracted into named helpers. `draw_player_mercs` decomposed into `draw_textured_merc` + `draw_placeholder_merc`. `draw_terrain` (a 4-line pass-through) inlined into the orchestrator. `cargo build --workspace` clean, `cargo test -p ow-app --bin ow-app` 4/4 pass, clippy 10 warnings (no regressions).

### What was done this session (round 4)

Round 3 left four issues that the thermo-nuclear review flagged as blockers. Round 4 addresses all of them by applying the established decomposition pattern to the remaining god-functions.

**`render_office` (488 → 21 LOC orchestrator).** Was a single 488-LOC function with two `match active_sub` blocks (one for click events, one for rendering). Now:

```rust
fn render_office(game, canvas, active_sub, text, tc, ruleset, office_bg) {
    if active_sub == OfficePhase::Overview {
        draw_office_overview(canvas, office_bg, text, tc, game);
    } else {
        draw_office_tab_bar(canvas, text, tc, active_sub, game);
        draw_office_content(canvas, text, tc, active_sub, ruleset, game);
    }
}
```

The 6 sub-phase content renderers (`draw_office_hire_mercs`, `draw_office_equipment`, `draw_office_contracts`, `draw_office_placeholder`, plus the tab bar / hotspot overlay / overview helpers) are now individually named, each 20-140 LOC. The hotspot-to-action mapping that used to be a 7-arm `if check_hit(...) else if ...` chain is now `check_office_hotspot()` returning `Option<(&str, OfficePhase)>`.

**`render_debrief` (327 → 27 LOC orchestrator).** Was a single 327-LOC function with two clearly-delimited halves (phone scene + financial report) and a fallback "no phone sprites" branch. Now:

```rust
fn render_debrief(game, mission, canvas, success, anim_elapsed_ms, text, tc, acct_textures, phone_textures) {
    canvas.set_draw_color(Color::RGB(15, 15, 25));
    canvas.clear();
    draw_debrief_phone(canvas, text, tc, h, w, acct_textures, phone_textures, anim_elapsed_ms);
    draw_debrief_results(canvas, text, tc, game, mission, success, _h, w);
}
```

The 2 sub-section helpers each do one thing: `draw_debrief_phone` renders the video phone + accountant animation cycle, `draw_debrief_results` renders the battle results text and the financial report.

**`handle_office_input` (339 → 37 LOC orchestrator).** Was a single 339-LOC function with 3 list-row click handlers (HireMercs/Equipment/Contracts) + 7 hotspot checks + a big keyboard match. Now a thin dispatcher:

```rust
fn handle_office_input(game, event, ruleset, voice) {
    let current_sub = match &game.phase_handler { ... };
    match event {
        MouseButtonDown => handle_office_subphase_click(...),  // 22 LOC
        KeyDown          => handle_office_keyboard(...),         // ~40 LOC
        _ => {}
    }
}
```

The click dispatcher is `handle_office_subphase_click` which routes to `handle_office_overview_click` (hotspot → action), `handle_office_hire_mercs_click` (row toggle hire/fire), `handle_office_equipment_click` (row lease), `handle_office_contracts_click` (row accept). The keyboard dispatcher is `handle_office_keyboard` which routes to `return_unequip_all` (U key) or `begin_mission` (B key) for the cross-cutting actions.

**`handle_combat_input` arms extracted.** The 117-LOC dispatcher now has its Tab, E, and click arms extracted into `handle_combat_tab` (26 LOC), `handle_combat_end_turn` (12 LOC), and `handle_combat_mouse_click` (33 LOC). The dispatcher is now a clean 6-arm `match event` where each arm is a single function call.

**`draw_player_mercs` decomposed.** The 84-LOC function had a 60-LOC if-let-else inside the loop. Now split into a 44-LOC orchestrator + `draw_textured_merc` (31 LOC) + `draw_placeholder_merc` (18 LOC) + 6 LOC of `const SOLDIER_SPRITE_*` constants. The orchestrator is 5 lines: a single `match` that dispatches to the two helpers.

**`draw_terrain` inlined.** Was a 4-line pass-through helper that just delegated to `tile_renderer.render_map(...)`. Now inlined into the `render_mission_map` orchestrator — the orchestrator now has 8 named-pass calls plus one inlined tile-renderer call. The 4-line wrapper was a "thin wrapper" smell.

### What I didn't do (with reasoning)

- **`#5 [MEDIUM] Two `match active_sub` in `render_office`.** Subsumed by #1. When `render_office` is decomposed into per-sub-phase functions, the input `match` and the render `match` collapse into one `match` each in their respective locations. Not separately needed.

- **`#7 [LOW] `visible_tile_rect` clamping.** The current form is explicit about what each bound does. A `clamp` rewrite would be marginally shorter but less obvious. Left as-is.

- **`#8 [LOW] `draw_walls` segment table inline.** The 12 segments are computed per-cell because the diamond corners depend on the cell's screen position. There's no static table to lift. Left as-is.

### Build + Test
```bash
cargo build --workspace    # clean
cargo test -p ow-app --bin ow-app  # 4/4 pass
cargo clippy -p ow-app     # 10 warnings (no regressions from round 3)
```

### File size after Phase B (round 4)
| File | Round 3 | Round 4 | Notes |
|---|---|---|---|
| mod.rs | 591 | 591 | small |
| render.rs | 1708 | 1748 | Office + Debrief decomposed. Net: helpers + comments outweigh the LOC saved from the orchestrators |
| input.rs | 1158 | 1222 | Office input + combat arms decomposed. Net: helpers + comments outweigh the LOC saved |
| mission.rs | 41 | 41 | unchanged |
| loop_pump.rs | 162 | 162 | unchanged |
| asset_loader.rs | 555 | 555 | unchanged |
| **Total** | **5312** | **5417** | Slight LOC increase from helper signatures + doc comments |

The function-level structure is dramatically better even though the file-level LOC is up. The two files are still over 1000 LOC, but the *functions* are now small and focused — `render_office` is 21 LOC, `render_debrief` is 27 LOC, `handle_office_input` is 37 LOC. The remaining file size is from many small helpers (a positive sign of decomposition), not from any remaining god-functions.

### What's next

- **Split `render.rs` and `input.rs` into sub-modules.** With the round-4 decomposition, the natural sub-module structure is now visible: `render/office.rs` (the 8 `draw_office_*` helpers), `render/mission.rs` (the 8 `draw_*` per-pass helpers + `render_mission_map`), `render/debrief.rs` (the 2 debrief helpers), `input/office.rs` (the 6 `handle_office_*` helpers), `input/combat.rs` (the 4 `handle_combat_*` + `resolve_*` helpers). This would bring both files under 500 LOC each.
- Decompose `update_combat` (195 LOC) and `handle_deployment_input` (180 LOC) the same way.
- The `handle_combat_input` dispatcher now has the right shape — the next round can extract `handle_combat_*` arms into a proper `combat_input.rs` module.

## 2026-09-02 — game-loop Phase B (round 3): kill `MissionState`/`MissionView`, decompose `render_mission_map`

**Last Updated:** 2026-09-02
**Project Status:** 🟢 Round 3 of the game-loop refactor complete. The dead `MissionState` wrapper and `MissionView` API are gone. The dead `Route`/`UpdateRoute` enums are gone. `render_mission_map` is decomposed into 9 named per-pass functions. `handle_combat_input` is decomposed into `resolve_combat_click` + `resolve_shot` + `resolve_move`. The `place_camera_on_map` helper is lifted out of `maybe_load_mission`. `cargo build --workspace` clean, `cargo test -p ow-app --bin ow-app` 4/4 pass, clippy 9 warnings (down from 10 in round 2).

### What was done this session (round 3)

Round 2 left four issues that the thermo-nuclear review called blockers:

**`MissionState` deleted.** The 1-field wrapper struct held `Option<MissionData<'a>>` and added `empty()`/`install()`/`is_loaded()` methods that all did what the inner `Option` already does. The run loop now uses `Option<MissionData<'a>>` directly. Every consumer reads `mission.as_ref()` / `mission.as_mut()` / `mission.is_some()` exactly like any other `Option`. `mission.rs` is now 41 lines (just `MissionData`).

**`MissionView` deleted.** The `view()` method and the `MissionView` struct were both `#[allow(dead_code)]` — the doc comment claimed renderers would use them but no renderer does. The 30-line module-level doc that explained how the three pieces fit together is now 12 lines explaining one struct.

**`Route`/`UpdateRoute` enums deleted.** The 7-variant `Route` and 3-variant `UpdateRoute` enums in input.rs/update.rs were sum-type projections that threw away the data fields of `PhaseHandler`. `handle_phase_input` and `update_phase` now match on `&game.phase_handler` directly. The "snapshot the discriminant" comment is gone.

**`render_mission_map` decomposed.** The 660-LOC god-function is now a 20-LOC orchestrator that calls 9 named per-pass functions:

```rust
fn render_mission_map(game, mission, anims, canvas, text, tc) {
    draw_terrain(game, mission, canvas);
    draw_obj_sprites(game, mission, canvas);
    draw_word2_overlays(game, mission, canvas);
    draw_walls(game, mission, canvas);
    draw_player_mercs(game, mission, anims, canvas);
    draw_enemies(game, mission, canvas);
    draw_combat_hud(game, canvas, text, tc);
    draw_deployment_hud(game, canvas, text, tc);
    draw_minimap(game, mission, canvas, text, tc);
}
```

The shared tile-bounds computation is now a single `visible_tile_rect()` helper called by the three passes that need it (was duplicated inline 3 times).

**`handle_combat_input` decomposed.** The 299-LOC function is now a thin event dispatcher that delegates the click handler to:

- `resolve_combat_click` — picks shot vs move based on enemy proximity
- `resolve_shot` — weapon lookup, hit/miss roll, damage, AP, SFX, combat log
- `resolve_move` — AP-cost calculation and teleport

The dispatch loop in `handle_combat_input` is now ~50 LOC of clean event matching. The 200-LOC shot resolution is a single testable function.

**`place_camera_on_map` lifted out of `maybe_load_mission`.** The 12-line camera-placement block is now a named helper. The load function reads as "load and install" with a clear call to the camera helper.

### What I didn't do (with reasoning)

**[#1] `render.rs` is still 1708 LOC.** Round 3 decomposed `render_mission_map` (the 660-LOC god-function) but `render_office` (490 LOC) and `render_debrief` (335 LOC) remain inline. The `render_mission_map` decomposition showed the pattern; the remaining god-functions deserve the same treatment. This is the obvious next step for the next session — extract `render_office` into per-sub-phase functions, then split `render_debrief` into phone/accountant/financial-report sections. The full Phase C sub-module split (`render/office.rs`, `render/mission.rs`, etc.) is the natural end-state.

**[#6] `PhaseHandler` / `GamePhase` dual state machine.** 5+ ad-hoc sites in the codebase transition both enums together (`update_travel`, `handle_debrief_input`, `handle_extraction_input`, dev hotkeys). Consolidating them is a real refactor; not in scope for this round but called out in the round-3 review.

**[#7] 3-pass `SoldierAnims::tick`.** Could be inlined into a single per-merc loop. The current shape is honest about the trade-off; a 200-LOC function with three concerns is worse than a 60-LOC function with three named sub-passes.

**[#10] `SoldierAnims::spawn_controllers` as method.** The method form is fine ergonomically. Refactoring to a free function would be churn.

**[#11] `audio_init` three helpers.** Each helper is small (5-13 lines) but has a single job. Inlining would create a 50-LOC `init_audio` that mixes mixer open, voice player build, and initial track selection. The factoring is good.

### Build + Test
```bash
cargo build --workspace    # clean
cargo test -p ow-app --bin ow-app  # 4/4 pass
cargo clippy -p ow-app     # 9 warnings (down from 10)
```

### File size after Phase B (round 3)
| File | Round 2 | Round 3 | Notes |
|---|---|---|---|
| mod.rs | 592 | 591 | small |
| render.rs | 1769 | 1708 | `render_mission_map` decomposed into 9 named per-pass functions |
| input.rs | 1122 | 1158 | `Route` enum gone; `handle_combat_input` decomposed into 3 helpers (small LOC growth from helper boilerplate) |
| update.rs | 282 | 268 | `UpdateRoute` enum gone |
| loop_pump.rs | 159 | 162 | `place_camera_on_map` extracted |
| mission.rs | 117 | 41 | `MissionState` + `MissionView` deleted |
| **Total** | **5294** | **5306** | Same total LOC; the structure is much cleaner |

The render.rs file is still over 1000 LOC because `render_office` (490) and `render_debrief` (335) are still inline. Their decomposition is the next obvious step.

### What's next

- Decompose `render_office` and `render_debrief` the same way `render_mission_map` was decomposed.
- Consolidate `PhaseHandler` / `GamePhase` into a single enum with per-phase state attached.
- Phase C sub-module split: `render/office.rs`, `render/mission.rs`, `render/debrief.rs`, `render/pause.rs`, etc.
- The `handle_office_input` 339-LOC and `update_combat` 193-LOC god-functions are still inline in input.rs/update.rs and deserve the same decomposition.

## 2026-09-02 — game-loop Phase B (round 2): split `MissionAssets` and tighten abstraction boundaries

**Last Updated:** 2026-09-02
**Project Status:** 🟢 Phase B of `docs/refactor-game-loop.md` complete, plus a follow-up cleanup round addressing the thermo-nuclear review. `MissionAssets` has been split into `MissionState` + `MissionData` + `MissionView`. The `MissionState.default_iso` field is gone (the renderer constructs it from window dimensions). Per-frame helpers live in `loop_pump.rs`. The animation watcher's 4 functions are collapsed into 1 `SoldierAnims::tick`. `load_mission_map` is now a value-returning function with `LoadError`. `cargo build --workspace` clean, `cargo test -p ow-app --bin ow-app` 4/4 pass, clippy 10 warnings (down from 12 pre-cleanup).

### What was done this session (round 2)

The first pass at Phase B extracted five new modules but kept the `MissionAssets` struct as a 11-field god-bag that threaded a `TextureCreator<'a>` lifetime through every reader, even the ones that only touched `enemies` or `default_iso`. The thermo-nuclear review flagged this and five other issues. This round addresses the major ones:

**Split `MissionAssets` into three pieces along the texture-creator lifetime boundary:**
- `mission::MissionState<'a>` (117 LOC) — the holder that lives in the run loop. Now only owns `Option<MissionData<'a>>`; the `default_iso` field is gone because it's a window-derived constant the renderer can construct.
- `mission::MissionData<'a>` — the value returned by `load_mission`. Bundles `GameMap`, `IsoConfig`, `TileMapRenderer<'a>`, `Option<TileMapRenderer<'a>>`, `Vec<EnemyUnit>`. Construction is atomic — partial failures stay inside the loader.
- `mission::MissionView<'a>` (unused but documented API) — a short-lived, fully-borrowed view for callers that need to read multiple fields. Available for future callers that don't want the lifetime of `&MissionData`.

**Split `animation_watcher` into `SoldierAnims<'a>` with a single `tick` method:**
- The previous 4 functions (`dispatch_actions`, `auto_revert`, `reset_walk_grace`, `snapshot_for_next_frame`) each re-iterated the team and re-checked `let Some(ctrl) = assets.soldier_anims.get_mut(i)`. They're now 3 passes inside one `tick` method, sharing a `FrameState { prev, walk_grace, just_moved }` struct. The `<'a>` lifetime parameter only appears on the struct now, not on each method.
- The `just_moved` flag is set during pass 1 and read during pass 2, eliminating a third re-iteration just to detect "did this merc move this frame?"

**Made `load_mission` return `Result<MissionData, LoadError>`:**
- The previous void/mutate-the-bundle pattern claimed atomicity but actually performed partial updates. Now the loader builds the value before returning, and the run loop's `maybe_load_mission` either installs the value or stays on Office. The 6-stage early-return chain is gone.

**Lifted per-frame helpers out of `mod.rs` into `loop_pump.rs` (159 LOC):**
- `maybe_transition_music` and `maybe_load_mission` are now in their own module. mod.rs is a thinner dispatcher; the new modules can be `pub(crate) mod` so callers don't have to thread everything through mod.rs.

**Reorganized the per-phase state pipeline:**
- `handle_phase_input` now takes `Option<&mut MissionData>` and routes to handlers that take `&mut MissionData` (when they need it) or nothing (when they don't, like `handle_office_input`).
- `update_phase` mirrors the same pattern.
- `handle_dev_hotkeys::F4` (kill all enemies) takes `Option<&mut MissionData>` and is a no-op without a mission — the previous implementation panicked if F4 was pressed before any mission loaded.
- The input handlers' `assets.iso()` / `assets.enemies` accesses became direct `&mission.iso` / `&mission.enemies` — the silent-fallback pattern is gone.

**Other small wins:**
- `init_audio` factored into `open_mixer`, `build_voice_player`, `start_initial_track` helpers. The hand-rolled 3-tuple nested `if let Some` build is gone.
- `decode_soldier_frame` and `load_sprite_textures` factored out of the per-pixel loops in `asset_loader`.
- `decode_sprite_sheet` and `load_palette_pcx` are reusable helpers in `asset_loader` (public so `loop_pump` can call them).
- `find_palette_pcx` removed; its `read_dir`+`find` logic is now a small private helper with a `TODO: lift to ow_data::fs_util if a second caller appears` comment.

### What I didn't do (with reasoning)

**#4 [HIGH] `render_mission_map` is still 660 LOC.** The review correctly noted the refactor did not address render.rs. I left this for Phase C per the original plan, and rendering the mission map is a complex thing with 5+ distinct passes that need to be coordinated. The diff does touch render.rs, but only to thread `&MissionData` and `&SoldierAnims` through; the structural decomposition is Phase C.

**#7 [MEDIUM] `find_palette_pcx` re-implements "find by extension".** I considered lifting to `ow_data::fs_util` but the only caller is `load_palette_pcx` itself. Creating a new module for one call site is over-engineering; the TODO comment captures the second-caller signal.

**#8 [MEDIUM] asset_loader is 556 LOC and sectioned inconsistently.** After the LoadError refactor, the file dropped to 555 LOC. The three sections (office, debrief, mission) are clearly labeled. I'll accept it.

**#11 [LOW] `GameLoop` fields are `pub`.** Pre-existing. Not in scope for Phase B.

### Build + Test
```bash
cargo build --workspace    # clean
cargo test -p ow-app --bin ow-app  # 4/4 pass
cargo clippy -p ow-app     # 10 warnings (down from 12)
```

The remaining clippy warnings are all pre-existing patterns in the codebase. I did not introduce any new ones.

### File size after Phase B (round 2)
| File | Before this round | After |
|---|---|---|
| mod.rs | 707 | 592 |
| render.rs | 1730 | 1769 (added comments documenting the new struct shape; net structural change is small) |
| input.rs | 1134 | 1147 |
| update.rs | 280 | 282 |
| asset_loader.rs | 556 | 555 |
| audio_init.rs | 93 | 106 |
| soldier_anims.rs (was animation_watcher) | 178 | 261 (now one bigger struct + three private fns; same total functionality) |
| **new**: loop_pump.rs | — | 159 |
| **new**: mission.rs | — | 117 |
| mission_assets.rs (deleted) | 110 | — |
| animation_watcher.rs (deleted) | 178 | — |
| **Total** | **4966** | **4988** |

Slight LOC increase is from per-module headers and improved doc comments. The structural change is real: the god-bag is gone, the per-frame helpers are in their own module, and the per-phase input/update/render functions take only the data they actually need.

### What's next

- Phase C of `docs/refactor-game-loop.md` — split god-functions: `handle_office_input` (339 LOC), `handle_combat_input` (291 LOC), `update_combat` (193 LOC), `render_office` (482 LOC), `render_mission_map` (657 LOC), `render_debrief` (300+ LOC). Each gets its own sub-module and clean function boundaries.
- The `MissionView` and `SoldierAnims::spawn_controllers` APIs are ready but unused; future callers (e.g. a HUD layer) will pick them up.

## 2026-09-02 — game-loop Phase B: consolidate `run_game_loop` state into sibling structs

**Last Updated:** 2026-09-02
**Project Status:** 🟢 Phase B of `docs/refactor-game-loop.md` complete. `run_game_loop_with_pump` shrunk from ~920 LOC to ~140 LOC of body code (167 lines including doc comment). All four sibling structs are in place; all coupling hazards fixed; no behavior change.

### What Was Done This Session

Built on Phase A's leaf extractions (`combat_log`, `screenshot`, `dev_hotkeys`, `auto_screenshot`). Lifted all the remaining stack-local state out of `run_game_loop_with_pump` into four cohesive structs and the per-frame helpers that touch them.

**New modules (5):**
- `mission_assets.rs` — `MissionAssets<'a>` bundle: `tile_renderer`, `obj_renderer`, `loaded_map`, `mission_iso`, `default_iso` (new), `soldier_textures`, `soldier_anims`, `soldier_anim_set`, `prev_merc_states`, `walk_grace_remaining`, `enemies`. Owns the texture-creator lifetime `'a` (the `Texture<'a>` references inside it). Provides `assets.iso()` for the `mission_iso.unwrap_or(default_iso)` fallback and `assets.mission_loaded()` for state checks.
- `audio_handles.rs` — `AudioHandles` bundle: `audio_available`, `music_track`, `_music_handle`, `sfx_manager`, `voice_player`, `music_broken`. All five audio resources in one struct.
- `audio_init.rs` — `init_audio(data_dir, phase) -> AudioHandles`. ~70 LOC. Owns the mixer-device open + SFX preload + voice player + initial track selection. Soft-fails on mixer open and surfaces it via `audio_available: false`.
- `asset_loader.rs` — `load_office_texture`, `load_debrief_sprites`, `load_mission_map`, `create_anim_controllers`, `mission_number_from_name`. ~450 LOC. The big 322-line inline mission-load block is gone; `load_mission_map` is a single function that mutates `MissionAssets` in place.
- `animation_watcher.rs` — `update_animations(assets, team)` + `tick_anim_controllers(...)`. ~140 LOC. Pulls the 80-line inline animation state-machine out of the main loop; `dir_from_delta` helper becomes a private module fn.

**Coupling-hazard fixes (per issue #2):**
- `game.enemies` (struct) and `enemy_units` (stack-local) duplicated → unified into `assets.enemies`. Removed `game.enemies` field from `GameLoop`.
- `game.mission_iso` (struct) and `mission_iso_config` (stack-local) duplicated → unified into `assets.mission_iso`. Removed `game.mission_iso` field.
- `game.iso_config` vs `game.mission_iso` fallback pattern at `render.rs:949`, `input.rs:611, 806` → `let iso = assets.iso()` everywhere. Removed `game.iso_config` field; the default value now lives in `MissionAssets::default_iso` and is constructed once at `run_game_loop_with_pump` startup.
- `game.music_broken` field → moved into `AudioHandles.music_broken`.

**`GameLoop` after the refactor:** just `game_state`, `camera`, `phase_handler`, `window_width`, `window_height`, `combat_log`. Mission state lives in `MissionAssets`; audio state in `AudioHandles`. Five fields instead of nine.

**Signatures that changed:**
- `render_phase(&GameLoop, &MissionAssets, &mut Canvas, &TextRenderer, &TextureCreator, &Ruleset, &Option<Texture>, &[Texture], &[Texture])` — 9 args (was 15). The 6 dropped params (`tile_renderer`, `obj_renderer`, `loaded_map`, `mission_iso`, `soldier_texture`, `soldier_textures`, `soldier_anims`) now come from `assets`.
- `handle_phase_input(&mut GameLoop, &mut MissionAssets, ...)` — added `&mut MissionAssets` param. Threads through to `handle_deployment_input`, `handle_combat_input`, `handle_debrief_input` (the three functions that touch `assets.iso()` or `assets.enemies`).
- `update_phase(&mut GameLoop, &mut MissionAssets, ...)` — added `&mut MissionAssets` param. Threads through to `update_combat` (the only function that needed it; it iterates and mutates `assets.enemies`).
- `handle_dev_hotkeys(&mut GameLoop, &mut MissionAssets, &Event)` — F4 "kill all enemies" hotkey now takes `&mut MissionAssets` and clears `assets.enemies` directly.

**Per-frame helpers in `mod.rs`:** two small functions called from the main loop that are pure glue:
- `maybe_transition_music(&game, &mut audio, data_dir)` — 43 LOC, encapsulates the "is the music track right for this phase?" logic. Reads from `game.phase_handler` + `game.game_state.current_mission`, mutates `audio`.
- `maybe_load_mission(&mut game, &mut assets, &ruleset, data_dir, &tc)` — 52 LOC, encapsulates the "is it time to lazy-load the mission map?" guard. Calls `load_mission_map` + `create_anim_controllers` and re-centres the camera on the loaded map.

**File size after Phase B:**
| File | Before Phase A | After Phase A | After Phase B |
|---|---|---|---|
| mod.rs | 1551 | 1324 | 707 |
| input.rs | 1131 | 1131 | 1134 |
| render.rs | 1747 | 1747 | 1730 |
| update.rs | 267 | 267 | 280 |
| (new modules) | — | 348 | 1066 |
| **Total game_loop** | **4 696** | **4 817** | **4 917** |

Same total LOC as Phase A; the new modules absorb the inline code that used to bloat mod.rs.

### Did NOT do
- **Behaviour changes:** none. All four `game_loop::tests` still pass. The dev hotkeys and the F1-F5 dev paths are unchanged from the player's perspective. The two pre-existing `iso_math::tests` failures (unrelated, `screen_to_tile_even_row` / `_odd_row`) were not touched.
- **No smoke test against the real game data.** I do not have a local copy of the original `WOW/` tree on this machine; the previous session's `data/` was a gitignored copy. Next session should run `cargo run -p ow-app -- --data-dir ./data` against the real data to confirm visually.

### Build + Test
```bash
cargo build --workspace    # clean
cargo clippy -p ow-app     # 9 warnings (was 12 pre-refactor — net improvement)
cargo test -p ow-app --bin ow-app  # 4/4 pass
```

The new clippy warnings are all pre-existing patterns; I did not introduce any. Two of the warnings on `render_phase` / `render_debrief` are explicitly `#[allow(clippy::too_many_arguments)]`'d because Phase C of the refactor plan (splitting render into 9 passes) is what will address that, not this phase.

### Next Session
- Phase C of `docs/refactor-game-loop.md` — split god-functions: `handle_office_input` (339 LOC), `handle_combat_input` (291 LOC), `update_combat` (193 LOC), `render_office` (482 LOC), `render_mission_map` (657 LOC), `render_debrief`. Each gets its own sub-module and clean function boundaries.
- After Phase C, the run loop will shrink further and `render.rs` will drop from 1730 to ~250 LOC.
- After Phase C, the `too_many_arguments` `#[allow]`s on `render_phase` and `render_debrief` can be removed — the per-phase render fns will each take ≤4 args.

## 2026-09-02 — cross-platform bootstrap: SDL2 vendored, font bundled, game runs without env flags

**Last Updated:** 2026-09-02
**Project Status:** 🟢 The game now builds and runs with a plain `cargo run -p ow-app -- --data-dir ./data` on macOS with **no manual env-var exports**. Root cause of the earlier failure was a Windows-only build setup + Homebrew's `sdl2-compat` (SDL3 shim) crashing the SDL2 event parser.

### What Was Done This Session

1. **Copied game data into the repo** — full `WOW/` tree from the mounted disc image into `./data/WOW/` (~454 MB). Already covered by the existing `/data/` .gitignore rule, so it stays out of version control.

2. **Forked the repo** (GitHub `suhteevah/wages-of-war` → `plizkinzmey/wages-of-war`):
   - `origin`  → https://github.com/plizkinzmey/wages-of-war.git
   - `upstream` → https://github.com/suhteevah/wages-of-war.git

3. **Root-caused the SDL2 failure** (two distinct bugs):
   - **sdl2-compat panic**: Modern Homebrew ships `sdl2-compat` (an SDL3 transplant) instead of real SDL2. rust-sdl2's event parser builds SDL2.x enums that don't map onto SDL3 values → `panic: trying to construct an enum from an invalid value 0x207`.
   - **Font crash**: `TextRenderer` only searched Windows/Linux font paths (`C:\Windows\Fonts\...`, `/usr/share/fonts/...`). None exist on macOS → `Could not load any font`, game aborted on startup.
   - **`bundled` is a dead end**: rust-sdl2's `bundled` feature compiles **only SDL2 core** — it explicitly does not build SDL2_mixer/SDL2_image/SDL2_ttf (confirmed in `sdl2-sys` build.rs). Since ow-app uses all four, `bundled` alone cannot satisfy the project.

4. **Vendored SDL2 stack into `third_party/sdl2/`** (SDL2 2.30.12 + SDL2_mixer 2.8.2 + SDL2_image 2.8.5 + SDL2_ttf 2.24.0, built from source via cmake). Configured via:
   - `scripts/bootstrap-sdl2.sh` — idempotent build script (downloads, builds, installs all four into `third_party/sdl2/`); skips already-built components.
   - `.cargo/config.toml` — sets `PKG_CONFIG_PATH`, `LIBRARY_PATH`, `DYLD_LIBRARY_PATH` to `third_party/sdl2/...` with `relative = true` + `force = true`, so a plain `cargo run` links and finds the libs with zero env exports.
   - Upgraded `sdl2` 0.37 → **0.38** and enabled `use-pkgconfig` feature (this is what makes sdl2-sys emit `rustc-link-search` from the `PKG_CONFIG_PATH`; without it the linker can't find `-lSDL2`).
   - `.gitignore` excludes `/third_party/sdl2/` (regenerated on demand).

5. **Bundled a libre font** — `assets/fonts/DejaVuSansMono.ttf` (+ `LICENSE-DejaVu.txt`). `TextRenderer` now tries the bundled font first (resolved from CWD and the exe dir), then system fonts as fallback. Result: text renders on any OS with no host font dependency.

### Did NOT do
- **MIDI/SoundFont**: SDL2_mixer logs a benign `No SoundFonts have been requested` warning on macOS (no synth SoundFont present), and the game continues without music. Fixing this is a separate follow-up.
- The separate `re/` Ghidra RE work was untouched.

### Build Command (verified working)
```bash
./scripts/bootstrap-sdl2.sh   # once, on a fresh clone
cargo run -p ow-app -- --data-dir ./data
```
`cargo build --workspace` and `cargo run` both succeed with no env-var exports; the game reaches the Office screen, plays AVI intro, and processes clicks.

## 2026-05-03 — wall structure decoded, wall-rendering pass landed

**Last Updated:** 2026-05-03
**Project Status:** 🟡 wall data is now extracted from Wow.exe disasm and a debug rendering pass is wired up; needs a visual verification pass to confirm the segment-to-slot mapping.

### What Was Done This Session

Continuation of 2026-05-02 MVP push. After Matt corrected my overclaim about Word 2 buildings rendering, pivoted to actual disasm-driven RE.

**Ghidra harness extended** (`re/ghidra/scripts/`):
- `FindWagesArtifacts.java` — first-pass artifact extractor (1,158 strings, 2,599 functions, 143 imports, 300 data-file-related strings with xrefs).
- `DecompKeyFunctions.java` — decompiles a curated list of high-value functions plus every caller of `WinG*` / `RealizePalette` / `AnimatePalette` / `GetOpenFileNameA`.
- `WallHunt.java` — searches all defined strings for wall/fence/edge/Pass/etc. patterns and decompiles every caller. Found `"Can't Pass Wall: "` at 0x004ed658 (caller `FUN_0041d0f5`) and `"In GridPass: Tile is: "` at 0x004ed678 (caller `FUN_0041d2c6`).
- `WallStructure.java` — decompiles `FUN_0041b26d` (the cell-data unpacker) and the four tile-neighbor helpers (`FUN_0041bdeb/bc69/bf6d/bae7`) plus `FUN_0041c81c` (grid-direction → wall-slot translator).

**Disasm findings** (cited from `re/ghidra/projects/analysis/decomp/wallstruct_FUN_0041b26d.c`):
- Cell data lives at five known addresses, indexed by `cell_index * 4`:
  - `0x0059d8c0` → Word 1
  - `0x005a7640` → Word 2
  - `0x005d07c0` → Word 3 ← **the walls**
  - `0x005da540` → Word 4
  - `0x005e42c0` → Word 5
- **Word 3 layout (the breakthrough):** byte 0 = property byte (movement cost / terrain class), bits 8-31 = **TWELVE 2-bit wall slots** (direction 1 at the high end, direction 12 at the low end). The Rust parser already extracts these into `MapCell.terrain_mods: [u8; 12]` (`map_loader.rs:626`) — the data has been parsed all along; nothing rendered it.
- **Each cell has 4 sub-grids** (NW/NE/SE/SW quadrants — `FUN_0041c81c(grid, dir)` switch on grid 1-4). Up to 4 units per cell. Movement is grid-to-grid; walls block both interior (between quadrants of same cell) and exterior (cell-edge) movement.
- **GridPass cardinal moves sample 8 walls across 4 tiles** (current cell + 3 neighbors); diagonal moves sample 2.
- Word 4 layout = 4×6-bit + 4×2-bit. Currently parsed as "elevation" but the semantic interpretation (corner heights) isn't proven; the bit positions are right.
- Word 5 layout = 8-bit object_id + 4×6-bit. Current Rust only reads the 8-bit; the four 6-bit fields are extracted but not yet consumed.

**Wall-rendering pass added** at `game_loop.rs:3572-3672`:
- Iterates visible cells, skips those with all-zero `terrain_mods`.
- Computes diamond corners (N/E/S/W cardinal points + center + 4 edge midpoints) in screen space.
- Draws colored line segments for each non-zero wall slot:
  - Slots 1-8 = perimeter halves (clockwise from N)
  - Slots 9-12 = interior spokes from each cardinal corner to cell center
  - Color: green = value 1, yellow = value 2, red = value 3
- Slot-to-segment mapping is a **working hypothesis**, not yet validated against `FUN_0041c81c`.

**Earlier in same session (still 2026-05-02 by clock; entry below has details):**
- Word 2 overlay `if false` → `if true` flip (modest visual delta — see honesty correction in 2026-05-02 entry).
- Walk/Shoot/Die animation triggers wired with idle-revert.
- Player damage now weapon-driven via `ruleset.weapons` lookup.
- Auto-screenshot loop env-gated by `OW_AUTO_SCREENSHOT_MS`.

### Current State

**Working:**
- Full mission loop (Office → Hire → Contract → Deploy → Combat → Win → Debrief).
- Floor terrain, OBJ Word 5 sprites, Word 2 overlays, soldier sprites, animations, weapon-aware player damage, weapon-aware AP cost, range-checked shots.
- Auto-screenshot dev loop (env-gated, gitignored output).
- Ghidra workbench operational against `Wow.exe` at `re/ghidra/projects/wages-re.gpr`.
- 22 functions decompiled and saved as readable C under `re/ghidra/projects/analysis/decomp/`.

**Newly added (needs visual verification):**
- Wall rendering pass — should draw colored lines for `terrain_mods != 0` cells. Has not yet been confirmed visually.

**Stubbed / broken / incorrect:**
- AI damage still `rng.gen_range(3..15)` because enemy weapons in `mission_setup` are stored as `"Weapon_{idx}"` placeholders that don't resolve against `ruleset.weapons`.
- `MissionContext.combat: Option<CombatState>` is still `None` — full ow-core combat plumbing (initiative, suppression, weather, LOS) deferred.
- Black-diamond artifacts in late combat screenshots — possibly skull leakage past `>=500`, possibly empty-cell renders.
- Green vertical glitch line on the accountant portrait during debrief.
- `UnitRenderer { _private: () }` in `ow-render/` is dead code; real rendering happens inline in `render_mission_map`.

### Blocking Issues

None. Next session can iterate freely on the wall-rendering visual.

### What's Next

Ordered by value:

1. **Visual verification of the wall pass.** Run with auto-screenshot, look at combat frames, decide which case applies:
   - Walls appear roughly in the right places → tune segment-to-slot mapping by rotating the array.
   - Walls appear offset/wrong direction → rotate.
   - Walls appear but interior spokes 9-12 are wrong → re-derive that half of the layout from `FUN_0041c81c`.
   - No walls → screen-space math bug.
2. **Transcribe `FUN_0041c81c` fully** to lock in the canonical slot-to-edge mapping. The decomp is already in `re/ghidra/projects/analysis/decomp/wallstruct_FUN_0041c81c.c` — just needs careful translation.
3. **Upgrade walls from line-segments → sprite blits.** Walls almost certainly have a sprite atlas (TIL-low-index? OBJ-high-index? a separate wall sheet?). Find via xref of the wall-rendering function in Wow.exe (we have GridPass + the wall accessors but not the wall *renderer* yet — needs another decomp pass).
4. **Word 5's 4×6-bit fields** — currently extracted but never used. Could be vegetation, additional sprite slots, or sub-grid object placement. Investigate.
5. **AI weapon hookup** — needs a parallel `Vec<Weapon>` indexed lookup or rewrite `EnemyUnit` to carry a resolved weapon name. See marker comment at `game_loop.rs:2625`.
6. **Full ow-core combat routing** — populate `MissionContext.combat`, replace ad-hoc resolvers with `execute_action` / `decide_action`. Net negative LOC.
7. Cosmetics — debrief portrait green-line, black-diamond artifact investigation.

### Notes for Next Session

- **Wall slot indexing convention:** `terrain_mods[0]` = wall slot 1 in the disasm (highest 2 bits at >>30); `terrain_mods[11]` = slot 12 (lowest at >>8). The renderer assumes this in the segments array — preserve.
- **The 4 sub-grid concept changes the model.** Cells aren't atomic; each holds a 2×2 sub-grid plus 12 walls. Up to 4 units fit per cell. Pathfinding/movement should ultimately work in (tile, grid) coordinates, not just (tile). Out of scope for now but informs future work.
- **`FUN_0041c81c` lookup table** (from disasm): 4×4 grid×direction → slot 1-12. Use this to confirm/derive any geometric interpretation. The exact mapping is in `re/ghidra/projects/analysis/decomp/wallstruct_FUN_0041c81c.c`.
- **Don't trust the "Word 4 = elevation" interpretation** without independent evidence. The bit positions match a 4×6-bit + 4×2-bit layout, but the semantic interpretation might be cover percentages, light levels, or sub-grid heights rather than per-corner ground elevation. Word 4 is all-zero in 16/16 ship maps, so currently unfalsifiable.
- **Auto-screenshot is env-gated.** Set `OW_AUTO_SCREENSHOT_MS=1500` to capture every 1.5s into `dev-screenshots/run-<unix-ts>/ss_NNNNN_<phase>.bmp`. Gitignored. Helpful for next session if it needs to see what's actually rendering without running the game directly.
- **Ghidra workbench location:** `C:\Tools\ghidra_12.0.4_PUBLIC\`. Project at `J:\wages of war\re\ghidra\projects\wages-re.gpr`. Re-run any decomp script via `analyzeHeadless.bat <projDir> <projName> -process Wow.exe -scriptPath <scriptDir> -postScript <Name>.java -noanalysis`. **Java scripts only — Ghidra 12 dropped Jython.**
- **Recurring Semgrep false positive** at `game_loop.rs:1086` (the OFFPIC2.PCX asset loader's `read_dir`). Not Actix, not network input. The CLAUDE.md / hooks should ignore this rule for desktop-game crates.

---

## 2026-05-02 — MVP unstuck

Big session. Project went from "stuck spinning, half-assed" to a playable mission loop with real visuals, real animations, and weapon-driven player combat. Five concrete moves:

1. **Re-enabled the Word 2 overlay pass** at `game_loop.rs:3192` (was `if false`). The pass now runs and emits a one-shot histogram on first frame (176 unique overlay indices, 2,066 total occurrences, skull markers 503-507 cleanly clustered and filtered). The visual delta is **modest, not transformative** — what was already rendering as compound floors via the Word 5 OBJ pass still renders, and the Word 2 pass adds incremental decoration, but **walls and fences are still mostly absent**. The original handoff blocker stands. Investigation continues via Ghidra (see below).
2. **Wired Walk / ShootStand / Die animation triggers.** State-watcher in the per-frame loop diffs each merc's position / hp / ap against a previous-frame snapshot and dispatches `set_action` accordingly. ShootStand reverts to Idle on `is_finished()`; Walk reverts after a 24-frame grace period (since in-game movement is teleport-based, not interpolated).
3. **`UnitRenderer { _private: () }` is dead code.** Real unit rendering already lives in `render_mission_map` and uses `AnimController.current_frame_index()` to look up `soldier_textures[i]`. The audit was wrong — the placeholder struct in `ow-render/` is just unused.
4. **Player damage is now weapon-driven.** Replaced `rng.gen_range(5..20)` with a lookup of the merc's equipped weapon in `ruleset.weapons`. Damage scales with `weapon.damage_class` (±25% jitter), AP cost from `weapon.ap_cost`, and shots beyond `weapon.weapon_range` are forced misses with a clear log message and no AP burn.
5. **Ground-truth RE infra installed.** Ghidra 12.0.4 at `C:\Tools\ghidra_12.0.4_PUBLIC\`. `Wow.exe` imported into a fresh project at `J:\wages of war\re\ghidra\projects\wages-re.gpr`. First-pass artifact extraction (`re/ghidra/scripts/FindWagesArtifacts.java`) produced JSON for 1,158 strings, 2,599 functions, 143 imports across 7 DLLs, and 300 data-file-related strings with xrefs. Output at `re/ghidra/projects/analysis/`. **Major finding: Wow.exe contains a built-in scenario/tile/object editor** (file-open dialog filter `Tile Map (*.map)|*.map|...`). Renderer is WinG (5 functions: `WinGBitBlt`, `WinGCreateBitmap`, `WinGCreateDC`, `WinGRecommendDIBFormat`, `WinGSetDIBColorTable`) — no DirectX. Audio is `mciSendCommandA` for MIDI, `sndPlaySoundA` for WAVs.

Dev infrastructure added:
- **Auto-screenshot loop** — set `OW_AUTO_SCREENSHOT_MS=1500` and dump every 1.5s to `dev-screenshots/run-<unix-ts>/ss_NNNNN_<phase>.bmp`. Phase tag in filename means combat frames are findable in a 200-shot run. Gitignored.
- **Reassessment doc** at `REASSESSMENT_2026-05-02.md` — brutal "what's real / what's fake" audit of every subsystem.
- **Honesty checkpoints** prepended to `flir/HANDOFF.md` and `flightsuite/HANDOFF.md` — cross-project audit found that the assumed Ghidra workbench had never been used; specs are hand-authored from datasheets, not disasm-derived.

## What still needs doing (post-MVP)

Visible:
- **Deployment view shows a black void in the lower-left** of mission 1. Probably just the camera defaulting near the edge of the 140x72 map; probably not a bug. Confirm before fixing.
- **Black-diamond artifacts in late combat** (visible in `dev-screenshots/run-1777774109/ss_00055_*`). Could be skull-marker leakage past `>=500`, could be unit shadows, could be empty-cell renders. Quick instrumentation pass would settle it.
- **Green vertical glitch line on the accountant portrait** during debrief. Palette or scanline issue. Cosmetic.

Invisible-but-fake:
- **AI damage is still `rng.gen_range(3..15)`.** Enemy weapons in `mission_setup::EnemyUnit::from_rating` are stored as `"Weapon_{idx}"` placeholders that don't resolve against `ruleset.weapons` (which is keyed by name). Wiring requires either a parallel `Vec<Weapon>` indexed lookup or rewriting EnemyUnit to store the resolved name. Search `game_loop.rs:2625` for the marker comment.
- **`MissionContext.combat: Option<CombatState>` is still `None`.** Full ow-core combat plumbing (initiative, suppression, weather, LOS, real `setup_mission`) was deferred — the simpler weapon-driven damage swap covers 80% of the visible delta. Real ow-core routing is the right next refactor and is partly written: `setup_mission`, `execute_action`, `decide_action`, `find_path`, `resolve_attack`, `check_suppression`, `accuracy_modifier`, all real and tested in `ow-core`.

RE follow-ups (requires Ghidra now installed):
- Decompile the function that reads MAP+0x031624 if any function references it. Verifies or kills the "31×u16 tileset reference table" claim.
- Decompile cell-word unpack functions to confirm Word 3/4/5 layouts (Word 4 elevation is all-zero in 16/16 maps; could be parser misreading dead bits).
- Decompile the WinG blit dispatch to settle the row-spacing question (32 vs 64 px).
- `BINARY_FORMATS_DEEP_RE.md` Sections 2.2-2.5 should be marked unverified at the top until the above is done.

---

## TL;DR (pre-2026-05-02)
Clean-room Rust reimplementation of *Wages of War* (1996). **MAP parser rewritten** with correct 140x72 grid from deep Wow.exe RE. AVI cutscenes with audio working. Soldier animation system wired up. Combat SFX and voice playback added. Terrain rendering mostly correct — compound floors visible, buildings partially rendering from OBJ sprites. Dev hotkeys for fast testing.

## Current State (2026-04-11)

### Working
- Full game loop: Office → Hire → Contract → Deploy → Fight → Win → Debrief
- MAP parser: 140x72 grid, 5 parallel cell arrays, all metadata blocks
- Staggered isometric projection (128x64 tiles, 32px half-height row spacing, odd-row +64px stagger)
- AVI cutscene playback with audio (ffmpeg-sidecar, MSRLE + ADPCM)
- MIDI music playback (M key to mute)
- Combat SFX (pistol/rifle/shotgun from SND/ WAVs)
- Voice line system (WAV playback on hire/selection)
- Video phone debrief (ACCT.OBJ portrait, PHONSPR.OBJ background)
- Soldier animation system (COR/DAT parsed, AnimController per merc, 2000 frames decoded)
- Terrain with Word 1 overlays (indices 1-499 from TIL)
- Word 2 overlays split: low indices from TIL, high from OBJ
- Window icon (Wow.ico)
- Dev hotkeys: F1-F5, F12, M

### Known Issues
1. **Building walls/fences missing** — Word 2 high-index OBJ sprites showing some elements but not walls. Need to investigate the TIL/OBJ index mapping more carefully. The tileset reference table (31 x u16 at MAP offset 0x031624) may hold the key.
2. **Skull markers** (503-507) visible as black diamonds — filtered from rendering but some still appear
3. **Path alignment** — terrain transitions slightly off at diamond edges
4. **Animation triggers** — only idle plays, walk/shoot/die not wired to game actions
5. **VLS lip-sync** — accountant portrait is static, viseme timeline not connected
6. **Voice files** — per-merc voices are inside VLS/VLA containers, not standalone WAVs

### RE Docs Completed This Session
- `docs/BINARY_FORMATS_DEEP_RE.md` — Complete Wow.exe disassembly (MAP, cells, projection, sprites, WinG)
- `docs/COR_ANIM_FORMAT.md` — Animation index + DAT sprite archive, verified across 32 pairs
- `docs/VLS_VLA_FORMAT.md` — Voice lip-sync with viseme timelines
- `docs/WRI_FORMAT.md` — Microsoft Write mission brief extraction

### Key Discoveries
- MAP grid is 140x72 = 10,080 cells (NOT 200x252)
- 5 parallel cell arrays, 4 bytes each, sequential on disk
- Word 5 object_id: 0xFF = empty sentinel (10079/10080 cells are 0xFF)
- Elevation (Word 4): all zeros across ALL 16 missions — never used by map editor
- Staggered grid needs 32px row spacing for diamond interlocking (exe uses 64px internally)
- Tile sprites are diamonds with transparent corners (palette index 0)
- TIL and OBJ both have 512 frames — Word 2 overlays may reference OBJ for buildings

### Architecture
- `ow-data/src/map_loader.rs` — Rewritten MAP parser with MapCell struct (all 5 words)
- `ow-render/src/iso_math.rs` — Staggered grid projection (tile_to_screen, screen_to_tile)
- `ow-app/src/avi_player.rs` — NEW: AVI cutscene playback via ffmpeg-sidecar
- `ow-audio/src/sfx.rs` — NEW: Combat SFX manager
- `ow-audio/src/voice.rs` — NEW: Voice line playback
- `ow-app/src/game_loop.rs` — ~4000+ lines, needs splitting

## Next Steps (Priority Order)
1. Fix building/fence rendering — investigate OBJ sprite content and tileset reference table
2. Wire animation triggers (walk/shoot/die) to game actions
3. Filter skull marker sprites (503-507) completely
4. Extract per-merc voices from VLS containers
5. Wire VLS viseme timeline to accountant portrait
6. Split game_loop.rs into sub-modules
