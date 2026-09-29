//! The in-game link panel: a field over the pause menu that takes a soulsplanner or MugenMonkey build
//! link.
//!
//! This replaces what the row used to open on a desktop Steam -- a modal Win32 dialog, measured
//! 2026-09-29 as `no Steam field (the Steam overlay is disabled ...) -- opening the link dialog
//! instead` with the dialog up at hwnd `0x1010a` over the game -- with a panel drawn the way the
//! save picker and the build recommender are drawn: the same dim cover, the same bordered panel,
//! the same colours, over the pause menu rather than beside it in another window.
//!
//! Every decision about the text is `ds2_build_url_core`'s and is tested on the host: the editor
//! ([`ds2_build_url_core::Editor`]), what it opens with
//! ([`ds2_build_url_core::clipboard::initial_text`]), and what a refused link re-opens with
//! ([`ds2_build_url_core::Session`]). This module draws them and routes presses into them.
//!
//! # Who calls what
//!
//! | who | when | what it does here |
//! |---|---|---|
//! | the row's press | the pause menu's confirm | [`open`]: build the editor, take the hold |
//! | [`on_frame`] | `ds2-overlay`'s clock, every `Present` | keys and pad into the editor; release the hold after closing |
//! | [`draw`] | `ds2-overlay`'s panel table, the same `Present` | draw, and turn mouse clicks into presses |
//! | [`finished`] | the fetch worker, when the fetch is over | close on a build, re-open the field on a failure |
//!
//! A submitted link goes to the same worker and the same `load` the dialog's did, and a fetched
//! build is applied by the same pause-menu tick. The panel stays up while the fetch runs, saying
//! so, and closes when the build has been handed over -- or goes back to the field with the reason
//! when it has not, so a typo is corrected rather than retyped.
//!
//! # The pause menu stays still underneath
//!
//! While the panel is up, `ds2_input_harness::hold` blanks every device the game reads and
//! `ds2-overlay` keeps keyboard and mouse window messages from the game window. After the panel
//! closes the hold stays until every key and button is released, so the Enter or B that closed it
//! does not also press the pause menu. Same arrangement as the save picker's, for the same reason.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ds2_build_url_core::{Editor, Event, Session, Verdict};
use ds2_overlay::style;
use hudhook::imgui::{MouseButton, Ui};

use crate::panel_input::{Press, Reader};
use crate::{LOG_PREFIX, log_line};

/// Where the panel is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Taking presses.
    Open,
    /// A link was accepted and the worker is fetching it. Presses other than close are dropped.
    Fetching,
    /// Gone from the screen, holding input until every key is up. Frames waited so far.
    Closing(u32),
}

/// What the banner says, and in which colour.
#[derive(Clone, Debug)]
struct Status {
    headline: String,
    detail: String,
    warn: bool,
}

struct Panel {
    editor: Editor,
    session: Session,
    phase: Phase,
    reader: Reader,
    status: Option<Status>,
}

impl Panel {
    /// Enter, the A button or the load button.
    fn submit(&mut self) {
        let Event::Submitted(text) = self.editor.enter() else {
            return;
        };
        let clipboard = crate::clipboard::text();
        match self.session.on_submitted(&text, clipboard.as_deref()) {
            Verdict::Accept { build_id, url } => {
                log_line(format_args!(
                    "{LOG_PREFIX} panel submitted \"{url}\" (build {build_id})"
                ));
                let host = ds2_build_import_core::build_link_from_url(&url)
                    .map_or("the planner", |link| link.site.host());
                let job = crate::flow::Job::Link {
                    text: url.clone(),
                    source: crate::flow::Source::Panel,
                };
                if crate::install::submit(job) {
                    self.phase = Phase::Fetching;
                    self.status = Some(Status {
                        headline: "Fetching".to_owned(),
                        detail: format!("build {build_id} from {host}"),
                        warn: false,
                    });
                } else {
                    // Another fetch holds the session. Put the link back rather than lose it.
                    self.editor = Editor::with_text(&url);
                    self.status = Some(Status {
                        headline: "Busy".to_owned(),
                        detail: "Another build is still being fetched.".to_owned(),
                        warn: true,
                    });
                }
            }
            Verdict::Reopen { text, rejection } => {
                log_line(format_args!(
                    "{LOG_PREFIX} panel refused \"{}\": {rejection}",
                    text.trim()
                ));
                self.editor = Editor::with_text(&text);
                self.status = Some(Status {
                    headline: crate::flow::short_rejection(rejection).to_owned(),
                    detail: rejection.to_string(),
                    warn: true,
                });
            }
            Verdict::GiveUp { rejection } => {
                // SAFETY: presses are applied from the `Present` clock and the draw, and DARK SOULS
                // II presents from its simulation thread -- the game thread, with the menu up.
                unsafe { crate::flow::say_now(crate::flow::short_rejection(rejection)) };
                self.close("too many refused links in one press");
            }
        }
    }

    /// Ctrl+V, Shift+Insert or Y.
    fn paste(&mut self) {
        if let Some(text) = crate::clipboard::text() {
            self.editor.paste(&text);
        }
    }

    /// Route one press through the editor.
    fn apply(&mut self, press: Press) {
        if press == Press::Close {
            return self.close("closed by the player");
        }
        if self.phase != Phase::Open {
            return;
        }
        match press {
            Press::Confirm => self.submit(),
            Press::Paste => self.paste(),
            Press::Left => drop(self.editor.move_left()),
            Press::Right => drop(self.editor.move_right()),
            Press::Home => drop(self.editor.move_home()),
            Press::End => drop(self.editor.move_end()),
            Press::Backspace => drop(self.editor.backspace()),
            Press::Delete => drop(self.editor.delete()),
            Press::Clear => drop(self.editor.clear()),
            Press::Char(typed) => drop(self.editor.insert_char(typed)),
            Press::Close => {}
        }
    }

    fn close(&mut self, why: &str) {
        log_line(format_args!("{LOG_PREFIX} panel closed -- {why}"));
        self.phase = Phase::Closing(0);
    }
}

/// The panel, while it is on screen or releasing its hold.
static PANEL: Mutex<Option<Panel>> = Mutex::new(None);

/// Set once the panel and its clock are registered with `ds2-overlay`.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Frames the hold outlasts the panel at most, waiting for every key to be released.
const RELEASE_DEADLINE_FRAMES: u32 = 90;

/// Register the panel's draw function and its clock with `ds2-overlay`. `false` has been logged,
/// and the row keeps opening the Steam field or the Win32 dialog.
pub(crate) fn install() -> bool {
    if !ds2_overlay::panels::add_panel(draw, is_up, Some(wants_input)) {
        log_line(format_args!(
            "{LOG_PREFIX} panel: every ds2-overlay panel slot is taken"
        ));
        return false;
    }
    if !ds2_overlay::frame_hook::add_frame_hook(on_frame) {
        log_line(format_args!(
            "{LOG_PREFIX} panel: every ds2-overlay frame-hook slot is taken"
        ));
        return false;
    }
    INSTALLED.store(true, Ordering::Release);
    log_line(format_args!(
        "{LOG_PREFIX} panel: the in-game field answers the row"
    ));
    true
}

/// Whether the in-game panel answers the row.
pub(crate) fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Open the panel. **Game thread**, from the row's confirm, with the menu up.
pub(crate) fn open() {
    let frame = ds2_overlay::frame_hook::ticks();
    let Ok(mut guard) = PANEL.lock() else {
        return;
    };
    if guard
        .as_ref()
        .is_some_and(|panel| !matches!(panel.phase, Phase::Closing(_)))
    {
        log_line(format_args!(
            "{LOG_PREFIX} panel press frame={frame} -- ignored, the panel is already up"
        ));
        return;
    }
    let clipboard = crate::clipboard::text();
    let prefill = ds2_build_url_core::clipboard::initial_text(clipboard.as_deref());
    let mut reader = Reader::new();
    reader.swallow_held();
    *guard = Some(Panel {
        editor: Editor::with_text(&prefill),
        session: Session::new(),
        phase: Phase::Open,
        reader,
        status: None,
    });
    drop(guard);
    ds2_input_harness::hold(true);
    log_line(format_args!(
        "{LOG_PREFIX} panel open frame={frame}, prefilled \"{prefill}\""
    ));
}

/// The fetch the panel started is over. **Any thread**: the worker calls this.
///
/// `Ok` carries what the row now says; the build is on its way to the pause menu's tick and the
/// panel closes. `Err` carries why not; the field comes back with the link in it, to correct.
pub(crate) fn finished(result: &Result<String, String>, link: &str) {
    let Ok(mut guard) = PANEL.lock() else {
        return;
    };
    let Some(panel) = guard.as_mut() else {
        return;
    };
    if panel.phase != Phase::Fetching {
        return;
    }
    match result {
        Ok(caption) => panel.close(&format!("loaded, the row says \"{caption}\"")),
        Err(reason) => {
            panel.editor = Editor::with_text(link);
            panel.phase = Phase::Open;
            panel.status = Some(Status {
                headline: reason.to_string(),
                detail: String::new(),
                warn: true,
            });
        }
    }
}

/// Whether the panel is on screen. `ds2-overlay` renders no imgui frame while no panel is.
fn is_up() -> bool {
    PANEL.try_lock().ok().is_some_and(|guard| {
        guard
            .as_ref()
            .is_some_and(|panel| !matches!(panel.phase, Phase::Closing(_)))
    })
}

/// `ds2-overlay` asks this every frame: while the panel is up, the game window gets no keyboard or
/// mouse message.
fn wants_input() -> bool {
    // Closing included: the click that closed the panel must not reach the pause menu either.
    PANEL.try_lock().ok().is_some_and(|guard| guard.is_some())
}

/// One frame: keys and pad into the editor, and the hold's release.
fn on_frame() {
    let Ok(mut guard) = PANEL.lock() else {
        return;
    };
    let Some(panel) = guard.as_mut() else {
        return;
    };
    if let Phase::Closing(frames) = panel.phase {
        if !panel.reader.anything_down() || frames >= RELEASE_DEADLINE_FRAMES {
            *guard = None;
            drop(guard);
            ds2_input_harness::hold(false);
        } else {
            panel.phase = Phase::Closing(frames + 1);
        }
        return;
    }
    for press in panel.reader.poll() {
        panel.apply(press);
        if matches!(panel.phase, Phase::Closing(_)) {
            break;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing, in the game's own palette (`ds2_overlay::style`, docs/DS2-UI-DESIGN.md).
// ---------------------------------------------------------------------------------------------

const PANEL_BG: [f32; 4] = style::PANEL_BG;
const PANEL_EDGE: [f32; 4] = style::BRONZE;
const DIM_COVER: [f32; 4] = style::DIM_COVER;
const TITLE: [f32; 4] = style::TEXT;
const TEXT: [f32; 4] = style::TEXT;
const DIM: [f32; 4] = style::BRONZE;
const DISABLED: [f32; 4] = style::ASH;
const WARN: [f32; 4] = style::WARN_TEXT;
const FIELD_BG: [f32; 4] = style::SLATE;
const FIELD_EDIT: [f32; 4] = style::SLATE_EDIT;
const CELL_BG: [f32; 4] = style::INK_1;
const FOCUS_EDGE: [f32; 4] = style::BRONZE;
const PAD: f32 = 14.0;

const PANEL_TITLE: &str = "Load Build";
const PANEL_SUBTITLE: &str = "soulsplanner or mugenmonkey link";
/// The key help, one button and one word each, for the keyboard and for the pad.
const KEY_HINT: [(&str, &str); 4] = [
    ("Enter", "Load"),
    ("Ctrl+V", "Paste"),
    ("Ctrl+Backspace", "Clear"),
    ("Esc", "Close"),
];
const PAD_HINT: [(&str, &str); 4] = [
    ("A", "Load"),
    ("Y", "Paste"),
    ("X", "Clear"),
    ("B", "Close"),
];

/// The panel's draw function, called by `ds2-overlay` once per frame.
fn draw(ui: &Ui) {
    let Ok(mut guard) = PANEL.try_lock() else {
        return;
    };
    let Some(panel) = guard.as_mut() else {
        return;
    };
    if matches!(panel.phase, Phase::Closing(_)) {
        return;
    }
    let display = ui.io().display_size;
    let line = ui.current_font_size();
    let field_height = line + 14.0;
    let width = (display[0] * 0.5)
        .clamp(560.0, 960.0)
        .min(display[0] - 32.0);
    // Title (the game's big face), subtitle, banner (two lines), field, buttons, the hint bar.
    let title_line = ds2_overlay::panels::title_height(ui);
    let height = PAD * 2.0
        + title_line
        + line
        + 10.0
        + (line * 2.0 + 12.0)
        + field_height
        + 14.0
        + (line + 10.0)
        + 12.0
        + line
        + 2.0;
    let left = (display[0] - width) * 0.5;
    let top = (display[1] - height) * 0.5;

    let list = ui.get_foreground_draw_list();
    list.add_rect([0.0, 0.0], display, DIM_COVER)
        .filled(true)
        .build();
    list.add_rect([left, top], [left + width, top + height], PANEL_BG)
        .filled(true)
        .rounding(style::ROUNDING)
        .build();
    list.add_rect([left, top], [left + width, top + height], PANEL_EDGE)
        .rounding(style::ROUNDING)
        .thickness(style::FRAME_PX)
        .build();

    // In back-buffer pixels; imgui's own position is in window pixels here. See `panels::mouse`.
    let mouse = ds2_overlay::panels::mouse().unwrap_or(ui.io().mouse_pos);
    let inside = |min: [f32; 2], max: [f32; 2]| {
        mouse[0] >= min[0] && mouse[0] < max[0] && mouse[1] >= min[1] && mouse[1] < max[1]
    };
    let clicked = ui.is_mouse_clicked(MouseButton::Left);
    let mut press: Option<Press> = None;

    // Header: title and close, then what to do.
    let inner_left = left + PAD;
    let inner_right = left + width - PAD;
    let mut y = top + PAD;
    let title_line = ds2_overlay::panels::title(ui, &list, [inner_left, y], TITLE, PANEL_TITLE);
    let close_label = "Close";
    let close_width = ui.calc_text_size(close_label)[0];
    let close_min = [inner_right - close_width, y];
    let close_max = [inner_right, y + line];
    let close_hover = inside(close_min, close_max);
    list.add_text(
        close_min,
        if close_hover { TITLE } else { DIM },
        close_label,
    );
    if close_hover && clicked {
        press = Some(Press::Close);
    }
    y += title_line + 2.0;
    list.add_text([inner_left, y], DIM, PANEL_SUBTITLE);
    y += line + 10.0;

    // The banner: why a link was refused, or that one is being fetched.
    let banner_top = y;
    if let Some(status) = &panel.status {
        // A refusal stands beside the game's own red X, in body text: the red alone is too dark
        // to read. Without the atlas it falls back to the warning colour.
        let mark = if status.warn {
            ds2_overlay::panels::refusal_mark(&list, [inner_left, y], line)
        } else {
            0.0
        };
        let colour = match (status.warn, mark > 0.0) {
            (true, true) => TEXT,
            (true, false) => WARN,
            (false, _) => TITLE,
        };
        list.add_text(
            [inner_left + mark, y],
            colour,
            clip(ui, &status.headline, inner_right - inner_left - mark),
        );
        y += line;
        list.add_text(
            [inner_left + 12.0, y],
            TEXT,
            clip(ui, &status.detail, inner_right - inner_left - 12.0),
        );
    }
    y = banner_top + line * 2.0 + 12.0;

    // The field.
    let open = panel.phase == Phase::Open;
    let field_min = [inner_left, y];
    let field_max = [inner_right, y + field_height];
    list.add_rect(
        field_min,
        field_max,
        if open { FIELD_EDIT } else { FIELD_BG },
    )
    .filled(true)
    .rounding(style::ROUNDING)
    .build();
    if open {
        list.add_rect(field_min, field_max, FOCUS_EDGE)
            .rounding(style::ROUNDING)
            .thickness(1.5)
            .build();
    }
    let text_y = y + (field_height - line) * 0.5;
    let space = field_max[0] - field_min[0] - 12.0;
    let text = panel.editor.text();
    let caret = panel.editor.caret();
    let before: String = text.chars().take(caret).collect();
    let after: String = text.chars().skip(caret).collect();
    // Keep the caret on screen: the part before it is cut from the left, the rest from the right.
    let shown_before = clip_left(ui, &before, space * 0.85);
    let before_width = ui.calc_text_size(&shown_before)[0];
    let shown_after = clip(ui, &after, space - before_width);
    let text_colour = if open { TEXT } else { DISABLED };
    list.add_text([field_min[0] + 6.0, text_y], text_colour, &shown_before);
    list.add_text(
        [field_min[0] + 6.0 + before_width, text_y],
        text_colour,
        &shown_after,
    );
    if open {
        let caret_x = field_min[0] + 6.0 + before_width;
        list.add_line([caret_x, text_y], [caret_x, text_y + line], TITLE)
            .build();
    }
    y += field_height + 14.0;

    // The buttons, as cells like the picker's drive strip.
    let mut x = inner_left;
    for (label, action) in [("Load", Press::Confirm), ("Paste", Press::Paste)] {
        let cell_width = ui.calc_text_size(label)[0] + 24.0;
        let min = [x, y];
        let max = [x + cell_width, y + line + 10.0];
        let hovered = open && inside(min, max);
        list.add_rect(min, max, CELL_BG)
            .filled(true)
            .rounding(style::ROUNDING)
            .build();
        if hovered {
            list.add_rect(min, max, FOCUS_EDGE)
                .rounding(style::ROUNDING)
                .thickness(1.5)
                .build();
        }
        list.add_text(
            [x + 12.0, y + 5.0],
            if open { TEXT } else { DISABLED },
            label,
        );
        if hovered && clicked {
            press = Some(action);
        }
        x += cell_width + 8.0;
    }
    y += line + 10.0 + 12.0;

    // The footer: the buttons of the device the player is on.
    let hints = if panel.reader.pad_last() {
        &PAD_HINT
    } else {
        &KEY_HINT
    };
    ds2_overlay::panels::hint_bar(ui, &list, [inner_left, y], inner_right, hints);

    if ui.is_mouse_clicked(MouseButton::Right) {
        press = Some(Press::Close);
    }
    if let Some(press) = press {
        panel.apply(press);
    }
}

/// `text`, cut at the right with an ellipsis to fit `width` pixels.
fn clip(ui: &Ui, text: &str, width: f32) -> String {
    if ui.calc_text_size(text)[0] <= width {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "...";
        if ui.calc_text_size(&candidate)[0] <= width {
            return candidate;
        }
    }
    String::new()
}

/// `text`, cut at the LEFT to fit, because the part next to the caret is the part being edited.
fn clip_left(ui: &Ui, text: &str, width: f32) -> String {
    if ui.calc_text_size(text)[0] <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let candidate = format!("...{}", chars[start..].iter().collect::<String>());
        if ui.calc_text_size(&candidate)[0] <= width {
            return candidate;
        }
    }
    String::new()
}
