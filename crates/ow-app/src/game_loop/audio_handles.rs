//! # Audio Handles — bundle of audio resources for the lifetime of the game loop
//!
//! The five audio-related state items (music device liveness, current track
//! name, the live `Music` handle, the SFX manager, the voice player, the
//! "music is broken" latch) used to be a mix of stack locals and a single
//! `GameLoop` field. Grouping them keeps the loop body's music/SFX section
//! readable and makes the cleanup at end-of-loop (drop voice before closing
//! the mixer) easier to audit.
//!
//! `SfxManager` is owned here because it's the only writer; `VoicePlayer` is
//! wrapped in `Option` because the mixer may have failed to open at startup
//! and we degrade to "no voice, no music, no SFX" in that case.

use ow_audio::sfx::SfxManager;
use ow_audio::voice::VoicePlayer;
use sdl2::mixer::Music;

/// All audio resources in one place. Lives for the entire game-loop body.
pub struct AudioHandles {
    /// True if `sdl2::mixer::open_audio` succeeded. False means no music,
    /// SFX, or voice can play and we skip every audio-side effect.
    pub audio_available: bool,

    /// Name of the track currently playing (or last-played). Used to skip
    /// redundant `start_music` calls when the phase hasn't changed.
    pub music_track: Option<String>,

    /// The live SDL2_mixer `Music` handle. Must stay alive while the track
    /// is playing; dropping it stops the music. The leading underscore
    /// preserves the original name from `mod.rs` (it signals "kept alive
    /// for its drop side-effect, not read directly").
    pub _music_handle: Option<Music<'static>>,

    /// SFX manager. Channels 2-7 are reserved for combat SFX; 0-1 stay
    /// free for voice and music so they don't trample each other.
    pub sfx_manager: SfxManager,

    /// Voice line player. Plays merc greetings on hire and tab-cycle. None
    /// when audio wasn't available at startup.
    pub voice_player: Option<VoicePlayer>,

    /// Persistent latch: once SDL2_mixer fails to load a MIDI file (no
    /// SoundFont), we stop retrying for the rest of the session. Stops
    /// the MIDI loader from spamming stderr on every phase change.
    pub music_broken: bool,
}
