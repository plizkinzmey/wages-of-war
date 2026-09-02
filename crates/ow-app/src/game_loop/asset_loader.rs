//! # Asset Loader — file → texture / data conversions used at startup
//!
//! Three pure-function entry points replace the inline PCX / sprite / map
//! loading that used to live inside `run_game_loop_with_pump`:
//!
//! - [`load_office_texture`] — turn `OFFPIC2.PCX` into an SDL2 texture
//!   for the office background.
//! - [`load_debrief_sprites`] — decode every frame in `ACCT.OBJ` and
//!   `PHONSPR.OBJ` (the accountant and video-phone sprite sheets) into
//!   textures, using the OFFPIC2 palette.
//! - [`load_mission`] — parse `SCEN{n}.MAP` / `SCEN{n}A.MAP`, load the
//!   referenced TIL tileset, OBJ sprite sheet, soldier `COR` animation
//!   index, and the `JUNGSLD.DAT` frame atlas. Also generates enemy
//!   units from the mission's `enemy_ratings`.
//!
//! [`load_mission`] returns `Result<MissionData, LoadError>`. A mission
//! is either fully usable (returned `Ok`) or not loaded at all
//! (returned `Err`); partial states are impossible because the loader
//! builds the value before returning. The caller decides whether a
//! load failure is fatal — the run loop just logs and stays on the
//! Office screen.
//!
//! [`load_office_texture`]: fn@load_office_texture
//! [`load_debrief_sprites`]: fn@load_debrief_sprites
//! [`load_mission`]: fn@load_mission

use std::path::{Path, PathBuf};

use rand::Rng;
use sdl2::render::{Texture, TextureCreator};
use sdl2::video::WindowContext;
use tracing::{info, trace, warn};

use ow_core::merc::TilePos;
use ow_core::mission_setup::EnemyUnit;
use ow_core::ruleset::Ruleset;
use ow_render::iso_math::IsoConfig;
use ow_render::palette::Palette256;
use ow_render::tile_renderer::TileMapRenderer;

use super::mission::MissionData;
use super::soldier_anims::SoldierAnims;

// ---------------------------------------------------------------------------
// Office background
// ---------------------------------------------------------------------------

/// Load the office scene background (`OFFPIC2.PCX`) as a single texture.
///
/// The original game composites the office scene from `OFFICE.PCX` plus
/// overlaid OBJ sprites. We use `OFFPIC2.PCX`, the pre-composited version
/// the original game shipped as a fallback, so we get all the office
/// objects (phone, fax, pizza, filing cabinet) in one blit.
///
/// Returns `None` if the file is missing or texture creation fails —
/// the renderer falls back to the placeholder office background.
pub fn load_office_texture<'a>(
    data_dir: &Path,
    tc: &'a TextureCreator<WindowContext>,
) -> Option<Texture<'a>> {
    let pcx_path = data_dir.join("WOW").join("PIC").join("OFFPIC2.PCX");
    let img = match ow_render::pcx::load_pcx(&pcx_path) {
        Ok(img) => {
            info!(
                width = img.width,
                height = img.height,
                "Office background loaded"
            );
            img
        }
        Err(e) => {
            warn!("Failed to load OFFICE.PCX: {e}");
            return None;
        }
    };
    ow_render::pcx::pcx_to_texture(&img, tc)
        .map_err(|e| warn!("Failed to create office texture: {e}"))
        .ok()
}

// ---------------------------------------------------------------------------
// Debrief sprite sheets (accountant + video phone)
// ---------------------------------------------------------------------------

/// Load the accountant (`ACCT.OBJ`) and video phone (`PHONSPR.OBJ`)
/// sprite sheets into SDL2 textures.
///
/// Both sheets use the same FLC sprite container format as tilesets and
/// other OBJ files. We decode all frames at startup and convert to
/// textures so the debrief renderer can just index into them by frame
/// number. The OFFPIC2 palette is used as the closest match to the
/// game's master VGA palette.
///
/// Returns `(empty, empty)` if the palette can't be loaded — both
/// sheets are then skipped and the renderer falls back to the
/// placeholder debrief display.
pub fn load_debrief_sprites<'a>(
    data_dir: &Path,
    tc: &'a TextureCreator<WindowContext>,
) -> (Vec<Texture<'a>>, Vec<Texture<'a>>) {
    let pic_dir = data_dir.join("WOW").join("PIC");
    let palette = load_palette_pcx(&pic_dir);
    let palette = match palette {
        Some(p) => p,
        None => return (Vec::new(), Vec::new()),
    };
    let spr_dir = data_dir.join("WOW").join("SPR");
    let acct = decode_sprite_sheet(&spr_dir.join("ACCT.OBJ"), &palette, tc);
    let phone = decode_sprite_sheet(&spr_dir.join("PHONSPR.OBJ"), &palette, tc);
    (acct, phone)
}

/// Load the OFFPIC2 palette from the PIC directory, or any .PCX in PIC
/// if OFFPIC2 is missing. Returns `None` if neither exists. This is
/// the closest match to the game's master VGA palette; individual
/// mission palettes may differ but the office background and debrief
/// screens were designed against this one.
///
/// Public so callers (e.g. the soldier-animation loader) can reuse
/// the same palette without re-scanning the PIC directory.
pub fn load_palette_pcx(pic_dir: &Path) -> Option<Palette256> {
    let offpic = pic_dir.join("OFFPIC2.PCX");
    let path = if offpic.exists() {
        offpic
    } else {
        // TODO: lift "find a file by extension in a directory" to
        // `ow_data::fs_util` if a second caller appears. For now this
        // is the only consumer.
        std::fs::read_dir(pic_dir).ok()?.flatten().find_map(|e| {
            let ext = e.path().extension()?.to_ascii_uppercase();
            (ext == "PCX").then(|| e.path())
        })?
    };
    match ow_render::palette::load_pcx_palette(&path) {
        Ok(pal) => Some(pal),
        Err(e) => {
            warn!("Failed to load palette: {e}");
            None
        }
    }
}

/// Decode every frame of a sprite sheet into RGBA SDL2 textures using
/// `palette` for colour lookup. Returns an empty vec on any failure —
/// the caller is expected to render a fallback.
fn decode_sprite_sheet<'a>(
    path: &Path,
    palette: &Palette256,
    tc: &'a TextureCreator<WindowContext>,
) -> Vec<Texture<'a>> {
    let sheet = match ow_data::sprite::parse_sprite_file(path) {
        Ok(s) => {
            info!(
                path = %path.display(),
                frames = s.file_header.sprite_count,
                "sprite sheet loaded"
            );
            s
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to parse sprite sheet");
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
        if let Some(tex) = decode_one_frame(frame, i, fw, fh, palette, tc) {
            textures.push(tex);
        }
    }
    info!(
        path = %path.display(),
        decoded = textures.len(),
        total = sheet.frames.len(),
        "sprite textures ready"
    );
    textures
}

fn decode_one_frame<'a>(
    frame: &ow_data::sprite::SpriteFrame,
    index: usize,
    fw: u32,
    fh: u32,
    palette: &Palette256,
    tc: &'a TextureCreator<WindowContext>,
) -> Option<Texture<'a>> {
    let pixels = ow_data::sprite::decode_rle(
        &frame.compressed_data,
        frame.header.width,
        frame.header.height,
        index,
    )
    .ok()?;
    // Brightness boost of 1.5 to compensate for CRT->LCD gamma.
    let rgba = ow_render::palette::apply_palette_with_brightness(&pixels, palette, 1.5);
    let mut tex = tc
        .create_texture_static(sdl2::pixels::PixelFormatEnum::RGBA32, fw, fh)
        .ok()?;
    tex.set_blend_mode(sdl2::render::BlendMode::Blend);
    if tex.update(None, &rgba, (fw * 4) as usize).is_err() {
        warn!(frame = index, "failed to upload sprite texture");
        return None;
    }
    Some(tex)
}

// ---------------------------------------------------------------------------
// Mission map + renderers + animation + enemies
// ---------------------------------------------------------------------------

/// What went wrong while loading a mission. The loader is best-effort —
/// any single failure is recoverable, but the value isn't built unless
/// the core (map + tileset + palette) succeeds. The variant tells the
/// caller which step failed so the log line is informative.
#[derive(Debug)]
pub enum LoadError {
    MapNotFound(PathBuf),
    MapParseFailed(String),
    TilesetParseFailed(String),
    NoPaletteFile,
    TilesetTexturesFailed(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MapNotFound(p) => write!(f, "map file not found: {}", p.display()),
            Self::MapParseFailed(s) => write!(f, "map parse failed: {s}"),
            Self::TilesetParseFailed(s) => write!(f, "tileset parse failed: {s}"),
            Self::NoPaletteFile => write!(f, "no .PCX palette file in WOW/PIC/"),
            Self::TilesetTexturesFailed(s) => {
                write!(f, "tileset texture upload failed: {s}")
            }
        }
    }
}

/// Locate the `.MAP` file for mission `n` — tries `SCEN{n}.MAP` first,
/// then `SCEN{n}A.MAP`. Both filename variants exist in the original
/// game's data tree; the renderer needs the one that's actually
/// present.
fn find_map_path(data_dir: &Path, n: u32) -> PathBuf {
    let scen_dir = data_dir.join("WOW").join("MAPS").join(format!("SCEN{n}"));
    let try1 = scen_dir.join(format!("SCEN{n}.MAP"));
    if try1.exists() {
        try1
    } else {
        scen_dir.join(format!("SCEN{n}A.MAP"))
    }
}

/// Wages of War's staggered grid is 128x64. Every mission uses the same
/// projection, so the iso config is a constant inside the loader rather
/// than a parameter from the caller.
fn mission_iso_config() -> IsoConfig {
    IsoConfig {
        tile_width: 128.0,
        tile_height: 64.0,
        origin_x: 0.0,
        origin_y: 0.0,
    }
}

/// Load a mission map, tileset, OBJ sprites, soldier animation, and
/// generate enemy units.
///
/// `mission_n` is the integer mission number from the MSSN context.
/// `team_ids` is the current set of player team IDs; used to assign
/// non-colliding enemy IDs (offset by +1000 to keep enemy IDs out of
/// the player space).
///
/// **Atomicity:** the returned `MissionData` is either fully usable
/// (Ok) or doesn't exist (Err). No partial-update semantics — the
/// loader builds the value before returning. The OBJ sprite sheet and
/// soldier animation are best-effort: a missing OBJ results in
/// `obj_renderer: None` but a successful Ok, and a missing COR/DAT
/// results in `anim_set: None` on the returned data. Both are
/// non-fatal.
pub fn load_mission<'a>(
    data_dir: &Path,
    tc: &'a TextureCreator<WindowContext>,
    ruleset: &Ruleset,
    mission_n: u32,
    team_ids: &[u32],
) -> Result<MissionData<'a>, LoadError> {
    info!(mission = mission_n, "Loading mission");

    // -- Map --
    let map_path = find_map_path(data_dir, mission_n);
    let map = ow_data::map_loader::parse_map(&map_path).map_err(|e| {
        warn!("Failed to load map {}: {e}", map_path.display());
        // Distinguish "no file at either variant" from "file exists but is corrupt"
        if !map_path.exists() {
            LoadError::MapNotFound(map_path)
        } else {
            LoadError::MapParseFailed(e.to_string())
        }
    })?;
    info!(
        width = map.width(),
        height = map.height(),
        tileset = %map.asset_refs.tileset_path,
        "Map loaded"
    );

    // -- Tileset (TIL) --
    // The MAP references the original install path (e.g.
    // "C:\WOW\SPR\..."); we strip down to the filename and re-anchor
    // in the per-mission SPR directory.
    let spr_scen_dir = data_dir
        .join("WOW")
        .join("SPR")
        .join(format!("SCEN{mission_n}"));
    let til_name = ow_data::map_loader::filename_from_build_path(&map.asset_refs.tileset_path);
    let til_path = spr_scen_dir.join(til_name);
    let tileset = ow_data::sprite::parse_sprite_file(&til_path).map_err(|e| {
        warn!("Failed to load tileset {til_name}: {e}");
        LoadError::TilesetParseFailed(e.to_string())
    })?;
    info!(sprites = tileset.file_header.sprite_count, "Tileset loaded");

    // -- Palette --
    let pic_dir = data_dir.join("WOW").join("PIC");
    let pal = load_palette_pcx(&pic_dir).ok_or(LoadError::NoPaletteFile)?;

    // -- Tile renderer (TIL) --
    let mut tr = ow_render::tile_renderer::TileMapRenderer::new(tc);
    tr.load_tileset(&tileset, &pal).map_err(|e| {
        warn!("Failed to load tileset textures: {e}");
        LoadError::TilesetTexturesFailed(e.to_string())
    })?;
    info!(
        tile_w = tr.tile_pixel_width(),
        tile_h = tr.tile_pixel_height(),
        tiles = tr.tile_count(),
        "Tiles ready"
    );

    // -- OBJ sprite sheet (buildings, walls, fences, trees) --
    // Same sprite container format as TIL, lives in the same
    // SPR/SCEN{n}/ directory. Best-effort: missing OBJ is fine, the
    // render_mission_map skips the OBJ pass in that case.
    let obj_name =
        ow_data::map_loader::filename_from_build_path(&map.asset_refs.object_sprite_path);
    let obj_renderer = load_obj_renderer(&spr_scen_dir.join(obj_name), &pal, tc);

    // -- Enemies --
    let enemies = generate_enemies(ruleset, mission_n, team_ids);

    Ok(MissionData {
        map,
        iso: mission_iso_config(),
        tile_renderer: tr,
        obj_renderer,
        enemies,
    })
}

fn load_obj_renderer<'a>(
    obj_path: &Path,
    pal: &Palette256,
    tc: &'a TextureCreator<WindowContext>,
) -> Option<TileMapRenderer<'a>> {
    if !obj_path.exists() {
        warn!(path = %obj_path.display(), "OBJ sprite file not found");
        return None;
    }
    let sheet = match ow_data::sprite::parse_sprite_file(obj_path) {
        Ok(s) => {
            info!(
                sprites = s.file_header.sprite_count,
                path = %obj_path.display(),
                "OBJ sprite sheet loaded"
            );
            s
        }
        Err(e) => {
            warn!("Failed to load OBJ sheet: {e}");
            return None;
        }
    };
    let mut or = TileMapRenderer::new(tc);
    if let Err(e) = or.load_tileset(&sheet, pal) {
        warn!("Failed to load OBJ textures: {e}");
        return None;
    }
    info!(
        obj_tiles = or.tile_count(),
        obj_w = or.tile_pixel_width(),
        obj_h = or.tile_pixel_height(),
        "OBJ textures ready"
    );
    Some(or)
}

fn generate_enemies(ruleset: &Ruleset, mission_n: u32, team_ids: &[u32]) -> Vec<EnemyUnit> {
    let mission_key = format!("MSSN{mission_n:02}");
    let Some(mission_data) = ruleset.missions.get(&mission_key) else {
        warn!(mission = %mission_key, "No mission data in ruleset; no enemies generated");
        return Vec::new();
    };

    let mut rng = rand::thread_rng();
    let max_player_id = team_ids.iter().copied().max().unwrap_or(0);
    let mut next_id = max_player_id + 1000;
    let mut enemies = Vec::new();

    for (i, rating) in mission_data.enemy_ratings.iter().enumerate() {
        let roll: u8 = rng.gen_range(0..100);
        if roll >= rating.presence_chance {
            continue;
        }
        // Random position in the upper portion of the map. The exact
        // range is a placeholder — the real placement logic should
        // come from the map's spawn markers.
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
        let weapon = mission_data.enemy_weapons.get(i).unwrap_or(&default_weapon);
        let mut enemy = EnemyUnit::from_rating(next_id, rating, weapon);
        enemy.position = Some(TilePos { x: ex, y: ey });
        next_id += 1;
        enemies.push(enemy);
    }
    info!(enemies = enemies.len(), "Enemies generated for mission");
    enemies
}

// ---------------------------------------------------------------------------
// Soldier AnimController setup (separated from SoldierAnims so the loader
// can populate the bundle's textures/anim_set without needing the team)
// ---------------------------------------------------------------------------

/// Load the soldier animation index (`JUNGSLD.COR`) and frame atlas
/// (`JUNGSLD.DAT`). Populates `anims.textures` and `anims.anim_set` in
/// place. The actual `AnimController` instances are created later via
/// [`SoldierAnims::spawn_controllers`] once the team is known.
pub fn load_soldier_animation<'a>(
    data_dir: &Path,
    tc: &'a TextureCreator<WindowContext>,
    pal: &Palette256,
    anims: &mut SoldierAnims<'a>,
) {
    let anim_dir = data_dir.join("WOW").join("ANIM");

    // COR animation index
    let cor_path = anim_dir.join("JUNGSLD.COR");
    if cor_path.exists() {
        match ow_data::animation::parse_animation(&cor_path) {
            Ok(anim_set) => {
                info!(
                    entries = anim_set.entries.len(),
                    "COR animation index loaded"
                );
                anims.anim_set = Some(anim_set);
            }
            Err(e) => warn!("Failed to parse JUNGSLD.COR: {e}"),
        }
    } else {
        warn!(path = %cor_path.display(), "JUNGSLD.COR not found");
    }

    // DAT frame atlas
    let sld_path = anim_dir.join("JUNGSLD.DAT");
    if !sld_path.exists() {
        warn!(path = %sld_path.display(), "JUNGSLD.DAT not found");
        return;
    }
    let sheet = match ow_data::sprite::parse_sprite_file(&sld_path) {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to load JUNGSLD.DAT: {e}");
            return;
        }
    };
    let total = sheet.frames.len();
    let max_frames = total.min(2000);
    info!(total, loading = max_frames, "Decoding soldier frames");

    anims.textures.clear();
    let mut decoded = 0u32;
    for i in 0..max_frames {
        let frame = &sheet.frames[i];
        let fw = frame.header.width as u32;
        let fh = frame.header.height as u32;
        if fw == 0 || fh == 0 {
            anims.textures.push(None);
            continue;
        }
        // Soldier DAT frames are palette-indexed; we apply the same
        // OFFPIC2 palette + 1.5× brightness boost as the rest of the
        // renderer so the colours match the in-game originals.
        let tex_opt = decode_soldier_frame(frame, i, fw, fh, &pal, tc);
        if tex_opt.is_some() {
            decoded += 1;
        }
        anims.textures.push(tex_opt);
    }
    info!(decoded, "Soldier animation frames ready");
}

/// Decode a single JUNGSLD frame, applying the OFFPIC2 palette to
/// convert raw 8-bit indices into RGBA pixels with a 1.5× brightness
/// boost (the same treatment the rest of the renderer uses).
fn decode_soldier_frame<'a>(
    frame: &ow_data::sprite::SpriteFrame,
    index: usize,
    fw: u32,
    fh: u32,
    palette: &Palette256,
    tc: &'a TextureCreator<WindowContext>,
) -> Option<Texture<'a>> {
    let pixels = ow_data::sprite::decode_rle(
        &frame.compressed_data,
        frame.header.width,
        frame.header.height,
        index,
    )
    .ok()?;
    let rgba = ow_render::palette::apply_palette_with_brightness(&pixels, palette, 1.5);
    let mut tex = tc
        .create_texture_static(sdl2::pixels::PixelFormatEnum::RGBA32, fw, fh)
        .ok()?;
    tex.set_blend_mode(sdl2::render::BlendMode::Blend);
    tex.update(None, &rgba, (fw * 4) as usize).ok()?;
    Some(tex)
}

// ---------------------------------------------------------------------------
// Mission-number parsing (used by the run loop and by `load_mission`)
// ---------------------------------------------------------------------------

/// Re-export the `mission_num` helper so the run loop can compute the
/// integer mission number from `game.game_state.current_mission.name`
/// before calling `load_mission`.
pub fn mission_number_from_name(name: Option<&str>) -> u32 {
    name.and_then(|n| n.strip_prefix("MSSN"))
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(1)
}
