//! The census in the game: the one query in flight, the clock, the arrivals file, and the view the
//! panel draws.
//!
//! The counting is `census`, tested on the host. This half owns what the game has: Steam (through
//! `steam`), the time, the file, and this character's level.
//!
//! # When it asks
//!
//! Only after the panel has been opened once this session, and from then on in the background, so
//! a player who opens it once and plays on gets an average that keeps growing. A player who never
//! opens it sends no query and has the matchmaking interface left exactly as Steam built it.
//! Counting asks only while Seamless has nothing staged and nothing out (`steam::seamless_busy`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Instant;

use ds2_game_base::mem::{game_module_base, safe_read_u32, safe_read_usize};
use ds2_seamless::lobby;

use crate::census::{self, Answer, Arrivals, Cell, Census, Query, View};
use crate::install::log;
use crate::steam::{self, Sent};
use crate::{ARRIVALS_FILE_NAME, LOG_PREFIX};

/// The census, created the first time the panel opens.
static CENSUS: Mutex<Option<Census>> = Mutex::new(None);
/// The query out now.
static IN_FLIGHT: Mutex<Option<(Query, Sent)>> = Mutex::new(None);
/// Set when the panel first opens; nothing is asked before it.
static ARMED: AtomicBool = AtomicBool::new(false);
/// The interface could not be wrapped; the census stays off and the panel says why.
static STEAM_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// Ticks the census held off because Seamless was staging or searching.
static HELD_FOR_SEAMLESS: AtomicU64 = AtomicU64::new(0);
/// Queries sent this session.
static SENT: AtomicU64 = AtomicU64::new(0);

/// Everything the panel shows, owned, so the panel never takes the census lock.
#[derive(Clone, Debug, Default)]
pub(crate) struct PanelView {
    /// The grid in each view, `None` while that view has nothing to show yet.
    pub(crate) hosts: Option<Vec<Cell>>,
    /// The rate view's cells.
    pub(crate) rate: Option<Vec<Cell>>,
    /// Hosts in every bracket in the last finished sweep.
    pub(crate) total: usize,
    /// Brackets answered in the sweep under way.
    pub(crate) progress: Option<usize>,
    /// Minutes the rate is averaged over.
    pub(crate) watched_minutes: f64,
    /// This character's own bracket, to mark its cell.
    pub(crate) own_bracket: Option<usize>,
    /// Where `own_bracket` came from: Seamless's own search or publish, or the level table.
    pub(crate) own_from_seamless: bool,
    /// The pool (regulation) was read from Seamless this session, not assumed.
    pub(crate) pool_known: bool,
    /// Seamless is staging or searching, so counting waits.
    pub(crate) waiting_for_seamless: bool,
    /// Why there is no count at all, when there is none.
    pub(crate) error: Option<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Milliseconds since this module first asked, the census's clock.
pub(crate) fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    u64::try_from(START.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn path() -> Option<PathBuf> {
    Some(ds2_game_base::log::game_directory_path()?.join(ARRIVALS_FILE_NAME))
}

/// Start counting: called when the panel opens. The first call wraps the interface and reads the
/// saved arrivals.
pub(crate) fn arm() {
    if ARMED.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Err(error) = steam::install() {
        log(format_args!(
            "{LOG_PREFIX} census OFF: the matchmaking interface could not be wrapped ({error}); \
             nothing is counted this session"
        ));
        *lock(&STEAM_ERROR) = Some(error);
        return;
    }
    let file = path();
    let arrivals = file
        .as_ref()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .map(|text| Arrivals::from_text(&text))
        .unwrap_or_default();
    log(format_args!(
        "{LOG_PREFIX} census armed by the panel: one lobby query per bracket while Seamless is \
         not searching, a sweep a minute; {:.1} minute(s) of arrivals carried over from {}",
        arrivals.minutes,
        file.as_ref().map_or_else(
            || "<no game directory>".to_owned(),
            |f| f.display().to_string()
        )
    ));
    *lock(&CENSUS) = Some(Census::new(arrivals));
}

/// The regulation to filter on: Seamless's own, once seen, else the value measured on 0.0.5.
fn regulation() -> (String, bool) {
    match steam::seen_regulation() {
        Some(value) => (value, true),
        None => (lobby::REGULATION_VALUE_MEASURED.to_owned(), false),
    }
}

/// Send the next query or collect the one out. Called once per frame.
pub(crate) fn tick() {
    if !ARMED.load(Ordering::Acquire) || lock(&STEAM_ERROR).is_some() {
        return;
    }
    let now = now_ms();
    let out = *lock(&IN_FLIGHT);
    if let Some((query, sent)) = out {
        let Some(answer) = steam::poll(&sent, now) else {
            return;
        };
        *lock(&IN_FLIGHT) = None;
        let finished = {
            let mut slot = lock(&CENSUS);
            let Some(census) = slot.as_mut() else {
                return;
            };
            let sweeps = census.sweeps;
            census.answer(now, &answer);
            if matches!(answer, Answer::Failed) {
                log(format_args!(
                    "{LOG_PREFIX} the query for bracket {} ({:?}) did not come back, or Seamless \
                     searched over it; the sweep starts over in {} s and counts no arrivals \
                     across the gap",
                    query.bracket,
                    query.split,
                    census::RETRY_AFTER_FAILURE_MS / 1000
                ));
            }
            (census.sweeps != sweeps).then(|| {
                (
                    census.arrivals.to_text(),
                    census.snapshot().map_or(0, census::Snapshot::total),
                    census.sweeps,
                )
            })
        };
        if let Some((text, total, sweeps)) = finished {
            save(&text, total, sweeps);
        }
        return;
    }
    let query = {
        let mut slot = lock(&CENSUS);
        let Some(census) = slot.as_mut() else {
            return;
        };
        census.due(now)
    };
    let Some(query) = query else {
        return;
    };
    let (pool, _) = regulation();
    match steam::send(&query, &pool, now) {
        Some(sent) => {
            SENT.fetch_add(1, Ordering::Relaxed);
            *lock(&IN_FLIGHT) = Some((query, sent));
        }
        // Seamless is staging or searching: nothing went out, and the query keeps its turn.
        None => {
            HELD_FOR_SEAMLESS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Write the arrivals after a finished sweep. Once a minute at most.
fn save(text: &str, total: usize, sweeps: u64) {
    let Some(file) = path() else {
        return;
    };
    let temp = file.with_extension("txt.tmp");
    let written = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, &file));
    if let Err(error) = written {
        log(format_args!(
            "{LOG_PREFIX} could not save {}: {error}",
            file.display()
        ));
        return;
    }
    if sweeps == 1 {
        log(format_args!(
            "{LOG_PREFIX} first sweep finished, {total} open host(s) across every bracket; saved \
             to {}",
            file.display()
        ));
    }
}

/// This character's soul level, `None` at the title and during a load.
fn soul_level() -> Option<u32> {
    let base = game_module_base().ok()?;
    // SAFETY: fault-safe reads at every hop: GameManagerImp -> PlayerCtrl -> PlayerParam -> level.
    let level = unsafe {
        safe_read_usize(base + ds2_rva::GAME_MANAGER_IMP as usize)
            .filter(|gm| *gm != 0)
            .and_then(|gm| safe_read_usize(gm + ds2_rva::PLAYER_CTRL_OFFSET))
            .filter(|ctrl| *ctrl != 0)
            .and_then(|ctrl| safe_read_usize(ctrl + ds2_rva::PLAYER_PARAM_OFFSET))
            .filter(|param| *param != 0)
            .and_then(|param| safe_read_u32(param + ds2_rva::PLAYER_PARAM_SOUL_LEVEL_OFFSET))
    }?;
    (level > 0).then_some(level)
}

/// The view the panel draws, built now.
pub(crate) fn view() -> PanelView {
    let error = lock(&STEAM_ERROR).clone();
    let (_, pool_known) = regulation();
    let (own_bracket, own_from_seamless) = match steam::seen_own_bracket() {
        Some(bracket) => (Some(bracket), true),
        None => (soul_level().map(census::bracket_of_level), false),
    };
    let slot = lock(&CENSUS);
    let Some(census) = slot.as_ref() else {
        return PanelView {
            own_bracket,
            own_from_seamless,
            pool_known,
            error,
            ..PanelView::default()
        };
    };
    PanelView {
        hosts: census::cells(census, View::Hosts),
        rate: census::cells(census, View::Rate),
        total: census.snapshot().map_or(0, census::Snapshot::total),
        progress: census.progress(),
        watched_minutes: census.arrivals.minutes,
        own_bracket,
        own_from_seamless,
        pool_known,
        waiting_for_seamless: steam::seamless_busy(),
        error,
    }
}

/// The census's counters for the heartbeat: sweeps, failed queries, queries sent, ticks held for
/// Seamless.
#[must_use]
pub(crate) fn tallies() -> (u64, u64, u64, u64) {
    let (sweeps, failures) = lock(&CENSUS)
        .as_ref()
        .map_or((0, 0), |census| (census.sweeps, census.failures));
    (
        sweeps,
        failures,
        SENT.load(Ordering::Relaxed),
        HELD_FOR_SEAMLESS.load(Ordering::Relaxed),
    )
}
