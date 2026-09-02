//! MIDI music playback via SDL2_mixer.
//!
//! Tracks are loaded from the original game's `WOW/MIDI/` directory. The
//! active backend (FluidSynth, native MIDI, etc.) depends on the SDL2_mixer
//! build and platform — on macOS without a SoundFont, MIDI is silently
//! disabled via the [`music_broken`] latch.

use std::path::Path;

use sdl2::mixer::Music;
use tracing::{debug, info, warn};

use super::PhaseHandler;

/// Default music volume: 50% of SDL2_mixer's 0–128 range.
/// The original game's MIDI can be grating at full volume.
pub(crate) const MUSIC_VOLUME: i32 = 64;

/// Returns the MIDI track filename (stem only, no extension) appropriate for
/// the given phase, or `None` if no music should play.
///
/// For mission phases (deployment, combat, extraction), the track is selected
/// based on the current mission number (1–9). Falls back to `WOWMIS01` if the
/// mission number is out of range or unknown.
pub(crate) fn music_track_for_phase(handler: &PhaseHandler) -> Option<&'static str> {
    match handler {
        PhaseHandler::Office { .. } => Some("WOWOFICE"),
        PhaseHandler::Travel { .. } => Some("WOWARIVE"),
        PhaseHandler::Deployment { .. } => Some("WOWMIS01"),
        PhaseHandler::Combat(_) => Some("WOWMIS01"),
        PhaseHandler::Extraction => Some("WOWMIS01"),
        PhaseHandler::Debrief { success: true, .. } => Some("WOWDPARW"),
        PhaseHandler::Debrief { success: false, .. } => Some("WOWDPARL"),
        PhaseHandler::Paused { .. } => None, // keep whatever was playing
    }
}

/// Like [`music_track_for_phase`] but uses the mission number (1–9) to pick
/// the correct `WOWMISxx` track instead of always defaulting to 01.
pub(crate) fn music_track_for_phase_with_mission(
    handler: &PhaseHandler,
    mission_num: Option<u32>,
) -> Option<String> {
    match handler {
        PhaseHandler::Deployment { .. } | PhaseHandler::Combat(_) | PhaseHandler::Extraction => {
            let n = mission_num.unwrap_or(1).clamp(1, 9);
            Some(format!("WOWMIS{n:02}"))
        }
        _ => music_track_for_phase(handler).map(String::from),
    }
}

/// Try to load and play a MIDI track, returning the `Music` handle that must
/// be kept alive for the duration of playback. Returns `None` (with a warning
/// logged) if the file is missing or SDL2_mixer can't play it.
///
/// `music_broken` is a persistent latch: once SDL2_mixer fails to *load* a
/// MIDI file (`Music::from_file` returns `Err`), the underlying issue is
/// environmental — on macOS SDL2_mixer's MIDI backend needs a SoundFont
/// installed, and without one it prints "No SoundFonts have been requested"
/// for every load attempt. We latch that case so the caller stops hammering
/// the loader every frame (which would spam stderr). A *missing file* is not
/// latched: that's per-track and may simply mean the game doesn't ship that
/// track, so we keep trying other phases.
pub(crate) fn start_music<'a>(
    midi_dir: &Path,
    track_name: &str,
    music_broken: &mut bool,
) -> Option<Music<'a>> {
    let mid_path = midi_dir.join(format!("{track_name}.MID"));
    if !mid_path.exists() {
        warn!(track = track_name, path = %mid_path.display(),
              "MIDI file not found -- skipping music");
        return None;
    }
    if *music_broken {
        // We already proved MIDI can't load (no SoundFont). Don't retry.
        debug!(
            track = track_name,
            "Skipping MIDI -- backend known broken (no SoundFont)"
        );
        return None;
    }
    match Music::from_file(&mid_path) {
        Ok(music) => {
            Music::set_volume(MUSIC_VOLUME);
            if let Err(e) = music.play(-1) {
                warn!(track = track_name, error = %e,
                      "SDL2_mixer failed to play MIDI -- continuing without music");
                None
            } else {
                info!(
                    track = track_name,
                    volume = MUSIC_VOLUME,
                    "Now playing MIDI track"
                );
                Some(music)
            }
        }
        Err(e) => {
            warn!(track = track_name, error = %e,
                  "SDL2_mixer failed to load MIDI -- music unavailable (no SoundFont?) -- disabling MIDI for this session");
            *music_broken = true;
            None
        }
    }
}

/// Stop any currently playing music. Safe to call even if nothing is playing.
pub(crate) fn stop_music() {
    Music::halt();
    debug!("Music halted");
}
