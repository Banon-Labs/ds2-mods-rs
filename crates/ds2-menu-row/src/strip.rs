//! The seventh tab's cell in the strip along the top, added to the layout document in memory.
//!
//! # The same substitution a row gets, one level up
//!
//! [`crate::layout`] hands back a copy of the quit tab's container definition with two more children
//! per added row. The tab strip is the same kind of object: definition
//! [`ds2_rva::FLO_TAB_STRIP_DEFINITION`] with [`ds2_rva::FLO_TAB_STRIP_CHILDREN`] children, the last
//! six of which are the tab cells, and a child count that is also the display-list capacity
//! (`FUN_140b6bd80` refuses to attach past it). So a seventh tab is three more records -- a cell,
//! the panel that cell selects, and the hexagon it is drawn on -- and a count of twenty-one.
//!
//! The record is a copy of the sixth cell's with three fields changed:
//!
//! | field | from | to |
//! |---|---|---|
//! | element id | `0x1eaba5` | [`ds2_rva::FLO_ADDED_TAB_ID`] |
//! | transform x | `265.05` | `+ `[`ds2_rva::FLO_TAB_PITCH`] |
//! | depth | `89` | `+ `[`ds2_rva::FLO_TAB_DEPTH_PITCH`] |
//!
//! Its definition is left as the shipped [`ds2_rva::FLO_TAB_STRIP_CELL_DEFINITION`], unchanged and
//! uncopied, because a tab cell authors only its selection highlight. That is not a convenience --
//! it is the reason the seventh tab drew no icon for three commits. A cell holds two copies of one
//! highlight shape and nothing in it carries an element id, so no glyph is bound there and none can
//! be. The icon is a third record, sliced out of the strip's own plate by [`crate::icon`].
//!
//! # The second record, which is the tab itself
//!
//! A cell is the icon in the strip. What the tab shows when it is selected is a separate child of
//! the same container -- [`ds2_rva::FLO_TAB_STRIP_PANEL`], the System tab's subtree -- and the
//! seventh tab gets one of those too, cloned from it with two fields changed:
//!
//! | field | from | to |
//! |---|---|---|
//! | element id | [`ds2_rva::FLO_TAB_STRIP_PANEL_ID`] | [`ds2_rva::FLO_ADDED_TAB_SUBTREE_ID`] |
//! | definition | [`ds2_rva::FLO_TAB_SUBTREE_DEFINITION`] | [`ds2_rva::FLO_ADDED_TAB_SUBTREE_DEFINITION`] |
//! | transform x | `-5.9` | `+ `[`ds2_rva::FLO_TAB_PITCH`] |
//!
//! That definition is [`crate::layout`]'s copy, and it is what the seventh tab's rows hang under
//! instead of the System tab's. Sharing the System tab's subtree is what the first run did, and it
//! drew the System tab's three captions on top of the seventh tab's first three rows: a row record
//! is a grid cell and a namer can decline to name it, a caption is a plain child and nothing can
//! decline to draw it.
//!
//! The clone is inserted beside its template rather than appended after the cells. Depth is not
//! read back for a nested-definition record, so two subtrees draw in array order, and a tab's panel
//! placed after the strip's own cells would draw over them.
//!
//! # Why every transform is copied before it moves
//!
//! A record's `+0x08` is a pointer to a transform block in the document, and two records pointing at
//! one block are one x between them. Everything this module moves therefore gets a copy of its
//! block first, exactly as [`crate::layout`] does for an added row.
//!
//! The added subtree moves by the same [`ds2_rva::FLO_TAB_PITCH`] the cell does, and the first run
//! of the seventh tab is what proved it has to. The subtree was cloned sharing the System tab's
//! transform on the reasoning that it is the same panel with different rows in it -- which is true
//! about its contents and wrong about its position. A tab's panel drops out from under that tab's
//! own hexagon: the System tab's sits at `-5.9 + 288.8 = 282.9`, which is `17.85` right of the
//! sixth cell's `265.05`, and the seventh tab's rows rolled out under the sixth tab's icon.
//!
//! The added icon copies its block without moving it. The slice [`crate::icon`] builds carries the
//! offset inside its own quad, so the copy exists for the colour written into it rather than for a
//! position -- see the hexagon's section below.
//!
//! # What makes this safe to be wrong about
//!
//! A definition index is a number, and `0x0271` on a document this was not read from is some other
//! container. The substitution happens only when the definition the game returned has exactly
//! [`ds2_rva::FLO_TAB_STRIP_CHILDREN`] children whose last six ids are
//! [`ds2_rva::FLO_TAB_STRIP_CELL_IDS`], in order, and whose [`ds2_rva::FLO_TAB_STRIP_PANEL`] is a
//! record naming [`ds2_rva::FLO_TAB_SUBTREE_DEFINITION`] under [`ds2_rva::FLO_TAB_STRIP_PANEL_ID`].
//! Anything else passes through untouched and says so.
//!
//! # The third record, which is the hexagon
//!
//! A cell draws a highlight and a subtree draws rows; neither draws the picture of a tab. That is
//! [`ds2_rva::FLO_TAB_STRIP_PLATE`], one textured quad holding all six hexagons and all six glyphs,
//! and a seventh needs one more quad beside it. The record is a copy of the plate's with two fields
//! changed:
//!
//! | field | from | to |
//! |---|---|---|
//! | shape index | [`ds2_rva::FLO_TAB_PLATE_SHAPE`] | [`ds2_rva::FLO_ADDED_TAB_ICON_SHAPE`] |
//! | depth | `60` | [`ds2_rva::FLO_ADDED_TAB_ICON_DEPTH`] |
//! | transform | the plate's block | a copy of it, tinted [`ds2_rva::FLO_ADDED_TAB_ICON_HUE`] |
//!
//! Its position is not in that copy and never was -- the offset that moves the hexagon lives in the
//! quad [`crate::icon`] builds rather than in the record, so the block goes back byte for byte
//! except for the colour. The copy exists only so the colour has somewhere private to land: the
//! pointer the record arrives with is the plate's, and the plate is the six shipped hexagons.
//!
//! The tint is the only thing distinguishing this tab from the sixth. Its art is the sixth tab's,
//! shifted one [`ds2_rva::FLO_TAB_PITCH`], because the atlas holds six hexagons and this mod ships
//! no texture.
//!
//! This record goes in only when [`crate::icon::armed`] says the shape lookup is hooked. Without
//! it there is nothing for the index to resolve to, and the `RB` prompt moved below stays where the
//! game put it -- a seventh tab with no hexagon, which is what three commits shipped, rather than a
//! gap where that prompt used to be.
//!
//! # The fourth record, which is the rest of the hexagon
//!
//! [`ds2_rva::FLO_TAB_STRIP_END_CAP`] was read as a cap at the end of the strip and moved one pitch
//! along to get it out from under the added hexagon. It is not a cap. Its record sits at `271.05`
//! against a sixth tab that begins at `1.10 + 5 * `[`ds2_rva::FLO_TAB_PITCH`]` = 271.10`, and its
//! art is `78.35` wide against a cell highlight's `77.70` -- it is the sixth tab's own hexagon,
//! drawn over the plate's last period the way the plate's own periods are drawn under the cells.
//! Moving it took the sixth tab's button off the screen, which is the one thing a flourish at the
//! end of a strip could not have done.
//!
//! So the original stays where the game put it and the seventh tab takes a copy: the same record,
//! with its transform block copied and one pitch added. The seventh tab then wears the sixth tab's
//! hexagon in the same two layers the sixth tab does -- the plate's period underneath, from the
//! slice above, and this plate over it.
//!
//! # What a run has shown, and what it has not
//!
//! The cell is established. A run logged `strip cell added id=0x1eaba8 x=319.05 children=18->19`
//! and `strip count raised tabs=6 -> items=7` with no mismatch, and the seventh tab drew its rows.
//!
//! The subtree record is established and its position was wrong: a second run drew the seventh
//! tab's rows under the sixth tab's hexagon, which is what the transform move above now fixes.
//!
//! The missing tab button is established and so is its cause: two runs drew five hexagons, a gap
//! and the seventh tab's own, which is what moving the sixth tab's plate out from under the
//! seventh does. The copy that replaces that move has not been in front of a running game.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// Children the replacement carries: the shipped eighteen, the seventh tab's subtree, its icon, a
/// copy of the hexagon plate that only the sixth tab has, and its cell.
///
/// The last two are only added when [`crate::icon`] is in, but the array is sized for them either
/// way: a slot is twenty bytes and a conditional length is a second thing to get wrong. The count
/// written into the definition is [`Plan::children`], which is the number actually filled.
const CHILDREN: usize = ds2_rva::FLO_TAB_STRIP_CHILDREN + 4;

/// Transform blocks the replacement owns: the added cell's, the added panel's, the added hexagon
/// plate's, the `RB` label's and the added icon's.
///
/// A record's `+0x08` points at a block in the document, and two records pointing at one block are
/// one position -- and one colour -- between them, so anything this moves or tints needs a copy
/// first.
///
/// The icon is here for the colour rather than for a move. It does not move: the slice
/// [`crate::icon`] builds carries its own offset, and this copy goes back verbatim except for the
/// tint. What it cannot do is write that tint through the block it arrives pointing at, which is
/// the plate's -- the one record holding all six shipped hexagons. A colour written there repaints
/// the whole strip.
const MOVED: usize = 5;

/// Which of [`Strip::transforms`] belongs to what.
const CELL_TRANSFORM: usize = 0;
const END_CAP_TRANSFORM: usize = 1;
const RB_LABEL_TRANSFORM: usize = 2;
const PANEL_TRANSFORM: usize = 3;
const ICON_TRANSFORM: usize = 4;

/// The colour the seventh tab's hexagon is drawn in.
///
/// The same [`crate::Tint`] a registered row's icon takes, on the same machinery, so the hue and
/// the byte order it is laid down in are the ones a run already settled -- what differs is the
/// strength, and [`ds2_rva::FLO_ADDED_TAB_ICON_TINT_STRENGTH`] says why.
const TAB_ICON_TINT: crate::Tint = crate::Tint {
    rgb: ds2_rva::FLO_ADDED_TAB_ICON_HUE,
    strength: ds2_rva::FLO_ADDED_TAB_ICON_TINT_STRENGTH,
};

/// Where the sixth tab's own hexagon starts, which is what the end cap turned out to be.
///
/// The plate `0x0268` draws across `1.10..337.60` and the end cap's record sits at `271.05`.
/// `1.10 + 5 * FLO_TAB_PITCH` is `271.10`: the cap begins on the sixth tab's boundary to within
/// half a tenth, and its art is `78.35` wide against the cell highlight's `77.70`. That is a tab's
/// hexagon, not a flourish at the end of a strip, and moving it is what took the sixth tab's button
/// off the screen for a commit.
const SIXTH_TAB_LEFT: f32 = 1.10 + 5.0 * ds2_rva::FLO_TAB_PITCH;

/// Where each added record ends up, and how many records the definition then claims.
///
/// Built by one walk over the shipped array rather than by arithmetic on insertion points, because
/// there are two insertions and the second one's index depends on the first. Every index below is a
/// slot in the replacement, so the moves that follow can address the shipped furniture after it has
/// shifted without recomputing by how much.
struct Plan {
    /// Slot the seventh tab's subtree record goes in: directly after the template it is cloned
    /// from, so the two tabs' panels are adjacent and both precede the strip's cells.
    panel: usize,
    /// Slot the seventh tab's hexagon goes in: directly after the plate it is sliced out of.
    /// [`usize::MAX`] when [`crate::icon`] is not in, and then nothing is written there.
    icon: usize,
    /// Slot the copy of the sixth tab's own hexagon plate goes in: directly after the record it is
    /// cloned from, so the seventh tab wears the same art the sixth does. [`usize::MAX`] on the
    /// same condition as [`Plan::icon`], because both exist only to draw a seventh hexagon.
    end_cap: usize,
    /// Slot the seventh cell goes in: last, after the six the game ships.
    cell: usize,
    /// Where each shipped record ended up.
    moved: [usize; ds2_rva::FLO_TAB_STRIP_CHILDREN],
    /// How many slots are filled, which is the count the definition carries -- and, because the
    /// game reads one field for both, the display list's capacity.
    children: usize,
}

impl Plan {
    /// The plan for a strip that does or does not get an icon.
    const fn new(with_icon: bool) -> Self {
        let mut plan = Plan {
            panel: 0,
            icon: usize::MAX,
            end_cap: usize::MAX,
            cell: 0,
            moved: [0; ds2_rva::FLO_TAB_STRIP_CHILDREN],
            children: 0,
        };
        let mut shipped = 0;
        let mut at = 0;
        while shipped < ds2_rva::FLO_TAB_STRIP_CHILDREN {
            plan.moved[shipped] = at;
            at += 1;
            if shipped == ds2_rva::FLO_TAB_STRIP_PANEL {
                plan.panel = at;
                at += 1;
            }
            if with_icon && shipped == ds2_rva::FLO_TAB_STRIP_PLATE {
                plan.icon = at;
                at += 1;
            }
            if with_icon && shipped == ds2_rva::FLO_TAB_STRIP_END_CAP {
                plan.end_cap = at;
                at += 1;
            }
            shipped += 1;
        }
        plan.cell = at;
        plan.children = at + 1;
        plan
    }
}

/// A replacement strip definition, its child records, and the transform blocks the moved records
/// point at -- one allocation, so the pointers between them cannot outlive their targets.
#[repr(C, align(16))]
struct Strip {
    definition: [u8; ds2_rva::FLO_DEFINITION_STRIDE],
    records: [u8; ds2_rva::FLO_RECORD_STRIDE * CHILDREN],
    transforms: [u8; ds2_rva::FLO_TRANSFORM_SIZE * MOVED],
    /// The shipped records exactly as the game had them, so a cache hit can be checked rather than
    /// assumed. Not `records`, which has its added entries in it and would never match.
    shipped: [u8; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_TAB_STRIP_CHILDREN],
}

/// Substitutions already built, as `(definition the game returned, definition we return)`.
static BUILT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Most substitutions to keep, for the same reason [`crate::layout`] has one.
const MAX_BUILT: usize = 64;

static SUBSTITUTED: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);

/// Whether the definition the game returned is the strip this crate read.
///
/// # Safety
///
/// `definition` must be a live `.flo` definition.
unsafe fn is_the_strip(definition: *const u8) -> Option<*const u8> {
    // SAFETY: the caller guarantees a definition, and these are the two fields the game reads first.
    let (count, children) = unsafe {
        (
            definition
                .add(ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET)
                .cast::<u16>()
                .read() as usize,
            definition
                .add(ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET)
                .cast::<*const u8>()
                .read(),
        )
    };
    if count != ds2_rva::FLO_TAB_STRIP_CHILDREN || (children as usize) < 0x1_0000 {
        return None;
    }
    for (offset, id) in ds2_rva::FLO_TAB_STRIP_CELL_IDS.iter().enumerate() {
        let index = ds2_rva::FLO_TAB_STRIP_FIRST_CELL + offset;
        // SAFETY: `index < count`, and the caller guarantees that many records are live.
        let found = unsafe {
            children
                .add(index * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_ID_OFFSET)
                .cast::<u32>()
                .read()
        };
        if found != *id {
            return None;
        }
    }
    // The subtree template, checked on both of the fields the clone changes. A record at this index
    // naming something else is a document this was not read from, and cloning it would hang the
    // seventh tab's rows off whatever it happens to be.
    let at = ds2_rva::FLO_TAB_STRIP_PANEL * ds2_rva::FLO_RECORD_STRIDE;
    // SAFETY: the index is below `count`, which the caller guarantees is live at `children`.
    let (id, definition) = unsafe {
        (
            children
                .add(at + ds2_rva::FLO_RECORD_ID_OFFSET)
                .cast::<u32>()
                .read(),
            children
                .add(at + ds2_rva::FLO_RECORD_DEFINITION_OFFSET)
                .cast::<u16>()
                .read() as u32,
        )
    };
    if id != ds2_rva::FLO_TAB_STRIP_PANEL_ID || definition != ds2_rva::FLO_TAB_SUBTREE_DEFINITION {
        return None;
    }
    // The three records the icon work touches, each on the definition index it names. The plate is
    // what the seventh hexagon is cloned from; the other two are the furniture standing where that
    // hexagon goes. A record at one of these indices naming something else is a document this was
    // not read from, and moving it would move whatever it happens to be.
    for (index, want) in [
        (ds2_rva::FLO_TAB_STRIP_PLATE, ds2_rva::FLO_TAB_PLATE_SHAPE),
        (
            ds2_rva::FLO_TAB_STRIP_END_CAP,
            ds2_rva::FLO_TAB_STRIP_END_CAP_DEFINITION,
        ),
        (
            ds2_rva::FLO_TAB_STRIP_RB_LABEL,
            ds2_rva::FLO_TAB_STRIP_RB_LABEL_DEFINITION,
        ),
    ] {
        // SAFETY: the index is below `count`, which the caller guarantees is live at `children`.
        let found = unsafe {
            children
                .add(index * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_DEFINITION_OFFSET)
                .cast::<u16>()
                .read() as u32
        };
        if found != want {
            return None;
        }
    }
    Some(children)
}

/// Copy the block a record points at into `which` of `transforms`, point the record at the copy,
/// and answer where the copy is.
///
/// The copy is what keeps an edit local. Two records pointing at one block are one position and one
/// colour between them, so writing through the document's own block edits the shipped furniture for
/// every other record -- and every other document -- that shares it. Both callers below are that
/// case: the furniture this moves is pointed at by records it must not move, and the hexagon this
/// tints arrives pointing at the plate all six shipped hexagons are drawn through.
fn own_transform(
    records: &mut [u8],
    transforms: &mut [u8],
    slot: usize,
    which: usize,
    what: &str,
) -> Option<usize> {
    let at = slot * ds2_rva::FLO_RECORD_STRIDE;
    let source = u64::from_le_bytes(
        records[at + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET..][..8]
            .try_into()
            .ok()?,
    ) as usize;
    if source < 0x1_0000 {
        log(format_args!(
            "{LOG_PREFIX} strip REFUSED reason=transform-not-a-pointer what={what} \
             at=0x{source:016x}"
        ));
        return None;
    }
    let block = which * ds2_rva::FLO_TRANSFORM_SIZE;
    // SAFETY: a record's `+0x08` is a pointer to a `FLO_TRANSFORM_SIZE` block in the loaded
    // document, which `is_the_strip` established this record to be part of.
    transforms[block..][..ds2_rva::FLO_TRANSFORM_SIZE].copy_from_slice(&unsafe {
        std::ptr::read_unaligned((source as *const u8).cast::<[u8; ds2_rva::FLO_TRANSFORM_SIZE]>())
    });
    let pointer = transforms[block..].as_ptr() as u64;
    records[at + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET..][..8]
        .copy_from_slice(&pointer.to_le_bytes());
    Some(block)
}

/// Take a copy of the block a record points at, move it along by one [`ds2_rva::FLO_TAB_PITCH`],
/// and answer where it ended up.
fn move_along(
    records: &mut [u8],
    transforms: &mut [u8],
    slot: usize,
    which: usize,
    what: &str,
) -> Option<f32> {
    let block = own_transform(records, transforms, slot, which, what)?;
    let x = f32::from_le_bytes(
        transforms[block + ds2_rva::FLO_TRANSFORM_X_OFFSET..][..4]
            .try_into()
            .ok()?,
    );
    let moved = x + ds2_rva::FLO_TAB_PITCH;
    transforms[block + ds2_rva::FLO_TRANSFORM_X_OFFSET..][..4]
        .copy_from_slice(&moved.to_le_bytes());
    Some(moved)
}

/// Build the replacement. `None` with a log line if the definition is not the one this read.
///
/// # Safety
///
/// `original` must be the live definition the game's own lookup just returned.
unsafe fn build(original: *mut u8) -> Option<*mut u8> {
    // SAFETY: the caller guarantees a live definition.
    let children = unsafe { is_the_strip(original) }?;
    let mut strip = Box::new(Strip {
        // SAFETY: a definition is `FLO_DEFINITION_STRIDE` bytes, which is what is being copied.
        definition: unsafe {
            std::ptr::read_unaligned(original.cast::<[u8; ds2_rva::FLO_DEFINITION_STRIDE]>())
        },
        records: [0; ds2_rva::FLO_RECORD_STRIDE * CHILDREN],
        transforms: [0; ds2_rva::FLO_TRANSFORM_SIZE * MOVED],
        // SAFETY: `is_the_strip` established this many records are live at `children`.
        shipped: unsafe {
            std::ptr::read_unaligned(
                children
                    .cast::<[u8; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_TAB_STRIP_CHILDREN]>(),
            )
        },
    });
    let plan = Plan::new(crate::icon::armed());
    let stride = ds2_rva::FLO_RECORD_STRIDE;

    // The shipped records, each into the slot the plan gives it. Written from the pristine snapshot
    // rather than from the document, so a template read below cannot pick up a record this loop has
    // already moved.
    for (from, to) in plan.moved.iter().copied().enumerate() {
        strip.records[to * stride..][..stride]
            .copy_from_slice(&strip.shipped[from * stride..][..stride]);
    }

    // The last cell is the template, copied whole and then edited. Copying the last rather than the
    // first means the added cell inherits whatever the sixth's authoring says about a tab at the end
    // of the strip, which is where ours is.
    let last = (ds2_rva::FLO_TAB_STRIP_CHILDREN - 1) * stride;
    let added = plan.cell * stride;
    strip.records[added..added + stride].copy_from_slice(&strip.shipped[last..last + stride]);

    // The subtree, cloned from the record beside it, with its definition pointed at this crate's
    // copy and a new element id so a path can tell the two tabs apart. Its transform is copied and
    // moved along with the cell, because a tab's panel drops out from under its own hexagon.
    {
        let template = ds2_rva::FLO_TAB_STRIP_PANEL * stride;
        let at = plan.panel * stride;
        strip.records[at..at + stride].copy_from_slice(&strip.shipped[template..template + stride]);
        strip.records[at + ds2_rva::FLO_RECORD_DEFINITION_OFFSET..][..2]
            .copy_from_slice(&(ds2_rva::FLO_ADDED_TAB_SUBTREE_DEFINITION as u16).to_le_bytes());
        strip.records[at + ds2_rva::FLO_RECORD_ID_OFFSET..][..4]
            .copy_from_slice(&ds2_rva::FLO_ADDED_TAB_SUBTREE_ID.to_le_bytes());
    }
    let panel_x = move_along(
        &mut strip.records,
        &mut strip.transforms,
        plan.panel,
        PANEL_TRANSFORM,
        "panel",
    )?;

    // The hexagon, cloned from the plate beside it, with its shape pointed at `crate::icon`'s slice
    // and a depth that puts it over the plate and under the cells. It does not move -- the slice
    // carries its own offset -- but it does take a copy of the plate's transform block, because the
    // colour below has to land on this hexagon and not on the six the plate draws.
    if plan.icon != usize::MAX {
        let template = ds2_rva::FLO_TAB_STRIP_PLATE * stride;
        let at = plan.icon * stride;
        strip.records[at..at + stride].copy_from_slice(&strip.shipped[template..template + stride]);
        strip.records[at + ds2_rva::FLO_RECORD_DEFINITION_OFFSET..][..2]
            .copy_from_slice(&(ds2_rva::FLO_ADDED_TAB_ICON_SHAPE as u16).to_le_bytes());
        strip.records[at + ds2_rva::FLO_RECORD_DEPTH_OFFSET..][..2]
            .copy_from_slice(&ds2_rva::FLO_ADDED_TAB_ICON_DEPTH.to_le_bytes());
        let block = own_transform(
            &mut strip.records,
            &mut strip.transforms,
            plan.icon,
            ICON_TRANSFORM,
            "icon",
        )?;
        // The tint, and the licence to use it. A colour with no flag bits is inert and a run proved
        // it -- see `FLO_TRANSFORM_FLAGS_OFFSET`. The bits are or-ed into what the plate's block
        // already carried rather than written over it, the way a row's icon does it in
        // `crate::layout`: the other bits in that word are the plate's own and none of them are
        // about colour.
        let flags_at = ds2_rva::FLO_TRANSFORM_FLAGS_OFFSET;
        let flags = u32::from_le_bytes(strip.transforms[block + flags_at..][..4].try_into().ok()?)
            | TAB_ICON_TINT.flags();
        strip.transforms[block + flags_at..][..4].copy_from_slice(&flags.to_le_bytes());
        strip.transforms[block + ds2_rva::FLO_TRANSFORM_COLOUR_OFFSET..][..4]
            .copy_from_slice(&TAB_ICON_TINT.bytes());
        log(format_args!(
            "{LOG_PREFIX} strip hexagon tinted slot={} colour={:02x?} flags={flags:#x} \
             -- the seventh tab wears the sixth tab's art, and this is what tells them apart",
            plan.icon,
            TAB_ICON_TINT.bytes(),
        ));
    }

    // The cell template's transform, copied so moving ours does not move the sixth tab -- and, when
    // there is an icon, the two pieces of right-hand furniture standing where it goes.
    let x = move_along(
        &mut strip.records,
        &mut strip.transforms,
        plan.cell,
        CELL_TRANSFORM,
        "cell",
    )?;
    // THE SIXTH TAB'S OWN HEXAGON, COPIED RATHER THAN MOVED. This record was read as the strip's
    // end cap and moved one pitch along, "out from under the seventh tab's hexagon" -- and the
    // sixth tab's button vanished from the screen, because the record is that button. It begins at
    // `SIXTH_TAB_LEFT` and is one cell wide. So the original stays where the game put it and the
    // seventh tab gets a copy of it, one pitch on.
    if plan.end_cap != usize::MAX {
        let template = ds2_rva::FLO_TAB_STRIP_END_CAP * stride;
        let at = plan.end_cap * stride;
        strip.records[at..at + stride].copy_from_slice(&strip.shipped[template..template + stride]);
        let moved = move_along(
            &mut strip.records,
            &mut strip.transforms,
            plan.end_cap,
            END_CAP_TRANSFORM,
            "hexagon-plate",
        )?;
        // THE CHECK THAT SAYS THIS RECORD IS A TAB'S HEXAGON. Its x is what identifies it -- a
        // record one pitch short of the plate's right edge, on the sixth tab's own boundary. On a
        // document where it sits somewhere else it is something else, and copying it would put an
        // unknown picture beside the strip.
        if (moved - ds2_rva::FLO_TAB_PITCH - SIXTH_TAB_LEFT).abs() > 0.1 {
            log(format_args!(
                "{LOG_PREFIX} strip REFUSED reason=hexagon-plate-not-on-a-tab x={} expected={}",
                moved - ds2_rva::FLO_TAB_PITCH,
                SIXTH_TAB_LEFT
            ));
            REFUSED.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        log(format_args!(
            "{LOG_PREFIX} strip hexagon copied slot={} x={moved} -- the sixth tab's own plate, \
             drawn again one tab along, and the sixth tab keeps the one it had",
            plan.end_cap
        ));
    }
    // The `RB` prompt really is furniture, and it really is standing where the seventh hexagon
    // goes: its label sits at `347.05`, past the last tab, and the chevron `crate::icon` moves with
    // it lands at `336.70..361.70`.
    if plan.icon != usize::MAX {
        let slot = plan.moved[ds2_rva::FLO_TAB_STRIP_RB_LABEL];
        let moved = move_along(
            &mut strip.records,
            &mut strip.transforms,
            slot,
            RB_LABEL_TRANSFORM,
            "rb-label",
        )?;
        log(format_args!(
            "{LOG_PREFIX} strip furniture moved what=rb-label slot={slot} x={moved} \
             -- out from under the seventh tab's hexagon"
        ));
    }

    let record = &mut strip.records[added..added + ds2_rva::FLO_RECORD_STRIDE];
    record[ds2_rva::FLO_RECORD_ID_OFFSET..][..4]
        .copy_from_slice(&ds2_rva::FLO_ADDED_TAB_ID.to_le_bytes());
    let depth = u16::from_le_bytes(
        record[ds2_rva::FLO_RECORD_DEPTH_OFFSET..][..2]
            .try_into()
            .ok()?,
    );
    record[ds2_rva::FLO_RECORD_DEPTH_OFFSET..][..2].copy_from_slice(
        &depth
            .wrapping_add(ds2_rva::FLO_TAB_DEPTH_PITCH)
            .to_le_bytes(),
    );

    // The definition last, pointed at our records and carrying the raised count -- which is also the
    // capacity, so this one number is what gets the record walked AND lets it attach.
    let records = strip.records.as_ptr() as u64;
    let children = plan.children;
    strip.definition[ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET..][..8]
        .copy_from_slice(&records.to_le_bytes());
    strip.definition[ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET..][..2]
        .copy_from_slice(&(children as u16).to_le_bytes());

    let leaked: &'static mut Strip = Box::leak(strip);
    let n = SUBSTITUTED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} strip cell added id={:#x} x={x} depth={} children={}->{children} \
         subtree={:#x}@{}+x{panel_x} icon={} definition={:#x} substitutions={n}",
        ds2_rva::FLO_ADDED_TAB_ID,
        depth.wrapping_add(ds2_rva::FLO_TAB_DEPTH_PITCH),
        ds2_rva::FLO_TAB_STRIP_CHILDREN,
        ds2_rva::FLO_ADDED_TAB_SUBTREE_ID,
        plan.panel,
        if plan.icon == usize::MAX {
            "none -- the shape lookup is not hooked, so the tab keeps no hexagon".to_string()
        } else {
            format!("{:#x}@{}", ds2_rva::FLO_ADDED_TAB_ICON_SHAPE, plan.icon)
        },
        ds2_rva::FLO_ADDED_TAB_SUBTREE_DEFINITION,
    ));
    Some((&raw mut leaked.definition).cast::<u8>())
}

/// Whether a cached substitution still describes the document the game is holding.
///
/// # Safety
///
/// `original` must be a live definition and `cached` a [`Strip`] this module built from it.
unsafe fn still_current(original: *const u8, cached: *const Strip) -> bool {
    // SAFETY: the caller guarantees a live definition.
    let Some(children) = (unsafe { is_the_strip(original) }) else {
        return false;
    };
    let shipped = ds2_rva::FLO_TAB_STRIP_CHILDREN * ds2_rva::FLO_RECORD_STRIDE;
    // SAFETY: `is_the_strip` established that many records are live, and the cached snapshot is
    // exactly that size.
    unsafe {
        std::slice::from_raw_parts(children, shipped)
            == std::slice::from_raw_parts((*cached).shipped.as_ptr(), shipped)
    }
}

/// The strip definition this crate returns for `original`, building it on first sight.
///
/// # Safety
///
/// `original` must be the definition the game's own lookup just returned for
/// [`ds2_rva::FLO_TAB_STRIP_DEFINITION`].
pub(crate) unsafe fn substitute(original: *mut u8) -> *mut u8 {
    let Ok(mut built) = BUILT.lock() else {
        return original;
    };
    if let Some(slot) = built.iter().position(|(key, _)| *key == original as usize) {
        let replacement = built[slot].1 as *mut Strip;
        // SAFETY: `replacement` is a leaked `Strip` this module built from `original`.
        if unsafe { still_current(original, replacement) } {
            // SAFETY: `replacement` is the leaked `Strip` this module built, and `still_current`
            // has just established the document it was built from is the one the game holds.
            return unsafe { (&raw mut (*replacement).definition).cast::<u8>() };
        }
        log(format_args!(
            "{LOG_PREFIX} strip stale original=0x{:016x} -- the document was reloaded, rebuilding",
            original as usize
        ));
        built.remove(slot);
    }
    if built.len() >= MAX_BUILT {
        let n = REFUSED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} strip REFUSED reason=too-many-documents built={MAX_BUILT} refusals={n}"
        ));
        return original;
    }
    // SAFETY: the caller established this is the strip's definition index.
    match unsafe { build(original) } {
        Some(replacement) => {
            // The `Strip` begins with its definition, so the definition's address is the struct's.
            built.push((original as usize, replacement as usize));
            replacement
        }
        None => original,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four more children than the game ships -- the subtree, the sliced hexagon, the copy of the
    /// sixth tab's plate and the cell -- and the added cell is the last one. Two of the four exist
    /// only to draw a hexagon, so a strip without one carries two fewer.
    #[test]
    fn the_replacement_is_the_shipped_strip_plus_four() {
        assert_eq!(CHILDREN, ds2_rva::FLO_TAB_STRIP_CHILDREN + 4);
        assert_eq!(Plan::new(true).children, CHILDREN);
        assert_eq!(Plan::new(false).children, CHILDREN - 2);
        assert_eq!(
            ds2_rva::FLO_TAB_STRIP_FIRST_CELL + ds2_rva::FLO_TAB_STRIP_CELL_IDS.len(),
            ds2_rva::FLO_TAB_STRIP_CHILDREN,
            "the six cells must be the last six children, or the template is the wrong record"
        );
    }

    /// The record this module adds and the entry the strip's namer stand-in carries are the same
    /// cell.
    ///
    /// Two halves of one tab, written in two files: a record here, and an entry in
    /// [`crate::install::adopt_strip_namer`]. A run had the first without the second, and the tab
    /// was reachable by cursor and invisible -- the grid binds a column by resolving its namer
    /// entry, so a record nobody names is never asked for. The two id lists agreeing is what says
    /// the stand-in is naming a cell that exists.
    #[test]
    fn the_layouts_cells_and_the_namers_cells_are_the_same_cells() {
        assert_eq!(
            ds2_rva::FLO_TAB_STRIP_CELL_IDS,
            ds2_rva::FE_INGAME_TOP_SELECT_NAMER_CELL_IDS,
            "the strip's namer and its layout disagree about which cells exist"
        );
        // The stand-in clones the namer's LAST entry and rewrites its id, so the cell it names must
        // be the one this module's added record carries.
        assert!(!ds2_rva::FE_INGAME_TOP_SELECT_NAMER_CELL_IDS.contains(&ds2_rva::FLO_ADDED_TAB_ID));
    }

    /// The added tab's id is not one of the six, which is what keeps the identity check meaningful.
    #[test]
    fn the_added_id_is_not_one_the_strip_already_uses() {
        assert!(!ds2_rva::FLO_TAB_STRIP_CELL_IDS.contains(&ds2_rva::FLO_ADDED_TAB_ID));
        assert!(!ds2_rva::FLO_TAB_STRIP_CELL_IDS.contains(&ds2_rva::FLO_ADDED_TAB_SUBTREE_ID));
        assert_ne!(
            ds2_rva::FLO_ADDED_TAB_SUBTREE_ID,
            ds2_rva::FLO_TAB_STRIP_PANEL_ID
        );
    }

    /// The added subtree goes in beside its template and before the cells, and the added cell goes
    /// after them. Both halves matter: two panels drawn in the wrong order is the defect this
    /// record exists to avoid, one level up from the one it fixes.
    #[test]
    fn the_added_subtree_precedes_the_cells_and_the_added_cell_follows_them() {
        for with_icon in [false, true] {
            let plan = Plan::new(with_icon);
            assert_eq!(plan.panel, ds2_rva::FLO_TAB_STRIP_PANEL + 1);
            // A tab's panel inserted among the cells would draw over them.
            assert!(plan.panel <= plan.moved[ds2_rva::FLO_TAB_STRIP_FIRST_CELL]);
            assert_eq!(plan.cell, plan.children - 1);
            assert!(plan.cell > plan.moved[ds2_rva::FLO_TAB_STRIP_CHILDREN - 1]);
        }
    }

    /// The added hexagon goes in beside the plate it is sliced out of, and still before the cells.
    ///
    /// Array order and depth then say the same thing, which is the point: a leaf's depth is read
    /// back and a nested record's is not, and this record has to sit under the cells either way or
    /// it draws over the highlight it is supposed to sit beneath.
    #[test]
    fn the_added_hexagon_sits_beside_the_plate_and_before_the_cells() {
        let plan = Plan::new(true);
        assert_eq!(plan.icon, plan.moved[ds2_rva::FLO_TAB_STRIP_PLATE] + 1);
        assert!(plan.icon < plan.moved[ds2_rva::FLO_TAB_STRIP_FIRST_CELL]);
        assert!(plan.icon != plan.panel && plan.icon != plan.cell);
        assert_eq!(
            Plan::new(false).icon,
            usize::MAX,
            "without the shape lookup there is nothing for an icon record to resolve to"
        );
    }

    /// The record read as an end cap is the sixth tab's own hexagon, so the seventh tab copies it
    /// and the sixth keeps it. Moving it is what put a gap in the strip.
    #[test]
    fn the_end_cap_is_the_sixth_tabs_hexagon() {
        // Its record's x, as `scripts/ds2-flo.py tree --def 0x271` prints it.
        const END_CAP_X: f32 = 271.05;
        assert!(
            (END_CAP_X - SIXTH_TAB_LEFT).abs() < 0.1,
            "the cap starts at {END_CAP_X} against a sixth tab at {SIXTH_TAB_LEFT}"
        );
        let plan = Plan::new(true);
        assert_eq!(plan.end_cap, plan.moved[ds2_rva::FLO_TAB_STRIP_END_CAP] + 1);
        assert!(plan.end_cap < plan.moved[ds2_rva::FLO_TAB_STRIP_FIRST_CELL]);
        assert_eq!(
            Plan::new(false).end_cap,
            usize::MAX,
            "without a seventh hexagon there is nothing for a second plate to sit on"
        );
    }

    /// Every shipped record lands in its own slot, and the added ones land in slots nobody else
    /// claimed. A collision here would silently drop a record the game ships.
    #[test]
    fn the_plan_gives_every_record_a_slot_of_its_own() {
        for with_icon in [false, true] {
            let plan = Plan::new(with_icon);
            let mut taken = vec![false; plan.children];
            for slot in plan
                .moved
                .iter()
                .copied()
                .chain([plan.panel, plan.cell])
                .chain(if with_icon {
                    vec![plan.icon, plan.end_cap]
                } else {
                    Vec::new()
                })
            {
                assert!(slot < plan.children, "slot {slot} is past the child count");
                assert!(!taken[slot], "two records claim slot {slot}");
                taken[slot] = true;
            }
            assert!(taken.iter().all(|&t| t), "a slot was left unwritten");
        }
    }

    /// The furniture this moves is the furniture the seventh hexagon lands on, and it is moved by
    /// exactly the distance the hexagon is.
    #[test]
    fn the_moved_furniture_is_what_the_hexagon_lands_on() {
        // The plate's right edge on screen is where the seventh hexagon starts.
        let hexagon = ds2_rva::FLO_TAB_PLATE_SOURCE[2] + ds2_rva::FLO_TAB_PLATE_OFFSET[0];
        // The `RB` chevron's own left edge, which `crate::icon` moves by the same pitch. Mirrored
        // art subtracts, so the rect's right edge is the chevron's left one.
        let chevron = ds2_rva::FLO_TAB_ARROWS_RIGHT_X - ds2_rva::FLO_TAB_ARROWS_RIGHT_SOURCE[2];
        assert!(
            chevron < hexagon + ds2_rva::FLO_TAB_PITCH,
            "the chevron would not be under the hexagon, so moving it is gratuitous"
        );
        const {
            // Five records, five blocks: two sharing one would move -- or tint -- both.
            let slots = [
                CELL_TRANSFORM,
                END_CAP_TRANSFORM,
                RB_LABEL_TRANSFORM,
                PANEL_TRANSFORM,
                ICON_TRANSFORM,
            ];
            assert!(MOVED == slots.len());
            let mut i = 0;
            while i < slots.len() {
                assert!(slots[i] < MOVED);
                let mut j = i + 1;
                while j < slots.len() {
                    assert!(slots[i] != slots[j]);
                    j += 1;
                }
                i += 1;
            }
        }
    }

    /// The seventh tab's colour is one the draw will actually apply.
    ///
    /// The inert case has already cost a run: a colour word with no flag bits beside it changed
    /// nothing on screen and said nothing in the log either way. `Tint::flags` returns zero for a
    /// mix that came out white, so a strength turned down to nothing takes the licence away with
    /// it -- and a white tint on a hexagon whose art is the sixth tab's is a seventh tab nobody can
    /// pick out of the strip.
    #[test]
    fn the_tab_hexagon_is_tinted_with_a_colour_the_draw_will_use() {
        assert_ne!(TAB_ICON_TINT.bytes(), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(
            TAB_ICON_TINT.flags(),
            ds2_rva::FLO_TRANSFORM_COLOUR_LIVE | ds2_rva::FLO_TRANSFORM_COLOUR_RGB
        );
        // Opaque. The builder's own test for a child it can flatten away is `+0x1b == 0xff`, and a
        // tinted hexagon that gets flattened is a hexagon the colour never reaches.
        assert_eq!(TAB_ICON_TINT.bytes()[ds2_rva::FLO_TINT_ALPHA], 0xff);
        const {
            // Both writes land inside the block that was copied, or they are off the end of it.
            assert!(ds2_rva::FLO_TRANSFORM_COLOUR_OFFSET + 4 <= ds2_rva::FLO_TRANSFORM_SIZE);
            assert!(ds2_rva::FLO_TRANSFORM_FLAGS_OFFSET + 4 <= ds2_rva::FLO_TRANSFORM_SIZE);
        }
    }

    /// The seventh tab's panel moves with its cell, because a tab's rows drop out from under that
    /// tab's own hexagon. Sharing the System tab's block put them under the sixth tab.
    #[test]
    fn the_panel_moves_by_one_tab() {
        // The System tab's panel, as the document authors it: the strip's record plus the subtree's
        // own child. `scripts/ds2-flo.py tree --def 0x271` and `--def 0x265`.
        const SHIPPED_PANEL_X: f32 = -5.9 + 288.8;
        // The sixth cell's x, which is the tab that panel drops under.
        const SIXTH_CELL_X: f32 = 265.05;
        let offset = SHIPPED_PANEL_X - SIXTH_CELL_X;
        let seventh_cell = SIXTH_CELL_X + ds2_rva::FLO_TAB_PITCH;
        let seventh_panel = SHIPPED_PANEL_X + ds2_rva::FLO_TAB_PITCH;
        assert!(
            (seventh_panel - seventh_cell - offset).abs() < 0.01,
            "the panel sits {offset} right of its own cell on the System tab and must keep that"
        );
    }

    /// The definition is first in the struct, because [`substitute`] returns its address as the
    /// struct's and the cache keys on that.
    #[test]
    fn the_definition_is_at_the_front_of_the_allocation() {
        let strip = Strip {
            definition: [0; ds2_rva::FLO_DEFINITION_STRIDE],
            records: [0; ds2_rva::FLO_RECORD_STRIDE * CHILDREN],
            transforms: [0; ds2_rva::FLO_TRANSFORM_SIZE * MOVED],
            shipped: [0; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_TAB_STRIP_CHILDREN],
        };
        assert_eq!(
            (&raw const strip.definition).cast::<u8>(),
            (&raw const strip).cast::<u8>()
        );
    }
}
