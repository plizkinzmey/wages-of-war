//! Phase rendering — per-phase draw routines for the SDL2 canvas.

use sdl2::pixels::Color;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use tracing::{info, trace};

use ow_core::game_state::OfficePhase;
use ow_core::ruleset::Ruleset;
use ow_render::camera::Camera;
use ow_render::iso_math::{IsoConfig, TilePos};
use ow_render::text::TextRenderer;

use super::mission::MissionData;
use super::office_layout::{HOTSPOTS, OFFICE_H, OFFICE_W};
use super::soldier_anims::SoldierAnims;
use super::{GameLoop, PhaseHandler, WINDOW_HEIGHT, WINDOW_WIDTH};

// ===========================================================================
// Phase rendering
// ===========================================================================

/// Render the current phase to the canvas.
///
/// Most phases render a colored background (set in the main loop) with
/// geometric placeholders. Combat renders an isometric grid plus unit
/// markers.
///
/// `mission` is `Some` when a map is loaded (Deployment, Combat,
/// Extraction, Debrief) and `None` for Office and Travel. The
/// mission-aware renderers branch on it internally; the mission-
/// agnostic renderers ignore the parameter.
///
/// `anims` is a borrowed read-only view into the soldier animation
/// state (textures, controllers). The other `&Option<Texture>` and
/// `&[Texture]` parameters are scene assets that live in the main
/// loop (office background, debrief sprites) and have a different
/// lifetime story from mission assets.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_phase(
    game: &GameLoop,
    mission: Option<&MissionData>,
    anims: &SoldierAnims,
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    office_bg: &Option<Texture>,
    acct_textures: &[Texture],
    phone_textures: &[Texture],
) {
    match &game.phase_handler {
        PhaseHandler::Office { sub_phase } => {
            render_office(game, canvas, *sub_phase, text, tc, ruleset, office_bg)
        }
        PhaseHandler::Travel { elapsed_ms } => render_travel(canvas, *elapsed_ms, text, tc),
        PhaseHandler::Deployment { .. } | PhaseHandler::Combat(_) => {
            if let Some(m) = mission {
                render_mission_map(game, m, anims, canvas, text, tc);
            } else {
                // Mission phase without a loaded mission — shouldn't
                // happen in practice; render an empty placeholder so a
                // bug is visible rather than crashing.
                canvas.set_draw_color(Color::RGB(0, 0, 0));
                canvas.clear();
            }
        }
        PhaseHandler::Extraction => {
            // No mission: use the default iso derived from the window
            // dimensions. With a mission: use the mission's iso. The
            // IsoConfig is small and Copy so this branch is free.
            let default_iso = IsoConfig {
                tile_width: 64.0,
                tile_height: 32.0,
                origin_x: (WINDOW_WIDTH as f32) / 2.0,
                origin_y: 64.0,
            };
            let iso: &IsoConfig = mission.map(|m| &m.iso).unwrap_or(&default_iso);
            render_extraction(game, canvas, iso);
        }
        PhaseHandler::Debrief {
            success,
            anim_elapsed_ms,
        } => render_debrief(
            game,
            mission,
            canvas,
            *success,
            *anim_elapsed_ms,
            text,
            tc,
            acct_textures,
            phone_textures,
        ),
        PhaseHandler::Paused { .. } => render_pause(canvas, text, tc),
    }
}

// ---------------------------------------------------------------------------
// Office rendering
// ---------------------------------------------------------------------------

/// Render the office screen.
///
/// The Office phase has six sub-phases. Overview draws the original
/// OFFICE.PCX background plus a status bar. The other sub-phases draw
/// a tab bar + content area. Each sub-phase rendering is its own named
/// helper; this function is a thin dispatcher.
fn render_office(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    active_sub: OfficePhase,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    office_bg: &Option<Texture>,
) {
    if active_sub == OfficePhase::Overview {
        draw_office_overview(canvas, office_bg, text, tc, game);
    } else {
        draw_office_tab_bar(canvas, text, tc, active_sub, game);
        draw_office_content(canvas, text, tc, active_sub, ruleset, game);
    }
}

/// Overview: the iconic OFFICE.PCX background with a status bar at
/// the bottom (funds / team size / missions) and a hot-key reminder.
/// Also draws labeled debug hotspot overlays during development.
fn draw_office_overview(
    canvas: &mut Canvas<Window>,
    office_bg: &Option<Texture>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    game: &GameLoop,
) {
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    if let Some(bg_tex) = office_bg {
        // Scale the 640x480 office background to fill the window.
        canvas.copy(bg_tex, None, Some(Rect::new(0, 0, w, h))).ok();
    }

    // Bottom status bar
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    canvas.fill_rect(Rect::new(0, (h - 55) as i32, w, 55)).ok();
    canvas.set_blend_mode(sdl2::render::BlendMode::None);

    let funds_text = format!(
        "Funds: ${:>12}  |  Team: {}/8  |  Missions: {}",
        game.game_state.funds,
        game.game_state.team.len(),
        game.game_state.missions_completed
    );
    text.draw(
        canvas,
        tc,
        &funds_text,
        15,
        (h - 45) as i32,
        Color::RGB(220, 220, 220),
    )
    .ok();
    text.draw_small(
        canvas,
        tc,
        "1:Hire  2:Equip  3:Intel  4:Contracts  5:Train  |  B:Begin Mission  |  ESC:Quit",
        15,
        (h - 22) as i32,
        Color::RGB(160, 160, 180),
    )
    .ok();

    draw_office_hotspot_overlays(canvas, text, tc, game);
}

/// Debug-only: draw labeled rect overlays for each clickable hotspot on
/// the office background. The hotspot list itself lives in
/// `super::office_layout::HOTSPOTS` so the click handler and overlay
/// renderer can never disagree.
fn draw_office_hotspot_overlays(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    game: &GameLoop,
) {
    let ww = game.window_width as f32;
    let wh = game.window_height as f32;
    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    for h in HOTSPOTS {
        // Scale 640x480 → window size.
        let sx1 = (h.rect.0 as f32 * ww / OFFICE_W as f32) as i32;
        let sy1 = (h.rect.1 as f32 * wh / OFFICE_H as f32) as i32;
        let sx2 = (h.rect.2 as f32 * ww / OFFICE_W as f32) as i32;
        let sy2 = (h.rect.3 as f32 * wh / OFFICE_H as f32) as i32;
        canvas.set_draw_color(h.color);
        canvas
            .fill_rect(Rect::new(sx1, sy1, (sx2 - sx1) as u32, (sy2 - sy1) as u32))
            .ok();
        canvas.set_draw_color(Color::RGB(255, 255, 255));
        canvas
            .draw_rect(Rect::new(sx1, sy1, (sx2 - sx1) as u32, (sy2 - sy1) as u32))
            .ok();
        text.draw_small(
            canvas,
            tc,
            h.label,
            sx1 + 4,
            sy1 + 4,
            Color::RGB(255, 255, 255),
        )
        .ok();
    }
    canvas.set_blend_mode(sdl2::render::BlendMode::None);
}

/// Tab bar along the top: 5 numbered sub-phase tabs (1:Hire, 2:Equip,
/// 3:Intel, 4:Contracts, 5:Train) plus a [ESC] Office button. The
/// active tab is highlighted.
fn draw_office_tab_bar(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    active_sub: OfficePhase,
    game: &GameLoop,
) {
    let w = canvas.output_size().map(|(w, _)| w).unwrap_or(WINDOW_WIDTH);

    canvas.set_draw_color(Color::RGB(15, 15, 25));
    canvas.fill_rect(Rect::new(0, 0, w, 35)).ok();

    text.draw_small(
        canvas,
        tc,
        "[ESC] Office",
        10,
        10,
        Color::RGB(140, 140, 160),
    )
    .ok();

    let tab_names = ["1:Hire", "2:Equip", "3:Intel", "4:Contracts", "5:Train"];
    let sub_phases = [
        OfficePhase::HireMercs,
        OfficePhase::Equipment,
        OfficePhase::Intel,
        OfficePhase::Contracts,
        OfficePhase::Training,
    ];
    for (i, (sp, name)) in sub_phases.iter().zip(tab_names.iter()).enumerate() {
        let x = 130 + (i as i32) * 130;
        let active = *sp == active_sub;
        let bg = if active {
            Color::RGB(60, 60, 100)
        } else {
            Color::RGB(30, 30, 45)
        };
        let fg = if active {
            Color::RGB(255, 255, 200)
        } else {
            Color::RGB(140, 140, 140)
        };
        canvas.set_draw_color(bg);
        canvas.fill_rect(Rect::new(x, 5, 120, 25)).ok();
        text.draw_small(canvas, tc, name, x + 8, 10, fg).ok();
    }

    // Status bar at bottom
    let h = canvas
        .output_size()
        .map(|(_, h)| h)
        .unwrap_or(WINDOW_HEIGHT);
    canvas.set_draw_color(Color::RGB(20, 20, 30));
    canvas.fill_rect(Rect::new(0, (h - 50) as i32, w, 50)).ok();
    let funds_text = format!(
        "Funds: ${:>12}  |  Team: {}/8  |  Missions: {}",
        game.game_state.funds,
        game.game_state.team.len(),
        game.game_state.missions_completed
    );
    text.draw(
        canvas,
        tc,
        &funds_text,
        15,
        (h - 35) as i32,
        Color::RGB(200, 200, 200),
    )
    .ok();
}

/// Main content area below the tab bar. Dispatches to the per-sub-phase
/// content renderer; Intel and Training fall through to a placeholder
/// since they aren't implemented yet.
fn draw_office_content(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    active_sub: OfficePhase,
    ruleset: &Ruleset,
    game: &GameLoop,
) {
    let h = canvas
        .output_size()
        .map(|(_, h)| h)
        .unwrap_or(WINDOW_HEIGHT);
    let content_y = 50;
    let content_h = h as i32 - 50 - 55;

    match active_sub {
        OfficePhase::Overview => {} // handled by the caller
        OfficePhase::HireMercs => {
            draw_office_hire_mercs(canvas, text, tc, ruleset, game, content_y, content_h)
        }
        OfficePhase::Equipment => {
            draw_office_equipment(canvas, text, tc, ruleset, game, content_y, content_h)
        }
        OfficePhase::Contracts => {
            draw_office_contracts(canvas, text, tc, ruleset, game, content_y, content_h)
        }
        OfficePhase::Intel | OfficePhase::Training => {
            draw_office_placeholder(canvas, text, tc, active_sub, content_y)
        }
    }

    // Help text in the bottom-right corner.
    let w = canvas.output_size().map(|(w, _)| w).unwrap_or(WINDOW_WIDTH);
    text.draw_small(
        canvas,
        tc,
        "ESC: Pause  |  B: Begin Mission",
        (w - 280) as i32,
        (h - 35) as i32,
        Color::RGB(100, 100, 120),
    )
    .ok();
}

/// HireMercs: scrollable list of available mercs sorted by rating.
/// Each row shows stats; already-hired mercs are green, unavailable
/// ones are gray.
fn draw_office_hire_mercs(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    game: &GameLoop,
    content_y: i32,
    content_h: i32,
) {
    text.draw_header(
        canvas,
        tc,
        "Mercenary Roster",
        20,
        content_y,
        Color::RGB(220, 200, 100),
    )
    .ok();

    let mut y = content_y + 35;
    let mut sorted_mercs: Vec<_> = ruleset.mercs.values().collect();
    sorted_mercs.sort_by(|a, b| b.rating.cmp(&a.rating)); // best first

    for merc in sorted_mercs.iter().take(25) {
        let hired = game.game_state.team.iter().any(|m| m.name == merc.name);
        let (status_color, status_tag) = if hired {
            (Color::RGB(100, 200, 100), "[HIRED]")
        } else if merc.avail == 0 {
            (Color::RGB(100, 100, 100), "[N/A]")
        } else {
            (Color::RGB(200, 200, 200), "")
        };
        let line = format!(
            "{:<25} RAT:{:>3}  EXP:{:>3}  WSK:{:>3}  AGL:{:>3}  Hire:${:>7}  {}",
            merc.name, merc.rating, merc.exp, merc.wsk, merc.agl, merc.fee_hire, status_tag
        );
        text.draw_small(canvas, tc, &line, 20, y, status_color).ok();
        y += 16;
        if y > (content_y + content_h - 20) {
            break;
        }
    }

    let count = sorted_mercs.len().min(25);
    text.draw_small(
        canvas,
        tc,
        &format!(
            "Showing {count}/{} mercs (sorted by rating)",
            ruleset.mercs.len()
        ),
        20,
        content_y + content_h,
        Color::RGB(100, 100, 100),
    )
    .ok();
}

/// Equipment: two-pane view. Left pane is the weapons catalog
/// (clickable), right pane is the team's current loadout. The
/// per-pane rendering lives in `draw_equipment_weapons_pane` and
/// `draw_equipment_team_pane`; the bottom status row is its own
/// helper so each one is small and focused.
fn draw_office_equipment(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    game: &GameLoop,
    content_y: i32,
    content_h: i32,
) {
    text.draw_header(
        canvas,
        tc,
        "Equipment Catalog — Click weapon to lease",
        20,
        content_y,
        Color::RGB(220, 200, 100),
    )
    .ok();

    draw_equipment_weapons_pane(canvas, text, tc, ruleset, game, content_y, content_h);
    draw_equipment_team_pane(canvas, text, tc, game, content_y);
    draw_equipment_status_row(canvas, text, tc, game, content_y + content_h);
}

/// Left pane: weapon catalog. Sorted by `WeaponType` (rifle, pistol,
/// shotgun, ...) so the player sees weapons grouped by class. Each
/// row shows a "xN leased" badge when the team has copies.
fn draw_equipment_weapons_pane(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    game: &GameLoop,
    content_y: i32,
    content_h: i32,
) {
    let mut y = content_y + 35;
    text.draw(
        canvas,
        tc,
        "--- AVAILABLE WEAPONS ---",
        20,
        y,
        Color::RGB(180, 140, 80),
    )
    .ok();
    y += 20;

    let mut sorted_weapons: Vec<_> = ruleset.weapons.values().collect();
    sorted_weapons.sort_by_key(|w| w.weapon_type);
    let leased_names: Vec<String> = game
        .game_state
        .team
        .iter()
        .flat_map(|m| m.inventory.iter().map(|i| i.name.clone()))
        .collect();

    for w in sorted_weapons.iter().take(25) {
        let leased_count = leased_names.iter().filter(|n| *n == &w.name).count();
        let tag = if leased_count > 0 {
            format!(" [x{}]", leased_count)
        } else {
            String::new()
        };
        let affordable = game.game_state.funds >= w.cost as i64;
        let color = if leased_count > 0 {
            Color::RGB(100, 200, 100)
        } else if !affordable {
            Color::RGB(120, 80, 80)
        } else {
            Color::RGB(200, 200, 200)
        };
        let line = format!(
            "{:<22} RNG:{:>2} DMG:{:>2} PEN:{:>2} AP:{:>2} ${:>5}{}",
            w.name, w.weapon_range, w.damage_class, w.penetration, w.ap_cost, w.cost, tag
        );
        text.draw_small(canvas, tc, &line, 20, y, color).ok();
        y += 14;
        if y > (content_y + content_h - 40) {
            break;
        }
    }
}

/// Right pane: team roster with each merc's current inventory. The
/// x-offset sits at half the window width + 20px so the pane doesn't
/// overlap the weapons catalog.
fn draw_equipment_team_pane(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    game: &GameLoop,
    content_y: i32,
) {
    let w = canvas.output_size().map(|(w, _)| w).unwrap_or(WINDOW_WIDTH);
    let team_x = (w / 2) as i32 + 20;
    let mut ty = content_y + 35;
    text.draw(
        canvas,
        tc,
        "--- YOUR TEAM ---",
        team_x,
        ty,
        Color::RGB(100, 180, 100),
    )
    .ok();
    ty += 20;

    if game.game_state.team.is_empty() {
        text.draw_small(
            canvas,
            tc,
            "No mercs hired yet",
            team_x,
            ty,
            Color::RGB(140, 140, 140),
        )
        .ok();
        return;
    }

    for merc in &game.game_state.team {
        let equip_info = if merc.inventory.is_empty() {
            "  [UNARMED]".to_string()
        } else {
            format!(
                "  [{}]",
                merc.inventory
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let line = format!("{}{}", merc.name, equip_info);
        let color = if merc.inventory.is_empty() {
            Color::RGB(200, 100, 100)
        } else {
            Color::RGB(100, 200, 100)
        };
        text.draw_small(canvas, tc, &line, team_x, ty, color).ok();
        ty += 16;
    }
}

/// Bottom-of-pane status row. "Click weapon to lease" instruction
/// + the armed/total count so the player can see the team loadout
/// at a glance without reading the right pane.
fn draw_equipment_status_row(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    game: &GameLoop,
    status_y: i32,
) {
    let armed_count = game
        .game_state
        .team
        .iter()
        .filter(|m| !m.inventory.is_empty())
        .count();
    let team_count = game.game_state.team.len();
    let equip_status = format!(
        "Click weapon to lease → assigned to first unarmed merc  |  U: Return all weapons  |  Armed: {}/{}",
        armed_count, team_count
    );
    text.draw_small(
        canvas,
        tc,
        &equip_status,
        20,
        status_y,
        Color::RGB(140, 140, 100),
    )
    .ok();
}

/// Contracts: scrollable list of mission contracts from the ruleset.
/// The currently-accepted contract is highlighted in green.
fn draw_office_contracts(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    game: &GameLoop,
    content_y: i32,
    content_h: i32,
) {
    text.draw_header(
        canvas,
        tc,
        "Available Contracts — Click to Accept",
        20,
        content_y,
        Color::RGB(220, 200, 100),
    )
    .ok();
    let mut y = content_y + 35;

    let accepted_id = game
        .game_state
        .current_mission
        .as_ref()
        .map(|m| m.name.clone());
    if let Some(ref aid) = accepted_id {
        text.draw(
            canvas,
            tc,
            &format!("ACCEPTED: {} — Press B or click door to deploy!", aid),
            20,
            y,
            Color::RGB(100, 255, 100),
        )
        .ok();
        y += 22;
    }

    let mut mission_ids: Vec<_> = ruleset.missions.keys().collect();
    mission_ids.sort();
    for mid in &mission_ids {
        if let Some(mission) = ruleset.missions.get(*mid) {
            let is_accepted = accepted_id.as_deref() == Some(mid.as_str());
            let color = if is_accepted {
                Color::RGB(100, 255, 100)
            } else {
                Color::RGB(200, 200, 200)
            };
            let tag = if is_accepted { " [ACCEPTED]" } else { "" };
            let terms = if mission.contract.terms.len() > 60 {
                &mission.contract.terms[..60]
            } else {
                &mission.contract.terms
            };
            let line = format!(
                "{}: {}... Adv:${} Bon:${}{}",
                mid, terms, mission.contract.advance, mission.contract.bonus, tag
            );
            text.draw_small(canvas, tc, &line, 20, y, color).ok();
            y += 18;
            if y > (content_y + content_h - 20) {
                break;
            }
        }
    }
}

/// Placeholder for sub-phases that aren't implemented yet (Intel,
/// Training). Shows a "Coming soon..." message so the empty tab doesn't
/// look broken.
fn draw_office_placeholder(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    active_sub: OfficePhase,
    content_y: i32,
) {
    let label = format!("{:?}", active_sub);
    text.draw_header(canvas, tc, &label, 20, content_y, Color::RGB(220, 200, 100))
        .ok();
    text.draw(
        canvas,
        tc,
        "Coming soon...",
        20,
        content_y + 35,
        Color::RGB(140, 140, 140),
    )
    .ok();
}

// ---------------------------------------------------------------------------
// Travel rendering
// ---------------------------------------------------------------------------

/// Render the travel screen — a simple progress bar.
fn render_travel(
    canvas: &mut Canvas<Window>,
    elapsed_ms: u32,
    _text: &TextRenderer,
    _tc: &TextureCreator<WindowContext>,
) {
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));
    let progress = (elapsed_ms as f32 / 2000.0).min(1.0);
    let bar_width = (w as f32 * 0.6) as u32;
    let bar_x = ((w - bar_width) / 2) as i32;
    let bar_y = (h / 2) as i32;

    // Background bar
    canvas.set_draw_color(Color::RGB(40, 40, 40));
    canvas
        .fill_rect(sdl2::rect::Rect::new(bar_x, bar_y, bar_width, 20))
        .ok();

    // Filled portion
    let fill = (bar_width as f32 * progress) as u32;
    canvas.set_draw_color(Color::RGB(100, 180, 100));
    canvas
        .fill_rect(sdl2::rect::Rect::new(bar_x, bar_y, fill, 20))
        .ok();
}

// ---------------------------------------------------------------------------
// Deployment rendering
// ---------------------------------------------------------------------------

/// Render the mission map for the Deployment and Combat phases. The
/// function is an orchestrator over eight named per-pass helpers plus
/// the inlined terrain pass. The passes run in this order:
/// 1. Terrain tiles — inlined; a single delegation to the tile
///    renderer, no extra logic.
/// 2. `draw_obj_sprites` — buildings/walls/trees from the OBJ sheet.
/// 3. `draw_word2_overlays` — secondary decoration overlays (trees,
///    buildings) at the cell level.
/// 4. `draw_walls` — 12-slot wall geometry from cell Word 3.
/// 5. `draw_player_mercs` — animated soldier sprites on the team.
/// 6. `draw_enemies` — enemy markers with fog-of-war visibility.
/// 7. `draw_combat_hud` — top/bottom panels + combat log (Combat only).
/// 8. `draw_deployment_hud` — placement progress (Deployment only).
/// 9. `draw_minimap` — top-down overview in the bottom-right corner.
fn render_mission_map(
    game: &GameLoop,
    mission: &MissionData,
    anims: &SoldierAnims,
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
) {
    // Terrain tiles: a single delegation to the tile renderer. Inlined
    // here rather than wrapped in `draw_terrain` because the wrapper
    // would be a pass-through with no extra logic.
    mission
        .tile_renderer
        .render_map(canvas, &mission.map, &game.camera, &mission.iso);
    draw_obj_sprites(game, mission, canvas);
    draw_word2_overlays(game, mission, canvas);
    draw_walls(game, mission, canvas);
    draw_player_mercs(game, mission, anims, canvas);
    draw_enemies(game, mission, canvas);
    draw_combat_hud(game, canvas, text, tc);
    draw_deployment_hud(game, canvas, text, tc);
    draw_minimap(game, mission, canvas, text, tc);
}

// ---------------------------------------------------------------------------
// Per-pass drawing helpers
// ---------------------------------------------------------------------------

/// Compute the visible-tile rectangle for a mission, clamped to the
/// map bounds. Each draw pass over the map calls this once instead of
/// re-deriving the bounds.
fn visible_tile_rect(
    game: &GameLoop,
    iso: &IsoConfig,
    map: &ow_data::map_loader::GameMap,
) -> (usize, usize, usize, usize) {
    map.clamp_tile_rect(game.camera.visible_tile_bounds(iso))
}

/// Pass 2: OBJ sprites (Cell Word 5). Each non-empty cell renders one
/// sprite from the OBJ sheet at the cell's screen position.
fn draw_obj_sprites(game: &GameLoop, mission: &MissionData, canvas: &mut Canvas<Window>) {
    let Some(or) = mission.obj_renderer.as_ref() else {
        return;
    };
    let iso = &mission.iso;
    let map = &mission.map;
    let (min_x, min_y, max_x, max_y) = visible_tile_rect(game, iso, map);

    let obj_pw = or.tile_pixel_width() as f32;
    let obj_ph = or.tile_pixel_height() as f32;
    // OBJ sprites are often taller than terrain tiles (e.g. 128x128 vs
    // 128x64). Offset upward so the bottom of the OBJ sprite sits on the
    // terrain surface.
    let y_offset_base = obj_ph - 64.0;

    let mut objs_drawn: u32 = 0;
    for ty in min_y..=max_y {
        for tx in min_x..=max_x {
            let Some(cell) = map.get_cell(tx, ty) else {
                continue;
            };
            // object_id == 0 or 255 means no object. The game uses 0xFF
            // as the "empty" sentinel (10079/10080 cells have
            // object_id=255 in a typical map).
            if cell.object_id == 0 || cell.object_id == 255 {
                continue;
            }
            let Some(obj_tex) = or.get_texture(cell.object_id as usize) else {
                continue;
            };
            let world_pos = iso.tile_to_screen(TilePos {
                x: tx as i32,
                y: ty as i32,
            });
            let screen_pos = game.camera.world_to_screen(world_pos);
            let draw_x = screen_pos.x;
            let draw_y = screen_pos.y - (y_offset_base * game.camera.zoom);
            let dst_w = (obj_pw * game.camera.zoom) as u32;
            let dst_h = (obj_ph * game.camera.zoom) as u32;
            let dst = Rect::new(draw_x as i32, draw_y as i32, dst_w, dst_h);
            if let Err(e) = canvas.copy(obj_tex, None, dst) {
                trace!(tx, ty, obj_idx = cell.object_id, error = %e, "OBJ sprite draw failed");
            }
            objs_drawn += 1;
        }
    }
    trace!(objs_drawn, "OBJ pass complete (Cell Word 5)");
}

/// Pass 3: Cell Word 2 overlay. Each cell carries up to three overlay
/// indices; index 0 is unused, 1-49 are TIL terrain decorations, 50+
/// are OBJ buildings, and 500+ are skull/debug markers (skipped).
///
/// On the first call, we also log a one-shot histogram of the overlay
/// distribution across the whole map so future tuners can see the
/// real cluster shape.
fn draw_word2_overlays(game: &GameLoop, mission: &MissionData, canvas: &mut Canvas<Window>) {
    static OVERLAY_HISTOGRAM_LOGGED: std::sync::Once = std::sync::Once::new();
    OVERLAY_HISTOGRAM_LOGGED.call_once(|| {
        let mut hist = std::collections::BTreeMap::<u16, u32>::new();
        for ty in 0..mission.map.height() {
            for tx in 0..mission.map.width() {
                if let Some(cell) = mission.map.get_cell(tx, ty) {
                    for idx in [cell.overlay_0, cell.overlay_1, cell.overlay_2] {
                        if idx != 0 {
                            *hist.entry(idx).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        let total: u32 = hist.values().sum();
        info!(
            unique_indices = hist.len(),
            total_overlays = total,
            map_w = mission.map.width(),
            map_h = mission.map.height(),
            "Word 2 overlay histogram (one-shot, all cells)"
        );
        for (idx, count) in &hist {
            info!(idx = *idx, count = *count, "  overlay-idx");
        }
    });

    let iso = &mission.iso;
    let tr = &mission.tile_renderer;
    let obj_renderer = mission.obj_renderer.as_ref();
    let map = &mission.map;
    let (min_x, min_y, max_x, max_y) = visible_tile_rect(game, iso, map);

    let dst_w = (iso.tile_width * game.camera.zoom) as u32;
    let dst_h = (iso.tile_height * game.camera.zoom) as u32;

    let mut overlays_drawn: u32 = 0;
    for ty in min_y..=max_y {
        for tx in min_x..=max_x {
            let Some(cell) = map.get_cell(tx, ty) else {
                continue;
            };

            // Word 4 elevation: shift the tile vertically based on the
            // average corner height. Each unit ≈ 2px of visual offset
            // (tuned to look reasonable at 128x64 tile size).
            let avg_elev = (cell.elevation_sw as f32
                + cell.elevation_se as f32
                + cell.elevation_ne as f32
                + cell.elevation_nw as f32)
                / 4.0;
            let elev_offset = avg_elev * 2.0 * game.camera.zoom;

            let world_pos = iso.tile_to_screen(TilePos {
                x: tx as i32,
                y: ty as i32,
            });
            let screen_pos = game.camera.world_to_screen(world_pos);
            let draw_x = screen_pos.x as i32;
            let draw_y = screen_pos.y as i32 - elev_offset as i32;
            let dst = Rect::new(draw_x, draw_y, dst_w, dst_h);

            for overlay_idx in [cell.overlay_0, cell.overlay_1, cell.overlay_2] {
                if overlay_idx == 0 || overlay_idx >= 500 {
                    continue;
                }
                if overlay_idx < 50 {
                    // TIL terrain decoration at tile size.
                    if let Some(tex) = tr.get_texture(overlay_idx as usize) {
                        canvas.copy(tex, None, dst).ok();
                        overlays_drawn += 1;
                    }
                } else if let Some(or) = obj_renderer {
                    // OBJ sprite (128x128), offset up to sit on terrain.
                    let obj_pw = or.tile_pixel_width() as f32;
                    let obj_ph = or.tile_pixel_height() as f32;
                    let y_off = (obj_ph - iso.tile_height) * game.camera.zoom;
                    let obj_dst_w = (obj_pw * game.camera.zoom) as u32;
                    let obj_dst_h = (obj_ph * game.camera.zoom) as u32;
                    if let Some(tex) = or.get_texture(overlay_idx as usize) {
                        let obj_dst =
                            Rect::new(draw_x, draw_y - y_off as i32, obj_dst_w, obj_dst_h);
                        canvas.copy(tex, None, obj_dst).ok();
                        overlays_drawn += 1;
                    }
                }
            }
        }
    }
    trace!(overlays_drawn, "Word 2 overlay pass complete");
}

/// Pass 4: walls (Cell Word 3, terrain_mods[0..12]). Each cell has 12
/// two-bit wall slots laid out around its diamond. Confirmed against
/// Wow.exe disasm (`FUN_0041d0f5` direction switch, `FUN_0041b26d`
/// bit-extraction). The exact direction-to-edge mapping is a working
/// hypothesis until `FUN_0041c81c(grid, dir)` is fully transcribed;
/// for now we render each non-zero slot as a colored line segment so
/// the data is visually inspectable.
///
/// Color codes: value 1 = green (low cover / fence), 2 = yellow
/// (mid wall), 3 = red (full wall / impassable).
fn draw_walls(game: &GameLoop, mission: &MissionData, canvas: &mut Canvas<Window>) {
    let iso = &mission.iso;
    let map = &mission.map;
    let (min_x, min_y, max_x, max_y) = visible_tile_rect(game, iso, map);

    // Tile diamond geometry. The renderer projects each cell to a
    // 128×64 staggered diamond. We compute the four cardinal corner
    // points (top/right/bottom/left) for each cell and derive edge
    // midpoints so we can place the 12 wall segments around them.
    let half_w = (iso.tile_width as f32 * 0.5) * game.camera.zoom;
    let half_h = (iso.tile_height as f32 * 0.5) * game.camera.zoom;

    let mut walls_drawn: u32 = 0;
    for ty in min_y..=max_y {
        for tx in min_x..=max_x {
            let Some(cell) = map.get_cell(tx, ty) else {
                continue;
            };
            // Quick-skip: if every wall slot is zero, no work to do.
            if !cell.terrain_mods.iter().any(|w| *w != 0) {
                continue;
            }

            let world = iso.tile_to_screen(TilePos {
                x: tx as i32,
                y: ty as i32,
            });
            let s = game.camera.world_to_screen(world);

            // Diamond corners in screen space. tile_to_screen returns
            // the diamond's *top-left bounding-box* origin, so the
            // four cardinal points are at predictable offsets.
            let north = (s.x + half_w, s.y);
            let east = (s.x + half_w * 2.0, s.y + half_h);
            let south = (s.x + half_w, s.y + half_h * 2.0);
            let west = (s.x, s.y + half_h);
            let center = (s.x + half_w, s.y + half_h);
            let ne_mid = ((north.0 + east.0) * 0.5, (north.1 + east.1) * 0.5);
            let se_mid = ((east.0 + south.0) * 0.5, (east.1 + south.1) * 0.5);
            let sw_mid = ((south.0 + west.0) * 0.5, (south.1 + west.1) * 0.5);
            let nw_mid = ((west.0 + north.0) * 0.5, (west.1 + north.1) * 0.5);

            // 12 segments around the cell. The exact assignment of
            // wall-slot index → segment is a working hypothesis: 8
            // perimeter halves clockwise from N, then 4 interior
            // spokes. Once we transcribe `FUN_0041c81c` fully we can
            // confirm or rotate this.
            let segments: [((f32, f32), (f32, f32)); 12] = [
                (north, ne_mid),
                (ne_mid, east),
                (east, se_mid),
                (se_mid, south),
                (south, sw_mid),
                (sw_mid, west),
                (west, nw_mid),
                (nw_mid, north),
                (north, center),
                (east, center),
                (south, center),
                (west, center),
            ];

            for (slot_idx, wall_value) in cell.terrain_mods.iter().enumerate() {
                if *wall_value == 0 {
                    continue;
                }
                let color = match *wall_value {
                    1 => Color::RGB(60, 200, 60),
                    2 => Color::RGB(220, 200, 40),
                    _ => Color::RGB(220, 60, 60),
                };
                let (a, b) = segments[slot_idx];
                canvas.set_draw_color(color);
                canvas
                    .draw_line((a.0 as i32, a.1 as i32), (b.0 as i32, b.1 as i32))
                    .ok();
                walls_drawn += 1;
            }
        }
    }
    trace!(walls_drawn, "Wall pass complete");
}

/// Pass 5: player mercs. Each living merc on the team is drawn at
/// their tile position using the current animation frame from
/// `SoldierAnims`. Mercs with no loaded animation fall back to a
/// colored square so the player still sees them.
fn draw_player_mercs(
    game: &GameLoop,
    mission: &MissionData,
    anims: &SoldierAnims,
    canvas: &mut Canvas<Window>,
) {
    let selected_id = match &game.phase_handler {
        PhaseHandler::Combat(ch) => ch.selected_unit_id,
        _ => None,
    };
    let iso = &mission.iso;

    for (merc_idx, merc) in game.game_state.team.iter().enumerate() {
        if !merc.is_alive() {
            continue;
        }
        let Some(pos) = merc.position else { continue };
        // Convert core's TilePos to iso_math's TilePos. The two are
        // structurally identical but distinct types; only iso_math's
        // has the projection methods.
        let world = iso.tile_to_screen(TilePos { x: pos.x, y: pos.y });
        let screen = game.camera.world_to_screen(world);
        let is_selected = selected_id == Some(merc.id);

        // Extract frame + mirror in one pass over the AnimController so
        // we don't have to re-look-up `anims.anims[merc_idx]` per-attribute
        // (and so the controller borrow doesn't outlive the loop body).
        let frame = anims.anims.get(merc_idx).and_then(|ctrl| {
            let fi = ctrl.current_frame_index() as usize;
            let mirror = ctrl.mirror_horizontal();
            anims
                .textures
                .get(fi)
                .and_then(|opt| opt.as_ref())
                .map(|tex| (tex, mirror))
        });

        match frame {
            Some((tex, mirror)) => {
                draw_textured_merc(canvas, tex, screen, game.camera.zoom, is_selected, mirror)
            }
            None => draw_placeholder_merc(canvas, screen, is_selected),
        }
    }
}

/// Soldier sprite dimensions. The frame is 128x138, offset upward by
/// the difference between sprite and tile height so the figure stands
/// on the terrain rather than floating.
const SOLDIER_SPRITE_W: f32 = 128.0;
const SOLDIER_SPRITE_H: f32 = 138.0;
const SOLDIER_HOVER: f32 = SOLDIER_SPRITE_H - 64.0; // tile height

/// Draw a single textured merc sprite at `screen`, optionally mirrored
/// along the vertical axis, with a yellow selection rectangle if
/// `is_selected`.
fn draw_textured_merc(
    canvas: &mut Canvas<Window>,
    tex: &sdl2::render::Texture,
    screen: ow_render::iso_math::ScreenPos,
    zoom: f32,
    is_selected: bool,
    mirror: bool,
) {
    let draw_w = (SOLDIER_SPRITE_W * zoom) as u32;
    let draw_h = (SOLDIER_SPRITE_H * zoom) as u32;
    let draw_x = screen.x;
    let draw_y = screen.y - (SOLDIER_HOVER * zoom);
    let dst = Rect::new(draw_x as i32, draw_y as i32, draw_w, draw_h);

    if mirror {
        canvas.copy_ex(tex, None, dst, 0.0, None, true, false).ok();
    } else {
        canvas.copy(tex, None, dst).ok();
    }

    if is_selected {
        canvas.set_draw_color(Color::RGB(255, 255, 0));
        canvas
            .draw_rect(Rect::new(
                draw_x as i32 - 1,
                draw_y as i32 - 1,
                draw_w + 2,
                draw_h + 2,
            ))
            .ok();
    }
}

/// Draw the untextured fallback: a 12x12 colored square (green for
/// normal mercs, yellow if selected) with a black outline. Used when
/// no COR/DAT animation is loaded for this merc.
fn draw_placeholder_merc(
    canvas: &mut Canvas<Window>,
    screen: ow_render::iso_math::ScreenPos,
    is_selected: bool,
) {
    let color = if is_selected {
        Color::RGB(255, 255, 0)
    } else {
        Color::RGB(0, 220, 0)
    };
    canvas.set_draw_color(color);
    canvas
        .fill_rect(Rect::new(screen.x as i32 - 6, screen.y as i32 - 6, 12, 12))
        .ok();
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas
        .draw_rect(Rect::new(screen.x as i32 - 6, screen.y as i32 - 6, 12, 12))
        .ok();
}

/// Pass 6: enemies. Each living enemy is drawn as a red diamond at its
/// tile position, with a 20-tile fog-of-war radius (any merc on the
/// team within 20 tiles makes the enemy visible).
fn draw_enemies(game: &GameLoop, mission: &MissionData, canvas: &mut Canvas<Window>) {
    const FOW_RANGE: i32 = 20;
    let iso = &mission.iso;
    for enemy in &mission.enemies {
        if enemy.current_hp == 0 {
            continue;
        }
        let Some(pos) = enemy.position else { continue };
        let seen = game.game_state.team.iter().any(|m| {
            m.is_alive()
                && m.position
                    .map(|mp| (mp.x - pos.x).abs() + (mp.y - pos.y).abs() <= FOW_RANGE)
                    .unwrap_or(false)
        });
        if !seen {
            continue;
        }
        let world = iso.tile_to_screen(TilePos { x: pos.x, y: pos.y });
        let screen = game.camera.world_to_screen(world);
        canvas.set_draw_color(Color::RGB(220, 30, 30));
        canvas
            .fill_rect(Rect::new(screen.x as i32 - 5, screen.y as i32 - 5, 10, 10))
            .ok();
        canvas.set_draw_color(Color::RGB(0, 0, 0));
        canvas
            .draw_rect(Rect::new(screen.x as i32 - 5, screen.y as i32 - 5, 10, 10))
            .ok();
    }
}

/// Pass 7: combat HUD (Combat phase only). Top banner with turn
/// indicator, bottom panel with selected-unit info and combat log.
fn draw_combat_hud(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
) {
    if !matches!(game.phase_handler, PhaseHandler::Combat(_)) {
        return;
    }
    let (w, h) = (game.window_width, game.window_height);
    let is_ai = matches!(&game.phase_handler, PhaseHandler::Combat(c) if c.ai_acting);

    // Top banner
    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
    let banner_w: u32 = 220;
    let banner_x = (w as i32 - banner_w as i32) / 2;
    canvas.fill_rect(Rect::new(banner_x, 4, banner_w, 28)).ok();
    canvas.set_blend_mode(sdl2::render::BlendMode::None);

    let (label, color) = if is_ai {
        ("ENEMY TURN", Color::RGB(220, 50, 50))
    } else {
        ("YOUR TURN", Color::RGB(50, 220, 50))
    };
    text.draw(canvas, tc, label, banner_x + 10, 8, color).ok();

    // Bottom panel
    let panel_height: u32 = 80;
    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 200));
    canvas
        .fill_rect(Rect::new(
            0,
            h as i32 - panel_height as i32,
            w,
            panel_height,
        ))
        .ok();
    canvas.set_blend_mode(sdl2::render::BlendMode::None);

    // Thin top border on the panel
    canvas.set_draw_color(Color::RGB(80, 80, 80));
    canvas
        .draw_line(
            sdl2::rect::Point::new(0, h as i32 - panel_height as i32),
            sdl2::rect::Point::new(w as i32, h as i32 - panel_height as i32),
        )
        .ok();

    // Selected-unit info (left side of panel)
    if let PhaseHandler::Combat(c) = &game.phase_handler {
        if let Some(sel_id) = c.selected_unit_id {
            if let Some(merc) = game.game_state.team.iter().find(|m| m.id == sel_id) {
                let info = format!(
                    "{} | HP: {}/{} | AP: {}/{} | Tab=Next  Click=Move  E=EndTurn",
                    merc.name, merc.current_hp, merc.max_hp, merc.current_ap, merc.base_aps,
                );
                text.draw(
                    canvas,
                    tc,
                    &info,
                    15,
                    h as i32 - (panel_height as i32) + 10,
                    Color::RGB(220, 220, 220),
                )
                .ok();
            }
        } else {
            text.draw(
                canvas,
                tc,
                "No unit selected | Tab=Next  Enter=NextPhase",
                15,
                h as i32 - (panel_height as i32) + 10,
                Color::RGB(180, 180, 180),
            )
            .ok();
        }
    }

    // Combat log (right side of panel). Tight 9px line spacing.
    let log_x = (w as i32 / 2) + 40;
    let log_start_y = h as i32 - (panel_height as i32) + 6;
    let line_h = 9i32;
    for (i, entry) in game.combat_log.iter().enumerate() {
        let y_pos = log_start_y + (i as i32) * line_h;
        let color = entry.kind.color();
        text.draw_small(canvas, tc, &entry.text, log_x, y_pos, color)
            .ok();
    }
}

/// Pass 8: deployment HUD (Deployment phase only). A bottom strip
/// showing how many mercs are placed.
fn draw_deployment_hud(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
) {
    if !matches!(game.phase_handler, PhaseHandler::Deployment { .. }) {
        return;
    }
    let (w, h) = (game.window_width, game.window_height);
    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 200));
    canvas.fill_rect(Rect::new(0, h as i32 - 40, w, 40)).ok();
    canvas.set_blend_mode(sdl2::render::BlendMode::None);

    let placed = game
        .game_state
        .team
        .iter()
        .filter(|m| m.position.is_some())
        .count();
    let total = game.game_state.team.len();
    let msg = format!(
        "DEPLOYMENT: Click map to place mercs ({placed}/{total} placed) | Enter=Start Combat"
    );
    text.draw(
        canvas,
        tc,
        &msg,
        15,
        h as i32 - 28,
        Color::RGB(220, 200, 100),
    )
    .ok();
}

/// Pass 9: minimap. Top-down (not isometric) overview in the
/// bottom-right corner. One pixel per tile, colored by terrain type.
/// Player mercs are overlaid as bright green dots.
fn draw_minimap(
    game: &GameLoop,
    mission: &MissionData,
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
) {
    let map = &mission.map;
    let (win_w, win_h) = (game.window_width, game.window_height);

    // 140x72 tiles at 1px each.
    let mm_w = map.width() as u32;
    let mm_h = map.height() as u32;
    let mm_x = win_w as i32 - mm_w as i32 - 15;
    let mm_y = win_h as i32 - mm_h as i32 - 15;

    canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
    canvas
        .fill_rect(Rect::new(mm_x - 3, mm_y - 3, mm_w + 6, mm_h + 6))
        .ok();
    canvas.set_blend_mode(sdl2::render::BlendMode::None);

    for ty in 0..map.height() {
        for tx in 0..map.width() {
            let Some(tile) = map.get_tile(tx, ty) else {
                continue;
            };
            let sid = tile.layer0() as u32;
            // Color the pixel by terrain index bucket. These thresholds
            // are tuned to look reasonable at 1px per tile; refine once
            // we have actual terrain classification data.
            let color = if sid == 0 {
                Color::RGB(25, 45, 20)
            } else if sid < 50 {
                Color::RGB(35, 65, 30)
            } else if sid < 150 {
                Color::RGB(45, 75, 35)
            } else if sid < 250 {
                Color::RGB(55, 85, 45)
            } else if sid < 350 {
                Color::RGB(70, 60, 40)
            } else {
                Color::RGB(40, 55, 75)
            };
            canvas.set_draw_color(color);
            canvas
                .draw_point(sdl2::rect::Point::new(mm_x + tx as i32, mm_y + ty as i32))
                .ok();
        }
    }

    // Player mercs as bright green dots.
    for merc in &game.game_state.team {
        let Some(pos) = merc.position else { continue };
        canvas.set_draw_color(Color::RGB(0, 255, 0));
        canvas
            .fill_rect(Rect::new(mm_x + pos.x, mm_y + pos.y, 3, 3))
            .ok();
    }

    // Border + label
    canvas.set_draw_color(Color::RGB(120, 120, 120));
    canvas
        .draw_rect(Rect::new(mm_x - 3, mm_y - 3, mm_w + 6, mm_h + 6))
        .ok();
    text.draw_small(
        canvas,
        tc,
        "MAP",
        mm_x,
        mm_y - 14,
        Color::RGB(180, 180, 180),
    )
    .ok();
}

// ---------------------------------------------------------------------------
// Extraction rendering
// ---------------------------------------------------------------------------

/// Render the extraction screen: map, extraction zone marker, player units.
///
/// `iso` is the iso config to use for projection — the mission's iso when
/// a mission is loaded, the default otherwise. The render_phase
/// dispatcher picks which one to pass.
fn render_extraction(game: &GameLoop, canvas: &mut Canvas<Window>, iso: &IsoConfig) {
    render_placeholder_grid(canvas, &game.camera, iso);

    // Extraction zone indicator at tile (0,0)
    let exit_tile = TilePos { x: 0, y: 0 };
    let world = iso.tile_to_screen(exit_tile);
    let screen = game.camera.world_to_screen(world);
    canvas.set_draw_color(Color::RGB(255, 200, 0));
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            screen.x as i32 - 16,
            screen.y as i32 - 16,
            32,
            32,
        ))
        .ok();

    // Player units
    for merc in &game.game_state.team {
        if !merc.is_alive() {
            continue;
        }
        if let Some(pos) = merc.position {
            let t = TilePos { x: pos.x, y: pos.y };
            let w = iso.tile_to_screen(t);
            let s = game.camera.world_to_screen(w);
            canvas.set_draw_color(Color::RGB(0, 200, 0));
            canvas
                .fill_rect(sdl2::rect::Rect::new(
                    s.x as i32 - 8,
                    s.y as i32 - 8,
                    16,
                    16,
                ))
                .ok();
        }
    }
}

// ---------------------------------------------------------------------------
// Debrief rendering
// ---------------------------------------------------------------------------

/// Render the debrief screen with the video phone showing the accountant.
///
/// The original game shows the accountant calling on a video phone to
/// deliver the post-mission financial report. PHONSPR.OBJ frames render
/// the phone scene background; ACCT.OBJ frames animate the accountant
/// character talking on screen. The financial text overlay sits to the
/// right of the phone.
///
/// Layout (at 1280x720):
///   Left half:  phone background + animated accountant sprite
///   Right half: battle results + financial report text
fn render_debrief(
    game: &GameLoop,
    mission: Option<&MissionData>,
    canvas: &mut Canvas<Window>,
    success: bool,
    anim_elapsed_ms: u32,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    acct_textures: &[Texture],
    phone_textures: &[Texture],
) {
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    canvas.set_draw_color(Color::RGB(15, 15, 25));
    canvas.clear();

    draw_debrief_phone(
        canvas,
        text,
        tc,
        h,
        w,
        acct_textures,
        phone_textures,
        anim_elapsed_ms,
    );
    draw_debrief_results(canvas, text, tc, game, mission, success, h, w);
}

/// Left half: the video phone with the animated accountant. Cycles
/// through the ACCT.OBJ frames based on `anim_elapsed_ms` (a clock
/// driven by the PhaseHandler's `anim_elapsed_ms` field). Falls back
/// to a placeholder rectangle if no phone sprites loaded.
#[allow(clippy::too_many_arguments)]
fn draw_debrief_phone(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    h: u32,
    w: u32,
    acct_textures: &[Texture],
    phone_textures: &[Texture],
    anim_elapsed_ms: u32,
) {
    // 200ms gives a natural feel for a "talking head" without
    // looking jittery.
    const ACCT_FRAME_PERIOD_MS: u32 = 200;
    let phone_area_w = (w * 2 / 5) as i32; // left 40% of screen

    if !phone_textures.is_empty() {
        // Phone scene background -- use frame 0 as the static backdrop.
        let phone_tex = &phone_textures[0];
        let query = phone_tex.query();
        // Scale the phone scene to fit nicely in the left panel,
        // maintaining aspect ratio.
        let scale = ((h as f32 - 60.0) / query.height as f32)
            .min((phone_area_w as f32 - 40.0) / query.width as f32);
        let draw_w = (query.width as f32 * scale) as u32;
        let draw_h = (query.height as f32 * scale) as u32;
        let draw_x = (phone_area_w as u32 / 2).saturating_sub(draw_w / 2) as i32 + 20;
        let draw_y = ((h / 2).saturating_sub(draw_h / 2)) as i32;
        canvas
            .copy(
                phone_tex,
                None,
                Some(Rect::new(draw_x, draw_y, draw_w, draw_h)),
            )
            .ok();

        // Accountant animation -- cycle through ACCT.OBJ frames. The
        // first frame is the base portrait; later frames are mouth
        // visemes. When VLS lip-sync data is wired up we will drive
        // the frame index from the viseme timeline instead.
        if !acct_textures.is_empty() {
            let frame_idx = if acct_textures.len() > 1 {
                (anim_elapsed_ms / ACCT_FRAME_PERIOD_MS) as usize % acct_textures.len()
            } else {
                0
            };
            let acct_tex = &acct_textures[frame_idx];
            let aq = acct_tex.query();
            // Scale accountant to fit inside the phone screen area
            // (~60% of phone dimensions, leaving room for the bezel).
            let acct_scale =
                (draw_h as f32 * 0.6 / aq.height as f32).min(draw_w as f32 * 0.6 / aq.width as f32);
            let acct_w = (aq.width as f32 * acct_scale) as u32;
            let acct_h = (aq.height as f32 * acct_scale) as u32;
            let acct_x = draw_x + (draw_w / 2) as i32 - (acct_w / 2) as i32;
            let acct_y = draw_y + (draw_h / 2) as i32 - (acct_h / 2) as i32;
            canvas
                .copy(
                    acct_tex,
                    None,
                    Some(Rect::new(acct_x, acct_y, acct_w, acct_h)),
                )
                .ok();
        }
    } else {
        // Fallback: draw a placeholder phone frame so the screen is
        // not empty when no sprites are loaded.
        let phone_rect = Rect::new(40, 80, (phone_area_w - 60) as u32, h - 160);
        canvas.set_draw_color(Color::RGB(30, 35, 50));
        canvas.fill_rect(phone_rect).ok();
        canvas.set_draw_color(Color::RGB(60, 70, 90));
        canvas.draw_rect(phone_rect).ok();
        text.draw(
            canvas,
            tc,
            "[ VIDEO PHONE ]",
            phone_rect.x() + 20,
            phone_rect.y() + 20,
            Color::RGB(100, 120, 160),
        )
        .ok();
        if acct_textures.is_empty() {
            text.draw(
                canvas,
                tc,
                "[ ACCOUNTANT ]",
                phone_rect.x() + 20,
                phone_rect.y() + 50,
                Color::RGB(80, 100, 140),
            )
            .ok();
        }
    }
}

/// Right half: mission results (survived / eliminated) and the
/// financial report (advance / bonus / hiring / medical / death
/// insurance / profit). When `mission` is None (developer hotkey
/// path) the kill/total counts show as zeros.
#[allow(clippy::too_many_arguments)]
fn draw_debrief_results(
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    game: &GameLoop,
    mission: Option<&MissionData>,
    success: bool,
    _h: u32,
    w: u32,
) {
    // Text sits to the right of the phone, starting at ~45% of width.
    let text_x = (w * 9 / 20) as i32;

    // Title banner
    let (title, title_color) = if success {
        ("MISSION COMPLETE", Color::RGB(80, 255, 80))
    } else {
        ("MISSION FAILED", Color::RGB(255, 80, 80))
    };
    text.draw_header(canvas, tc, title, text_x, 40, title_color)
        .ok();

    // -- Battle Results --
    let mut y = 100i32;
    let survived = game.game_state.team.iter().filter(|m| m.is_alive()).count();
    let total = game.game_state.team.len();
    let (killed, total_enemies) = mission
        .map(|m| {
            let k = m.enemies.iter().filter(|e| e.current_hp == 0).count();
            (k, m.enemies.len())
        })
        .unwrap_or((0, 0));

    text.draw(
        canvas,
        tc,
        "BATTLE RESULTS",
        text_x,
        y,
        Color::RGB(180, 180, 100),
    )
    .ok();
    y += 25;
    text.draw(
        canvas,
        tc,
        &format!("  Mercs survived:      {survived}/{total}"),
        text_x,
        y,
        Color::RGB(200, 200, 200),
    )
    .ok();
    y += 20;
    text.draw(
        canvas,
        tc,
        &format!("  Enemies eliminated:  {killed}/{total_enemies}"),
        text_x,
        y,
        Color::RGB(200, 200, 200),
    )
    .ok();
    y += 20;

    let kia = total - survived;
    let wia = game
        .game_state
        .team
        .iter()
        .filter(|m| m.is_alive() && m.current_hp < m.max_hp)
        .count();
    text.draw(
        canvas,
        tc,
        &format!("  KIA: {}  WIA: {}", kia, wia),
        text_x,
        y,
        if kia > 0 {
            Color::RGB(255, 100, 100)
        } else {
            Color::RGB(100, 200, 100)
        },
    )
    .ok();
    y += 35;

    // -- Financial Report --
    text.draw(
        canvas,
        tc,
        "FINANCIAL REPORT",
        text_x,
        y,
        Color::RGB(180, 180, 100),
    )
    .ok();
    y += 25;

    // TODO: get advance/bonus from the accepted contract, not hardcoded.
    let advance = 324_000i64;
    let bonus = if success { 200_000i64 } else { 0 };
    let hiring_costs = game.game_state.team.len() as i64 * 50_000;
    let medical = wia as i64 * 79_000;
    let death_insurance = kia as i64 * 89_000;
    let total_income = advance + bonus;
    let total_expenses = hiring_costs + medical + death_insurance;
    let profit = total_income - total_expenses;

    text.draw(
        canvas,
        tc,
        &format!("  Contract advance:    ${:>12}", advance),
        text_x,
        y,
        Color::RGB(150, 200, 150),
    )
    .ok();
    y += 18;
    if success {
        text.draw(
            canvas,
            tc,
            &format!("  Completion bonus:    ${:>12}", bonus),
            text_x,
            y,
            Color::RGB(150, 200, 150),
        )
        .ok();
        y += 18;
    }
    text.draw(
        canvas,
        tc,
        &format!("  Hiring costs:       -${:>12}", hiring_costs),
        text_x,
        y,
        Color::RGB(200, 150, 150),
    )
    .ok();
    y += 18;
    if medical > 0 {
        text.draw(
            canvas,
            tc,
            &format!("  Medical (WIA):      -${:>12}", medical),
            text_x,
            y,
            Color::RGB(200, 150, 150),
        )
        .ok();
        y += 18;
    }
    if death_insurance > 0 {
        text.draw(
            canvas,
            tc,
            &format!("  Death insurance:    -${:>12}", death_insurance),
            text_x,
            y,
            Color::RGB(255, 100, 100),
        )
        .ok();
        y += 18;
    }
    text.draw(
        canvas,
        tc,
        "  ─────────────────────────────",
        text_x,
        y,
        Color::RGB(100, 100, 100),
    )
    .ok();
    y += 18;
    let profit_color = if profit >= 0 {
        Color::RGB(100, 255, 100)
    } else {
        Color::RGB(255, 100, 100)
    };
    text.draw(
        canvas,
        tc,
        &format!("  NET PROFIT:          ${:>12}", profit),
        text_x,
        y,
        profit_color,
    )
    .ok();
    y += 25;
    text.draw(
        canvas,
        tc,
        &format!("  Current funds:       ${:>12}", game.game_state.funds),
        text_x,
        y,
        Color::RGB(200, 200, 200),
    )
    .ok();
    y += 35;

    text.draw(
        canvas,
        tc,
        "Press ENTER to return to office",
        text_x,
        y,
        Color::RGB(150, 150, 180),
    )
    .ok();
}

// ---------------------------------------------------------------------------
// Pause rendering
// ---------------------------------------------------------------------------

/// Render the pause overlay: dark fill + pause icon (two vertical bars).
fn render_pause(
    canvas: &mut Canvas<Window>,
    _text: &TextRenderer,
    _tc: &TextureCreator<WindowContext>,
) {
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    // Dark overlay
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
    canvas.fill_rect(sdl2::rect::Rect::new(0, 0, w, h)).ok();

    // Pause icon: two vertical bars
    let bar_w = 20u32;
    let bar_h = 60u32;
    let gap = 15i32;
    let cx = (w / 2) as i32;
    let cy = (h / 2) as i32;

    canvas.set_draw_color(Color::RGB(200, 200, 200));
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            cx - gap - bar_w as i32,
            cy - (bar_h / 2) as i32,
            bar_w,
            bar_h,
        ))
        .ok();
    canvas
        .fill_rect(sdl2::rect::Rect::new(
            cx + gap,
            cy - (bar_h / 2) as i32,
            bar_w,
            bar_h,
        ))
        .ok();
}

// ---------------------------------------------------------------------------
// Placeholder grid renderer
// ---------------------------------------------------------------------------

/// Draw a simple isometric diamond grid as a stand-in for the real tile map.
///
/// Renders a 20x20 grid of diamond outlines using the camera and iso config.
/// This lets us test camera scrolling, zoom, and tile picking before wiring up
/// loaded map data through TileMapRenderer.
fn render_placeholder_grid(canvas: &mut Canvas<Window>, camera: &Camera, iso: &IsoConfig) {
    let grid_size = 20;
    canvas.set_draw_color(Color::RGB(60, 60, 60));

    for ty in 0..grid_size {
        for tx in 0..grid_size {
            let tile = TilePos { x: tx, y: ty };
            let world = iso.tile_to_screen(tile);
            let screen = camera.world_to_screen(world);

            let hw = (iso.tile_width / 2.0) * camera.zoom;
            let hh = (iso.tile_height / 2.0) * camera.zoom;
            let cx = screen.x;
            let cy = screen.y;

            // Diamond outline: top -> right -> bottom -> left -> top
            let top = sdl2::rect::Point::new(cx as i32, (cy - hh) as i32);
            let right = sdl2::rect::Point::new((cx + hw) as i32, cy as i32);
            let bottom = sdl2::rect::Point::new(cx as i32, (cy + hh) as i32);
            let left = sdl2::rect::Point::new((cx - hw) as i32, cy as i32);

            canvas.draw_line(top, right).ok();
            canvas.draw_line(right, bottom).ok();
            canvas.draw_line(bottom, left).ok();
            canvas.draw_line(left, top).ok();
        }
    }
}
