//! # Dev auto-screenshot loop
//!
//! Env-gated. When `OW_AUTO_SCREENSHOT_MS` is set to a positive integer,
//! the game loop drops a BMP into `dev-screenshots/run-<unix-ts>/` every N
//! milliseconds, with a phase tag in the filename so a thousand frames are
//! still searchable. Disabled by default; pure dev tooling, gitignored
//! output dir.
//!
//! Per-frame cost when disabled: one struct field read. When enabled: a
//! `now.duration_since(last)` comparison and a single BMP write at the
//! configured cadence.

use std::path::PathBuf;
use std::time::Instant;

use sdl2::render::Canvas;
use sdl2::video::Window;
use tracing::{info, warn};

use super::write_canvas_as_bmp;

/// Dev auto-screenshot state machine. Construct via [`AutoScreenshot::from_env`]
/// at startup; call [`AutoScreenshot::tick`] once per rendered frame.
pub(crate) struct AutoScreenshot {
    /// Output directory (`dev-screenshots/run-<ts>/`).
    dir: PathBuf,
    /// Minimum milliseconds between frames.
    interval_ms: u128,
    /// Last time we wrote a frame.
    last: Instant,
    /// Frame counter used in the filename (zero-padded to 5 digits).
    count: u32,
}

impl AutoScreenshot {
    /// Build from the `OW_AUTO_SCREENSHOT_MS` env var. Returns `None` when
    /// the variable is unset, zero, or the mkdir fails (with a `warn!` log).
    pub(crate) fn from_env() -> Option<Self> {
        let interval_ms: Option<u128> = std::env::var("OW_AUTO_SCREENSHOT_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&n: &u128| n > 0);
        let interval_ms = interval_ms?;

        let run_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dir = PathBuf::from(format!("dev-screenshots/run-{run_id}"));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!(?dir, "auto-screenshot mkdir failed: {e} — feature disabled");
            return None;
        }
        info!(interval_ms, dir = %dir.display(), "Auto-screenshot enabled");
        Some(Self {
            dir,
            interval_ms,
            last: Instant::now(),
            count: 0,
        })
    }

    /// Per-frame tick. Writes a BMP if at least `interval_ms` has elapsed
    /// since the previous write. No-op when the feature is disabled.
    ///
    /// `phase_label` is the human-readable phase string from
    /// [`super::phase_label`]; sanitized for filename use here.
    pub(crate) fn tick(&mut self, canvas: &Canvas<Window>, phase_label: &str) {
        let now = Instant::now();
        if now.duration_since(self.last).as_millis() < self.interval_ms {
            return;
        }
        let phase = sanitize_phase_for_filename(phase_label);
        let path: PathBuf = self.dir.join(format!("ss_{:05}_{phase}.bmp", self.count));
        write_canvas_as_bmp(canvas, &path);
        self.count = self.count.saturating_add(1);
        self.last = now;
    }
}

/// Strip whitespace, em-dashes, and slashes from a phase label so it's
/// safe to use as a filename component. Lowercased for grep-ability.
fn sanitize_phase_for_filename(label: &str) -> String {
    label
        .replace(' ', "_")
        .replace('—', "-")
        .replace('/', "_")
        .to_lowercase()
}
