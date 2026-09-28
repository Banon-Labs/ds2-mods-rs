//! The region playlists, the file they are kept in, and what happens when a track ends. No game
//! here: every rule is tested on the host.

use std::fmt;
use std::fmt::Write as _;

/// One track: the FSB bank its audio lives in and the FMOD event name.
///
/// The name alone is not a key: `m101900001` is two different tracks, in `frpg2_sm1019` and in
/// `frpg2_sm5036`. When FMOD does not say which bank an event plays from, the bank is written as
/// `p<project id>` instead.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackKey {
    /// The bank, e.g. `frpg2_sm1004`.
    pub bank: String,
    /// The event name, e.g. `m100400001`.
    pub name: String,
}

impl TrackKey {
    /// A key from its two halves.
    pub fn new(bank: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            bank: bank.into(),
            name: name.into(),
        }
    }

    /// `bank/name`, split at the last `/`. `None` when either half is empty.
    pub fn parse(text: &str) -> Option<Self> {
        let (bank, name) = text.trim().rsplit_once('/')?;
        (!bank.is_empty() && !name.is_empty()).then(|| Self::new(bank, name))
    }
}

impl fmt::Display for TrackKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.bank, self.name)
    }
}

/// One region's playlist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    /// The track the game plays here, which is also the region's key.
    pub default: TrackKey,
    /// The map index the player was in when the region was first edited. A label for a reader of
    /// the file; nothing is keyed on it.
    pub map: Option<u32>,
    /// Whether the game's own track is in the playlist.
    pub include_default: bool,
    /// Loop the playing track forever, as the game does. Off: play it once, then the next.
    pub repeat: bool,
    /// Tracks added to the region, in order.
    pub tracks: Vec<TrackKey>,
}

impl Region {
    /// A region as the game has it: its own track, looping.
    pub fn new(default: TrackKey, map: Option<u32>) -> Self {
        Self {
            default,
            map,
            include_default: true,
            repeat: true,
            tracks: Vec::new(),
        }
    }

    /// Whether this says nothing the game does not already do.
    pub fn is_vanilla(&self) -> bool {
        self.include_default && self.repeat && self.tracks.is_empty()
    }

    /// The tracks to play, in order: the default first when it is included, then the added ones.
    pub fn entries(&self) -> Vec<TrackKey> {
        let mut out = Vec::new();
        if self.include_default {
            out.push(self.default.clone());
        }
        for track in &self.tracks {
            if !out.contains(track) {
                out.push(track.clone());
            }
        }
        out
    }

    /// Add a track at the end. Adding the default puts it back. `false` when it was already in.
    pub fn add(&mut self, track: TrackKey) -> bool {
        if track == self.default {
            return !std::mem::replace(&mut self.include_default, true);
        }
        if self.tracks.contains(&track) {
            return false;
        }
        self.tracks.push(track);
        true
    }

    /// Take a track out. Taking the default out leaves it off the playlist but keeps the region.
    pub fn remove(&mut self, track: &TrackKey) -> bool {
        if *track == self.default {
            return std::mem::replace(&mut self.include_default, false);
        }
        let before = self.tracks.len();
        self.tracks.retain(|t| t != track);
        self.tracks.len() != before
    }

    /// Move an added track one place earlier (`up`) or later. `false` at either end.
    pub fn move_track(&mut self, track: &TrackKey, up: bool) -> bool {
        let Some(index) = self.tracks.iter().position(|t| t == track) else {
            return false;
        };
        let other = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|i| *i < self.tracks.len())
        };
        match other {
            Some(other) => {
                self.tracks.swap(index, other);
                true
            }
            None => false,
        }
    }
}

/// What to do when the playing track reaches its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterEnd {
    /// Repeat is on: the track loops, which the channel does by itself.
    Loop,
    /// Play this entry next.
    Play(usize),
    /// Nothing more to play: repeat is off and the playlist has at most one track.
    Silence,
}

/// The rule for the end of a track. With repeat off a playlist of several tracks goes round; a
/// playlist of one track plays it once and stops, which is what "stop it repeating" asks for.
pub fn after_end(entries: usize, index: usize, repeat: bool) -> AfterEnd {
    if repeat {
        AfterEnd::Loop
    } else if entries <= 1 {
        AfterEnd::Silence
    } else {
        AfterEnd::Play((index + 1) % entries)
    }
}

/// The entry after (`forward`) or before `index`, going round. `None` for an empty playlist.
pub fn step(entries: usize, index: usize, forward: bool) -> Option<usize> {
    if entries == 0 {
        return None;
    }
    let index = index.min(entries - 1);
    Some(if forward {
        (index + 1) % entries
    } else {
        (index + entries - 1) % entries
    })
}

/// Every region the player has been told about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Playlists {
    /// In the order they were first edited, which is the order they are written.
    pub regions: Vec<Region>,
}

/// What the file opens with.
const FILE_HEADER: &str = "# Region music playlists, written by the in-game Music panel (ds2-music-probe).\n\
# A region is the track the game plays there, as bank/name. Edit in the panel, or here with the\n\
# game closed.\n";

impl Playlists {
    /// An empty set; `const` so it can seed a static.
    pub const fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// The region whose default track is `default`.
    pub fn get(&self, default: &TrackKey) -> Option<&Region> {
        self.regions.iter().find(|r| r.default == *default)
    }

    /// The region whose default track is `default`, made as the game has it when there is none.
    pub fn get_or_insert(&mut self, default: &TrackKey, map: Option<u32>) -> &mut Region {
        let index = match self.regions.iter().position(|r| r.default == *default) {
            Some(index) => index,
            None => {
                self.regions.push(Region::new(default.clone(), map));
                self.regions.len() - 1
            }
        };
        &mut self.regions[index]
    }

    /// Read a file's text. Every line that could not be used is named in the second half, and the
    /// rest of the file is still read.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut out = Self::new();
        let mut problems = Vec::new();
        let mut current: Option<usize> = None;
        for (number, raw) in text.lines().enumerate() {
            let number = number + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(header) = line
                .strip_prefix("[region.")
                .and_then(|rest| rest.strip_suffix(']'))
            {
                let key = header.trim().trim_matches('"');
                current = None;
                match TrackKey::parse(key) {
                    Some(key) if out.get(&key).is_some() => {
                        problems.push(format!("line {number}: region {key} appears twice"));
                    }
                    Some(key) => {
                        out.regions.push(Region::new(key, None));
                        current = Some(out.regions.len() - 1);
                    }
                    None => problems.push(format!(
                        "line {number}: region key {key:?} is not bank/name"
                    )),
                }
                continue;
            }
            if line.starts_with('[') {
                problems.push(format!("line {number}: unknown section {line}"));
                current = None;
                continue;
            }
            let Some(index) = current else {
                problems.push(format!(
                    "line {number}: {line:?} is outside a [region.\"bank/name\"] section"
                ));
                continue;
            };
            let Some((key, value)) = line.split_once('=') else {
                problems.push(format!("line {number}: {line:?} is not key = value"));
                continue;
            };
            let region = &mut out.regions[index];
            let (key, value) = (key.trim(), value.trim());
            let bad = || format!("line {number}: {key} = {value} is not usable");
            match key {
                "map" => match value.parse::<u32>() {
                    Ok(map) => region.map = Some(map),
                    Err(_) => problems.push(bad()),
                },
                "include_default" => match parse_bool(value) {
                    Some(on) => region.include_default = on,
                    None => problems.push(bad()),
                },
                "repeat" => match parse_bool(value) {
                    Some(on) => region.repeat = on,
                    None => problems.push(bad()),
                },
                "tracks" => match parse_list(value) {
                    Some(list) => {
                        for item in list {
                            match TrackKey::parse(&item) {
                                Some(track) => {
                                    region.add(track);
                                }
                                None => problems.push(format!(
                                    "line {number}: track {item:?} is not bank/name"
                                )),
                            }
                        }
                    }
                    None => problems.push(bad()),
                },
                other => problems.push(format!("line {number}: unknown key {other}")),
            }
        }
        (out, problems)
    }

    /// The file's text. [`Self::parse`] reads it back to the same value.
    pub fn to_text(&self) -> String {
        let mut text = String::from(FILE_HEADER);
        for region in &self.regions {
            let _ = writeln!(text, "\n[region.\"{}\"]", region.default);
            if let Some(map) = region.map {
                let _ = writeln!(text, "map = {map}");
            }
            let _ = writeln!(text, "include_default = {}", region.include_default);
            let _ = writeln!(text, "repeat = {}", region.repeat);
            let tracks: Vec<String> = region.tracks.iter().map(|t| format!("\"{t}\"")).collect();
            let _ = writeln!(text, "tracks = [{}]", tracks.join(", "));
        }
        text
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// `["a/b", "c/d"]`, with an optional trailing comma.
fn parse_list(value: &str) -> Option<Vec<String>> {
    let inner = value.strip_prefix('[')?.strip_suffix(']')?.trim();
    let mut list = Vec::new();
    for item in inner.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        list.push(item.strip_prefix('"')?.strip_suffix('"')?.to_owned());
    }
    Some(list)
}

#[cfg(test)]
mod tests {
    use super::{AfterEnd, Playlists, Region, TrackKey, after_end, step};

    fn key(text: &str) -> TrackKey {
        TrackKey::parse(text).expect("a bank/name key")
    }

    #[test]
    fn a_file_reads_back_to_what_was_written() {
        let mut lists = Playlists::new();
        let region = lists.get_or_insert(&key("frpg2_sm1004/m100400001"), Some(1));
        region.repeat = false;
        region.add(key("frpg2_sm1016/m101600001"));
        region.add(key("frpg2_sm2021/m202100004_andeal"));
        let other = lists.get_or_insert(&key("frpg2_smain/m000000002"), None);
        other.include_default = false;
        let (read, problems) = Playlists::parse(&lists.to_text());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(read, lists);
    }

    #[test]
    fn a_bad_line_is_named_and_the_rest_is_read() {
        let text = "stray = 1\n[region.\"frpg2_sm1004/m100400001\"]\nrepeat = maybe\n\
                    tracks = [\"frpg2_sm1016/m101600001\", \"nobank\"]\n[other]\n";
        let (read, problems) = Playlists::parse(text);
        assert_eq!(problems.len(), 4, "{problems:?}");
        let region = read.get(&key("frpg2_sm1004/m100400001")).expect("region");
        assert!(region.repeat);
        assert_eq!(region.tracks, vec![key("frpg2_sm1016/m101600001")]);
    }

    #[test]
    fn the_same_name_in_two_banks_is_two_tracks() {
        let mut region = Region::new(key("frpg2_sm1004/m100400001"), None);
        assert!(region.add(key("frpg2_sm1019/m101900001")));
        assert!(region.add(key("frpg2_sm5036/m101900001")));
        assert!(!region.add(key("frpg2_sm5036/m101900001")));
        assert_eq!(region.entries().len(), 3);
    }

    #[test]
    fn removing_the_default_keeps_the_region() {
        let default = key("frpg2_sm1004/m100400001");
        let mut region = Region::new(default.clone(), None);
        region.add(key("frpg2_sm1016/m101600001"));
        assert!(region.remove(&default));
        assert_eq!(region.entries(), vec![key("frpg2_sm1016/m101600001")]);
        assert!(!region.is_vanilla());
        assert!(region.add(default.clone()));
        assert_eq!(region.entries()[0], default);
    }

    #[test]
    fn a_region_that_says_nothing_new_is_vanilla() {
        let mut region = Region::new(key("frpg2_sm1004/m100400001"), Some(1));
        assert!(region.is_vanilla());
        region.repeat = false;
        assert!(!region.is_vanilla());
    }

    #[test]
    fn moving_a_track() {
        let mut region = Region::new(key("b/d"), None);
        region.add(key("b/one"));
        region.add(key("b/two"));
        assert!(region.move_track(&key("b/two"), true));
        assert_eq!(region.tracks, vec![key("b/two"), key("b/one")]);
        assert!(!region.move_track(&key("b/two"), true));
        assert!(!region.move_track(&key("b/one"), false));
    }

    #[test]
    fn what_happens_at_the_end_of_a_track() {
        assert_eq!(after_end(3, 0, true), AfterEnd::Loop);
        assert_eq!(after_end(1, 0, false), AfterEnd::Silence);
        assert_eq!(after_end(0, 0, false), AfterEnd::Silence);
        assert_eq!(after_end(3, 0, false), AfterEnd::Play(1));
        assert_eq!(after_end(3, 2, false), AfterEnd::Play(0));
    }

    #[test]
    fn next_and_previous_go_round() {
        assert_eq!(step(3, 2, true), Some(0));
        assert_eq!(step(3, 0, false), Some(2));
        assert_eq!(step(0, 0, true), None);
        assert_eq!(step(2, 9, true), Some(0));
    }
}
