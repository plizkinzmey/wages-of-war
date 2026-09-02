//! Office scene layout: hotspot rectangles + list-row geometry.
//!
//! The original office background is 640x480 pixels (OFFPIC2.PCX). We
//! store all clickable regions and per-list row geometry here so the
//! input handler and the renderer agree on coordinates, row heights,
//! and the hotspot → sub-phase mapping.
//!
//! Both the click hit-test in `input.rs` and the debug overlay in
//! `render.rs` read from this single source of truth. Edit a rectangle
//! or a row height here and both sides update.

use sdl2::pixels::Color;

use ow_core::game_state::OfficePhase;
use ow_render::iso_math::ScreenPos;

// ===========================================================================
// Office background geometry
// ===========================================================================

/// Native resolution of the original office background. All hotspot
/// rectangles below are measured in this coordinate space; the input
/// and render code scales the actual window size down (or up) to this
/// grid for hit testing.
pub const OFFICE_W: i32 = 640;
pub const OFFICE_H: i32 = 480;

// ===========================================================================
// Office hotspots
// ===========================================================================

/// A clickable region on the office background. The `target` is the
/// sub-phase the click should switch to (or `Overview` for the
/// "begin mission" door which stays in Overview but the player expects
/// feedback).
pub struct OfficeHotspot {
    /// `(x1, y1, x2, y2)` in 640x480 office space.
    pub rect: (i32, i32, i32, i32),
    /// Short label used by the debug overlay and the click log.
    pub label: &'static str,
    /// Sub-phase to switch into when this hotspot is clicked.
    pub target: OfficePhase,
    /// Debug overlay fill color (semi-transparent RGBA).
    pub color: Color,
}

/// The seven clickable objects on the office desk. Coordinates were
/// measured from a 640x480 grid overlay on OFFPIC2.PCX. Order is from
/// most-specific to least-specific to avoid overlap hits going to the
/// wrong object.
pub const HOTSPOTS: &[OfficeHotspot] = &[
    OfficeHotspot {
        rect: (400, 340, 520, 430),
        label: "Hire (Phone)",
        target: OfficePhase::HireMercs,
        color: Color::RGBA(255, 50, 50, 100),
    },
    OfficeHotspot {
        rect: (480, 230, 560, 310),
        label: "Contracts (Fax)",
        target: OfficePhase::Contracts,
        color: Color::RGBA(50, 50, 255, 100),
    },
    OfficeHotspot {
        rect: (490, 50, 620, 190),
        label: "Intel (Map)",
        target: OfficePhase::Intel,
        color: Color::RGBA(255, 255, 50, 100),
    },
    OfficeHotspot {
        rect: (70, 170, 130, 370),
        label: "Files (Cabinet)",
        target: OfficePhase::Intel,
        color: Color::RGBA(50, 255, 50, 100),
    },
    OfficeHotspot {
        rect: (100, 360, 220, 430),
        label: "Equip (Mags)",
        target: OfficePhase::Equipment,
        color: Color::RGBA(50, 255, 50, 100),
    },
    OfficeHotspot {
        rect: (230, 330, 310, 380),
        label: "Train (Calc)",
        target: OfficePhase::Training,
        color: Color::RGBA(255, 50, 255, 100),
    },
    OfficeHotspot {
        rect: (240, 40, 370, 250),
        label: "Mission (Door)",
        target: OfficePhase::Overview,
        color: Color::RGBA(255, 150, 0, 100),
    },
];

/// Find the first hotspot containing the given click, in declaration
/// order. The order matters — see the `HOTSPOTS` doc comment.
pub fn hotspot_at(
    click: ScreenPos,
    window_w: u32,
    window_h: u32,
) -> Option<&'static OfficeHotspot> {
    let sx = (click.x * OFFICE_W as f32 / window_w as f32) as i32;
    let sy = (click.y * OFFICE_H as f32 / window_h as f32) as i32;
    HOTSPOTS.iter().find(|h| {
        let (x1, y1, x2, y2) = h.rect;
        sx >= x1 && sx <= x2 && sy >= y1 && sy <= y2
    })
}

// ===========================================================================
// Office list-row geometry
// ===========================================================================

/// Geometry of the Hire Mercs list. The renderer draws rows starting
/// at `LIST_Y` with `ROW_H` pixels between baselines; the click
/// handler divides the click y by `ROW_H` to find the tapped row.
pub const HIRE_MERCS_LIST_Y: i32 = 85;
pub const HIRE_MERCS_ROW_H: i32 = 16;

/// Geometry of the Equipment weapons-catalog list.
pub const EQUIPMENT_LIST_Y: i32 = 105;
pub const EQUIPMENT_ROW_H: i32 = 14;

/// Geometry of the Contracts list. The list starts further down when
/// a contract is already accepted (the "ACCEPTED:" banner takes the
/// first 22 pixels).
pub const CONTRACTS_LIST_Y_EMPTY: i32 = 85;
pub const CONTRACTS_LIST_Y_ACCEPTED: i32 = 107;
pub const CONTRACTS_ROW_H: i32 = 18;

/// Convert a click y-coordinate (in office 640x480 space) to a row
/// index for the Hire Mercs list. Returns `None` if the click is
/// above the list.
pub fn hire_mercs_row(click_y: i32) -> Option<usize> {
    if click_y < HIRE_MERCS_LIST_Y {
        return None;
    }
    Some(((click_y - HIRE_MERCS_LIST_Y) / HIRE_MERCS_ROW_H) as usize)
}

/// Convert a click y-coordinate to a row index for the Equipment list.
pub fn equipment_row(click_y: i32) -> Option<usize> {
    if click_y < EQUIPMENT_LIST_Y {
        return None;
    }
    Some(((click_y - EQUIPMENT_LIST_Y) / EQUIPMENT_ROW_H) as usize)
}

/// Convert a click y-coordinate to a row index for the Contracts list.
/// `has_accepted` accounts for the "ACCEPTED:" banner offset.
pub fn contracts_row(click_y: i32, has_accepted: bool) -> Option<usize> {
    let list_y = if has_accepted {
        CONTRACTS_LIST_Y_ACCEPTED
    } else {
        CONTRACTS_LIST_Y_EMPTY
    };
    if click_y < list_y {
        return None;
    }
    Some(((click_y - list_y) / CONTRACTS_ROW_H) as usize)
}

/// Project a click in window space to office 640x480 space. Returns
/// `(sx, sy)`. Used by all office click handlers.
pub fn click_to_office(click: ScreenPos, window_w: u32, window_h: u32) -> (i32, i32) {
    (
        (click.x * OFFICE_W as f32 / window_w as f32) as i32,
        (click.y * OFFICE_H as f32 / window_h as f32) as i32,
    )
}
