//! Where the island is in its life, and the timings between the steps.
//!
//! ```text
//! Seed ─▶ Rest ⇄ Awake ─▶ Ok ─▶ Gone
//! ```
//!
//! It starts and ends as the seed, the dot a bar folds its own island into,
//! so the compositor's cut between the two processes falls under one dot that
//! does not move.

use std::time::Duration;

use guido::prelude::*;

use crate::services::auth::Auth;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The dot. The first frame, and the last.
    Seed,
    /// The small island at the top: clock, battery, "type to unlock".
    Rest,
    /// The card, where someone is typing.
    Awake,
    /// Accepted: the tick, before folding away.
    Ok,
    /// Folded back into the seed; the caller hands over.
    Gone,
}

/// How long the seed is held before it grows: the first frame has to reach
/// the screen before anything moves.
const GROW_MS: u64 = 350;
/// The tick is seen before the island folds.
const OK_MS: u64 = 600;
/// The fold into the seed, before the handover.
pub const FOLD_MS: u64 = 650;
/// After a card appears, the key or click that woke it cannot press a button.
pub const ARM_MS: u64 = 450;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Session,
    Power,
}

#[derive(Clone, Copy)]
pub struct Flow {
    pub phase: RwSignal<Phase>,
    /// The drawer open under the card.
    pub zone: RwSignal<Option<Zone>>,
    /// Show the password in the clear.
    pub reveal: RwSignal<bool>,
}

impl Flow {
    /// Starts as the seed, and folds back into it once `auth` accepts; `gone`
    /// runs when the fold is over. [`grow`](Self::grow) says when it may grow.
    pub fn start(auth: Auth, gone: impl Fn() + 'static) -> Self {
        let flow = Self {
            phase: create_signal(Phase::Seed),
            zone: create_signal(None),
            reveal: create_signal(false),
        };
        let phase = flow.phase;

        create_effect(move || {
            if !auth.state.authenticated.get() {
                return;
            }
            phase.set(Phase::Ok);
            flow.zone.set(None);
            set_timeout(Duration::from_millis(OK_MS), move || {
                phase.set(Phase::Gone);
            });
        });

        let gone = std::rc::Rc::new(gone);
        create_effect(move || {
            if phase.get() == Phase::Gone {
                let gone = gone.clone();
                set_timeout(Duration::from_millis(FOLD_MS), move || gone());
            }
        });

        // Half a password is not left in a field nobody is watching.
        create_effect(move || {
            if phase.get() == Phase::Rest {
                auth.state.clear_field();
                flow.zone.set(None);
                flow.reveal.set(false);
            }
        });

        flow
    }

    /// The seed is on screen: grow out of it once it has been seen.
    pub fn grow(&self) {
        let phase = self.phase;
        set_timeout(Duration::from_millis(GROW_MS), move || {
            if phase.get_untracked() == Phase::Seed {
                phase.set(Phase::Rest);
            }
        });
    }

    pub fn wake(&self) {
        if self.phase.get_untracked() == Phase::Rest {
            self.phase.set(Phase::Awake);
        }
    }

    pub fn rest(&self) {
        if self.phase.get_untracked() == Phase::Awake {
            self.phase.set(Phase::Rest);
        }
    }

    /// Tracked. The card is up, or about to be.
    pub fn is_down(&self) -> bool {
        matches!(self.phase.get(), Phase::Awake | Phase::Ok)
    }
}
