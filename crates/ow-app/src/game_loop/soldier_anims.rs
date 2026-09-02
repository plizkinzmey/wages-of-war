//! # Soldier Animations — per-merc AnimController state machine
//!
//! One [`SoldierAnims`] bundle owns:
//!
//! - The decoded `JUNGSLD.DAT` frame textures (indexed by the controller's
//!   `current_frame_index()`).
//! - The `COR` animation index — maps `(action, direction, weapon) →
//!   frame range`.
//! - One `AnimController` per merc on the team, in `team` order.
//! - Per-merc `Snapshot` of the previous frame's state, used to diff
//!   position/hp/ap and decide whether the controller should switch
//!   actions.
//! - Per-merc walk-grace counter, so a teleport-step (one tile per click)
//!   shows as one Walk cycle instead of marching in place.
//!
//! The single per-frame entry point is [`SoldierAnims::tick`], which runs
//! the dispatch + auto-revert + grace-reset + snapshot in three passes
//! over the team. The three passes are necessary because each pass
//! reads the team's current state and writes a different field of
//! `state`; collapsing further would mean either re-iterating or
//! duplicating the dead-merc and out-of-bounds checks in every branch.
//!
//! ## Frame timing
//!
//! Position delta fires `Walk` with an 8-way direction computed from the
//! move vector. AP drop without movement fires `ShootStand` (the player's
//! click resolved as an attack). HP just hitting zero fires `Die` (the
//! controller holds the final death frame thereafter). One-shot
//! animations auto-revert to `Idle` when the controller reports
//! `is_finished()`. `Walk` reverts after a 24-frame grace period
//! (~400ms at 60 fps) so the cycle is visible across the teleport
//! boundary.

use ow_core::merc::ActiveMerc;
use ow_data::animation::AnimationSet;
use ow_render::anim_controller::{AnimAction, AnimController, Direction};
use sdl2::render::Texture;

/// One merc's previous-frame snapshot. Captures the four fields the
/// animation watcher actually diffs: identity, position, HP, AP.
#[derive(Debug, Clone, Copy)]
struct Snapshot {
    #[allow(dead_code)] // kept for future action-context plumbing
    id: u32,
    pos: Option<ow_core::merc::TilePos>,
    hp: u32,
    ap: u32,
}

/// Per-merc bookkeeping between frames. `prev` is `None` until the first
/// tick on that index; `walk_grace` is the count-down timer for the
/// current Walk animation; `just_moved` is set during pass 1 and read
/// during pass 2.
#[derive(Debug, Clone, Copy, Default)]
struct FrameState {
    prev: Option<Snapshot>,
    walk_grace: u32,
    just_moved: bool,
}

/// Frames to hold Walk after a position change before reverting to Idle.
/// Tuned to ~400ms at 60 fps so the walk cycle is visible across the
/// teleport-frame boundary.
const WALK_GRACE_FRAMES: u32 = 24;

/// All per-merc animation state in one bundle. Lives on the run loop;
/// the texture lifetime is the SDL2 texture-creator lifetime.
pub struct SoldierAnims<'a> {
    /// Decoded soldier animation frames, indexed by
    /// `AnimController::current_frame_index()`. `None` entries are
    /// zero-size placeholder frames in the DAT file.
    pub textures: Vec<Option<Texture<'a>>>,
    /// COR animation index — maps `(action, direction, weapon) → frame
    /// range`. Needed by every controller; `None` means the COR file
    /// failed to load and the controllers are useless.
    pub anim_set: Option<AnimationSet>,
    /// One `AnimController` per merc on the team, in `team` order.
    pub anims: Vec<AnimController>,
    /// Per-merc state, lazily resized to match `team.len()` on every
    /// tick. Index `i` corresponds to `team[i]`.
    state: Vec<FrameState>,
}

impl<'a> SoldierAnims<'a> {
    /// Create an empty bundle. The team is empty, no controllers exist,
    /// and no textures are loaded.
    pub fn empty() -> Self {
        Self {
            textures: Vec::new(),
            anim_set: None,
            anims: Vec::new(),
            state: Vec::new(),
        }
    }

    /// Build one AnimController per merc on the team, all in idle pose.
    /// Call after the bundle has an `anim_set` (i.e. after the COR file
    /// has been loaded).
    pub fn spawn_controllers(&mut self, team: &[ActiveMerc]) {
        let Some(ref anim_set) = self.anim_set else {
            return;
        };
        self.anims.clear();
        for _ in team {
            let mut ctrl = AnimController::new(anim_set.clone());
            ctrl.set_action(AnimAction::Idle, Direction::S, 1);
            self.anims.push(ctrl);
        }
        tracing::info!(controllers = self.anims.len(), "AnimControllers ready");
    }

    /// Run one frame of the animation state-machine.
    ///
    /// Three passes:
    /// 1. **Dispatch** — for each living merc, decide the controller
    ///    action based on the prev/current state diff.
    /// 2. **Revert + grace** — for each living merc with a controller,
    ///    revert finished one-shots to Idle; reset and tick down the
    ///    walk-grace counter.
    /// 3. **Snapshot + tick** — capture the current state for next
    ///    frame; advance every controller by `delta_ms` so the
    ///    currently-playing action's frames move forward.
    pub fn tick(&mut self, team: &[ActiveMerc], delta_ms: u32) {
        // Lazy-resize the per-merc state vector. New mercs (e.g. a hire
        // mid-mission) get a fresh `Default` FrameState; removed mercs
        // are dropped from the tail.
        if self.state.len() != team.len() {
            self.state.resize(team.len(), FrameState::default());
        }

        // -- Pass 1: dispatch --
        for (i, merc) in team.iter().enumerate() {
            let s = &mut self.state[i];
            let Some(ctrl) = self.anims.get_mut(i) else {
                // No controller for this merc yet (COR/DAT failed to
                // load). Still snapshot the state so the next pass sees
                // up-to-date data.
                s.prev = Some(snapshot_of(merc));
                s.just_moved = false;
                continue;
            };
            let prev = s.prev;
            let prev_pos = prev.and_then(|p| p.pos);
            let prev_hp = prev.map(|p| p.hp).unwrap_or(merc.current_hp);
            let prev_ap = prev.map(|p| p.ap).unwrap_or(merc.current_ap);

            // Death edge: alive last frame, dead this frame. Set Die
            // and skip — the controller will hold the final death frame.
            if prev_hp > 0 && merc.current_hp == 0 {
                ctrl.set_action(AnimAction::Die, Direction::S, 1);
                s.just_moved = false;
                continue;
            }
            if merc.current_hp == 0 {
                s.just_moved = false;
                continue;
            }

            // Movement: position changed since last frame.
            if let (Some(np), Some(pp)) = (merc.position, prev_pos) {
                if np.x != pp.x || np.y != pp.y {
                    let dir = dir_from_delta(np.x - pp.x, np.y - pp.y);
                    ctrl.set_action(AnimAction::Walk, dir, 1);
                    s.just_moved = true;
                    continue;
                }
            }

            // Shoot: AP dropped without a position change, i.e. the
            // player's click resolved as an attack. Direction here is
            // a default (S) — refining this requires plumbing the
            // target tile through to the watcher; out of scope for
            // now. The animation still plays and looks correct because
            // the sprite mostly faces the camera at S.
            if merc.current_ap < prev_ap {
                ctrl.set_action(AnimAction::ShootStand, Direction::S, 1);
                s.just_moved = false;
                continue;
            }

            s.just_moved = false;
        }

        // -- Pass 2: revert finished one-shots, tick walk grace --
        for (i, merc) in team.iter().enumerate() {
            if merc.current_hp == 0 {
                continue;
            }
            let Some(ctrl) = self.anims.get_mut(i) else {
                continue;
            };
            let s = &mut self.state[i];

            // Reset the walk-grace counter for any merc that just (re)
            // fired Walk this frame. Resetting here (rather than in
            // pass 1) means we never overwrite a grace value that's
            // still being ticked down.
            if s.just_moved {
                s.walk_grace = WALK_GRACE_FRAMES;
            }

            match ctrl.state().map(|st| st.action) {
                Some(AnimAction::ShootStand)
                | Some(AnimAction::ShootCrouch)
                | Some(AnimAction::Hit)
                | Some(AnimAction::Throw)
                | Some(AnimAction::Melee)
                    if ctrl.is_finished() =>
                {
                    ctrl.set_action(AnimAction::Idle, Direction::S, 1);
                    s.walk_grace = 0;
                }
                Some(AnimAction::Walk) => {
                    if s.walk_grace == 0 {
                        ctrl.set_action(AnimAction::Idle, Direction::S, 1);
                    } else {
                        s.walk_grace -= 1;
                    }
                }
                _ => {}
            }
        }

        // -- Pass 3: snapshot for next frame, tick controllers --
        for (i, merc) in team.iter().enumerate() {
            self.state[i].prev = Some(snapshot_of(merc));
        }
        for ctrl in self.anims.iter_mut() {
            ctrl.update(delta_ms as f32);
        }
    }
}

fn snapshot_of(merc: &ActiveMerc) -> Snapshot {
    Snapshot {
        id: merc.id,
        pos: merc.position,
        hp: merc.current_hp,
        ap: merc.current_ap,
    }
}

/// Map a tile-delta to one of 8 cardinal/diagonal directions.
/// +y is south on the staggered isometric grid (row index grows downward),
/// so the dy sign maps directly to N/S.
fn dir_from_delta(dx: i32, dy: i32) -> Direction {
    match (dx.signum(), dy.signum()) {
        (0, -1) => Direction::N,
        (1, -1) => Direction::NE,
        (1, 0) => Direction::E,
        (1, 1) => Direction::SE,
        (0, 1) => Direction::S,
        (-1, 1) => Direction::SW,
        (-1, 0) => Direction::W,
        (-1, -1) => Direction::NW,
        _ => Direction::S,
    }
}
