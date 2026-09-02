//! # Camera — Viewport control for isometric map rendering
//!
//! The camera defines what portion of the isometric world is visible on screen.
//! It tracks a position in world space (pixel coordinates before projection)
//! and a zoom level. All rendering passes through the camera to determine
//! which tiles are visible and where they appear on screen.
//!
//! ## Coordinate spaces
//!
//! - **World space**: The full isometric map in pixel coordinates. Origin is
//!   the top-left of the diamond grid. Tile (0,0) maps to the IsoConfig origin.
//! - **Screen space**: The SDL2 window pixel coordinates. (0,0) is the
//!   top-left corner of the window.
//!
//! The camera transform is:
//!   `screen = (world - camera_offset) * zoom`
//!
//! Inverse:
//!   `world = screen / zoom + camera_offset`

use sdl2::keyboard::Keycode;
use tracing::trace;

use crate::iso_math::{IsoConfig, ScreenPos};

/// Camera for scrolling and zooming the isometric viewport.
///
/// The camera position `(x, y)` represents the world-space coordinate that
/// appears at the top-left corner of the viewport (before zoom). Increasing
/// `x` scrolls the view to the right; increasing `y` scrolls downward.
#[derive(Debug, Clone)]
pub struct Camera {
    /// World-space X offset (top-left of viewport).
    pub x: f32,
    /// World-space Y offset (top-left of viewport).
    pub y: f32,
    /// Zoom multiplier. 1.0 = native resolution. >1.0 = zoomed in.
    pub zoom: f32,
    /// Viewport width in screen pixels.
    pub viewport_width: u32,
    /// Viewport height in screen pixels.
    pub viewport_height: u32,
}

/// Minimum zoom level (zoomed far out).
const ZOOM_MIN: f32 = 0.25;
/// Maximum zoom level (zoomed far in).
const ZOOM_MAX: f32 = 4.0;
/// Zoom step multiplier for zoom-in (inverse used for zoom-out).
const ZOOM_STEP: f32 = 1.25;

impl Camera {
    /// Create a new camera centered at world origin with default zoom.
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
            viewport_width,
            viewport_height,
        }
    }

    /// Scroll the camera by a world-space delta.
    ///
    /// Positive `dx` moves the viewport to the right (reveals more of the
    /// world to the right). Positive `dy` moves the viewport downward.
    pub fn scroll(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
        trace!(x = self.x, y = self.y, "camera scrolled");
    }

    /// Apply a discrete scroll step for a WASD or arrow key. Returns
    /// `true` if `key` is a camera-scroll key and the camera was
    /// moved, `false` otherwise. Centralizes the WASD/arrow → scroll
    /// mapping so input handlers don't each inline their own version.
    ///
    /// `speed` is in world units per press (matching the previous
    /// inline blocks in `input.rs`).
    pub fn scroll_for_key(&mut self, key: Keycode, speed: f32) -> bool {
        match key {
            Keycode::W | Keycode::Up => {
                self.scroll(0.0, -speed);
                true
            }
            Keycode::S | Keycode::Down => {
                self.scroll(0.0, speed);
                true
            }
            Keycode::A | Keycode::Left => {
                self.scroll(-speed, 0.0);
                true
            }
            Keycode::D | Keycode::Right => {
                self.scroll(speed, 0.0);
                true
            }
            _ => false,
        }
    }

    /// Zoom in by one step (1.25x), clamped to the maximum zoom level.
    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom * ZOOM_STEP).min(ZOOM_MAX);
        trace!(zoom = self.zoom, "camera zoomed in");
    }

    /// Zoom out by one step (0.8x), clamped to the minimum zoom level.
    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom / ZOOM_STEP).max(ZOOM_MIN);
        trace!(zoom = self.zoom, "camera zoomed out");
    }

    /// Compute the range of tile coordinates that are potentially visible
    /// in the current viewport.
    ///
    /// Returns `(min_x, min_y, max_x, max_y)` in tile coordinates. This is
    /// a conservative estimate — some tiles at the edges may be off-screen,
    /// but all on-screen tiles are guaranteed to be within this range.
    ///
    /// The calculation works by converting the four viewport corners from
    /// screen space to world space to tile space, then taking the bounding
    /// box of those four tile positions with some padding.
    pub fn visible_tile_bounds(&self, iso: &IsoConfig) -> (i32, i32, i32, i32) {
        // Convert each viewport corner to world space, then to tile space.
        let corners = [
            ScreenPos { x: 0.0, y: 0.0 },
            ScreenPos {
                x: self.viewport_width as f32,
                y: 0.0,
            },
            ScreenPos {
                x: 0.0,
                y: self.viewport_height as f32,
            },
            ScreenPos {
                x: self.viewport_width as f32,
                y: self.viewport_height as f32,
            },
        ];

        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;

        for corner in &corners {
            // Screen -> world -> tile
            let world = self.screen_to_world(*corner);
            let tile = iso.screen_to_tile(world);

            min_x = min_x.min(tile.x);
            min_y = min_y.min(tile.y);
            max_x = max_x.max(tile.x);
            max_y = max_y.max(tile.y);
        }

        // Add padding to account for tile dimensions extending beyond their
        // anchor point. Isometric tiles are drawn from their top corner, so
        // tiles just outside the viewport corners may still be partially visible.
        let padding = 2;
        let bounds = (
            min_x - padding,
            min_y - padding,
            max_x + padding,
            max_y + padding,
        );

        trace!(
            min_x = bounds.0,
            min_y = bounds.1,
            max_x = bounds.2,
            max_y = bounds.3,
            "visible tile bounds"
        );

        bounds
    }

    /// Transform a world-space position to screen-space position.
    ///
    /// Applies the camera offset and zoom: `screen = (world - camera) * zoom`.
    pub fn world_to_screen(&self, world: ScreenPos) -> ScreenPos {
        ScreenPos {
            x: (world.x - self.x) * self.zoom,
            y: (world.y - self.y) * self.zoom,
        }
    }

    /// Transform a screen-space position to world-space position.
    ///
    /// Inverse of `world_to_screen`: `world = screen / zoom + camera`.
    pub fn screen_to_world(&self, screen: ScreenPos) -> ScreenPos {
        ScreenPos {
            x: screen.x / self.zoom + self.x,
            y: screen.y / self.zoom + self.y,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_screen_round_trip() {
        let cam = Camera {
            x: 100.0,
            y: 200.0,
            zoom: 2.0,
            viewport_width: 1280,
            viewport_height: 720,
        };

        let world = ScreenPos { x: 500.0, y: 300.0 };
        let screen = cam.world_to_screen(world);
        let back = cam.screen_to_world(screen);

        assert!((back.x - world.x).abs() < 0.01);
        assert!((back.y - world.y).abs() < 0.01);
    }

    #[test]
    fn zoom_clamps() {
        let mut cam = Camera::new(800, 600);

        // Zoom in repeatedly — should clamp at ZOOM_MAX
        for _ in 0..20 {
            cam.zoom_in();
        }
        assert!(cam.zoom <= ZOOM_MAX);
        assert!((cam.zoom - ZOOM_MAX).abs() < 0.01);

        // Zoom out repeatedly — should clamp at ZOOM_MIN
        for _ in 0..40 {
            cam.zoom_out();
        }
        assert!(cam.zoom >= ZOOM_MIN);
        assert!((cam.zoom - ZOOM_MIN).abs() < 0.01);
    }

    #[test]
    fn scroll_accumulates() {
        let mut cam = Camera::new(800, 600);
        cam.scroll(10.0, 20.0);
        cam.scroll(5.0, -10.0);
        assert!((cam.x - 15.0).abs() < 0.01);
        assert!((cam.y - 10.0).abs() < 0.01);
    }

    #[test]
    fn visible_bounds_include_origin() {
        let iso = IsoConfig {
            tile_width: 64.0,
            tile_height: 32.0,
            origin_x: 0.0,
            origin_y: 0.0,
        };
        let cam = Camera::new(1280, 720);
        let (min_x, min_y, max_x, max_y) = cam.visible_tile_bounds(&iso);

        // Tile (0,0) should be within bounds when camera is at origin
        assert!(min_x <= 0);
        assert!(min_y <= 0);
        assert!(max_x >= 0);
        assert!(max_y >= 0);
    }
}
