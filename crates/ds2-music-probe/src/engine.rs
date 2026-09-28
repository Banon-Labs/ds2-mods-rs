//! The player. Runs only on the game's sound thread, inside the fronted FMOD calls; the panel talks
//! to it through [`push`] and reads it through [`SNAPSHOT`].
//!
//! # Three rules that keep the music audible and the game alive
//!
//! Each is the answer to something a run showed (the evidence is in `ds2-rva` beside the FMOD
//! constants):
//!
//! 1. **The game's own track is silenced last, at its channel, and only once ours is heard.** Ours
//!    starts while the game's plays; the game's channel is muted (`Channel::setMute`) only when our
//!    channel's `getAudibility` is above [`AUDIBLE`]. If ours is not heard within [`CONFIRM_MS`] it
//!    is stopped and the game's track keeps playing. `Event::setMute` is never used to silence: an
//!    event mute does not come off the channel again, which left Majula silent.
//! 2. **Nothing touches the event system during a load.** A region track the game starts is only
//!    recorded; it is keyed by bank, its playlist started, and the catalog scanned, once the game
//!    has made it audible (or after [`LOAD_WAIT_MS`]). With the player on, Majula's own event read
//!    `Event::getVolume` 0 from its first second, before anything of ours played; with the player
//!    off it read 1, audibility 0.759. Repeating `Event::setMute(false)`, the catalog scan and the
//!    bank `getInfo` on that event after the load (`scripts/frida/fmod-volume-bisect.js`) left it
//!    at 1, so what the game resolves to 0 is something done during the load, and the player now
//!    does nothing then.
//! 3. **At most one instance of ours exists, and its memory goes back when it stops.** Every stop is
//!    followed by `EventGroup::freeEventData` on that instance, and a play is refused while FMOD's
//!    allocations are more than [`MEMORY_HEADROOM`] above the most the game used on its own. Eight
//!    banks' instances left alive filled the game's FMOD heap and killed it.
//!
//! Ours is also kept at the listener's position every tick, as the game keeps its own region music:
//! some music events are 3D, and one left at the origin is silent everywhere else.

use std::collections::{HashMap, HashSet};
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
    /// A playlist track is waiting for the load to finish.
    pub(crate) waiting: bool,
    /// Why the last track could not be played, if it could not.
    pub(crate) problem: Option<String>,
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

/// `Channel::getAudibility` above this is heard.
const AUDIBLE: f32 = 0.01;

/// How long a started track of ours has to become audible before the game's is kept instead.
const CONFIRM_MS: u64 = 4_000;

/// How long a playlist waits for the game's own track to be heard after a load before starting
/// anyway (the game's track stays unmuted until ours is heard, so this can never be silence).
const LOAD_WAIT_MS: u64 = 20_000;

/// Bytes FMOD may have allocated beyond the most the game has used without us, before a play is
/// refused. The game's FMOD heap size is not known, so this is kept small: one streamed track.
const MEMORY_HEADROOM: i32 = 8 * 1024 * 1024;

/// Ticks a stopped instance's `freeEventData` is retried while FMOD answers "not ready": 10 s.
const FREE_TRIES: u32 = 100;

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
    map: Option<u32>,
    /// The game's channel we muted, or `0`.
    muted_channel: usize,
}

/// The track that is playing.
#[derive(Clone, Debug)]
struct Playing {
    handle: usize,
    key: TrackKey,
    /// The player's own instance, rather than the game's event.
    ours: bool,
    index: Option<usize>,
    started_ms: u64,
    had_channel: bool,
    /// Ours has been heard, and the game's track is muted.
    confirmed: bool,
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
    /// A region track the game has started and not yet made audible -- a load is in progress --
    /// and when. Rule 2: nothing is done to it, or to the event system, until it is heard.
    arriving: Option<(usize, u64)>,
    silent: bool,
    problem: Option<String>,
    /// Tracks that were started and never heard, this session. Skipped by the playlist.
    unheard: HashSet<TrackKey>,
    /// Where each region's playlist was when the game last left it.
    resume: HashMap<TrackKey, usize>,
    /// The most FMOD had allocated at any tick when no instance of ours was alive.
    game_peak: i32,
    /// Stopped instances of ours whose memory has not been given back yet, with the ticks tried.
    to_free: Vec<(usize, TrackKey, u32)>,
    last_sample: u64,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

fn with_engine(f: impl FnOnce(&mut Engine)) {
    if let Ok(mut guard) = ENGINE.lock() {
        f(guard.get_or_insert_with(Engine::default));
    }
}

fn ms(value: Option<u32>) -> String {
    value.map_or_else(|| "-".to_owned(), |v| format!("{v}ms"))
}

fn memory() -> String {
    fmod::memory_stats().map_or_else(
        || "fmod-mem=?".to_owned(),
        |(current, max)| format!("fmod-mem={}KiB peak={}KiB", current / 1024, max / 1024),
    )
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
                "{LOG_PREFIX} follow pause={paused} ours={} handle=0x{:x} rc={rc}",
                playing.key, playing.handle
            ));
        }
    });
}

/// The game is stopping an event. Called before the original.
pub(crate) fn on_stop(event: usize, immediate: bool) {
    with_engine(|engine| {
        if engine.arriving.is_some_and(|(handle, _)| handle == event) {
            engine.arriving = None;
            return;
        }
        let Some(game) = engine.game.clone().filter(|g| g.handle == event) else {
            return;
        };
        if let Some(index) = engine.playing.as_ref().and_then(|p| p.index) {
            engine.resume.insert(game.key.clone(), index);
        }
        engine.stop_ours(immediate, "the game stopped the region's track");
        engine.unmute_game();
        engine.game = None;
        engine.playing = None;
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
        engine.track_memory();
        engine.free_stopped();
        for command in commands {
            engine.command(command);
        }
        engine.scan(SCAN_PER_TICK);
        engine.follow_listener();
        engine.settle_arrival(now);
        engine.confirm(now);
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
        if self.game.as_ref().is_some_and(|g| g.handle == event)
            || self.arriving.is_some_and(|(handle, _)| handle == event)
        {
            // The prepare pattern starts, pauses and unpauses the same event; one entry is enough.
            return;
        }
        self.stop_ours(true, "a new region track started");
        self.unmute_game();
        self.game = None;
        self.playing = None;
        self.silent = false;
        self.problem = None;
        // Rule 2: no more than that until the load is done with it. `settle_arrival` does the rest.
        self.arriving = Some((event, now_ms()));
        log(format_args!(
            "{LOG_PREFIX} region arriving name={} handle=0x{event:x} system={} -- keyed once the \
             game makes it audible",
            info.name, info.system_id
        ));
    }

    /// Once the arriving region track is heard (the load is done with it), or after
    /// [`LOAD_WAIT_MS`], key it by bank and name and start its playlist, if it has one.
    fn settle_arrival(&mut self, now: u64) {
        let Some((event, since)) = self.arriving else {
            return;
        };
        let heard = fmod::channel_of(event)
            .and_then(fmod::audibility)
            .filter(|a| *a > AUDIBLE);
        let waited = now.saturating_sub(since);
        if heard.is_none() && waited < LOAD_WAIT_MS {
            return;
        }
        self.arriving = None;
        let Some(full) = fmod::event_info(event, true) else {
            log(format_args!(
                "{LOG_PREFIX} region handle=0x{event:x} gone before it was heard"
            ));
            return;
        };
        let key = full.key();
        let map = map_index();
        let managed = managed_region(&key);
        log(format_args!(
            "{LOG_PREFIX} region key={key} handle=0x{event:x} system={} map={} managed={} \
             heard={heard:?} after {waited}ms {}",
            full.system_id,
            map.map_or_else(|| "-".to_owned(), |m| m.to_string()),
            managed.is_some(),
            memory()
        ));
        self.game = Some(Game {
            handle: event,
            key: key.clone(),
            map,
            muted_channel: 0,
        });
        self.follow_game_track(None);
        if let Some(region) = managed {
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
            self.play_entry(&entries, index, false, 0);
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
            had_channel: false,
            confirmed: true,
            loop_cleared: 0,
            last_position: None,
        });
    }

    /// Mute the game's channel. Rule 1: only called once ours is heard, or for a chosen silence.
    fn mute_game(&mut self) {
        let Some(game) = self.game.as_mut() else {
            return;
        };
        let Some(channel) = fmod::channel_of(game.handle) else {
            return;
        };
        if game.muted_channel == channel {
            return;
        }
        let rc = fmod::channel_set_mute(channel, true);
        game.muted_channel = channel;
        log(format_args!(
            "{LOG_PREFIX} game track muted key={} channel=0x{channel:x} rc={rc}",
            game.key
        ));
    }

    fn unmute_game(&mut self) {
        let Some(game) = self.game.as_mut() else {
            return;
        };
        let channel = fmod::channel_of(game.handle);
        let mut rcs = Vec::new();
        for c in [Some(game.muted_channel), channel].into_iter().flatten() {
            if c != 0 {
                rcs.push(fmod::channel_set_mute(c, false));
            }
        }
        let was = std::mem::replace(&mut game.muted_channel, 0);
        let audibility = channel.and_then(fmod::audibility);
        if was != 0 {
            log(format_args!(
                "{LOG_PREFIX} game track unmuted key={} channel=0x{was:x} rc={rcs:?} \
                 audibility={audibility:?}",
                game.key
            ));
        }
    }

    fn stop_ours(&mut self, immediate: bool, why: &str) {
        let Some(playing) = self.playing.take_if(|p| p.ours) else {
            return;
        };
        let rc = fmod::stop(playing.handle, immediate);
        log(format_args!(
            "{LOG_PREFIX} stop ours={} handle=0x{:x} rc={rc} -- {why}; {}",
            playing.key,
            playing.handle,
            memory()
        ));
        // Freed on the ticks that follow: straight after a stop, freeEventData answers
        // FMOD_ERR_NOTREADY (54, measured in the qa run at 8535a80) while the instance winds down.
        self.to_free.push((playing.handle, playing.key, 0));
    }

    /// Rule 3: give back the memory of every stopped instance of ours, retrying while FMOD says it
    /// is not ready yet.
    fn free_stopped(&mut self) {
        let mut still = Vec::new();
        for (handle, key, tries) in std::mem::take(&mut self.to_free) {
            let rc = fmod::free_event_data(handle);
            if rc == ds2_rva::FMOD_ERR_NOTREADY && tries < FREE_TRIES {
                still.push((handle, key, tries + 1));
                continue;
            }
            log(format_args!(
                "{LOG_PREFIX} freeEventData ours={key} handle=0x{handle:x} rc={rc} after {} ticks; \
                 {}",
                tries + 1,
                memory()
            ));
        }
        self.to_free = still;
    }

    /// Rule 1 for a chosen silence: the playlist has run out with repeat off.
    fn go_silent(&mut self, why: &str) {
        self.stop_ours(true, why);
        self.follow_game_track(None);
        self.mute_game();
        self.silent = true;
        log(format_args!("{LOG_PREFIX} silence -- {why}"));
    }

    /// Back to the game's own track, audible, after something of ours failed.
    fn fall_back(&mut self, why: String) {
        self.stop_ours(true, &why);
        self.unmute_game();
        self.follow_game_track(None);
        self.silent = false;
        log(format_args!(
            "{LOG_PREFIX} fallback: the game's own track plays -- {why}"
        ));
        self.problem = Some(why);
    }

    fn track_memory(&mut self) {
        if self.playing.as_ref().is_some_and(|p| p.ours) {
            return;
        }
        if let Some((current, _)) = fmod::memory_stats() {
            self.game_peak = self.game_peak.max(current);
        }
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
    /// one chosen. `tries` counts entries skipped on the way, so a playlist of unplayable tracks
    /// ends in the fallback rather than a loop.
    fn play_entry(&mut self, entries: &[TrackKey], index: usize, restart: bool, tries: usize) {
        let Some(game) = self.game.clone() else {
            return;
        };
        if tries >= entries.len() {
            self.fall_back("no track in the playlist could be played".to_owned());
            return;
        }
        let Some(key) = entries.get(index).cloned() else {
            self.fall_back("no such playlist entry".to_owned());
            return;
        };
        let next = (index + 1) % entries.len().max(1);
        self.stop_ours(true, "another track was chosen");
        self.silent = false;
        self.resume.insert(game.key.clone(), index);
        if key == game.key {
            let channel = fmod::channel_of(game.handle);
            let how = match channel {
                Some(channel) if restart => {
                    format!("seek-to-0 rc={}", fmod::set_position(channel, 0))
                }
                Some(_) => "as-is".to_owned(),
                None => format!("start rc={}", fmod::start(game.handle)),
            };
            self.unmute_game();
            log(format_args!(
                "{LOG_PREFIX} play index={index} key={key} the-game's-own handle=0x{:x} {how}",
                game.handle
            ));
            self.follow_game_track(Some(index));
            self.problem = None;
            return;
        }
        if self.unheard.contains(&key) {
            log(format_args!(
                "{LOG_PREFIX} play index={index} key={key} skipped: it was not heard when it was \
                 last started"
            ));
            self.play_entry(entries, next, restart, tries + 1);
            return;
        }
        let Some(system_id) = self.lookup(&key) else {
            log(format_args!(
                "{LOG_PREFIX} play index={index} key={key} FAILED: not among the {} music events \
                 the event system knows",
                self.catalog.tracks.len()
            ));
            self.play_entry(entries, next, restart, tries + 1);
            return;
        };
        // Rule 3: refuse rather than run the game's FMOD heap out.
        if let Some((current, _)) = fmod::memory_stats()
            && self.game_peak > 0
            && current > self.game_peak + MEMORY_HEADROOM
        {
            self.fall_back(format!(
                "refused {key}: FMOD has {}KiB allocated, more than {}KiB over the game's own peak \
                 of {}KiB",
                current / 1024,
                MEMORY_HEADROOM / 1024,
                self.game_peak / 1024
            ));
            return;
        }
        let Some(system) = fmod::event_system() else {
            self.fall_back("no event system".to_owned());
            return;
        };
        match fmod::event_by_system_id(system, system_id, ds2_rva::FMOD_EVENT_MODE_DEFAULT) {
            Err(rc) => {
                log(format_args!(
                    "{LOG_PREFIX} play index={index} key={key} system={system_id} FAILED: \
                     getEventBySystemID rc={rc}"
                ));
                self.play_entry(entries, next, restart, tries + 1);
            }
            Ok(handle) => {
                let placed = fmod::listener_position(system)
                    .map_or(-1, |p| fmod::set_3d_position(handle, p));
                let rc = fmod::start(handle);
                log(format_args!(
                    "{LOG_PREFIX} play index={index} key={key} system={system_id} ours \
                     handle=0x{handle:x} start-rc={rc} at-listener-rc={placed} {}",
                    memory()
                ));
                if rc != FMOD_OK {
                    fmod::free_event_data(handle);
                    self.play_entry(entries, next, restart, tries + 1);
                    return;
                }
                self.problem = None;
                self.playing = Some(Playing {
                    handle,
                    key,
                    ours: true,
                    index: Some(index),
                    started_ms: now_ms(),
                    had_channel: false,
                    confirmed: false,
                    loop_cleared: 0,
                    last_position: None,
                });
            }
        }
    }

    /// Keep ours at the listener, as the game keeps its own region music.
    fn follow_listener(&mut self) {
        let Some(playing) = self.playing.as_ref().filter(|p| p.ours) else {
            return;
        };
        if let Some(system) = fmod::event_system()
            && let Some(position) = fmod::listener_position(system)
        {
            fmod::set_3d_position(playing.handle, position);
        }
    }

    /// Rule 1: mute the game's track once ours is heard; give up on ours if it is not.
    fn confirm(&mut self, now: u64) {
        let Some(playing) = self.playing.clone().filter(|p| p.ours) else {
            return;
        };
        let audibility = fmod::channel_of(playing.handle).and_then(fmod::audibility);
        if playing.confirmed {
            // Keep the game's track muted, including a new channel it may have made.
            self.mute_game();
            return;
        }
        if audibility.is_some_and(|a| a > AUDIBLE) {
            log(format_args!(
                "{LOG_PREFIX} heard key={} audibility={:.3} after {}ms",
                playing.key,
                audibility.unwrap_or(0.0),
                now.saturating_sub(playing.started_ms)
            ));
            if let Some(p) = self.playing.as_mut() {
                p.confirmed = true;
            }
            self.mute_game();
            return;
        }
        if now.saturating_sub(playing.started_ms) >= CONFIRM_MS {
            self.unheard.insert(playing.key.clone());
            self.fall_back(format!(
                "{} was not heard in {CONFIRM_MS}ms (audibility {audibility:?})",
                playing.key
            ));
        }
    }

    fn command(&mut self, command: Command) {
        let Some(game) = self.game.clone() else {
            log(format_args!(
                "{LOG_PREFIX} command {command:?} ignored: no region track is playing"
            ));
            return;
        };
        match command {
            Command::Seek(to) => {
                let Some(playing) = &self.playing else {
                    return;
                };
                let Some(channel) = fmod::channel_of(playing.handle) else {
                    log(format_args!(
                        "{LOG_PREFIX} seek to={to}ms ignored: {} has no channel",
                        playing.key
                    ));
                    return;
                };
                let before = fmod::position(channel);
                let rc = fmod::set_position(channel, to);
                let after = fmod::position(channel);
                log(format_args!(
                    "{LOG_PREFIX} seek key={} from={} to={to}ms read-back={} rc={rc} \
                     audibility={:?}",
                    playing.key,
                    ms(before),
                    ms(after),
                    fmod::audibility(channel)
                ));
                if let Some(playing) = &mut self.playing {
                    playing.last_position = after;
                }
            }
            Command::Step(forward) => {
                let entries = any_region(&game).entries();
                let current = self.playing.as_ref().and_then(|p| p.index).unwrap_or(0);
                if let Some(next) = step(entries.len(), current, forward) {
                    self.play_entry(&entries, next, true, 0);
                }
            }
            Command::Play(index) => {
                let entries = any_region(&game).entries();
                // A track the player asks for by name gets another chance to be heard.
                if let Some(key) = entries.get(index) {
                    self.unheard.remove(key);
                }
                self.play_entry(&entries, index, true, 0);
            }
            Command::RegionChanged => self.region_changed(&game),
        }
    }

    fn region_changed(&mut self, game: &Game) {
        let Some(region) = managed_region(&game.key) else {
            // Back to the game's own: its event plays, unmuted, and `watch` puts its loop back.
            self.stop_ours(true, "the region is as shipped again");
            if fmod::channel_of(game.handle).is_none() {
                fmod::start(game.handle);
            }
            self.unmute_game();
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
            None => self.play_entry(&entries, 0, true, 0),
        }
    }

    /// Keep the loop the way the region wants it, and notice the end of a track.
    fn watch(&mut self, now: u64) {
        let Some(game) = self.game.clone() else {
            return;
        };
        if self.silent {
            return;
        }
        let region = managed_region(&game.key);
        let repeat = region.as_ref().is_none_or(|r| r.repeat);
        let Some(playing) = self.playing.as_mut() else {
            return;
        };
        let ended = match fmod::channel_of(playing.handle) {
            Some(channel) => {
                playing.had_channel = true;
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
            None => {
                let _ = now;
                None
            }
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
            ms(last)
        ));
        let region = region.unwrap_or_else(|| Region::new(game.key.clone(), game.map));
        let entries = region.entries();
        match after_end(entries.len(), index, region.repeat) {
            AfterEnd::Loop => {}
            AfterEnd::Silence => self.go_silent("repeat is off and the playlist has one track"),
            AfterEnd::Play(next) => self.play_entry(&entries, next, true, 0),
        }
    }

    /// Build the catalog a slice at a time: every event the event system knows, by system id, kept
    /// when it is music.
    fn scan(&mut self, budget: u32) {
        // Rule 2: never while a region track is arriving, and never before one has been heard --
        // the scan touches every event the system knows, and a load is when the game is setting
        // them up.
        if self.game.is_none() || self.arriving.is_some() {
            return;
        }
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
        let channel = self
            .playing
            .as_ref()
            .and_then(|p| fmod::channel_of(p.handle));
        let (position, length) =
            channel.map_or((None, None), |c| (fmod::position(c), fmod::sound_length(c)));
        let snapshot = Snapshot {
            now: self.playing.as_ref().map(|p| p.key.clone()),
            ours: self.playing.as_ref().is_some_and(|p| p.ours),
            position_ms: position,
            length_ms: length,
            silent: self.silent,
            waiting: self.arriving.is_some(),
            problem: self.problem.clone(),
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
            let game_audibility = self
                .game
                .as_ref()
                .and_then(|g| fmod::channel_of(g.handle))
                .and_then(fmod::audibility);
            log(format_args!(
                "{LOG_PREFIX} sample key={} ours={} handle=0x{:x} channel-pos={} sound-len={} \
                 audibility={:?} game-audibility={game_audibility:?} region={} map={} {}",
                playing.key,
                playing.ours,
                playing.handle,
                ms(position),
                ms(length),
                channel.and_then(fmod::audibility),
                snapshot
                    .region
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), ToString::to_string),
                snapshot
                    .map
                    .map_or_else(|| "-".to_owned(), |m| m.to_string()),
                memory()
            ));
        }
        if let Ok(mut guard) = SNAPSHOT.lock() {
            *guard = Some(snapshot);
        }
    }
}
