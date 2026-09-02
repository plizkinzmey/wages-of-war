//! # Audio Init — open the mixer and prepare music / SFX / voice for the loop
//!
//! This module exists to peel the audio setup out of
//! `run_game_loop_with_pump`. The [`init_audio`] function owns the
//! lifecycle of the mixer device: open it, allocate SFX chunks, build
//! the voice player, start the initial track for the current phase,
//! and pack everything into an [`AudioHandles`] that the loop body can
//! hold.
//!
//! Failure modes are deliberately soft: any of the three subsystems
//! (mixer, SFX, voice) failing to initialise results in
//! `audio_available: false` or the corresponding `None`, and the rest
//! of the game still runs.

use std::path::Path;

use tracing::{debug, info, warn};

use super::audio_handles::AudioHandles;
use super::music::{music_track_for_phase, start_music};
use super::PhaseHandler;

/// Open the SDL2_mixer device, pre-load SFX WAVs, build the voice
/// player, and start the initial music track for the current phase.
///
/// Returns an [`AudioHandles`] whose `audio_available` field reports
/// whether the mixer opened at all. When `false`, SFX/voice are absent
/// and music is skipped — but the game still runs and the rest of
/// the loop is unaffected.
///
/// The `music_broken` latch inside the returned struct is set on the
/// first MIDI load failure (no SoundFont on macOS) and is honoured by
/// every subsequent music transition in the run loop.
pub fn init_audio(data_dir: &Path, phase: &PhaseHandler) -> AudioHandles {
    let audio_available = open_mixer();
    let sfx_manager =
        ow_audio::sfx::SfxManager::new(&data_dir.join("WOW").join("SND"), audio_available);
    let voice_player = build_voice_player(data_dir, audio_available);
    let (music_track, _music_handle, music_broken) =
        start_initial_track(data_dir, phase, audio_available);

    AudioHandles {
        audio_available,
        music_track,
        _music_handle,
        sfx_manager,
        voice_player,
        music_broken,
    }
}

/// Open the mixer at 44.1 kHz / 16-bit signed / stereo with a
/// 1024-sample buffer. Returns `false` on failure; the rest of the
/// audio stack degrades accordingly.
fn open_mixer() -> bool {
    match sdl2::mixer::open_audio(44100, sdl2::mixer::AUDIO_S16LSB, 2, 1024) {
        Ok(()) => {
            info!("SDL2_mixer audio device opened (44100 Hz, S16LSB, stereo)");
            true
        }
        Err(e) => {
            warn!(error = %e, "SDL2_mixer failed to open audio -- continuing without music");
            false
        }
    }
}

/// Build a `VoicePlayer` only if the mixer is open. Voice lines use
/// mixer channel 1, kept separate from SFX and music.
fn build_voice_player(
    data_dir: &Path,
    audio_available: bool,
) -> Option<ow_audio::voice::VoicePlayer> {
    if !audio_available {
        debug!("Voice player disabled (no audio device)");
        return None;
    }
    let wav_dir = data_dir.join("WOW").join("WAV");
    Some(ow_audio::voice::VoicePlayer::new(wav_dir))
}

/// Start the music track for whatever phase the loop launched into,
/// returning `(current_track, live_handle, broken_latch)`.
///
/// When `audio_available` is `false` or the phase has no music, both
/// `current_track` and `live_handle` are `None` and the latch is
/// `false`. When the phase has a track but loading fails, the handle
/// is `None`, the track name is `None` (so the run loop will retry on
/// the next phase change), and the latch is whatever `start_music`
/// set — typically `true` (no SoundFont).
fn start_initial_track(
    data_dir: &Path,
    phase: &PhaseHandler,
    audio_available: bool,
) -> (Option<String>, Option<sdl2::mixer::Music<'static>>, bool) {
    if !audio_available {
        return (None, None, false);
    }
    let Some(track_name) = music_track_for_phase(phase) else {
        return (None, None, false);
    };
    let midi_dir = data_dir.join("WOW").join("MIDI");
    let mut broken = false;
    let handle = start_music(&midi_dir, track_name, &mut broken);
    let current = if handle.is_some() {
        Some(track_name.to_string())
    } else {
        None
    };
    (current, handle, broken)
}
