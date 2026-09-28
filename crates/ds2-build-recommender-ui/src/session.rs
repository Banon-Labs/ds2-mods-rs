//! The panel's life across openings: one panel per game session, hidden rather than dropped.
//!
//! Until 2026-09-28 [`crate::open`] built a fresh panel on every open and the release after a
//! close set the slot to `None`, so a B pressed by accident threw away the stats, the weapon, the
//! mode and the results the player had set. The fix is this type: the value is built once, on the
//! first open, and a close only moves it through [`Phase::Closing`] to [`Phase::Closed`]. The next
//! open finds it there and shows it again.
//!
//! Generic, and free of anything Windows, so the round trip is tested on the host with the real
//! [`PanelState`](ds2_build_recommender_core::PanelState) standing in for the panel.

/// Where the panel is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Never opened this game session.
    Unopened,
    /// On screen, taking presses.
    Open,
    /// Gone from the screen, holding input until every key is up. Frames waited so far.
    Closing(u32),
    /// Hidden, input released, everything the player set kept for the next open.
    Closed,
}

/// What [`Session::open`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    /// The first open: the value was built.
    Fresh,
    /// A later open: the kept value is shown again.
    Restored,
    /// It was already on screen; nothing changed.
    AlreadyOpen,
}

/// One value kept for the whole game session, and whether it is on screen.
pub struct Session<T> {
    value: Option<T>,
    phase: Phase,
}

impl<T> Default for Session<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Session<T> {
    /// Nothing built yet.
    pub const fn new() -> Self {
        Self {
            value: None,
            phase: Phase::Unopened,
        }
    }

    /// Show the panel. `fresh` builds it only if it has never existed; a closed one -- or one still
    /// releasing its hold -- is shown again as it was left.
    pub fn open(&mut self, fresh: impl FnOnce() -> T) -> Opened {
        if self.phase == Phase::Open {
            return Opened::AlreadyOpen;
        }
        self.phase = Phase::Open;
        if self.value.is_some() {
            Opened::Restored
        } else {
            self.value = Some(fresh());
            Opened::Fresh
        }
    }

    /// Take it off the screen. The value stays.
    pub fn close(&mut self) {
        if self.phase == Phase::Open {
            self.phase = Phase::Closing(0);
        }
    }

    /// One frame of a close: `released` is whether every key and button is up. Returns `true` on
    /// the frame the hold should be let go, which is when the panel becomes [`Phase::Closed`].
    pub fn tick_closing(&mut self, released: bool, deadline: u32) -> bool {
        let Phase::Closing(frames) = self.phase else {
            return false;
        };
        if released || frames >= deadline {
            self.phase = Phase::Closed;
            true
        } else {
            self.phase = Phase::Closing(frames + 1);
            false
        }
    }

    /// Where it is in its life.
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// The kept value, open or not.
    pub fn value_mut(&mut self) -> Option<&mut T> {
        self.value.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ds2_build_recommender_core::{Mode, Objective, PanelState};

    #[test]
    fn a_close_and_reopen_keeps_everything_the_player_set() {
        let mut session: Session<PanelState> = Session::new();
        assert_eq!(session.open(PanelState::default), Opened::Fresh);
        let state = session.value_mut().unwrap();
        state.set_stat(0, 40);
        state.set_sl_override(Some(150));
        state.choose_weapon("Claymore");
        state.mode = Mode::SimilarBuilds;
        state.objective = Objective::Bleed;
        state.weapons_for.window_s = 1.5;
        state.similar_k = 80;
        let before = state.clone();

        // B: closing, then every key up, then closed.
        session.close();
        assert_eq!(session.phase(), Phase::Closing(0));
        assert!(!session.tick_closing(false, 90));
        assert!(session.tick_closing(true, 90));
        assert_eq!(session.phase(), Phase::Closed);

        let mut built_again = false;
        let opened = session.open(|| {
            built_again = true;
            PanelState::default()
        });
        assert_eq!(opened, Opened::Restored);
        assert!(!built_again, "a reopen must not build a fresh panel");
        assert_eq!(session.value_mut().unwrap(), &before);
    }

    #[test]
    fn a_reopen_during_the_release_restores_too_and_a_second_open_is_ignored() {
        let mut session: Session<u32> = Session::new();
        session.open(|| 7);
        *session.value_mut().unwrap() = 9;
        session.close();
        assert_eq!(session.open(|| 0), Opened::Restored);
        assert_eq!(session.open(|| 0), Opened::AlreadyOpen);
        assert_eq!(session.value_mut(), Some(&mut 9));
    }

    #[test]
    fn the_hold_is_let_go_at_the_deadline_even_with_a_key_down() {
        let mut session: Session<()> = Session::new();
        session.open(|| ());
        session.close();
        for _ in 0..3 {
            assert!(!session.tick_closing(false, 3));
        }
        assert!(session.tick_closing(false, 3));
        assert!(!session.tick_closing(false, 3), "only once");
    }
}
