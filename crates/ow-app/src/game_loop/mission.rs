//! # Mission — mission-scoped resources loaded by `load_mission`
//!
//! `MissionData<'a>` is the value returned by
//! [`crate::game_loop::asset_loader::load_mission`] on success. It
//! bundles everything the game needs to play a mission: parsed map,
//! tile renderers, enemies, the mission-specific iso config. The `<'a>`
//! lifetime is the SDL2 texture-creator lifetime — it only matters for
//! the `TileMapRenderer` and `Texture` fields, but Rust requires it
//! on the whole struct.
//!
//! The run loop holds an `Option<MissionData<'a>>` directly. There's no
//! wrapper struct: the run loop uses `mission.as_ref()` / `.as_mut()` /
//! `.is_some()` exactly like it would with any other `Option`.

use ow_core::mission_setup::EnemyUnit;
use ow_data::map_loader::GameMap;
use ow_render::iso_math::IsoConfig;
use ow_render::tile_renderer::TileMapRenderer;

/// The full set of resources needed to play a mission.
///
/// Returned by [`crate::game_loop::asset_loader::load_mission`] on success.
/// Atomic: a `MissionData` is either fully usable or it doesn't exist
/// (the loader returns `Result<MissionData, LoadError>` and partial
/// failures stay inside).
pub struct MissionData<'a> {
    /// Parsed MAP file.
    pub map: GameMap,
    /// Per-mission iso config. Wages of War uses a 128x64 staggered grid;
    /// the loader sets this from the tileset's pixel dimensions.
    pub iso: IsoConfig,
    /// Tile renderer for the floor/terrain tileset (TIL).
    pub tile_renderer: TileMapRenderer<'a>,
    /// Tile renderer for the OBJ sprite sheet (buildings, walls, trees).
    /// `None` if the OBJ sheet was missing or failed to load — the
    /// renderer skips the OBJ pass in that case.
    pub obj_renderer: Option<TileMapRenderer<'a>>,
    /// Enemy units generated from mission data. Persists across
    /// Deployment and Combat; cleared on Debrief.
    pub enemies: Vec<EnemyUnit>,
}
