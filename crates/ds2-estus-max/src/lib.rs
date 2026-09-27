//! Keep MY Estus Flask at the game's maximum on both axes: uses (what Estus Flask Shards buy) and
//! effect (what Sublime Bone Dust buys). Solo; nothing here reads or sends anything about another
//! player.
//!
//! # How
//!
//! Twice a second, on the game thread (the net session update, shared through `ds2-net-tick`), it
//! looks for a loaded character with a flask. When either axis is below the maximum it calls the
//! Emerald Herald's own setter, [`ds2_rva::ESTUS_SET_PROPERTY`], asking for
//! [`ds2_rva::ESTUS_LEVEL_ASK`] and letting the game clamp it into `EstusFlaskMaxReinforceParam`,
//! then reads both levels back with [`ds2_rva::ESTUS_GET_LEVEL`] and [`ds2_rva::ESTUS_IS_MAX`] and
//! logs what the GAME says, not what was asked. See [`ds2_rva::ESTUS_USES_MAX_SHIPPED`] for where
//! the maxima (12 and 6 as shipped) were read.
//!
//! # Why a poll rather than a load hook
//!
//! "Every load" has no single instant: the character, then its inventory, then the saved flask
//! levels arrive at different points of the load, and a raise that lands before the saved levels do
//! is overwritten by them. Checking the game's own "at max" predicate twice a second catches every
//! one of those, costs two calls when there is nothing to do, and needs no knowledge of the order.
//! A new character (a different inventory or player pointer, or none in between) starts a new load
//! and gets its own line.
//!
//! # What it does not do
//!
//! It never refills the flask. The setter already adds the charges the new uses level buys, so a
//! raise from 1 to 12 uses leaves the flask 11 charges fuller -- and a flask that is already at max
//! is never touched, so loading does not become a free refill.

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-estus-max:";

/// Ticks of the net session update between checks. It runs at 60/s, so this is twice a second.
pub const CHECK_EVERY_TICKS: u64 = 30;

/// Ticks between the first load reaching max and the reload test's return to title.
///
/// Five seconds: long enough for the load's own saves and fades to settle, short enough that a
/// run does not idle. Only read with `[estus_max] reload_test`.
pub const RELOAD_TEST_DELAY_TICKS: u64 = 300;

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{Outcome, install, set_logger, set_reload_test};

/// Both levels of the flask, as the game reports them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Levels {
    /// The uses level, [`ds2_rva::ESTUS_GET_LEVEL`] for property 0.
    pub uses: u8,
    /// The effect level, property 1.
    pub effect: u8,
    /// [`ds2_rva::ESTUS_IS_MAX`] for property 0.
    pub uses_at_max: bool,
    /// [`ds2_rva::ESTUS_IS_MAX`] for property 1.
    pub effect_at_max: bool,
    /// Charges in the flask right now.
    pub charges: u8,
}

impl Levels {
    /// Whether the game says both axes are finished.
    pub const fn at_max(&self) -> bool {
        self.uses_at_max && self.effect_at_max
    }
}

/// One look at the game.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seen {
    /// No local player or no inventory: the title screen, or a load in progress.
    NoCharacter,
    /// A character. `levels` is `None` when it has no flask (the uses level reads 0).
    Character {
        /// `(inventory, player)`. A change is a new load.
        key: (usize, usize),
        /// The flask, if there is one.
        levels: Option<Levels>,
    },
}

/// What the tick should do next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// Nothing to do and nothing new to say.
    Nothing,
    /// Log this and do nothing else.
    Say(Note),
    /// Call the setter for each axis marked `true`, then report through [`Tracker::raised`].
    Raise {
        /// Raise the uses level.
        uses: bool,
        /// Raise the effect level.
        effect: bool,
    },
}

/// A line worth writing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Note {
    /// The character went away (title screen, or between loads).
    Left,
    /// This character has no flask, so there is nothing to raise.
    NoFlask {
        /// The local player.
        player: usize,
    },
    /// Loaded already at maximum; nothing was written.
    AlreadyMax {
        /// The local player.
        player: usize,
        /// What the game reported.
        levels: Levels,
    },
    /// The setter ran. `after` is the game's read-back.
    Raised {
        /// The local player.
        player: usize,
        /// Before the setter.
        before: Levels,
        /// After it, read back.
        after: Levels,
    },
}

impl core::fmt::Display for Note {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let shipped = |f: &mut core::fmt::Formatter<'_>| {
            write!(
                f,
                " shipped_max={}/{}",
                ds2_rva::ESTUS_USES_MAX_SHIPPED,
                ds2_rva::ESTUS_EFFECT_MAX_SHIPPED
            )
        };
        match *self {
            Note::Left => write!(
                f,
                "{LOG_PREFIX} character gone -- waiting for the next load"
            ),
            Note::NoFlask { player } => write!(
                f,
                "{LOG_PREFIX} load player=0x{player:x} has no Estus Flask -- nothing to raise"
            ),
            Note::AlreadyMax { player, levels } => {
                write!(
                    f,
                    "{LOG_PREFIX} load player=0x{player:x} already at max, nothing written: \
                     readback uses={} (at_max={}) effect={} (at_max={}) charges={}",
                    levels.uses,
                    levels.uses_at_max,
                    levels.effect,
                    levels.effect_at_max,
                    levels.charges
                )?;
                shipped(f)
            }
            Note::Raised {
                player,
                before,
                after,
            } => {
                write!(
                    f,
                    "{LOG_PREFIX} {} player=0x{player:x} readback uses {}->{} (at_max={}) \
                     effect {}->{} (at_max={}) charges {}->{}",
                    if after.at_max() {
                        "RAISED to max"
                    } else {
                        "NOT AT MAX after the setter, not retrying this load"
                    },
                    before.uses,
                    after.uses,
                    after.uses_at_max,
                    before.effect,
                    after.effect,
                    after.effect_at_max,
                    before.charges,
                    after.charges
                )?;
                shipped(f)
            }
        }
    }
}

/// Per-load state: what has been said, and whether a raise that did not take has been given up on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Tracker {
    character: Option<(usize, usize)>,
    reported: bool,
    no_flask_said: bool,
    gave_up: bool,
}

impl Tracker {
    /// A tracker that has seen nothing.
    pub const fn new() -> Self {
        Self {
            character: None,
            reported: false,
            no_flask_said: false,
            gave_up: false,
        }
    }

    /// Decide what to do about one look at the game.
    pub fn observe(&mut self, seen: Seen) -> Step {
        let (key, levels) = match seen {
            Seen::NoCharacter => {
                return if self.character.take().is_some() {
                    Step::Say(Note::Left)
                } else {
                    Step::Nothing
                };
            }
            Seen::Character { key, levels } => (key, levels),
        };
        if self.character != Some(key) {
            *self = Self {
                character: Some(key),
                ..Self::new()
            };
        }
        let player = key.1;
        match levels {
            None if self.no_flask_said => Step::Nothing,
            None => {
                self.no_flask_said = true;
                Step::Say(Note::NoFlask { player })
            }
            Some(levels) if levels.at_max() => {
                if self.reported {
                    Step::Nothing
                } else {
                    self.reported = true;
                    Step::Say(Note::AlreadyMax { player, levels })
                }
            }
            Some(_) if self.gave_up => Step::Nothing,
            Some(levels) => Step::Raise {
                uses: !levels.uses_at_max,
                effect: !levels.effect_at_max,
            },
        }
    }

    /// Record a raise and return its line. A raise that did not reach the maximum is not retried
    /// until the next load, so a setter that refuses cannot fill the log.
    pub fn raised(&mut self, before: Levels, after: Levels) -> Note {
        self.reported = true;
        if !after.at_max() {
            self.gave_up = true;
        }
        Note::Raised {
            player: self.character.map_or(0, |key| key.1),
            before,
            after,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: (usize, usize) = (0x1000, 0x2000);

    fn levels(uses: u8, effect: u8) -> Levels {
        Levels {
            uses,
            effect,
            uses_at_max: uses == ds2_rva::ESTUS_USES_MAX_SHIPPED,
            effect_at_max: effect == ds2_rva::ESTUS_EFFECT_MAX_SHIPPED,
            charges: uses,
        }
    }

    fn seen(l: Option<Levels>) -> Seen {
        Seen::Character {
            key: KEY,
            levels: l,
        }
    }

    #[test]
    fn a_low_flask_is_raised_on_the_axes_that_are_low() {
        let mut t = Tracker::new();
        assert_eq!(
            t.observe(seen(Some(levels(1, 6)))),
            Step::Raise {
                uses: true,
                effect: false
            }
        );
    }

    #[test]
    fn a_maxed_flask_is_said_once_and_never_written() {
        let mut t = Tracker::new();
        assert!(matches!(
            t.observe(seen(Some(levels(12, 6)))),
            Step::Say(Note::AlreadyMax { .. })
        ));
        assert_eq!(t.observe(seen(Some(levels(12, 6)))), Step::Nothing);
    }

    #[test]
    fn a_save_that_lowers_it_again_after_the_raise_is_raised_again() {
        let mut t = Tracker::new();
        let _ = t.observe(seen(Some(levels(1, 1))));
        let note = t.raised(levels(1, 1), levels(12, 6));
        assert!(note.to_string().contains("RAISED to max"), "{note}");
        assert_eq!(
            t.observe(seen(Some(levels(3, 2)))),
            Step::Raise {
                uses: true,
                effect: true
            }
        );
    }

    #[test]
    fn a_raise_that_does_not_take_is_not_retried_until_the_next_load() {
        let mut t = Tracker::new();
        let _ = t.observe(seen(Some(levels(1, 1))));
        let note = t.raised(levels(1, 1), levels(1, 1));
        assert!(note.to_string().contains("NOT AT MAX"), "{note}");
        assert_eq!(t.observe(seen(Some(levels(1, 1)))), Step::Nothing);
        assert_eq!(t.observe(Seen::NoCharacter), Step::Say(Note::Left));
        assert!(matches!(
            t.observe(seen(Some(levels(1, 1)))),
            Step::Raise { .. }
        ));
    }

    #[test]
    fn a_different_character_is_a_new_load() {
        let mut t = Tracker::new();
        let _ = t.observe(seen(Some(levels(12, 6))));
        let other = Seen::Character {
            key: (0x1000, 0x3000),
            levels: Some(levels(12, 6)),
        };
        assert!(matches!(
            t.observe(other),
            Step::Say(Note::AlreadyMax { .. })
        ));
    }

    #[test]
    fn no_flask_is_said_once_and_nothing_is_raised() {
        let mut t = Tracker::new();
        assert!(matches!(
            t.observe(seen(None)),
            Step::Say(Note::NoFlask { .. })
        ));
        assert_eq!(t.observe(seen(None)), Step::Nothing);
    }

    #[test]
    fn the_title_screen_says_nothing_until_a_character_has_been_seen() {
        let mut t = Tracker::new();
        assert_eq!(t.observe(Seen::NoCharacter), Step::Nothing);
    }
}
