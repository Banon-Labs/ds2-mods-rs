//! The in-game effect selector: its keys, its cursor, what it draws, and the kept effects.
//!
//! Ported from er-net-effects (`bindings.rs`, `selector_gate.rs`, the selector half of
//! `effects.rs`). The keys and their config names are ER's:
//!
//! | Config key (`[net_effects]`) | Default | Does |
//! | --- | --- | --- |
//! | `selector_show_hide_key` | `Alt+0, Alt+KP_0, Alt+Insert` | draw the bar or remove it |
//! | `selector_expand_key` | `Alt+9` | expand the bar to the list, or collapse it to its header |
//! | `selector_up_key` / `selector_down_key` | `Up` / `Down` | move the cursor one row |
//! | `selector_left_key` / `selector_right_key` | `Left` / `Right` | move the cursor one page |
//! | `selector_apply_key` | `Alt+'` | apply the highlighted effect once (a preview) |
//! | `selector_mark_key` | `Alt+M` | mark or unmark the highlighted effect to keep it applied |
//! | `selector_stack_add_key` / `selector_stack_remove_key` | `KP_Plus` / `KP_Minus` | mark / unmark |
//!
//! Two differences from ER, both because of what DS2 is here:
//!
//! * Left and right page the one catalog; ER switches between catalogs, and there is one here.
//! * ER's "stack" (effects kept on) and its "marked" list (a research note) are one thing here:
//!   a marked effect is kept applied and re-applied when it runs out ([`Keeper`]), which is what
//!   the player asked the mark for.
//!
//! As in ER, keys that act on the on-screen cursor -- the arrows, mark, add, remove -- act only
//! while the bar is open (shown and expanded); show/hide, expand and apply act whenever the game
//! has focus. Unlike ER nothing is taken from the game: this crate has no `DirectInput` detour,
//! so an arrow that moves the cursor also reaches the game.

use ds2_hotkey_config::keys::{Chord, KeyParseError, parse_chord};
use ds2_hotkey_config::kv::KeyValues;

use crate::catalog::CatalogEntry;
use crate::{ApplyReason, CONFIG_SECTION, Frame, Toggle, scalar};

/// What a bound key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Cursor up one row.
    Up,
    /// Cursor down one row.
    Down,
    /// Cursor up one page.
    PageUp,
    /// Cursor down one page.
    PageDown,
    /// Apply the highlighted effect once.
    Apply,
    /// Mark or unmark the highlighted effect.
    Mark,
    /// Mark the highlighted effect.
    KeepAdd,
    /// Unmark the highlighted effect.
    KeepRemove,
    /// Expand or collapse the bar.
    Expand,
    /// Draw or remove the bar.
    ShowHide,
}

/// Every action, in the order [`Bindings`] stores and polls them.
pub const ACTIONS: [Action; 10] = [
    Action::Up,
    Action::Down,
    Action::PageUp,
    Action::PageDown,
    Action::Apply,
    Action::Mark,
    Action::KeepAdd,
    Action::KeepRemove,
    Action::Expand,
    Action::ShowHide,
];

impl Action {
    /// The config key that binds it, er-net-effects' name.
    pub const fn config_key(self) -> &'static str {
        match self {
            Self::Up => "selector_up_key",
            Self::Down => "selector_down_key",
            Self::PageUp => "selector_left_key",
            Self::PageDown => "selector_right_key",
            Self::Apply => "selector_apply_key",
            Self::Mark => "selector_mark_key",
            Self::KeepAdd => "selector_stack_add_key",
            Self::KeepRemove => "selector_stack_remove_key",
            Self::Expand => "selector_expand_key",
            Self::ShowHide => "selector_show_hide_key",
        }
    }

    /// The shipped default: er-net-effects' default for the same key. A comma list binds several.
    pub const fn default_keys(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::PageUp => "left",
            Self::PageDown => "right",
            Self::Apply => "alt+quote",
            Self::Mark => "alt+m",
            Self::KeepAdd => "kp_plus",
            Self::KeepRemove => "kp_minus",
            Self::Expand => "alt+9",
            Self::ShowHide => "alt+0, alt+kp_0, alt+insert",
        }
    }

    /// Whether it acts only while the bar is open.
    pub const fn needs_open(self) -> bool {
        matches!(
            self,
            Self::Up
                | Self::Down
                | Self::PageUp
                | Self::PageDown
                | Self::Mark
                | Self::KeepAdd
                | Self::KeepRemove
        )
    }

    fn index(self) -> usize {
        ACTIONS.iter().position(|a| *a == self).unwrap_or_default()
    }
}

/// Parse a comma-separated chord list. All or nothing: one bad entry refuses the whole value.
///
/// # Errors
///
/// The first entry that does not parse.
pub fn parse_chord_list(value: &str) -> Result<Vec<Chord>, KeyParseError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(parse_chord)
        .collect()
}

/// The chords bound to each [`Action`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    chords: Vec<Vec<Chord>>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self {
            chords: ACTIONS
                .iter()
                .map(|a| parse_chord_list(a.default_keys()).unwrap_or_default())
                .collect(),
        }
    }
}

impl Bindings {
    /// The chords bound to `action`.
    pub fn chords(&self, action: Action) -> &[Chord] {
        self.chords
            .get(action.index())
            .map_or(&[][..], Vec::as_slice)
    }

    /// Apply the `[net_effects]` selector keys in a config text. A key that is absent keeps what
    /// is bound; a value that does not parse keeps it too and is named in the returned messages;
    /// an empty value unbinds. Returns whether anything moved.
    pub fn apply_config(&mut self, text: &str, messages: &mut Vec<String>) -> bool {
        let parsed = KeyValues::parse(text);
        let mut moved = false;
        for action in ACTIONS {
            let Some(raw) = parsed.get(CONFIG_SECTION, action.config_key()) else {
                continue;
            };
            let value = scalar(raw);
            match parse_chord_list(value) {
                Ok(chords) => {
                    if let Some(slot) = self.chords.get_mut(action.index())
                        && *slot != chords
                    {
                        *slot = chords;
                        moved = true;
                        messages.push(format!("{} = {value:?}", action.config_key()));
                    }
                }
                Err(error) => messages.push(format!(
                    "{} = {value:?} not understood ({error:?}) -- keeping the key already in force",
                    action.config_key()
                )),
            }
        }
        moved
    }
}

/// Rows the expanded bar lists at once, and how far a page moves.
pub const PAGE_ROWS: usize = 10;

/// What one action did to the selector. The caller logs it and does the game-side half.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The action needs the bar open and it was not, or the catalog is empty.
    Ignored,
    /// The cursor moved to this row.
    Moved(usize),
    /// Apply the effect on this row once.
    Apply(usize),
    /// Mark or unmark this row.
    Mark(usize),
    /// Mark this row.
    KeepAdd(usize),
    /// Unmark this row.
    KeepRemove(usize),
    /// The bar is expanded (`true`) or collapsed now.
    Expanded(bool),
    /// The bar is drawn (`true`) or removed now.
    Shown(bool),
}

/// The cursor and the two visibility flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selector {
    cursor: usize,
    shown: bool,
    expanded: bool,
}

impl Default for Selector {
    /// Shown and collapsed to its header, as ER's bar starts.
    fn default() -> Self {
        Self {
            cursor: 0,
            shown: true,
            expanded: false,
        }
    }
}

impl Selector {
    /// The highlighted row.
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether the bar is drawn at all.
    pub const fn shown(&self) -> bool {
        self.shown
    }

    /// Whether the bar lists the catalog.
    pub const fn expanded(&self) -> bool {
        self.expanded
    }

    /// Shown and expanded: the cursor keys act.
    pub const fn open(&self) -> bool {
        self.shown && self.expanded
    }

    /// One press of `action` over a catalog of `len` rows.
    pub fn act(&mut self, action: Action, len: usize) -> Step {
        if action.needs_open() && !self.open() {
            return Step::Ignored;
        }
        if len == 0 && !matches!(action, Action::Expand | Action::ShowHide) {
            return Step::Ignored;
        }
        self.cursor = self.cursor.min(len.saturating_sub(1));
        match action {
            Action::Up => {
                self.cursor = if self.cursor == 0 {
                    len - 1
                } else {
                    self.cursor - 1
                };
                Step::Moved(self.cursor)
            }
            Action::Down => {
                self.cursor = (self.cursor + 1) % len;
                Step::Moved(self.cursor)
            }
            Action::PageUp => {
                self.cursor = self.cursor.saturating_sub(PAGE_ROWS);
                Step::Moved(self.cursor)
            }
            Action::PageDown => {
                self.cursor = (self.cursor + PAGE_ROWS).min(len - 1);
                Step::Moved(self.cursor)
            }
            Action::Apply => Step::Apply(self.cursor),
            Action::Mark => Step::Mark(self.cursor),
            Action::KeepAdd => Step::KeepAdd(self.cursor),
            Action::KeepRemove => Step::KeepRemove(self.cursor),
            Action::Expand => {
                self.expanded = !self.expanded;
                Step::Expanded(self.expanded)
            }
            Action::ShowHide => {
                self.shown = !self.shown;
                Step::Shown(self.shown)
            }
        }
    }

    /// The first row of the visible page: the cursor's page, [`PAGE_ROWS`] rows long.
    pub const fn first_visible(&self) -> usize {
        self.cursor / PAGE_ROWS * PAGE_ROWS
    }
}

/// The title on the bar's header.
pub const TITLE: &str = "Net Effects";

/// One line of the bar, with how it should be coloured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The text.
    pub text: String,
    /// What the line is.
    pub kind: LineKind,
}

/// How a [`Line`] is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// The header.
    Title,
    /// A catalog row.
    Row,
    /// The highlighted catalog row.
    Cursor,
    /// Help and status text.
    Hint,
}

/// What the bar shows this frame: nothing when hidden, the header when collapsed, the header, a
/// page of rows and a key line when expanded.
///
/// A row reads `> [K] * 140001010 name`: `>` the cursor, `[K]` marked to keep applied, `*` on the
/// character right now.
pub fn view(
    selector: &Selector,
    catalog: &[CatalogEntry],
    marked: &[i32],
    on_player: &[i32],
) -> Vec<Line> {
    if !selector.shown() {
        return Vec::new();
    }
    let mut lines = vec![Line {
        text: format!(
            "{TITLE} {}  {} kept",
            if selector.expanded() { "[-]" } else { "[+]" },
            marked.len()
        ),
        kind: LineKind::Title,
    }];
    if !selector.expanded() {
        return lines;
    }
    if catalog.is_empty() {
        lines.push(Line {
            text: "No effects in the catalog".to_string(),
            kind: LineKind::Hint,
        });
        return lines;
    }
    let cursor = selector.cursor().min(catalog.len() - 1);
    let first = cursor / PAGE_ROWS * PAGE_ROWS;
    for (index, entry) in catalog.iter().enumerate().skip(first).take(PAGE_ROWS) {
        let here = index == cursor;
        lines.push(Line {
            text: format!(
                "{} {} {} {:>9} {}",
                if here { ">" } else { " " },
                if marked.contains(&entry.id) {
                    "[K]"
                } else {
                    "[ ]"
                },
                if on_player.contains(&entry.id) {
                    "*"
                } else {
                    " "
                },
                entry.id,
                entry.name
            ),
            kind: if here {
                LineKind::Cursor
            } else {
                LineKind::Row
            },
        });
    }
    let current = &catalog[cursor];
    let tags = if current.tags.is_empty() {
        String::new()
    } else {
        format!("  tags: {}", current.tags.join(", "))
    };
    lines.push(Line {
        text: format!("{}/{}{tags}", cursor + 1, catalog.len()),
        kind: LineKind::Hint,
    });
    lines.push(Line {
        text: "Up/Down Row   Left/Right Page   Alt+' Apply   Alt+M Keep   Alt+9 Fold".to_string(),
        kind: LineKind::Hint,
    });
    lines
}

/// The marked effects kept on the player: one [`Toggle`] per id, switched on when it is marked.
///
/// An id is applied once a character is loaded, and again each time it has been seen on the
/// player and then leaves -- the toggle's rules, per id. Unmarking one stops re-applying it; what
/// is on the player runs out by itself. The keeper runs only while net effects is on; turning it
/// off takes the kept effects off the player, and [`Keeper::restart`] applies them again.
#[derive(Clone, Debug, Default)]
pub struct Keeper {
    kept: Vec<Kept>,
}

#[derive(Clone, Copy, Debug)]
struct Kept {
    id: i32,
    toggle: Toggle,
    fresh: bool,
}

impl Keeper {
    /// Make the kept set exactly `ids`: new ids start fresh (applied at the next readable frame),
    /// ids no longer marked are dropped.
    pub fn sync(&mut self, ids: &[i32]) {
        self.kept.retain(|k| ids.contains(&k.id));
        for id in ids {
            if !self.kept.iter().any(|k| k.id == *id) {
                self.kept.push(Kept {
                    id: *id,
                    toggle: Toggle::new(),
                    fresh: true,
                });
            }
        }
    }

    /// Start every kept id over, as if it had just been marked: each is applied at the next
    /// readable frame and tracked from nothing. Net effects going back on calls this, because
    /// going off removed the kept effects from the player and stopped watching them.
    pub fn restart(&mut self) {
        for k in &mut self.kept {
            k.toggle = Toggle::new();
            k.fresh = true;
        }
    }

    /// Whether anything is kept.
    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    /// One frame. `on_player` is every id in the player's action list, `None` when it could not be
    /// read. Returns each kept id with what its toggle decided; the caller applies the ones with
    /// [`Frame::apply`] and reports each with [`Keeper::applied`].
    pub fn frame(&mut self, on_player: Option<&[i32]>) -> Vec<(i32, Frame)> {
        self.kept
            .iter_mut()
            .map(|k| {
                let active = on_player.map(|ids| ids.contains(&k.id));
                let frame = k.toggle.frame(k.fresh, active);
                k.fresh = false;
                (k.id, frame)
            })
            .collect()
    }

    /// The apply [`Keeper::frame`] asked for on `id` was made.
    pub fn applied(&mut self, id: i32) {
        if let Some(k) = self.kept.iter_mut().find(|k| k.id == id) {
            k.toggle.applied();
        }
    }
}

/// The log word for an apply the keeper asked for.
pub const fn keep_reason(reason: ApplyReason) -> &'static str {
    match reason {
        ApplyReason::Enabled => "kept",
        ApplyReason::Expired => "kept-reapply",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SIGHTING_FRAMES;

    fn entry(id: i32) -> CatalogEntry {
        CatalogEntry {
            id,
            name: format!("e{id}"),
            sfx: vec![],
            full_body_sfx: vec![],
            dummy_poly: vec![],
            duration: None,
            tags: vec![],
        }
    }

    #[test]
    fn the_defaults_are_er_net_effects_keys() {
        let b = Bindings::default();
        assert_eq!(b.chords(Action::Up), [parse_chord("up").expect("up")]);
        assert_eq!(
            b.chords(Action::Apply),
            [parse_chord("alt+'").expect("alt+'")]
        );
        assert_eq!(
            b.chords(Action::Mark),
            [parse_chord("alt+m").expect("alt+m")]
        );
        assert_eq!(
            b.chords(Action::Expand),
            [parse_chord("alt+9").expect("alt+9")]
        );
        assert_eq!(b.chords(Action::ShowHide).len(), 3);
        for action in ACTIONS {
            assert!(
                !b.chords(action).is_empty(),
                "{action:?}'s default {:?} parses",
                action.default_keys()
            );
        }
    }

    #[test]
    fn a_config_key_rebinds_and_junk_keeps_the_old_key() {
        let mut b = Bindings::default();
        let mut messages = Vec::new();
        assert!(b.apply_config(
            "[net_effects]\nselector_up_key = \"PageUp\"\nselector_mark_key = \"NotAKey\"\n",
            &mut messages
        ));
        assert_eq!(
            b.chords(Action::Up),
            [parse_chord("PageUp").expect("PageUp")]
        );
        assert_eq!(
            b.chords(Action::Mark),
            [parse_chord("alt+m").expect("alt+m")]
        );
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages[1].contains("NotAKey"), "{messages:?}");
        messages.clear();
        assert!(!b.apply_config(
            "[net_effects]\nselector_up_key = \"PageUp\"\n",
            &mut messages
        ));
        assert!(messages.is_empty(), "no change, no line");
        assert!(b.apply_config("[net_effects]\nselector_up_key = \"\"\n", &mut messages));
        assert!(b.chords(Action::Up).is_empty(), "an empty value unbinds");
    }

    #[test]
    fn cursor_keys_act_only_while_open() {
        let mut s = Selector::default();
        assert!(s.shown() && !s.expanded());
        assert_eq!(s.act(Action::Down, 5), Step::Ignored);
        assert_eq!(s.act(Action::Mark, 5), Step::Ignored);
        assert_eq!(
            s.act(Action::Apply, 5),
            Step::Apply(0),
            "apply is live collapsed"
        );
        assert_eq!(s.act(Action::Expand, 5), Step::Expanded(true));
        assert_eq!(s.act(Action::Down, 5), Step::Moved(1));
        assert_eq!(s.act(Action::ShowHide, 5), Step::Shown(false));
        assert_eq!(s.act(Action::Up, 5), Step::Ignored, "hidden is not open");
    }

    #[test]
    fn the_cursor_wraps_by_row_and_clamps_by_page() {
        let mut s = Selector::default();
        s.act(Action::Expand, 25);
        assert_eq!(s.act(Action::Up, 25), Step::Moved(24));
        assert_eq!(s.act(Action::Down, 25), Step::Moved(0));
        assert_eq!(s.act(Action::PageDown, 25), Step::Moved(10));
        assert_eq!(s.act(Action::PageDown, 25), Step::Moved(20));
        assert_eq!(s.act(Action::PageDown, 25), Step::Moved(24));
        assert_eq!(s.first_visible(), 20);
        assert_eq!(s.act(Action::PageUp, 25), Step::Moved(14));
        assert_eq!(s.act(Action::Mark, 25), Step::Mark(14));
        assert_eq!(s.act(Action::Down, 0), Step::Ignored);
    }

    #[test]
    fn the_view_shows_cursor_marks_and_what_is_on_the_player() {
        let catalog: Vec<_> = (1..=12).map(entry).collect();
        let mut s = Selector::default();
        let collapsed = view(&s, &catalog, &[2], &[]);
        assert_eq!(collapsed.len(), 1);
        assert!(collapsed[0].text.contains("[+]"));
        assert!(collapsed[0].text.contains("1 kept"));
        s.act(Action::Expand, catalog.len());
        s.act(Action::Down, catalog.len());
        let lines = view(&s, &catalog, &[2], &[2, 3]);
        assert_eq!(lines.len(), 1 + PAGE_ROWS + 2);
        assert_eq!(lines[2].kind, LineKind::Cursor);
        assert!(lines[2].text.starts_with("> [K] *"), "{:?}", lines[2]);
        assert!(lines[3].text.starts_with("  [ ] *"), "{:?}", lines[3]);
        assert!(lines[1].text.starts_with("  [ ]  "), "{:?}", lines[1]);
        assert!(lines[PAGE_ROWS + 1].text.starts_with("2/12"));
        s.act(Action::ShowHide, catalog.len());
        assert!(view(&s, &catalog, &[], &[]).is_empty());
    }

    /// A marked effect is applied once a list can be read, re-applied after it runs out, and
    /// dropped when unmarked.
    #[test]
    fn the_keeper_applies_reapplies_and_forgets() {
        let mut k = Keeper::default();
        k.sync(&[5, 6]);
        let f = k.frame(None);
        assert!(
            f.iter().all(|(_, f)| f.apply.is_none()),
            "no list, no apply"
        );
        let f = k.frame(Some(&[]));
        assert_eq!(
            f.iter()
                .filter(|(_, f)| f.apply == Some(ApplyReason::Enabled))
                .count(),
            2
        );
        k.applied(5);
        k.applied(6);
        assert!(k.frame(Some(&[5, 6])).iter().all(|(_, f)| f.seen));
        let f = k.frame(Some(&[6]));
        assert_eq!(f[0], (5, f[0].1));
        assert_eq!(f[0].1.apply, Some(ApplyReason::Expired));
        assert_eq!(f[1].1.apply, None);
        k.applied(5);
        k.sync(&[6]);
        let f = k.frame(Some(&[]));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].0, 6);
        assert_eq!(f[0].1.apply, Some(ApplyReason::Expired));
        k.sync(&[]);
        assert!(k.is_empty());
    }

    /// An id the game never shows is applied once and then left alone, per the toggle's rule.
    #[test]
    fn a_kept_id_that_never_shows_is_not_spammed() {
        let mut k = Keeper::default();
        k.sync(&[9]);
        assert!(k.frame(Some(&[]))[0].1.apply.is_some());
        k.applied(9);
        let applies = (0..SIGHTING_FRAMES * 3)
            .filter(|_| k.frame(Some(&[]))[0].1.apply.is_some())
            .count();
        assert_eq!(applies, 0);
    }

    /// Going back on applies every kept id again at once, even one that had been seen, one that
    /// never showed up, and one still waiting for a sighting.
    #[test]
    fn a_restart_applies_every_kept_id_again() {
        let mut k = Keeper::default();
        k.sync(&[1, 2, 3]);
        assert!(k.frame(Some(&[])).iter().all(|(_, f)| f.apply.is_some()));
        k.applied(1);
        k.applied(2);
        k.applied(3);
        assert!(k.frame(Some(&[1])).iter().all(|(_, f)| f.apply.is_none()));
        for _ in 0..SIGHTING_FRAMES {
            k.frame(Some(&[1]));
        }
        k.restart();
        let f = k.frame(Some(&[]));
        assert_eq!(f.len(), 3);
        assert!(
            f.iter().all(|(_, f)| f.apply == Some(ApplyReason::Enabled)),
            "{f:?}"
        );
    }
}
