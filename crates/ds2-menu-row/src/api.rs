//! The registry: what another crate fills in to put a row on a pause-menu tab.
//!
//! # Why registration and not a feature flag
//!
//! Everything this crate does was built for exactly one row -- quit to desktop -- and every piece
//! of it was a constant: one action id, one element id, one caption, one icon. A second row would
//! have been a second copy of all four, and a third would have been a third. So the constants
//! became a [`RowSpec`] and the four hooks became loops.
//!
//! The first consumer is `ds2-loader`, which registers the quit-to-desktop row the same way any
//! other crate would. That is deliberate: if the API were only good enough for someone else's row
//! and not for our own, the difference would be invisible until someone else tried it.
//!
//! # The ceilings are the game's, and they are not about display
//!
//! Two of the three bounds on "how many rows" are hard, and neither of them fails as a missing row:
//!
//! | bound | value | what happens above it |
//! |---|---|---|
//! | the tab's item vector | [`ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY`] = 5 | the game's own `panic("out of memory.")` |
//! | the tab's cell namer list | [`ds2_rva::FE_SCENE_NAMER_LIST_CAPACITY`] = 6 | a write through a null; this repo crashed the game at `0x141bee1c4` finding out |
//! | the layout's child count | a `u16` this crate substitutes | nothing -- ours to raise |
//!
//! The item vector is the binding one, and it is per TAB and counts the rows the game shipped. The
//! quit tab ships three, so [`MAX_ADDED_ROWS`] is two. [`add_row`] refuses the third at
//! REGISTRATION -- before anything is hooked, with a value in the error -- because the alternative
//! is finding out during a menu open, inside the game's allocator.
//!
//! # What is not here
//!
//! **Only the quit tab is measured.** Every tab has the same two hook points (`ctor` calls
//! builder-then-namer six times: `0x1400a42c0`, `0x1400a4381`/`0x1400a4393`, ...), so nothing about
//! the design is quit-specific -- but a tab needs its base path, container definition and shipped
//! child ids read out of the `.flo` before a row can be put on it, and three of the six tabs are
//! not even in this document. [`Tab`] therefore has one variant, and gains one per tab measured.
//!
//! **Rows are appended, not inserted.** A new row goes below the shipped ones, in registration
//! order. Inserting between shipped rows means moving their `y` down, and the shipped ladder is
//! not evenly spaced (`10.60`, `55.90`, `103.90` -- steps of `45.30` and `48.00`), so a clean
//! recomputation would visibly move rows nobody asked to move. That is its own arm.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// A pause-menu tab a row can be added to.
///
/// One variant, because one tab has been measured. See the module docs: the machinery is not
/// quit-specific, the DATA is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Tab {
    /// The tab carrying Game Options, Screen Options and Quit Game -- built by
    /// [`ds2_rva::FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS`].
    Quit,
}

impl Tab {
    /// Rows the game itself puts on this tab.
    pub const fn shipped_rows(self) -> usize {
        match self {
            Tab::Quit => ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len(),
        }
    }

    /// Rows that can still be added, before any registration.
    ///
    /// The item vector is the binding ceiling rather than the namer list: capacity `5` against a
    /// namer capacity of `6`, so the vector runs out first on any tab shipping three or more.
    pub const fn capacity(self) -> usize {
        ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY - self.shipped_rows()
    }
}

/// Most rows this crate can add to any one tab.
///
/// Two, on the quit tab: five slots in the item vector, three of them already spoken for. It is a
/// `const` because the [`Container`](crate::layout) that carries the added records is a fixed-size
/// struct -- there is no reason to grow an allocation for a bound the game caps at five.
pub const MAX_ADDED_ROWS: usize =
    ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY - ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len();

/// A colour laid over a row's icon.
///
/// `strength` is how far the icon is pushed from white toward `rgb`, out of `255`. The colour
/// MULTIPLIES -- the game's own greyed-out state is `ff808080` on this exact machinery -- so white
/// is the identity and a fraction of a hue is meaningful.
///
/// **The scale is not perceptual.** Measured on one glyph over the pause menu's background: `26`
/// (a tenth) read as no tint at all, `77` read as a tenth, `255` read as a re-skin. They fit
/// `perceived ~= (linear - 0.20) / 0.80`. See [`ds2_rva::FLO_ADDED_ROW_TINT_STRENGTH`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tint {
    pub rgb: [u8; 3],
    pub strength: u8,
}

impl Tint {
    /// The four bytes a transform block wants, in memory order: R, G, B, A.
    pub const fn bytes(self) -> [u8; 4] {
        [
            toward_white(self.rgb[0], self.strength),
            toward_white(self.rgb[1], self.strength),
            toward_white(self.rgb[2], self.strength),
            0xff,
        ]
    }

    /// Whether this tint needs [`ds2_rva::FLO_TRANSFORM_COLOUR_RGB`] as well as
    /// [`ds2_rva::FLO_TRANSFORM_COLOUR_LIVE`]: it does unless the mix came out white.
    pub const fn flags(self) -> u32 {
        let [r, g, b, _] = self.bytes();
        if r == 0xff && g == 0xff && b == 0xff {
            0
        } else {
            ds2_rva::FLO_TRANSFORM_COLOUR_LIVE | ds2_rva::FLO_TRANSFORM_COLOUR_RGB
        }
    }
}

/// Mix one channel `strength/255` of the way from white toward `channel`.
const fn toward_white(channel: u8, strength: u8) -> u8 {
    (255 - ((255 - channel as u16) * strength as u16) / 255) as u8
}

/// What a caller fills in to ask for a row.
#[derive(Clone, Copy)]
pub struct RowSpec {
    /// Which tab the row goes on.
    pub tab: Tab,
    /// The row's caption.
    ///
    /// `'static` because `FeElement::setText` only READS the string -- the caption is handed over
    /// as a pointer into this DLL and the game never owns it. Anything longer than seven UTF-16
    /// units uses the out-of-line string layout, which is why it cannot be a temporary.
    pub caption: &'static str,
    /// The `.flo` definition index of the glyph to draw, cloned rather than authored.
    ///
    /// [`ds2_rva::FLO_QUIT_ICON_DEFINITION`] is the Quit Game glyph on its own. Any nested
    /// definition in the pause menu's document works; a definition carrying an element id will put
    /// that id inside the row, which is a collision waiting to happen unless the caller knows
    /// otherwise.
    pub icon: u32,
    /// A colour over that glyph, or `None` to draw it as the game does.
    ///
    /// Worth setting whenever the icon is one a shipped row already uses, which is the usual case:
    /// there are ten icons in this document and all ten already mean something.
    pub tint: Option<Tint>,
    /// What pressing the row does.
    ///
    /// Called from the tab's dispatch detour, on the game thread, with the menu still up. The
    /// original dispatch is NOT called for a registered row -- the action id is outside the range
    /// the game's own `switch` handles, so there is nothing to call.
    pub on_confirm: fn(),
}

/// A row that has been accepted. Returned so a caller can tell its rows apart in a log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowId(pub usize);

/// Why a row was not accepted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddRowError {
    /// The tab has no slot left. `added` rows are already registered and the game's own item
    /// vector holds `shipped + added` of a possible
    /// [`ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY`].
    TabFull {
        tab: Tab,
        shipped: usize,
        added: usize,
        capacity: usize,
    },
    /// [`crate::install`] has already run. The hooks read the registry once, when the menu is
    /// built; a row registered afterwards would be a row that silently never appears.
    AlreadyInstalled,
    /// The registry's lock was poisoned by a panic in another caller.
    Poisoned,
}

impl std::fmt::Display for AddRowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddRowError::TabFull {
                tab,
                shipped,
                added,
                capacity,
            } => write!(
                f,
                "{tab:?} is full: {shipped} shipped + {added} added = {capacity}, and the game's \
                 item vector panics above {capacity}"
            ),
            AddRowError::AlreadyInstalled => {
                write!(f, "install() has already read the registry")
            }
            AddRowError::Poisoned => write!(f, "the registry lock is poisoned"),
        }
    }
}

/// A registered row, with everything the four hooks need already resolved.
#[derive(Clone, Copy)]
pub(crate) struct Row {
    pub(crate) tab: Tab,
    /// Index among the rows added to this tab: `0` is the first one below the shipped rows.
    pub(crate) slot: usize,
    /// The action id the item vector carries and the dispatch matches on.
    pub(crate) action: u32,
    /// The element id the layout record carries and the namer resolves.
    pub(crate) row_id: u32,
    /// The element id of the row's caption mark.
    pub(crate) label_id: u32,
    pub(crate) caption: &'static str,
    pub(crate) icon: u32,
    pub(crate) tint: Option<Tint>,
    pub(crate) on_confirm: fn(),
}

impl Row {
    /// Where this row's record goes, in the container's own coordinates.
    pub(crate) fn row_xy(&self) -> (f32, f32) {
        let (x, y) = ds2_rva::FLO_ADDED_ROW_XY;
        (x, y + ds2_rva::FLO_ROW_PITCH * self.slot as f32)
    }

    /// Where this row's caption mark goes. A different pitch from the row's, because the two
    /// shipped series step by different amounts.
    pub(crate) fn mark_xy(&self) -> (f32, f32) {
        let (x, y) = ds2_rva::FLO_ADDED_MARK_XY;
        (x, y + ds2_rva::FLO_MARK_PITCH * self.slot as f32)
    }
}

static ROWS: Mutex<Vec<Row>> = Mutex::new(Vec::new());
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Register a row. Call before [`crate::install`].
///
/// Refuses rather than truncates: a tab with no slot left comes back as
/// [`AddRowError::TabFull`] carrying the numbers, because the alternative is the game's own
/// allocator panic during a menu open.
pub fn add_row(spec: RowSpec) -> Result<RowId, AddRowError> {
    if INSTALLED.load(Ordering::Acquire) {
        return Err(AddRowError::AlreadyInstalled);
    }
    let mut rows = ROWS.lock().map_err(|_| AddRowError::Poisoned)?;
    let slot = rows.iter().filter(|row| row.tab == spec.tab).count();
    if slot >= spec.tab.capacity() {
        return Err(AddRowError::TabFull {
            tab: spec.tab,
            shipped: spec.tab.shipped_rows(),
            added: slot,
            capacity: ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY,
        });
    }
    let row = Row {
        tab: spec.tab,
        slot,
        action: ds2_rva::FE_INGAME_MENU_ACTION_BASE + slot as u32,
        row_id: ds2_rva::FLO_ADDED_ROW_IDS[slot],
        label_id: ds2_rva::FLO_ADDED_LABEL_IDS[slot],
        caption: spec.caption,
        icon: spec.icon,
        tint: spec.tint,
        on_confirm: spec.on_confirm,
    };
    let id = RowId(rows.len());
    rows.push(row);
    Ok(id)
}

/// The rows registered for a tab, in registration order.
pub(crate) fn rows_for(tab: Tab) -> Vec<Row> {
    match ROWS.lock() {
        Ok(rows) => rows.iter().filter(|row| row.tab == tab).copied().collect(),
        Err(_) => Vec::new(),
    }
}

/// The row a dispatched action belongs to.
pub(crate) fn row_for_action(action: u32) -> Option<Row> {
    match ROWS.lock() {
        Ok(rows) => rows.iter().find(|row| row.action == action).copied(),
        Err(_) => None,
    }
}

/// Close the registry. Called by [`crate::install`] before it reads anything, so a row registered
/// from another thread mid-install cannot be half-applied.
pub(crate) fn seal() {
    INSTALLED.store(true, Ordering::Release);
}

/// Whether anything is registered at all. `install` has nothing to do if not, and says so rather
/// than patching six sites for no rows.
pub(crate) fn any() -> bool {
    ROWS.lock().map(|rows| !rows.is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ceiling is the game's, and it is the ITEM VECTOR rather than the namer list -- the
    /// vector runs out first on any tab shipping three or more rows.
    #[test]
    fn the_ceiling_is_the_item_vector() {
        assert_eq!(MAX_ADDED_ROWS, 2);
        assert_eq!(Tab::Quit.capacity(), MAX_ADDED_ROWS);
        assert_eq!(Tab::Quit.shipped_rows(), 3);
        assert!(
            Tab::Quit.shipped_rows() + MAX_ADDED_ROWS <= ds2_rva::FE_SCENE_NAMER_LIST_CAPACITY,
            "the namer list must also hold every row the item vector allows"
        );
    }

    /// Every row needs an id of its own, and enough of them for the ceiling.
    #[test]
    fn there_are_enough_ids_for_the_ceiling() {
        assert!(ds2_rva::FLO_ADDED_ROW_IDS.len() >= MAX_ADDED_ROWS);
        assert!(ds2_rva::FLO_ADDED_LABEL_IDS.len() >= MAX_ADDED_ROWS);
        for slot in 0..MAX_ADDED_ROWS {
            for other in 0..MAX_ADDED_ROWS {
                if slot != other {
                    assert_ne!(
                        ds2_rva::FLO_ADDED_ROW_IDS[slot],
                        ds2_rva::FLO_ADDED_ROW_IDS[other]
                    );
                    assert_ne!(
                        ds2_rva::FLO_ADDED_LABEL_IDS[slot],
                        ds2_rva::FLO_ADDED_LABEL_IDS[other]
                    );
                }
            }
            // And no added id may collide with one the container already carries.
            assert!(!ds2_rva::FLO_QUIT_TAB_CHILD_IDS.contains(&ds2_rva::FLO_ADDED_ROW_IDS[slot]));
            assert!(!ds2_rva::FLO_QUIT_TAB_CHILD_IDS.contains(&ds2_rva::FLO_ADDED_LABEL_IDS[slot]));
        }
    }

    /// Action ids stay outside the range the game's own dispatch handles, for every slot.
    #[test]
    fn every_action_is_outside_the_games_range() {
        for slot in 0..MAX_ADDED_ROWS {
            let action = ds2_rva::FE_INGAME_MENU_ACTION_BASE + slot as u32;
            assert!(action > 0xd, "{action:#x} is inside the shipped switch");
        }
    }

    /// A tint mixes toward white, and white needs no flags at all.
    #[test]
    fn a_white_mix_needs_no_flags() {
        let none = Tint {
            rgb: [0xff, 0xff, 0xff],
            strength: 255,
        };
        assert_eq!(none.bytes(), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(none.flags(), 0);
        let red = Tint {
            rgb: [0xff, 0x64, 0x50],
            strength: 120,
        };
        assert_eq!(red.bytes(), [0xff, 0xb7, 0xad, 0xff]);
        assert_eq!(
            red.flags(),
            ds2_rva::FLO_TRANSFORM_COLOUR_LIVE | ds2_rva::FLO_TRANSFORM_COLOUR_RGB
        );
    }

    /// Rows step down by the pitch the shipped rows step by, and the mark keeps its own.
    #[test]
    fn each_slot_steps_down_one_row() {
        let row = |slot| Row {
            tab: Tab::Quit,
            slot,
            action: 0,
            row_id: 0,
            label_id: 0,
            caption: "",
            icon: 0,
            tint: None,
            on_confirm: || {},
        };
        assert_eq!(row(0).row_xy(), ds2_rva::FLO_ADDED_ROW_XY);
        assert_eq!(row(0).mark_xy(), ds2_rva::FLO_ADDED_MARK_XY);
        let step = row(1).row_xy().1 - row(0).row_xy().1;
        assert!((step - ds2_rva::FLO_ROW_PITCH).abs() < 0.01);
        // The mark stays below its own row at every slot, the way all three shipped pairs do.
        for slot in 0..MAX_ADDED_ROWS {
            assert!(row(slot).mark_xy().1 > row(slot).row_xy().1);
        }
    }
}
