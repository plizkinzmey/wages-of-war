//! # Screenshot — F12 saves the current frame to disk as BMP
//!
//! The F12 keypress handler is [`save_screenshot`]; it picks a free
//! `screenshot_NNN.bmp` filename in the CWD and writes via
//! [`write_canvas_as_bmp`]. The dev auto-screenshot loop also uses
//! [`write_canvas_as_bmp`] directly with its own pre-built path.

use std::path::Path;

use sdl2::render::Canvas;
use sdl2::video::Window;
use tracing::{info, warn};

/// Read the canvas's current framebuffer and write it to `path` as BMP.
/// RGB24 = 3 bytes per pixel, no alpha confusion.
pub(crate) fn write_canvas_as_bmp(canvas: &Canvas<Window>, path: &Path) {
    let (w, h) = canvas.output_size().unwrap_or((1280, 720));
    match canvas.read_pixels(None, sdl2::pixels::PixelFormatEnum::RGB24) {
        Ok(pixels) => match sdl2::surface::Surface::from_data_pixelmasks(
            &mut pixels.clone(),
            w,
            h,
            w * 3,
            &sdl2::pixels::PixelMasks {
                bpp: 24,
                rmask: 0xFF0000,
                gmask: 0x00FF00,
                bmask: 0x0000FF,
                amask: 0,
            },
        ) {
            Ok(surface) => {
                if let Err(e) = surface.save_bmp(path) {
                    warn!(path = %path.display(), "screenshot save_bmp: {e}");
                }
            }
            Err(e) => warn!(path = %path.display(), "screenshot surface: {e}"),
        },
        Err(e) => warn!(path = %path.display(), "screenshot read_pixels: {e}"),
    }
}

/// Save the current canvas contents to a BMP file.
/// Files are named screenshot_001.bmp, screenshot_002.bmp, etc.
pub(crate) fn save_screenshot(canvas: &Canvas<Window>) {
    // Find the next available screenshot number.
    for num in 1..=999u32 {
        let path = format!("screenshot_{num:03}.bmp");
        if !Path::new(&path).exists() {
            write_canvas_as_bmp(canvas, Path::new(&path));
            info!("Screenshot saved: {path}");
            return;
        }
    }
    warn!("Too many screenshots (>999)");
}
