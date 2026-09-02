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

use super::{GameLoop, PhaseHandler, WINDOW_HEIGHT, WINDOW_WIDTH};

// ===========================================================================
// Phase rendering
// ===========================================================================

/// Render the current phase to the canvas.
///
/// Most phases render a colored background (set in the main loop) with
/// geometric placeholders. Combat renders an isometric grid plus unit markers.
pub(crate) fn render_phase(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    office_bg: &Option<Texture>,
    tile_renderer: &Option<ow_render::tile_renderer::TileMapRenderer>,
    obj_renderer: &Option<ow_render::tile_renderer::TileMapRenderer>,
    loaded_map: &Option<ow_data::map_loader::GameMap>,
    mission_iso: &Option<IsoConfig>,
    soldier_texture: &Option<Texture>,
    acct_textures: &[Texture],
    phone_textures: &[Texture],
    soldier_textures: &[Option<Texture>],
    soldier_anims: &[ow_render::anim_controller::AnimController],
) {
    match &game.phase_handler {
        PhaseHandler::Office { sub_phase } => {
            render_office(game, canvas, *sub_phase, text, tc, ruleset, office_bg)
        }
        PhaseHandler::Travel { elapsed_ms } => render_travel(canvas, *elapsed_ms, text, tc),
        PhaseHandler::Deployment { .. } => {
            render_mission_map(
                game,
                canvas,
                tile_renderer,
                obj_renderer,
                loaded_map,
                mission_iso,
                text,
                tc,
                &game.enemies,
                soldier_texture,
                soldier_textures,
                soldier_anims,
            );
        }
        PhaseHandler::Combat(_) => {
            render_mission_map(
                game,
                canvas,
                tile_renderer,
                obj_renderer,
                loaded_map,
                mission_iso,
                text,
                tc,
                &game.enemies,
                soldier_texture,
                soldier_textures,
                soldier_anims,
            );
        }
        PhaseHandler::Extraction => render_extraction(game, canvas),
        PhaseHandler::Debrief {
            success,
            anim_elapsed_ms,
        } => render_debrief(
            game,
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
/// Placeholder: colored background per sub-phase, tab indicators at top,
/// team size / funds indicators at bottom.
fn render_office(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    active_sub: OfficePhase,
    text: &TextRenderer,
    tc: &TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    office_bg: &Option<Texture>,
) {
    let (w, h) = canvas
        .output_size()
        .unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));

    // -- For the Overview tab, render the original OFFICE.PCX background --
    // This is the iconic desk scene the player sees when the game starts.
    // Other sub-phases overlay their own content on a dark background.
    match active_sub {
        OfficePhase::Overview => {
            if let Some(bg_tex) = office_bg {
                // Scale the 640x480 office background to fill the window.
                canvas.copy(bg_tex, None, Some(Rect::new(0, 0, w, h))).ok();
            }

            // Overlay help text on the office background.
            // Semi-transparent bar at bottom for readability.
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

            // DEBUG: Draw labeled hotspot overlays so we can see where the
            // click regions are and fix them. Remove this once hotspots are correct.
            //
            // IMPORTANT: Use game.window_width/height (logical pixels from SDL2
            // mouse events), NOT canvas.output_size() (physical pixels). On high-DPI
            // displays these differ by the scale factor, causing misalignment.
            // Print dimensions to stdout for DPI debugging.
            // DEBUG: Hotspot overlays using window logical size (same as click handler).
            let ww = game.window_width as f32;
            let wh = game.window_height as f32;
            canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
            let hotspots: &[((i32, i32, i32, i32), &str, Color)] = &[
                (
                    (400, 340, 520, 430),
                    "HIRE (Phone)",
                    Color::RGBA(255, 50, 50, 100),
                ),
                (
                    (480, 230, 560, 310),
                    "CONTRACTS (Fax)",
                    Color::RGBA(50, 50, 255, 100),
                ),
                (
                    (490, 50, 620, 190),
                    "INTEL (Map)",
                    Color::RGBA(255, 255, 50, 100),
                ),
                (
                    (70, 170, 130, 370),
                    "FILES (Cabinet)",
                    Color::RGBA(50, 255, 50, 100),
                ),
                (
                    (100, 360, 220, 430),
                    "EQUIP (Mags)",
                    Color::RGBA(50, 255, 50, 100),
                ),
                (
                    (230, 330, 310, 380),
                    "TRAIN (Calc)",
                    Color::RGBA(255, 50, 255, 100),
                ),
                (
                    (240, 40, 370, 250),
                    "MISSION (Door)",
                    Color::RGBA(255, 150, 0, 100),
                ),
            ];
            for &((x1, y1, x2, y2), label, color) in hotspots {
                // Scale 640x480 → window size. Uses same math as check_hit (inverse).
                let sx1 = (x1 as f32 * ww / 640.0) as i32;
                let sy1 = (y1 as f32 * wh / 480.0) as i32;
                let sx2 = (x2 as f32 * ww / 640.0) as i32;
                let sy2 = (y2 as f32 * wh / 480.0) as i32;
                canvas.set_draw_color(color);
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
                    label,
                    sx1 + 4,
                    sy1 + 4,
                    Color::RGB(255, 255, 255),
                )
                .ok();
            }
            canvas.set_blend_mode(sdl2::render::BlendMode::None);

            return; // Overview renders the background only — no tab bar.
        }
        _ => {}
    }

    // -- For non-Overview tabs, dark background with tab bar --
    // -- Status bar at bottom: shows funds and team size --
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

    // -- Sub-phase tab bar along the top --
    let tab_names = ["1:Hire", "2:Equip", "3:Intel", "4:Contracts", "5:Train"];
    let sub_phases = [
        OfficePhase::HireMercs,
        OfficePhase::Equipment,
        OfficePhase::Intel,
        OfficePhase::Contracts,
        OfficePhase::Training,
    ];

    // Tab background
    canvas.set_draw_color(Color::RGB(15, 15, 25));
    canvas.fill_rect(Rect::new(0, 0, w, 35)).ok();

    // Back to office button
    text.draw_small(
        canvas,
        tc,
        "[ESC] Office",
        10,
        10,
        Color::RGB(140, 140, 160),
    )
    .ok();

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

    // -- Main content area depends on active sub-phase --
    let content_y = 50;
    let content_h = h as i32 - 50 - 55;

    match active_sub {
        OfficePhase::Overview => {
            // Handled above with the background image.
        }
        OfficePhase::HireMercs => {
            text.draw_header(
                canvas,
                tc,
                "Mercenary Roster",
                20,
                content_y,
                Color::RGB(220, 200, 100),
            )
            .ok();

            // List available mercs from the ruleset, scrollable
            let mut y = content_y + 35;
            let mut count = 0;
            let mut sorted_mercs: Vec<_> = ruleset.mercs.values().collect();
            sorted_mercs.sort_by(|a, b| b.rating.cmp(&a.rating)); // best first

            for merc in sorted_mercs.iter().take(25) {
                // Check if already hired
                let hired = game.game_state.team.iter().any(|m| m.name == merc.name);
                let status_color = if hired {
                    Color::RGB(100, 200, 100) // green = on your team
                } else if merc.avail == 1 {
                    Color::RGB(200, 200, 200) // white = available
                } else {
                    Color::RGB(100, 100, 100) // gray = unavailable
                };

                let status_tag = if hired {
                    "[HIRED]"
                } else if merc.avail == 0 {
                    "[N/A]"
                } else {
                    ""
                };
                let line = format!(
                    "{:<25} RAT:{:>3}  EXP:{:>3}  WSK:{:>3}  AGL:{:>3}  Hire:${:>7}  {}",
                    merc.name, merc.rating, merc.exp, merc.wsk, merc.agl, merc.fee_hire, status_tag
                );
                text.draw_small(canvas, tc, &line, 20, y, status_color).ok();
                y += 16;
                count += 1;
                if y > (content_y + content_h - 20) {
                    break;
                }
            }

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
        OfficePhase::Equipment => {
            text.draw_header(
                canvas,
                tc,
                "Equipment Catalog — Click weapon to lease",
                20,
                content_y,
                Color::RGB(220, 200, 100),
            )
            .ok();

            // Left pane: weapon list (clickable)
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
            sorted_weapons.sort_by_key(|w| format!("{:?}", w.weapon_type));
            // Collect names of all currently leased weapons for highlighting.
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
                    Color::RGB(100, 200, 100) // green = already leased
                } else if !affordable {
                    Color::RGB(120, 80, 80) // dim red = can't afford
                } else {
                    Color::RGB(200, 200, 200) // white = available
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

            // Right pane: your team with their equipment
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
            } else {
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
                        Color::RGB(200, 100, 100) // Red = unarmed
                    } else {
                        Color::RGB(100, 200, 100) // Green = armed
                    };
                    text.draw_small(canvas, tc, &line, team_x, ty, color).ok();
                    ty += 16;
                }
            }

            // Equipment instructions
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
                content_y + content_h,
                Color::RGB(140, 140, 100),
            )
            .ok();
        }
        OfficePhase::Contracts => {
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

            // Show which contract is currently accepted, if any.
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

            // Show mission contracts from the ruleset.
            // Accepted contract shown in green, others in white.
            let mut mission_ids: Vec<_> = ruleset.missions.keys().collect();
            mission_ids.sort();
            for mid in &mission_ids {
                if let Some(mission) = ruleset.missions.get(*mid) {
                    let is_accepted = accepted_id.as_deref() == Some(mid.as_str());
                    let color = if is_accepted {
                        Color::RGB(100, 255, 100) // green = accepted
                    } else {
                        Color::RGB(200, 200, 200) // white = available
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
        _ => {
            // Intel, Training — placeholder for now
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
    }

    // -- Help text --
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

/// Render the deployment screen: placeholder grid + placed merc markers.
/// Render the mission map using real tile sprites if loaded, or the placeholder grid.
/// Used by both Deployment and Combat phases.
fn render_mission_map(
    game: &GameLoop,
    canvas: &mut Canvas<Window>,
    tile_renderer: &Option<ow_render::tile_renderer::TileMapRenderer>,
    obj_renderer: &Option<ow_render::tile_renderer::TileMapRenderer>,
    loaded_map: &Option<ow_data::map_loader::GameMap>,
    mission_iso: &Option<IsoConfig>,
    _text: &TextRenderer,
    _tc: &TextureCreator<WindowContext>,
    enemies: &[ow_core::mission_setup::EnemyUnit],
    soldier_texture: &Option<Texture>,
    soldier_textures: &[Option<Texture>],
    soldier_anims: &[ow_render::anim_controller::AnimController],
) {
    // If we have real tile data, render the actual map. Otherwise fall back
    // to the wireframe placeholder grid.
    if let (Some(tr), Some(map), Some(iso)) = (tile_renderer, loaded_map, mission_iso) {
        tr.render_map(canvas, map, &game.camera, iso);

        // === OBJ sprite pass (Cell Word 5) ===
        // Objects (buildings, trees, walls, fences) are stored in Cell Word 5
        // with an 8-bit object_id (0=none, 1-255=OBJ sprite index).
        // This replaces the old hack of subtracting 100 from overlay indices.
        if let Some(or) = obj_renderer {
            let (min_x, min_y, max_x, max_y) = game.camera.visible_tile_bounds(iso);
            let min_x = min_x.max(0) as usize;
            let min_y = min_y.max(0) as usize;
            let max_x = (max_x as usize).min(map.width().saturating_sub(1));
            let max_y = (max_y as usize).min(map.height().saturating_sub(1));

            let obj_pw = or.tile_pixel_width() as f32;
            let obj_ph = or.tile_pixel_height() as f32;
            // OBJ sprites are often taller than terrain tiles (e.g. 128x128 vs 128x64).
            // Offset upward so the bottom of the OBJ sprite sits on the terrain surface.
            let tile_h = 64.0_f32; // staggered grid tile height
            let y_offset_base = obj_ph - tile_h;

            let mut objs_drawn: u32 = 0;

            for ty in min_y..=max_y {
                for tx in min_x..=max_x {
                    // Access the full MapCell to read object_id from Word 5.
                    let cell = match map.get_cell(tx, ty) {
                        Some(c) => c,
                        None => continue,
                    };

                    // object_id == 0 or 255 means no object in this cell.
                    // The game uses 0xFF as the "empty" sentinel (10079/10080 cells
                    // have object_id=255 in a typical map).
                    if cell.object_id == 0 || cell.object_id == 255 {
                        continue;
                    }

                    let obj_idx = cell.object_id as usize;
                    let obj_tex = match or.get_texture(obj_idx) {
                        Some(t) => t,
                        None => continue,
                    };

                    // Position the OBJ sprite at this cell's screen location.
                    let world_pos = iso.tile_to_screen(TilePos {
                        x: tx as i32,
                        y: ty as i32,
                    });
                    let screen_pos = game.camera.world_to_screen(world_pos);

                    // Draw at top-left of tile position, offset up by the
                    // height difference so OBJ sprites sit on the terrain.
                    let draw_x = screen_pos.x;
                    let draw_y = screen_pos.y - (y_offset_base * game.camera.zoom);

                    let dst_w = (obj_pw * game.camera.zoom) as u32;
                    let dst_h = (obj_ph * game.camera.zoom) as u32;

                    let dst = Rect::new(draw_x as i32, draw_y as i32, dst_w, dst_h);
                    if let Err(e) = canvas.copy(obj_tex, None, dst) {
                        trace!(tx, ty, obj_idx, error = %e, "OBJ sprite draw failed");
                    }
                    objs_drawn += 1;
                }
            }

            trace!(objs_drawn, "OBJ pass complete (Cell Word 5)");
        }

        // === Cell Word 2 overlay pass ===
        // Re-enabled 2026-05-02 to surface buildings / walls / fences that
        // sat invisible while this block was `if false`. Each cell carries
        // up to three overlay indices in MAP Word 2; previous sessions
        // hypothesised the split was `<50` = TIL terrain decoration,
        // `>=50` = OBJ buildings — that threshold has no exe-disassembly
        // basis. We emit a one-shot histogram of the overlay-index
        // distribution across the whole map on the first frame so we can
        // see the actual cluster shape and tune the threshold against
        // ground truth instead of guesses.
        {
            static OVERLAY_HISTOGRAM_LOGGED: std::sync::Once = std::sync::Once::new();
            OVERLAY_HISTOGRAM_LOGGED.call_once(|| {
                let mut hist = std::collections::BTreeMap::<u16, u32>::new();
                for ty in 0..map.height() {
                    for tx in 0..map.width() {
                        if let Some(cell) = map.get_cell(tx, ty) {
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
                    map_w = map.width(),
                    map_h = map.height(),
                    "Word 2 overlay histogram (one-shot, all cells)"
                );
                for (idx, count) in &hist {
                    info!(idx = *idx, count = *count, "  overlay-idx");
                }
            });

            let (min_x, min_y, max_x, max_y) = game.camera.visible_tile_bounds(iso);
            let min_x = min_x.max(0) as usize;
            let min_y = min_y.max(0) as usize;
            let max_x = (max_x as usize).min(map.width().saturating_sub(1));
            let max_y = (max_y as usize).min(map.height().saturating_sub(1));

            let dst_w = (iso.tile_width * game.camera.zoom) as u32;
            let dst_h = (iso.tile_height * game.camera.zoom) as u32;
            let mut overlays_drawn: u32 = 0;

            for ty in min_y..=max_y {
                for tx in min_x..=max_x {
                    let cell = match map.get_cell(tx, ty) {
                        Some(c) => c,
                        None => continue,
                    };

                    // Word 4 elevation: offset the tile vertically based on the
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
                    // Elevation shifts tiles UP (higher elevation = drawn higher on screen).
                    let draw_y = screen_pos.y as i32 - elev_offset as i32;

                    let dst = Rect::new(draw_x, draw_y, dst_w, dst_h);

                    // Draw Word 2 overlays — these are OBJECTS (buildings, fences,
                    // trees) from the OBJ sprite sheet, not terrain from TIL.
                    // Low indices (1-50) are terrain decorations from TIL.
                    // Higher indices (100+) are building sprites from OBJ.
                    // Skip marker sprites >= 500 (skulls/debug).
                    if let Some(or) = obj_renderer {
                        let obj_pw = or.tile_pixel_width() as f32;
                        let obj_ph = or.tile_pixel_height() as f32;
                        let y_off = (obj_ph - iso.tile_height) * game.camera.zoom;
                        let obj_dst_w = (obj_pw * game.camera.zoom) as u32;
                        let obj_dst_h = (obj_ph * game.camera.zoom) as u32;

                        for overlay_idx in [cell.overlay_0, cell.overlay_1, cell.overlay_2] {
                            if overlay_idx == 0 || overlay_idx >= 500 {
                                continue;
                            }
                            if overlay_idx < 50 {
                                // Low indices: terrain decoration from TIL at tile size.
                                if let Some(overlay_tex) = tr.get_texture(overlay_idx as usize) {
                                    canvas.copy(overlay_tex, None, dst).ok();
                                    overlays_drawn += 1;
                                }
                            } else {
                                // High indices: building/object from OBJ sprite sheet.
                                // OBJ sprites are 128x128, offset up to sit on terrain.
                                if let Some(obj_tex) = or.get_texture(overlay_idx as usize) {
                                    let obj_dst = Rect::new(
                                        draw_x,
                                        draw_y - y_off as i32,
                                        obj_dst_w,
                                        obj_dst_h,
                                    );
                                    canvas.copy(obj_tex, None, obj_dst).ok();
                                    overlays_drawn += 1;
                                }
                            }
                        }
                    }
                }
            }

            trace!(overlays_drawn, "Word 2 overlay pass complete");
        }

        // === Wall pass (Cell Word 3, terrain_mods[0..12]) ===
        //
        // Each MAP cell carries 12 two-bit wall slots in Word 3, parsed into
        // `MapCell.terrain_mods`. Confirmed against Wow.exe disasm
        // (`FUN_0041d0f5` switch on direction 1-12, `FUN_0041b26d` extracts
        // the bits at `>>(8 + j*2) & 3`). Each cell has 4 sub-grids (NW/NE/
        // SE/SW quadrants) and the 12 walls cover both perimeter and
        // interior boundaries. The exact direction-to-edge mapping isn't
        // fully nailed down — `FUN_0041c81c(grid, dir)` in the disasm is
        // the source of truth and we can refine the geometry once we have
        // it transcribed. For now we render each non-zero wall as a colored
        // line segment along an approximate edge so the data becomes
        // visually inspectable. Color codes:
        //   value 1 → green (low cover / fence)
        //   value 2 → yellow (mid wall)
        //   value 3 → red (full wall / impassable)
        // Map ground truth: `re/ghidra/projects/analysis/decomp/wallhunt_FUN_0041d0f5.c`
        // and `wallstruct_FUN_0041b26d.c`.
        {
            let mut walls_drawn: u32 = 0;
            let (min_x, min_y, max_x, max_y) = game.camera.visible_tile_bounds(iso);
            let min_x = min_x.max(0) as usize;
            let min_y = min_y.max(0) as usize;
            let max_x = (max_x as usize).min(map.width().saturating_sub(1));
            let max_y = (max_y as usize).min(map.height().saturating_sub(1));

            // Tile diamond geometry. The renderer projects each cell to a
            // 128×64 staggered diamond. We compute the four cardinal corner
            // points (top/right/bottom/left) for each cell and derive edge
            // midpoints so we can place the 12 wall segments around them.
            let half_w = (iso.tile_width as f32 * 0.5) * game.camera.zoom;
            let half_h = (iso.tile_height as f32 * 0.5) * game.camera.zoom;

            for ty in min_y..=max_y {
                for tx in min_x..=max_x {
                    let cell = match map.get_cell(tx, ty) {
                        Some(c) => c,
                        None => continue,
                    };
                    // Quick-skip: if every wall slot is zero, no work to do.
                    let any = cell.terrain_mods.iter().any(|w| *w != 0);
                    if !any {
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

                    // Edge midpoints (between cardinal corners).
                    let mid = |a: (f32, f32), b: (f32, f32)| -> (f32, f32) {
                        ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
                    };
                    let ne_mid = mid(north, east);
                    let se_mid = mid(east, south);
                    let sw_mid = mid(south, west);
                    let nw_mid = mid(west, north);

                    // 12 segments around the cell. The exact assignment of
                    // wall-slot index → segment is a working hypothesis: 8
                    // perimeter halves clockwise from N, then 4 interior
                    // spokes. Once we transcribe `FUN_0041c81c` fully we can
                    // confirm or rotate this.
                    let segments: [((f32, f32), (f32, f32)); 12] = [
                        (north, ne_mid), // 1 — NE-edge upper half
                        (ne_mid, east),  // 2 — NE-edge lower half
                        (east, se_mid),  // 3 — SE-edge upper half
                        (se_mid, south), // 4 — SE-edge lower half
                        (south, sw_mid), // 5 — SW-edge upper half
                        (sw_mid, west),  // 6 — SW-edge lower half
                        (west, nw_mid),  // 7 — NW-edge upper half
                        (nw_mid, north), // 8 — NW-edge lower half
                        (north, center), //  9 — interior spoke N
                        (east, center),  // 10 — interior spoke E
                        (south, center), // 11 — interior spoke S
                        (west, center),  // 12 — interior spoke W
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
    } else {
        render_placeholder_grid(canvas, &game.camera, &game.iso_config);
    }

    // Draw placed mercs as colored diamonds on the map.
    let iso = mission_iso.as_ref().unwrap_or(&game.iso_config);

    // Get selected unit ID if in combat
    let selected_id = match &game.phase_handler {
        PhaseHandler::Combat(ch) => ch.selected_unit_id,
        _ => None,
    };

    for merc in &game.game_state.team {
        if !merc.is_alive() {
            continue;
        }
        if let Some(pos) = merc.position {
            let iso_tile = TilePos { x: pos.x, y: pos.y };
            let world = iso.tile_to_screen(iso_tile);
            let screen = game.camera.world_to_screen(world);

            let is_selected = selected_id == Some(merc.id);

            // Try animated frame first (from COR/DAT system), then fall
            // back to the single static texture if animations aren't loaded.
            let merc_idx = game.game_state.team.iter().position(|m| m.id == merc.id);
            let anim_frame_tex = merc_idx
                .and_then(|idx| soldier_anims.get(idx))
                .map(|ctrl| ctrl.current_frame_index() as usize)
                .and_then(|fi| soldier_textures.get(fi))
                .and_then(|opt| opt.as_ref());

            let tex_to_draw = anim_frame_tex.or(soldier_texture.as_ref());

            if let Some(sld_tex) = tex_to_draw {
                // Draw the sprite frame at tile position. Frames are 128x138
                // with the soldier figure inside an isometric footprint.
                let sprite_w = 128.0;
                let sprite_h = 138.0;
                let draw_w = (sprite_w * game.camera.zoom) as u32;
                let draw_h = (sprite_h * game.camera.zoom) as u32;
                let draw_x = screen.x;
                let draw_y = screen.y - ((sprite_h - 64.0) * game.camera.zoom);

                let dst = Rect::new(draw_x as i32, draw_y as i32, draw_w, draw_h);

                // Check if the animation requires horizontal mirroring.
                let mirror = merc_idx
                    .and_then(|idx| soldier_anims.get(idx))
                    .map(|ctrl| ctrl.mirror_horizontal())
                    .unwrap_or(false);

                if mirror {
                    canvas
                        .copy_ex(sld_tex, None, dst, 0.0, None, true, false)
                        .ok();
                } else {
                    canvas.copy(sld_tex, None, dst).ok();
                }

                // Selection highlight
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
            } else {
                // Fallback: colored squares when no soldier sprite is loaded
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
        }
    }

    // Draw enemy units — fog of war hides enemies beyond 20 tiles from all mercs.
    let fow_range = 20i32;
    for enemy in enemies {
        if enemy.current_hp == 0 {
            continue;
        }
        if let Some(pos) = enemy.position {
            let seen = game.game_state.team.iter().any(|m| {
                m.is_alive()
                    && m.position
                        .map(|mp| (mp.x - pos.x).abs() + (mp.y - pos.y).abs() <= fow_range)
                        .unwrap_or(false)
            });
            if !seen {
                continue;
            }
            let iso_tile = TilePos { x: pos.x, y: pos.y };
            let world = iso.tile_to_screen(iso_tile);
            let screen = game.camera.world_to_screen(world);

            // Red for enemies
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

    // -- Combat HUD: bottom panel + combat log + turn indicator --
    if matches!(game.phase_handler, PhaseHandler::Combat(_)) {
        let (w, h) = (game.window_width, game.window_height);

        // ---- Turn indicator at top of screen ----
        let is_ai = match &game.phase_handler {
            PhaseHandler::Combat(c) => c.ai_acting,
            _ => false,
        };

        // Semi-transparent banner at top center
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
        let banner_w: u32 = 220;
        let banner_x = (w as i32 - banner_w as i32) / 2;
        canvas.fill_rect(Rect::new(banner_x, 4, banner_w, 28)).ok();
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        if is_ai {
            _text
                .draw(
                    canvas,
                    _tc,
                    "ENEMY TURN",
                    banner_x + 10,
                    8,
                    Color::RGB(220, 50, 50),
                )
                .ok();
        } else {
            _text
                .draw(
                    canvas,
                    _tc,
                    "YOUR TURN",
                    banner_x + 10,
                    8,
                    Color::RGB(50, 220, 50),
                )
                .ok();
        }

        // ---- Dark panel at bottom ----
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

        // ---- Selected unit info (left side) ----
        if let Some(sel_id) = selected_id {
            if let Some(merc) = game.game_state.team.iter().find(|m| m.id == sel_id) {
                let info = format!(
                    "{} | HP: {}/{} | AP: {}/{} | Tab=Next  Click=Move  E=EndTurn",
                    merc.name, merc.current_hp, merc.max_hp, merc.current_ap, merc.base_aps,
                );
                _text
                    .draw(
                        canvas,
                        _tc,
                        &info,
                        15,
                        h as i32 - (panel_height as i32) + 10,
                        Color::RGB(220, 220, 220),
                    )
                    .ok();
            }
        } else {
            _text
                .draw(
                    canvas,
                    _tc,
                    "No unit selected | Tab=Next  Enter=NextPhase",
                    15,
                    h as i32 - (panel_height as i32) + 10,
                    Color::RGB(180, 180, 180),
                )
                .ok();
        }

        // ---- Combat log (right side of bottom panel) ----
        // Draw up to COMBAT_LOG_MAX entries, small text, right-aligned area.
        let log_x = (w as i32 / 2) + 40; // Right half of the panel
        let log_start_y = h as i32 - (panel_height as i32) + 6;
        let line_h = 9i32; // Tight spacing for small text

        for (i, entry) in game.combat_log.iter().enumerate() {
            let y_pos = log_start_y + (i as i32) * line_h;
            let color = entry.kind.color();
            _text
                .draw_small(canvas, _tc, &entry.text, log_x, y_pos, color)
                .ok();
        }
    }

    // -- Deployment phase HUD --
    if matches!(game.phase_handler, PhaseHandler::Deployment { .. }) {
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
        _text
            .draw(
                canvas,
                _tc,
                &msg,
                15,
                h as i32 - 28,
                Color::RGB(220, 200, 100),
            )
            .ok();
    }

    // -- Minimap: overview in the bottom-right corner --
    // The minimap shows the map in top-down grid view (not isometric) since
    // an isometric minimap would be diamond-shaped and harder to read.
    // Each tile = 1 pixel, colored by terrain type.
    if let Some(map) = loaded_map {
        let (win_w, win_h) = (game.window_width, game.window_height);

        // Scale minimap to fit — 140x72 tiles at 1px each.
        let mm_w = map.width() as u32;
        let mm_h = map.height() as u32;
        let mm_x = win_w as i32 - mm_w as i32 - 15;
        let mm_y = win_h as i32 - mm_h as i32 - 15;

        // Semi-transparent background
        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(0, 0, 0, 180));
        canvas
            .fill_rect(Rect::new(mm_x - 3, mm_y - 3, mm_w + 6, mm_h + 6))
            .ok();
        canvas.set_blend_mode(sdl2::render::BlendMode::None);

        // Draw each tile as a colored pixel based on its sprite index.
        for ty in 0..map.height() {
            for tx in 0..map.width() {
                if let Some(tile) = map.get_tile(tx, ty) {
                    let sid = tile.layer0() as u32;
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
        }

        // Player mercs as bright green dots
        for merc in &game.game_state.team {
            if let Some(pos) = merc.position {
                canvas.set_draw_color(Color::RGB(0, 255, 0));
                canvas
                    .fill_rect(Rect::new(mm_x + pos.x, mm_y + pos.y, 3, 3))
                    .ok();
            }
        }

        // Border
        canvas.set_draw_color(Color::RGB(120, 120, 120));
        canvas
            .draw_rect(Rect::new(mm_x - 3, mm_y - 3, mm_w + 6, mm_h + 6))
            .ok();

        // "MINIMAP" label
        _text
            .draw_small(
                canvas,
                _tc,
                "MAP",
                mm_x,
                mm_y - 14,
                Color::RGB(180, 180, 180),
            )
            .ok();
    }
}

// ---------------------------------------------------------------------------
// Extraction rendering
// ---------------------------------------------------------------------------

/// Render the extraction screen: map, extraction zone marker, player units.
fn render_extraction(game: &GameLoop, canvas: &mut Canvas<Window>) {
    render_placeholder_grid(canvas, &game.camera, &game.iso_config);

    // Extraction zone indicator at tile (0,0)
    let exit_tile = TilePos { x: 0, y: 0 };
    let world = game.iso_config.tile_to_screen(exit_tile);
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
            let w = game.iso_config.tile_to_screen(t);
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

    // Dark background -- the original game uses a dark charcoal backdrop
    // behind the phone scene to focus attention on the video call.
    canvas.set_draw_color(Color::RGB(15, 15, 25));
    canvas.clear();

    // -----------------------------------------------------------------------
    // Left side: Video phone with animated accountant
    // -----------------------------------------------------------------------
    // The phone occupies roughly the left 40% of the screen. We draw the
    // PHONSPR background first, then layer the accountant animation on top.

    // Milliseconds per animation frame -- controls how fast the accountant
    // cycles through talking/gesturing sprites. 200ms gives a natural feel
    // for a "talking head" animation without looking jittery.
    const ACCT_FRAME_PERIOD_MS: u32 = 200;

    // Phone scene background -- use frame 0 as the static backdrop.
    // The phone sprite is drawn centered in the left portion of the screen.
    let phone_area_w = (w * 2 / 5) as i32; // left 40% of screen
    if !phone_textures.is_empty() {
        let phone_tex = &phone_textures[0];
        let query = phone_tex.query();
        // Scale the phone scene to fit nicely in the left panel.
        // Maintain aspect ratio, fitting to the available height.
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

        // Accountant animation -- cycle through ACCT.OBJ frames on top of
        // the phone scene. The accountant is composited at the center of
        // the phone screen area (offset slightly to match the phone bezel).
        if !acct_textures.is_empty() {
            // Cycle between available frames as the debrief clock advances.
            // Frame 0 is the base portrait; later frames are mouth visemes.
            // When VLS lip-sync data is wired up we will drive the frame index
            // from the viseme timeline instead of a simple time-based loop.
            let frame_idx = if acct_textures.len() > 1 {
                ((anim_elapsed_ms / ACCT_FRAME_PERIOD_MS) as usize) % acct_textures.len()
            } else {
                0
            };
            let acct_tex = &acct_textures[frame_idx];
            let aq = acct_tex.query();
            // Scale accountant to fit inside the phone screen area.
            // The accountant should be roughly 60% of the phone's dimensions
            // to leave room for the phone bezel/frame.
            let acct_scale =
                (draw_h as f32 * 0.6 / aq.height as f32).min(draw_w as f32 * 0.6 / aq.width as f32);
            let acct_w = (aq.width as f32 * acct_scale) as u32;
            let acct_h = (aq.height as f32 * acct_scale) as u32;
            // Center the accountant on the phone screen.
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
        // Fallback: no phone sprites loaded -- draw a placeholder phone frame
        // so the screen is not completely empty on the left side.
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

        // Placeholder accountant indicator
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

    // -----------------------------------------------------------------------
    // Right side: Mission results + financial report
    // -----------------------------------------------------------------------
    // The text sits to the right of the phone, starting at ~45% of screen width.
    let text_x = (w * 9 / 20) as i32;

    // Title banner
    let title = if success {
        "MISSION COMPLETE"
    } else {
        "MISSION FAILED"
    };
    let title_color = if success {
        Color::RGB(80, 255, 80)
    } else {
        Color::RGB(255, 80, 80)
    };
    text.draw_header(canvas, tc, title, text_x, 40, title_color)
        .ok();

    // -- Battle Results --
    let mut y = 100i32;
    let survived = game.game_state.team.iter().filter(|m| m.is_alive()).count();
    let total = game.game_state.team.len();
    let killed = game.enemies.iter().filter(|e| e.current_hp == 0).count();
    let total_enemies = game.enemies.len();

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

    // -- Financial Report (the accountant's video phone summary) --
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

    let advance = 324_000i64; // TODO: get from accepted contract
    let bonus = if success { 200_000i64 } else { 0 };
    let hiring_costs = game.game_state.team.len() as i64 * 50_000; // approximate
    let medical = wia as i64 * 79_000; // WIA medical costs
    let death_insurance = kia as i64 * 89_000; // KIA death payouts
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
