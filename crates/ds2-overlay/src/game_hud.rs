//! Whether the game has its own in-world player HUD -- the HP and stamina bar -- on screen.
//!
//! Every glyph this workspace draws beside the HUD asks [`game_hud_visible`] and draws nothing
//! while it says no: on the title screen, while loading, with a menu or the bonfire open, and
//! whenever else the game takes its HUD down. It reads the game's own record of that, not a proxy
//! such as "a character exists":
//!
//! * `FeOperatorFrontend`, the operator that owns the HUD's scenes, is at
//!   [`ds2_rva::FRONTEND_HUD_OPERATOR_OFFSET`] of the frontend root. It is null on the title screen.
//! * Its suspended byte ([`ds2_rva::FRONTEND_HUD_SUSPENDED_OFFSET`]) is set while a menu holds the
//!   frontend.
//! * Its HUD state ([`ds2_rva::FRONTEND_HUD_STATE_OFFSET`]) is the state the game last pushed into
//!   every HUD scene: shown, fading, hidden or forced off.
//! * The HP bar's own scene carries a hide count
//!   ([`ds2_rva::FE_SCENE_LINKED_HIDE_COUNT_OFFSET`]), the game's other way of taking a scene down.
//!
//! The decision is [`decide`], a pure function tested on the host; the reads are Windows-only.

/// What was read off the game for one frame. `None` for a hop that was null or unreadable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudFields {
    /// The frontend's HUD operator exists.
    pub operator: bool,
    /// Its suspended byte.
    pub suspended: Option<u8>,
    /// Its HUD state word.
    pub state: Option<u32>,
    /// The HP bar scene's hide count.
    pub hp_hide_count: Option<i32>,
}

/// Why the HUD is, or is not, on screen. Logged on every change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudVerdict {
    /// The HUD is up.
    Shown,
    /// No `FeOperatorFrontend`: the title screen, or before the first load.
    NoOperator,
    /// A menu has suspended the frontend.
    Suspended,
    /// The HUD state is not one of the on-screen ones.
    StateHidden(u32),
    /// The game has hidden the HP bar's scene through its hide count.
    HpSceneHidden(i32),
    /// A field could not be read.
    Unreadable,
}

impl HudVerdict {
    /// The verdict as a bool: only [`HudVerdict::Shown`] lets a glyph draw.
    #[must_use]
    pub fn visible(self) -> bool {
        self == Self::Shown
    }
}

/// The states in which the game has the HUD on screen, including the two fades.
#[must_use]
pub fn state_on_screen(state: u32) -> bool {
    matches!(
        state,
        ds2_rva::FRONTEND_HUD_STATE_SHOWN
            | ds2_rva::FRONTEND_HUD_STATE_FADING_OUT
            | ds2_rva::FRONTEND_HUD_STATE_FADING_IN
    )
}

/// Decide from one frame's reads. Anything unreadable counts as not on screen.
#[must_use]
pub fn decide(fields: HudFields) -> HudVerdict {
    if !fields.operator {
        return HudVerdict::NoOperator;
    }
    let (Some(suspended), Some(state), Some(count)) =
        (fields.suspended, fields.state, fields.hp_hide_count)
    else {
        return HudVerdict::Unreadable;
    };
    if suspended != 0 {
        return HudVerdict::Suspended;
    }
    if !state_on_screen(state) {
        return HudVerdict::StateHidden(state);
    }
    if count > 0 {
        return HudVerdict::HpSceneHidden(count);
    }
    HudVerdict::Shown
}

#[cfg(windows)]
mod read {
    use core::sync::atomic::{AtomicU8, Ordering};

    use ds2_game_base::mem::{
        game_rva, safe_read_i32, safe_read_u8, safe_read_u32, safe_read_usize,
    };

    use super::{HudFields, HudVerdict, decide};
    use crate::log::log;

    fn non_null(p: usize) -> Option<usize> {
        (p != 0).then_some(p)
    }

    /// One frame's reads. Every hop goes through the fault-safe readers.
    fn fields() -> HudFields {
        let none = HudFields {
            operator: false,
            suspended: None,
            state: None,
            hp_hide_count: None,
        };
        let Ok(global) = game_rva(ds2_rva::GAME_MANAGER_IMP) else {
            return none;
        };
        // SAFETY: every read is a fault-safe probe of the game's heap; a bad pointer is `None`.
        unsafe {
            let Some(op) = safe_read_usize(global)
                .and_then(non_null)
                .and_then(|gm| safe_read_usize(gm + ds2_rva::GAME_MANAGER_FRONTEND_ROOT_OFFSET))
                .and_then(non_null)
                .and_then(|root| safe_read_usize(root + ds2_rva::FRONTEND_HUD_OPERATOR_OFFSET))
                .and_then(non_null)
            else {
                return none;
            };
            let hp_hide_count = safe_read_usize(op + ds2_rva::FRONTEND_HUD_HP_GAUGE_OFFSET)
                .and_then(non_null)
                .and_then(|scene| safe_read_usize(scene + ds2_rva::FE_SCENE_LINKED_OFFSET))
                .and_then(non_null)
                .and_then(|linked| {
                    safe_read_i32(linked + ds2_rva::FE_SCENE_LINKED_HIDE_COUNT_OFFSET)
                });
            HudFields {
                operator: true,
                suspended: safe_read_u8(op + ds2_rva::FRONTEND_HUD_SUSPENDED_OFFSET),
                state: safe_read_u32(op + ds2_rva::FRONTEND_HUD_STATE_OFFSET),
                hp_hide_count,
            }
        }
    }

    /// The last verdict logged, as a small code, so only changes reach the log.
    static LAST: AtomicU8 = AtomicU8::new(u8::MAX);

    fn code(v: HudVerdict) -> u8 {
        match v {
            HudVerdict::Shown => 0,
            HudVerdict::NoOperator => 1,
            HudVerdict::Suspended => 2,
            HudVerdict::StateHidden(_) => 3,
            HudVerdict::HpSceneHidden(_) => 4,
            HudVerdict::Unreadable => 5,
        }
    }

    pub(super) fn verdict() -> HudVerdict {
        let f = fields();
        let v = decide(f);
        if LAST.swap(code(v), Ordering::AcqRel) != code(v) {
            let state = f
                .state
                .map_or_else(|| "-".to_owned(), |s| format!("{s:#x}"));
            log(format_args!(
                "game hud {} reason={v:?} operator={} suspended={:?} state={state} hp-hide-count={:?}",
                if v.visible() { "SHOWN" } else { "HIDDEN" },
                f.operator,
                f.suspended,
                f.hp_hide_count
            ));
        }
        v
    }
}

/// Is the game's own player HUD on screen right now? `false` on the host.
///
/// Cheap enough to call from every panel's visibility check each frame. A change of answer is
/// logged once, as `ds2-overlay: game hud SHOWN ...` or `... HIDDEN reason=...`.
#[must_use]
pub fn game_hud_visible() -> bool {
    #[cfg(windows)]
    {
        read::verdict().visible()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORLD: HudFields = HudFields {
        operator: true,
        suspended: Some(0),
        state: Some(ds2_rva::FRONTEND_HUD_STATE_SHOWN),
        hp_hide_count: Some(0),
    };

    #[test]
    fn the_world_with_the_bar_up_is_shown() {
        assert_eq!(decide(WORLD), HudVerdict::Shown);
    }

    #[test]
    fn the_title_screen_has_no_operator() {
        let title = HudFields {
            operator: false,
            suspended: None,
            state: None,
            hp_hide_count: None,
        };
        assert!(!decide(title).visible());
    }

    #[test]
    fn a_menu_or_the_bonfire_hides_it() {
        let menu = HudFields {
            suspended: Some(1),
            state: Some(ds2_rva::FRONTEND_HUD_STATE_HIDDEN),
            ..WORLD
        };
        assert_eq!(decide(menu), HudVerdict::Suspended);
    }

    #[test]
    fn loading_is_the_hidden_state() {
        let loading = HudFields {
            state: Some(ds2_rva::FRONTEND_HUD_STATE_HIDDEN),
            ..WORLD
        };
        assert!(!decide(loading).visible());
        let forced = HudFields {
            state: Some(ds2_rva::FRONTEND_HUD_STATE_FORCED_OFF),
            ..WORLD
        };
        assert!(!decide(forced).visible());
    }

    #[test]
    fn both_fades_are_still_on_screen() {
        for s in [
            ds2_rva::FRONTEND_HUD_STATE_FADING_OUT,
            ds2_rva::FRONTEND_HUD_STATE_FADING_IN,
        ] {
            assert!(
                decide(HudFields {
                    state: Some(s),
                    ..WORLD
                })
                .visible()
            );
        }
    }

    #[test]
    fn a_hidden_hp_scene_hides_it() {
        assert_eq!(
            decide(HudFields {
                hp_hide_count: Some(1),
                ..WORLD
            }),
            HudVerdict::HpSceneHidden(1)
        );
    }

    #[test]
    fn an_unreadable_field_is_not_on_screen() {
        assert_eq!(
            decide(HudFields {
                state: None,
                ..WORLD
            }),
            HudVerdict::Unreadable
        );
    }
}
