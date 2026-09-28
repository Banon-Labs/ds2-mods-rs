//! The player. Runs only on the game's sound thread, inside the fronted FMOD calls; the panel talks
//! to it through [`push`] and reads it through [`SNAPSHOT`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::fmod::{self, EventInfo, FMOD_OK, LOOP_FOREVER};
use crate::install::{log, map_index, now_ms};
use crate::playlist::{AfterEnd, Region, TrackKey, after_end, step};
use crate::store::PLAYLISTS;
use crate::{LOG_PREFIX, SAMPLE_EVERY_MS, is_music_name};

/// What the panel asks for.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Command {
    /// Move the playing track to this many ms.
    Seek(u32),
    /// The next (`true`) or previous playlist entry.
    Step(bool),
    /// Play this playlist entry now.
    Play(usize),
    /// The current region's playlist was edited.
    RegionChanged,
}

static COMMANDS: Mutex<Vec<Command>> = Mutex::new(Vec::new());

/// Queue a command for the sound thread.
pub(crate) fn push(command: Command) {
    if let Ok(mut queue) = COMMANDS.lock() {
        queue.push(command);
    }
}

/// What the panel shows, refreshed every tick.
#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    /// The track that is audible, if any.
    pub(crate) now: Option<TrackKey>,
    /// Whether it is the player's own instance rather than the game's.
    pub(crate) ours: bool,
    pub(crate) position_ms: Option<u32>,
    pub(crate) length_ms: Option<u32>,
    /// Repeat is off and the playlist ran out.
    pub(crate) silent: bool,
    /// The region: the looping track the game started last.
    pub(crate) region: Option<TrackKey>,
    pub(crate) map: Option<u32>,
    /// Which playlist entry is playing.
    pub(crate) index: Option<usize>,
    /// Every music track the event system knows, sorted.
    pub(crate) catalog: Arc<Vec<TrackKey>>,
    pub(crate) catalog_done: bool,
}

/// The latest [`Snapshot`].
pub(crate) static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);

/// Milliseconds between two ticks of the player. The game calls `getState` far more often.
const TICK_MS: u64 = 100;

/// A position this far below the last one is the loop going round, not jitter.
const WRAP_MS: u32 = 2_000;

/// A track that has no channel this long after it was started never will.
const NO_CHANNEL_MS: u64 = 6_000;

/// System ids read per tick while the catalog is being built.
const SCAN_PER_TICK: u32 = 256;

/// The highest system id tried, and how far past the last event found to keep looking.
const SCAN_MAX_ID: u32 = 1 << 16;
const SCAN_MISS_LIMIT: u32 = 4_096;

static LAST_TICK: AtomicU64 = AtomicU64::new(0);

/// The looping music event the game started last: the region.
#[derive(Clone, Debug)]
struct Game {
    handle: usize,
    key: TrackKey,
    system_id: u32,
    map: Option<u32>,
}

/// The track that is audible.
#[derive(Clone, Debug)]
struct Playing {
    handle: usize,
    key: TrackKey,
    /// The player's own instance, rather than the game's event.
    ours: bool,
    index: Option<usize>,
    started_ms: u64,
    channel: usize,
    had_channel: bool,
    /// The channel `setLoopCount(0)` was applied to, or `0`.
    loop_cleared: usize,
    last_position: Option<u32>,
}

#[derive(Default)]
struct Catalog {
    next_id: u32,
    done: bool,
    /// `getNumEvents` when this scan started; a different count later starts it again.
    total: Option<i32>,
    found: i32,
    last_hit: u32,
    tracks: Vec<(TrackKey, u32)>,
    published: Arc<Vec<TrackKey>>,
}

#[derive(Default)]
struct Engine {
    catalog: Catalog,
    game: Option<Game>,
    playing: Option<Playing>,
    silent: bool,
    /// Where each region's playlist was when the game last left it.
    resume: HashMap<TrackKey, usize>,
    failures: usize,
    last_sample: u64,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

fn with_engine(f: impl FnOnce(&mut Engine)) {
    if let Ok(mut guard) = ENGINE.lock() {
        f(guard.get_or_insert_with(Engine::default));
    }
}

/// The game started an event. Called after the original, with its `getInfo`.
pub(crate) fn on_start(event: usize, info: Option<&EventInfo>) {
    let Some(info) = info else {
        return;
    };
    if !is_music_name(&info.name) || !info.looping() {
        return;
    }
    with_engine(|engine| engine.game_started(event, info));
}

/// The game paused or unpaused an event.
pub(crate) fn on_paused(event: usize, paused: bool, info: Option<&EventInfo>) {
    with_engine(|engine| {
        let is_region = engine.game.as_ref().is_some_and(|g| g.handle == event);
        if !is_region {
            // A looping music event made audible without a start we saw.
            if !paused
                && let Some(info) = info
                && is_music_name(&info.name)
                && info.looping()
            {
                engine.game_started(event, info);
            }
            return;
        }
        if let Some(playing) = engine.playing.as_ref().filter(|p| p.ours) {
            let rc = fmod::set_paused(playing.handle, paused);
            log(format_args!(
                "{LOG_PREFIX} follow pause={paused} region={} ours={} handle=0x{:x} rc={rc}",
                engine
                    .game
                    .as_ref()
                    .map_or_else(String::new, |g| g.key.to_string()),
                playing.key,
                playing.handle
            ));
        }
    });
}

/// The game is stopping an event. Called before the original.
pub(crate) fn on_stop(event: usize, immediate: bool) {
    with_engine(|engine| {
        let Some(game) = engine.game.clone().filter(|g| g.handle == event) else {
            return;
        };
        if let Some(playing) = engine.playing.take() {
            if playing.ours {
                let rc = fmod::stop(playing.handle, immediate);
                log(format_args!(
                    "{LOG_PREFIX} follow stop region={} ours={} handle=0x{:x} immediate={immediate} \
                     rc={rc}",
                    game.key, playing.key, playing.handle
                ));
            }
            if let Some(index) = playing.index {
                engine.resume.insert(game.key.clone(), index);
            }
        }
        engine.game = None;
        engine.silent = false;
    });
}

/// One tick, on every `getState` call; does its work at most every [`TICK_MS`].
pub(crate) fn tick() {
    let now = now_ms();
    let last = LAST_TICK.load(Ordering::Relaxed);
    if now.saturating_sub(last) < TICK_MS
        || LAST_TICK
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    let commands = COMMANDS
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default();
    with_engine(|engine| {
        for command in commands {
            engine.command(command);
        }
        engine.scan(SCAN_PER_TICK);
        engine.watch(now);
        engine.publish(now);
    });
}

/// The region's playlist when it says something the game does not; `None` plays as shipped.
fn managed_region(key: &TrackKey) -> Option<Region> {
    PLAYLISTS
        .lock()
        .ok()?
        .get(key)
        .filter(|r| !r.is_vanilla())
        .cloned()
}

/// The region's playlist, or the game's own as a one-track playlist.
fn any_region(game: &Game) -> Region {
    managed_region(&game.key).unwrap_or_else(|| Region::new(game.key.clone(), game.map))
}

impl Engine {
    fn game_started(&mut self, event: usize, info: &EventInfo) {
        if self
            .playing
            .as_ref()
            .is_some_and(|p| p.ours && p.handle == event)
        {
            return;
        }
        if self.game.as_ref().is_some_and(|g| g.handle == event) {
            // The prepare pattern starts, pauses and unpauses the same event; one entry is enough.
            return;
        }
        // A mute can only be ours, and it must never outlive the start it was for.
        fmod::set_mute(event, false);
        self.stop_ours(true);
        let full = fmod::event_info(event, true).unwrap_or_else(|| info.clone());
        let key = full.key();
        let map = map_index();
        log(format_args!(
            "{LOG_PREFIX} region key={key} handle=0x{event:x} system={} map={} managed={}",
            full.system_id,
            map.map_or_else(|| "-".to_owned(), |m| m.to_string()),
            managed_region(&key).is_some()
        ));
        self.game = Some(Game {
            handle: event,
            key: key.clone(),
            system_id: full.system_id,
            map,
        });
        self.silent = false;
        self.failures = 0;
        match managed_region(&key) {
            None => self.follow_game_track(None),
            Some(region) => {
                let entries = region.entries();
                if entries.is_empty() {
                    self.go_silent("the region's playlist is empty");
                    return;
                }
                let index = self
                    .resume
                    .get(&key)
                    .copied()
                    .filter(|i| *i < entries.len())
                    .unwrap_or(0);
                self.play_entry(&entries, index, false);
            }
        }
    }

    /// Make the game's own event the playing track.
    fn follow_game_track(&mut self, index: Option<usize>) {
        let Some(game) = self.game.clone() else {
            return;
        };
        self.playing = Some(Playing {
            handle: game.handle,
            key: game.key,
            ours: false,
            index,
            started_ms: now_ms(),
            channel: 0,
            had_channel: false,
            loop_cleared: 0,
            last_position: None,
        });
    }

    fn stop_ours(&mut self, immediate: bool) {
        if let Some(playing) = self.playing.take_if(|p| p.ours) {
            let rc = fmod::stop(playing.handle, immediate);
            log(format_args!(
                "{LOG_PREFIX} stop ours={} handle=0x{:x} rc={rc}",
                playing.key, playing.handle
            ));
        }
    }

    fn go_silent(&mut self, why: &str) {
        self.stop_ours(true);
        if let Some(game) = &self.game {
            fmod::set_mute(game.handle, true);
        }
        self.playing = None;
        self.silent = true;
        log(format_args!("{LOG_PREFIX} silence -- {why}"));
    }

    fn lookup(&mut self, key: &TrackKey) -> Option<u32> {
        if !self.catalog.done {
            // Needed now, so finish the scan now rather than a tick at a time.
            self.scan(SCAN_MAX_ID);
        }
        let tracks = &self.catalog.tracks;
        tracks
            .iter()
            .find(|(k, _)| k == key)
            .or_else(|| {
                let mut by_name = tracks.iter().filter(|(k, _)| k.name == key.name);
                let first = by_name.next();
                by_name.next().is_none().then_some(first).flatten()
            })
            .map(|(_, id)| *id)
    }

    /// Play `entries[index]`. `restart` puts the game's own track back to its start when it is the
    /// one chosen; at the region's entry it has only just been started.
    fn play_entry(&mut self, entries: &[TrackKey], index: usize, restart: bool) {
        let Some(game) = self.game.clone() else {
            return;
        };
        let Some(key) = entries.get(index).cloned() else {
            self.go_silent("no such playlist entry");
            return;
        };
        self.stop_ours(true);
        self.silent = false;
        self.resume.insert(game.key.clone(), index);
        let system_id = if key == game.key {
            Some(game.system_id)
        } else {
            self.lookup(&key)
        };
        if system_id == Some(game.system_id) {
            let channel = fmod::channel_of(game.handle);
            let how = match channel {
                Some(channel) if restart => {
                    format!("seek-to-0 rc={}", fmod::set_position(channel, 0))
                }
                Some(_) => "as-is".to_owned(),
                None => format!("start rc={}", fmod::start(game.handle)),
            };
            let rc = fmod::set_mute(game.handle, false);
            log(format_args!(
                "{LOG_PREFIX} play index={index} key={key} the-game's-own handle=0x{:x} {how} \
                 unmute-rc={rc}",
                game.handle
            ));
            self.follow_game_track(Some(index));
            return;
        }
        let Some(system_id) = system_id else {
            log(format_args!(
                "{LOG_PREFIX} play index={index} key={key} FAILED: not among the {} music events \
                 the event system knows",
                self.catalog.tracks.len()
            ));
            self.fail_next(entries, index);
            return;
        };
        let Some(system) = fmod::event_system() else {
            self.fail_next(entries, index);
            return;
        };
        match fmod::event_by_system_id(system, system_id, ds2_rva::FMOD_EVENT_MODE_DEFAULT) {
            Err(rc) => {
                log(format_args!(
                    "{LOG_PREFIX} play index={index} key={key} system={system_id} FAILED: \
                     getEventBySystemID rc={rc}"
                ));
                self.fail_next(entries, index);
            }
            Ok(handle) => {
                let mute_rc = fmod::set_mute(game.handle, true);
                let rc = fmod::start(handle);
                log(format_args!(
                    "{LOG_PREFIX} play index={index} key={key} system={system_id} ours \
                     handle=0x{handle:x} start-rc={rc} game-track-muted-rc={mute_rc}"
                ));
                if rc != FMOD_OK {
                    self.fail_next(entries, index);
                    return;
                }
                self.playing = Some(Playing {
                    handle,
                    key,
                    ours: true,
                    index: Some(index),
                    started_ms: now_ms(),
                    channel: 0,
                    had_channel: false,
                    loop_cleared: 0,
                    last_position: None,
                });
            }
        }
    }

    fn fail_next(&mut self, entries: &[TrackKey], index: usize) {
        self.failures += 1;
        if self.failures >= entries.len() {
            self.go_silent("no track in the playlist could be played");
            return;
        }
        let next = (index + 1) % entries.len();
        self.play_entry(entries, next, true);
    }

    fn command(&mut self, command: Command) {
        let Some(game) = self.game.clone() else {
            log(format_args!(
                "{LOG_PREFIX} command {command:?} ignored: no region track is playing"
            ));
            return;
        };
        match command {
            Command::Seek(ms) => {
                let Some(playing) = &self.playing else {
                    return;
                };
                let Some(channel) = fmod::channel_of(playing.handle) else {
                    log(format_args!(
                        "{LOG_PREFIX} seek to={ms}ms ignored: {} has no channel",
                        playing.key
                    ));
                    return;
                };
                let before = fmod::position(channel);
                let rc = fmod::set_position(channel, ms);
                let after = fmod::position(channel);
                log(format_args!(
                    "{LOG_PREFIX} seek key={} from={} to={ms}ms read-back={} rc={rc}",
                    playing.key,
                    before.map_or_else(|| "-".to_owned(), |v| format!("{v}ms")),
                    after.map_or_else(|| "-".to_owned(), |v| format!("{v}ms"))
                ));
                if let Some(playing) = &mut self.playing {
                    playing.last_position = after;
                }
            }
            Command::Step(forward) => {
                let entries = any_region(&game).entries();
                let current = self.playing.as_ref().and_then(|p| p.index).unwrap_or(0);
                if let Some(next) = step(entries.len(), current, forward) {
                    self.failures = 0;
                    self.play_entry(&entries, next, true);
                }
            }
            Command::Play(index) => {
                let entries = any_region(&game).entries();
                self.failures = 0;
                self.play_entry(&entries, index, true);
            }
            Command::RegionChanged => self.region_changed(&game),
        }
    }

    fn region_changed(&mut self, game: &Game) {
        let Some(region) = managed_region(&game.key) else {
            // Back to the game's own: its event plays, unmuted, and the tick puts its loop back.
            self.stop_ours(true);
            let channel = fmod::channel_of(game.handle);
            if channel.is_none() {
                fmod::start(game.handle);
            }
            fmod::set_mute(game.handle, false);
            self.silent = false;
            let loop_cleared = self
                .playing
                .as_ref()
                .filter(|p| !p.ours)
                .map_or(0, |p| p.loop_cleared);
            self.follow_game_track(Some(0));
            if let Some(playing) = &mut self.playing {
                playing.loop_cleared = loop_cleared;
            }
            log(format_args!(
                "{LOG_PREFIX} region {} is as shipped again",
                game.key
            ));
            return;
        };
        let entries = region.entries();
        log(format_args!(
            "{LOG_PREFIX} region {} changed: include_default={} repeat={} entries={}",
            game.key,
            region.include_default,
            region.repeat,
            entries.len()
        ));
        if entries.is_empty() {
            self.go_silent("the region's playlist is empty");
            return;
        }
        let still = self
            .playing
            .as_ref()
            .and_then(|p| entries.iter().position(|k| *k == p.key));
        match still {
            Some(index) => {
                if let Some(playing) = &mut self.playing {
                    playing.index = Some(index);
                }
            }
            None => {
                self.failures = 0;
                self.play_entry(&entries, 0, true);
            }
        }
    }

    /// Keep the loop the way the region wants it, and notice the end of a track.
    fn watch(&mut self, now: u64) {
        let Some(game) = self.game.clone() else {
            return;
        };
        let region = managed_region(&game.key);
        let repeat = region.as_ref().is_none_or(|r| r.repeat);
        let Some(playing) = self.playing.as_mut() else {
            return;
        };
        let ended = match fmod::channel_of(playing.handle) {
            Some(channel) => {
                playing.had_channel = true;
                playing.channel = channel;
                self.failures = 0;
                if !repeat && playing.loop_cleared != channel {
                    let rc = fmod::set_loop_count(channel, 0);
                    playing.loop_cleared = channel;
                    log(format_args!(
                        "{LOG_PREFIX} repeat off: setLoopCount(0) key={} channel=0x{channel:x} \
                         rc={rc}",
                        playing.key
                    ));
                }
                if repeat && playing.loop_cleared == channel {
                    let rc = fmod::set_loop_count(channel, LOOP_FOREVER);
                    playing.loop_cleared = 0;
                    log(format_args!(
                        "{LOG_PREFIX} repeat on: setLoopCount(-1) key={} channel=0x{channel:x} \
                         rc={rc}",
                        playing.key
                    ));
                }
                let position = fmod::position(channel);
                let wrapped = !repeat
                    && matches!((playing.last_position, position), (Some(a), Some(b)) if b + WRAP_MS < a);
                playing.last_position = position;
                if wrapped {
                    Some("the loop went round despite setLoopCount(0)")
                } else if let Err(rc) = fmod::is_playing(channel)
                    && rc == ds2_rva::FMOD_ERR_INVALID_HANDLE
                {
                    Some("the channel played out")
                } else {
                    None
                }
            }
            None if playing.had_channel => Some("the channel played out"),
            None if now.saturating_sub(playing.started_ms) > NO_CHANNEL_MS => {
                Some("it never got a channel")
            }
            None => None,
        };
        let Some(reason) = ended else {
            return;
        };
        let (key, index, last) = (
            playing.key.clone(),
            playing.index.unwrap_or(0),
            playing.last_position,
        );
        log(format_args!(
            "{LOG_PREFIX} ended key={key} reason=\"{reason}\" last-pos={}",
            last.map_or_else(|| "-".to_owned(), |v| format!("{v}ms"))
        ));
        let region = region.unwrap_or_else(|| Region::new(game.key.clone(), game.map));
        let entries = region.entries();
        match after_end(entries.len(), index, region.repeat) {
            AfterEnd::Loop => {
                // Only a track that never started ends with repeat on; try the next one.
                if reason == "it never got a channel" {
                    self.fail_next(&entries, index);
                } else {
                    self.playing = None;
                }
            }
            AfterEnd::Silence => self.go_silent("repeat is off and the playlist has one track"),
            AfterEnd::Play(next) => self.play_entry(&entries, next, true),
        }
    }

    /// Build the catalog a slice at a time: every event the event system knows, by system id, kept
    /// when it is music.
    fn scan(&mut self, budget: u32) {
        let Some(system) = fmod::event_system() else {
            return;
        };
        let total = fmod::num_events(system);
        let catalog = &mut self.catalog;
        if catalog.done {
            if total.is_some() && total != catalog.total {
                log(format_args!(
                    "{LOG_PREFIX} catalog: the event count went from {:?} to {total:?}; reading \
                     it again",
                    catalog.total
                ));
                let published = catalog.published.clone();
                *catalog = Catalog {
                    published,
                    ..Catalog::default()
                };
            } else {
                return;
            }
        }
        if catalog.next_id == 0 {
            catalog.total = total;
            if total.is_none_or(|t| t <= 0) {
                return;
            }
        }
        for _ in 0..budget {
            let id = catalog.next_id;
            catalog.next_id += 1;
            if let Ok(handle) =
                fmod::event_by_system_id(system, id, ds2_rva::FMOD_EVENT_MODE_INFOONLY)
            {
                catalog.found += 1;
                catalog.last_hit = id;
                if let Some(info) = fmod::event_info(handle, true)
                    && is_music_name(&info.name)
                {
                    catalog.tracks.push((info.key(), id));
                }
            }
            let all_found = catalog.total.is_some_and(|t| catalog.found >= t);
            let gave_up =
                id >= SCAN_MAX_ID || (catalog.found > 0 && id - catalog.last_hit > SCAN_MISS_LIMIT);
            if all_found || gave_up {
                self.finish_scan(id);
                return;
            }
        }
    }

    fn finish_scan(&mut self, last_id: u32) {
        let catalog = &mut self.catalog;
        catalog.done = true;
        catalog.tracks.sort();
        catalog.tracks.dedup_by(|a, b| a.0 == b.0);
        catalog.published = Arc::new(catalog.tracks.iter().map(|(k, _)| k.clone()).collect());
        let mut banks: Vec<(String, Vec<String>)> = Vec::new();
        for (key, _) in &catalog.tracks {
            match banks.last_mut() {
                Some((bank, names)) if *bank == key.bank => names.push(key.name.clone()),
                _ => banks.push((key.bank.clone(), vec![key.name.clone()])),
            }
        }
        log(format_args!(
            "{LOG_PREFIX} catalog: {} events found of {:?}, last id {last_id}; {} music tracks in \
             {} banks",
            catalog.found,
            catalog.total,
            catalog.tracks.len(),
            banks.len()
        ));
        for (bank, names) in banks {
            log(format_args!(
                "{LOG_PREFIX} catalog bank={bank} tracks={}",
                names.join(" ")
            ));
        }
    }

    fn publish(&mut self, now: u64) {
        // The region track starts during the load, before the player's map entity exists, so the
        // map index read then is the sentinel. Fill it in once it resolves.
        if let Some(game) = self.game.as_mut().filter(|g| g.map.is_none()) {
            game.map = map_index();
        }
        let (position, length) = self
            .playing
            .as_ref()
            .and_then(|p| fmod::channel_of(p.handle))
            .map_or((None, None), |c| (fmod::position(c), fmod::sound_length(c)));
        let snapshot = Snapshot {
            now: self.playing.as_ref().map(|p| p.key.clone()),
            ours: self.playing.as_ref().is_some_and(|p| p.ours),
            position_ms: position,
            length_ms: length,
            silent: self.silent,
            region: self.game.as_ref().map(|g| g.key.clone()),
            map: self.game.as_ref().and_then(|g| g.map),
            index: self.playing.as_ref().and_then(|p| p.index),
            catalog: self.catalog.published.clone(),
            catalog_done: self.catalog.done,
        };
        if now.saturating_sub(self.last_sample) >= SAMPLE_EVERY_MS
            && let Some(playing) = &self.playing
        {
            self.last_sample = now;
            log(format_args!(
                "{LOG_PREFIX} sample key={} ours={} handle=0x{:x} channel-pos={} sound-len={} \
                 region={} map={}",
                playing.key,
                playing.ours,
                playing.handle,
                position.map_or_else(|| "-".to_owned(), |v| format!("{v}ms")),
                length.map_or_else(|| "-".to_owned(), |v| format!("{v}ms")),
                snapshot
                    .region
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), ToString::to_string),
                snapshot
                    .map
                    .map_or_else(|| "-".to_owned(), |m| m.to_string())
            ));
        }
        if let Ok(mut guard) = SNAPSHOT.lock() {
            *guard = Some(snapshot);
        }
    }
}
