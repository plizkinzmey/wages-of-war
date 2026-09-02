//! # Combat log
//!
//! On-screen rolling log of combat events shown during combat. Color-coded
//! by [`CombatLogKind`]. Capped at [`COMBAT_LOG_MAX`] entries; oldest are
//! dropped on overflow.

use sdl2::pixels::Color;
use tracing::debug;

use super::GameLoop;

/// Maximum number of combat log entries displayed on screen.
pub(crate) const COMBAT_LOG_MAX: usize = 8;

/// A single entry in the combat message log, with color-coding info.
#[derive(Debug, Clone)]
pub(crate) struct CombatLogEntry {
    /// The message text to display.
    pub text: String,
    /// The category determines the display color.
    pub kind: CombatLogKind,
}

/// Color categories for combat log entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CombatLogKind {
    /// Player hit on enemy — green.
    PlayerHit,
    /// Enemy hit on player merc — red.
    EnemyHit,
    /// Any miss — gray.
    Miss,
    /// A unit was killed — yellow.
    Kill,
    /// Informational (movement, round changes) — white.
    Info,
}

impl CombatLogKind {
    /// Return the SDL2 color for this log category.
    pub(crate) fn color(self) -> Color {
        match self {
            CombatLogKind::PlayerHit => Color::RGB(80, 220, 80),
            CombatLogKind::EnemyHit => Color::RGB(220, 60, 60),
            CombatLogKind::Miss => Color::RGB(160, 160, 160),
            CombatLogKind::Kill => Color::RGB(255, 220, 50),
            CombatLogKind::Info => Color::RGB(200, 200, 200),
        }
    }
}

/// Push a message to the combat log, trimming to [`COMBAT_LOG_MAX`] entries.
pub(crate) fn log_combat(game: &mut GameLoop, msg: String, kind: CombatLogKind) {
    debug!(combat_log = %msg, "Combat log entry");
    game.combat_log.push(CombatLogEntry { text: msg, kind });
    if game.combat_log.len() > COMBAT_LOG_MAX {
        let excess = game.combat_log.len() - COMBAT_LOG_MAX;
        game.combat_log.drain(..excess);
    }
}
