//! The kept list: the effects the player marked in the selector, which stay applied.
//!
//! Ported from er-net-effects' `marked_effects.rs`, file format and all: a `.jsonc` id array with
//! each id's name as a trailing comment. One difference in meaning: there the list is a research
//! note the DLL never reads back; here it is the set the DLL keeps on the player, re-applied each
//! time one runs out, and it is read back at the next launch.
//!
//! The file is [`MARKED_FILE_NAME`] beside `DarkSoulsII.exe`.

/// The file name beside the game.
pub const MARKED_FILE_NAME: &str = "ds2-net-effects-marked.jsonc";

/// The comment block at the top of the written file. Written every time, so the file explains
/// itself; the trailing newline is part of it.
pub const FILE_HEADER: &str = "\
// ds2-net-effects: SpEffect ids marked in the selector to stay applied.
//
// Written by the mark keys (Alt+M toggles, numpad + adds, numpad - removes). While net effects
// is on (F9), every id here is applied to your character once one is loaded, and applied again
// each time it runs out; turning net effects off takes them off. Remove a line (or unmark it in
// game) to stop keeping it.
";

/// Read the ids out of a marked file.
///
/// Tolerant, as a hand-edited file needs: comments, brackets, commas and blank lines are noise,
/// anything that parses as an integer is an id, a token that does not is skipped. Order is kept
/// and duplicates are dropped.
pub fn parse(text: &str) -> Vec<i32> {
    let mut ids = Vec::new();
    for line in text.lines() {
        let code = line.split("//").next().unwrap_or_default();
        for token in code.split([',', '[', ']', ' ', '\t']) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(id) = token.parse::<i32>()
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
    }
    ids
}

/// Render the list as the `.jsonc` written to disk, each id with its name as a comment.
pub fn render<'a>(entries: impl IntoIterator<Item = (i32, &'a str)>) -> String {
    let mut out = String::from(FILE_HEADER);
    out.push_str("[\n");
    for (id, name) in entries {
        let name = comment_safe(name);
        if name.is_empty() {
            out.push_str(&format!("  {id},\n"));
        } else {
            out.push_str(&format!("  {id}, // {name}\n"));
        }
    }
    out.push_str("]\n");
    out
}

/// Flatten a name into something that cannot end its own comment or start a new line.
fn comment_safe(name: &str) -> String {
    name.replace(['\r', '\n'], " ").replace("//", "/ /")
}

/// Add the id, or take it back out if it is there. Reports whether it is marked now.
pub fn toggle(ids: &mut Vec<i32>, id: i32) -> bool {
    if let Some(position) = ids.iter().position(|existing| *existing == id) {
        ids.remove(position);
        false
    } else {
        ids.push(id);
        true
    }
}

/// Mark the id. `false` when it already was.
pub fn add(ids: &mut Vec<i32>, id: i32) -> bool {
    if ids.contains(&id) {
        return false;
    }
    ids.push(id);
    true
}

/// Unmark the id. `false` when it was not marked.
pub fn remove(ids: &mut Vec<i32>, id: i32) -> bool {
    let before = ids.len();
    ids.retain(|existing| *existing != id);
    ids.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the DLL writes it must read back unchanged, or the list does not survive a relaunch.
    #[test]
    fn a_written_file_reads_back_as_the_same_ids() {
        let text = render([(140_001_010, "WideUse: sfx 659"), (2, ""), (-1, "x")]);
        assert_eq!(parse(&text), vec![140_001_010, 2, -1]);
        assert!(
            text.contains("  140001010, // WideUse: sfx 659\n"),
            "{text}"
        );
        assert!(text.trim_end().ends_with(']'));
    }

    #[test]
    fn a_name_cannot_end_its_own_comment() {
        let text = render([(7, "a // b\nc")]);
        assert_eq!(text.lines().filter(|line| line.contains('7')).count(), 1);
        assert_eq!(parse(&text), vec![7]);
    }

    #[test]
    fn a_hand_edited_file_still_reads_and_a_commented_id_is_not_a_mark() {
        assert_eq!(
            parse("// notes\n[\n 1, 2,\n\n 3 // three\n]\n"),
            vec![1, 2, 3]
        );
        assert_eq!(parse("[\n 1,\n // 999 -- ruled out\n]\n"), vec![1]);
        assert_eq!(parse("[1, 1, x, 2]"), vec![1, 2]);
    }

    #[test]
    fn toggle_add_and_remove() {
        let mut ids = vec![1, 2];
        assert!(toggle(&mut ids, 3));
        assert!(!toggle(&mut ids, 2));
        assert_eq!(ids, vec![1, 3]);
        assert!(!add(&mut ids, 1));
        assert!(add(&mut ids, 4));
        assert!(remove(&mut ids, 1));
        assert!(!remove(&mut ids, 1));
        assert_eq!(ids, vec![3, 4]);
    }
}
