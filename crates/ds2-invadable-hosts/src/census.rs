//! How many hosts this player could invade, per Seamless bracket, and how often new ones appear.
//!
//! Host-testable: no Windows, no game, no Steam. The driver (`driver.rs`) sends the queries this
//! asks for and hands back what Steam answered.
//!
//! # What is counted
//!
//! DARK SOULS II Seamless Co-op publishes one invasion bracket on every open host's lobby,
//! `ds2sc_yui_nexus_session_search_005_bimm_<N>_6`, and its own invasion search asks for exactly
//! the searcher's own value with an equality filter (measured 2026-10-21, `ds2sc-lobby-watch.js`:
//! one `RequestLobbyList` per search, no range). So one lobby query per bracket value counts that
//! bracket directly. The live half adds the three other filters Seamless's search carries: its
//! advertisement marker, the open flag, and the regulation value (the pool: only hosts on the same
//! regulation can be invaded). Nothing here joins or writes a lobby.
//!
//! # Steam's cap
//!
//! A lobby list returns at most [`STEAM_RESULT_CAP`] lobbies whatever the count filter asks for
//! (measured on DS2 2026-10-21: an unfiltered marker query returned 50 of more than 50 masters). A
//! bracket that hits the cap is asked again as two disjoint halves on the flags key every host
//! publishes, compared numerically ([`Split`]); a half that also hits the cap is a floor
//! ([`Snapshot::saturated`]).
//!
//! # New hosts per minute
//!
//! [`Arrivals`] compares each finished sweep's lobby ids with the previous sweep's: an id not seen
//! before is a host who opened their world since. The first sweep after a start, a failure or a
//! stall only sets the baseline, so a restart never reads the whole population as new arrivals.
//! The totals persist in a small text file ([`Arrivals::to_text`]).

use std::collections::BTreeSet;

/// Brackets Seamless publishes: 0 to 8 (every value seen on 45 open hosts, 2026-10-21, and the
/// values levels 1 and 838 search for).
pub const BRACKET_COUNT: usize = 9;

/// The first soul level of each bracket, measured 2026-10-21 by `scripts/frida/bracket-scout.js`:
/// the bracket Seamless's own search asked for at each of 52 probed levels (stats set so the game's
/// own level was the probe), bisected until every edge was two adjacent levels. Not Elden Ring
/// Seamless's table: the cuts at 156 and 251 differ from its 151 and 301.
pub const BRACKET_FIRST_LEVEL: [u32; BRACKET_COUNT] = [1, 21, 41, 71, 101, 126, 156, 201, 251];

/// The value Seamless publishes for a bracket, before its number.
pub const BRACKET_VALUE_PREFIX: &str = "ds2sc_yui_nexus_session_search_005_bimm_";
/// The value Seamless publishes for a bracket, after its number. Every open host and every search
/// measured carries `_6`; what the number is has not been established, so it is matched, not
/// computed.
pub const BRACKET_VALUE_SUFFIX: &str = "_6";

/// The most lobbies one lobby list returns, whatever the count filter asks for.
pub const STEAM_RESULT_CAP: usize = 50;
/// What the count filter asks for. Above the cap, so the cap is Steam's and never ours.
pub const RESULT_COUNT_REQUEST: i32 = 1000;

/// Where a full bracket is cut in two, on the flags key compared as a number.
///
/// Hosts publishing at most this, and hosts publishing more. Every flags value seen is a 32-digit
/// binary string that
/// Steam compares as a decimal number (measured: `<= 10111100` returned 10 open hosts, `>` returned
/// 41, disjoint), and most hosts publish `10111111`, so this cut puts that value alone on one side.
pub const SPLIT_FLAGS_AT: i32 = 10_111_110;

/// The pause between the end of one sweep and the start of the next.
pub const SWEEP_INTERVAL_MS: u64 = 60_000;
/// The pause after a query that did not come back, before the sweep starts over.
pub const RETRY_AFTER_FAILURE_MS: u64 = 15_000;
/// A sweep longer than this sets a baseline and counts no arrivals.
///
/// It was held off long enough (a search holds the census off the interface) that its first and
/// last brackets describe different moments.
pub const MAX_COHERENT_SWEEP_MS: u64 = 5 * 60_000;

/// The bracket a soul level searches in, by [`BRACKET_FIRST_LEVEL`].
#[must_use]
pub fn bracket_of_level(level: u32) -> usize {
    BRACKET_FIRST_LEVEL
        .iter()
        .rposition(|first| level >= *first)
        .unwrap_or(0)
}

/// The value Seamless publishes for a bracket.
#[must_use]
pub fn bracket_value(bracket: usize) -> String {
    format!("{BRACKET_VALUE_PREFIX}{bracket}{BRACKET_VALUE_SUFFIX}")
}

/// The bracket number in a published value, if it is one of ours.
#[must_use]
pub fn parse_bracket_value(value: &str) -> Option<usize> {
    let number = value
        .strip_prefix(BRACKET_VALUE_PREFIX)?
        .strip_suffix(BRACKET_VALUE_SUFFIX)?;
    let bracket = number.parse::<usize>().ok()?;
    (bracket < BRACKET_COUNT).then_some(bracket)
}

/// The soul levels a bracket covers, as the panel prints them: `"1-20"`, `"301+"`.
#[must_use]
pub fn bracket_label(bracket: usize) -> String {
    let first = BRACKET_FIRST_LEVEL[bracket];
    match BRACKET_FIRST_LEVEL.get(bracket + 1) {
        Some(next) => format!("{first}-{}", next - 1),
        None => format!("{first}+"),
    }
}

/// Which slice of a bracket one query asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// The whole bracket.
    Whole,
    /// Hosts whose flags value is at most [`SPLIT_FLAGS_AT`].
    FlagsAtMost,
    /// Hosts whose flags value is above [`SPLIT_FLAGS_AT`].
    FlagsAbove,
}

/// One lobby query the census wants sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query {
    /// The bracket.
    pub bracket: usize,
    /// The slice of it.
    pub split: Split,
}

impl Query {
    /// The bracket value to filter on.
    #[must_use]
    pub fn bracket_value(&self) -> String {
        bracket_value(self.bracket)
    }
}

/// What a sent query came back with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The lobby ids Steam returned.
    Lobbies(Vec<u64>),
    /// The call failed or was replaced. Never an answer of zero: counting it as one would empty a
    /// bracket that is full.
    Failed,
}

/// One finished sweep: the hosts in every bracket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Open hosts per bracket.
    pub counts: [usize; BRACKET_COUNT],
    /// The bracket hit Steam's cap even split in two, so its count is a floor.
    pub saturated: [bool; BRACKET_COUNT],
}

impl Snapshot {
    /// Hosts in every bracket together.
    #[must_use]
    pub fn total(&self) -> usize {
        self.counts.iter().sum()
    }
}

/// A sweep in progress: one bracket at a time, each asked whole and split only when it must be.
#[derive(Clone, Debug)]
struct Sweep {
    started_ms: u64,
    bracket: usize,
    split: Split,
    ids: Vec<BTreeSet<u64>>,
    saturated: [bool; BRACKET_COUNT],
}

impl Sweep {
    fn new(started_ms: u64) -> Self {
        Self {
            started_ms,
            bracket: 0,
            split: Split::Whole,
            ids: vec![BTreeSet::new(); BRACKET_COUNT],
            saturated: [false; BRACKET_COUNT],
        }
    }

    fn query(&self) -> Query {
        Query {
            bracket: self.bracket,
            split: self.split,
        }
    }

    /// Take the answer to [`Sweep::query`]. True when the last bracket has been answered.
    fn take(&mut self, lobbies: &[u64]) -> bool {
        let at_cap = lobbies.len() >= STEAM_RESULT_CAP;
        match self.split {
            Split::Whole if at_cap => {
                self.split = Split::FlagsAtMost;
                return false;
            }
            Split::Whole => self.ids[self.bracket].extend(lobbies),
            Split::FlagsAtMost => {
                self.ids[self.bracket].extend(lobbies);
                self.saturated[self.bracket] |= at_cap;
                self.split = Split::FlagsAbove;
                return false;
            }
            Split::FlagsAbove => {
                self.ids[self.bracket].extend(lobbies);
                self.saturated[self.bracket] |= at_cap;
            }
        }
        self.split = Split::Whole;
        self.bracket += 1;
        self.bracket >= BRACKET_COUNT
    }
}

/// Hosts that appeared per bracket, over the minutes the census has watched.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrivals {
    /// New lobby ids seen per bracket, summed over every pair of consecutive sweeps.
    pub new: [u64; BRACKET_COUNT],
    /// Minutes between those pairs of sweeps, summed.
    pub minutes: f64,
    /// The previous sweep's ids and when it finished; `None` until a baseline is set.
    previous: Option<(Vec<BTreeSet<u64>>, u64)>,
}

impl Default for Arrivals {
    fn default() -> Self {
        Self {
            new: [0; BRACKET_COUNT],
            minutes: 0.0,
            previous: None,
        }
    }
}

/// The first line of the persisted file.
const ARRIVALS_HEADER: &str = "# ds2-invadable-hosts: new hosts seen per Seamless bracket, and \
                               the minutes watched. Delete this file to start the average over.";

impl Arrivals {
    /// Count a finished sweep against the previous one.
    pub fn observe(&mut self, ids: &[BTreeSet<u64>], now_ms: u64) {
        if let Some((previous, then_ms)) = &self.previous {
            self.minutes += now_ms.saturating_sub(*then_ms) as f64 / 60_000.0;
            for (bracket, (current, before)) in ids.iter().zip(previous).enumerate() {
                self.new[bracket] += current.difference(before).count() as u64;
            }
        }
        self.previous = Some((ids.to_vec(), now_ms));
    }

    /// Drop the baseline, so the next sweep sets one instead of being counted against a stale one.
    pub fn forget_baseline(&mut self) {
        self.previous = None;
    }

    /// New hosts per minute in a bracket, or `None` before any minute has been watched.
    #[must_use]
    pub fn rate(&self, bracket: usize) -> Option<f64> {
        (self.minutes > 0.0).then(|| self.new[bracket] as f64 / self.minutes)
    }

    /// The persisted form: the header, the minutes, and one count per bracket.
    #[must_use]
    pub fn to_text(&self) -> String {
        let new = self
            .new
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{ARRIVALS_HEADER}\nminutes = {:.3}\nnew = {new}\n",
            self.minutes
        )
    }

    /// Read [`Arrivals::to_text`] back. Anything not understood starts the average over rather
    /// than inventing part of one.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let mut minutes = None;
        let mut new = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "minutes" => {
                    minutes = value
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|m| m.is_finite() && *m >= 0.0);
                }
                "new" => {
                    let parsed: Option<Vec<u64>> = value
                        .split(',')
                        .map(|count| count.trim().parse::<u64>().ok())
                        .collect();
                    new = parsed.and_then(|counts| <[u64; BRACKET_COUNT]>::try_from(counts).ok());
                }
                _ => {}
            }
        }
        match (minutes, new) {
            (Some(minutes), Some(new)) => Self {
                new,
                minutes,
                previous: None,
            },
            _ => Self::default(),
        }
    }
}

/// Where the census is between queries.
#[derive(Clone, Debug)]
enum State {
    /// Waiting until this time before the next sweep starts.
    Waiting {
        until_ms: u64,
    },
    Sweeping(Sweep),
}

/// The census: a sweep at a time, the last finished one, and the arrivals.
#[derive(Clone, Debug)]
pub struct Census {
    state: State,
    last: Option<Snapshot>,
    /// New hosts per bracket over every minute watched.
    pub arrivals: Arrivals,
    /// Sweeps finished this session.
    pub sweeps: u64,
    /// Queries that failed this session.
    pub failures: u64,
}

impl Census {
    /// A census that starts its first sweep at once.
    #[must_use]
    pub fn new(arrivals: Arrivals) -> Self {
        Self {
            state: State::Waiting { until_ms: 0 },
            last: None,
            arrivals,
            sweeps: 0,
            failures: 0,
        }
    }

    /// The query to send now, starting a sweep when one is due. `None` while waiting.
    pub fn due(&mut self, now_ms: u64) -> Option<Query> {
        if let State::Waiting { until_ms } = self.state {
            if now_ms < until_ms {
                return None;
            }
            self.state = State::Sweeping(Sweep::new(now_ms));
        }
        match &self.state {
            State::Sweeping(sweep) => Some(sweep.query()),
            State::Waiting { .. } => None,
        }
    }

    /// Take the answer to the query [`Census::due`] last returned.
    pub fn answer(&mut self, now_ms: u64, answer: &Answer) {
        let State::Sweeping(sweep) = &mut self.state else {
            return;
        };
        let lobbies = match answer {
            Answer::Lobbies(lobbies) => lobbies,
            Answer::Failed => {
                self.failures += 1;
                self.arrivals.forget_baseline();
                self.state = State::Waiting {
                    until_ms: now_ms + RETRY_AFTER_FAILURE_MS,
                };
                return;
            }
        };
        if !sweep.take(lobbies) {
            return;
        }
        let finished = std::mem::replace(
            &mut self.state,
            State::Waiting {
                until_ms: now_ms + SWEEP_INTERVAL_MS,
            },
        );
        let State::Sweeping(sweep) = finished else {
            return;
        };
        if now_ms.saturating_sub(sweep.started_ms) > MAX_COHERENT_SWEEP_MS {
            self.arrivals.forget_baseline();
        }
        self.arrivals.observe(&sweep.ids, now_ms);
        let mut counts = [0; BRACKET_COUNT];
        for (count, ids) in counts.iter_mut().zip(&sweep.ids) {
            *count = ids.len();
        }
        self.last = Some(Snapshot {
            counts,
            saturated: sweep.saturated,
        });
        self.sweeps += 1;
    }

    /// The last finished sweep.
    #[must_use]
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.last.as_ref()
    }

    /// Brackets answered so far in the sweep under way, or `None` between sweeps.
    #[must_use]
    pub fn progress(&self) -> Option<usize> {
        match &self.state {
            State::Sweeping(sweep) => Some(sweep.bracket),
            State::Waiting { .. } => None,
        }
    }
}

/// What the grid shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// Hosts there now. The default.
    #[default]
    Hosts,
    /// New hosts per minute, averaged over every minute watched.
    Rate,
}

impl View {
    /// The other view.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Hosts => Self::Rate,
            Self::Rate => Self::Hosts,
        }
    }
}

/// One cell of the grid, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// The number as printed, with `≥` on a floor.
    pub text: String,
    /// 0 to 1: how full the cell is against the fullest one, for its shade.
    pub weight: f32,
}

/// A rate as the grid prints it: two decimals, a floor for a rate too small to show.
#[must_use]
pub fn rate_text(rate: f64) -> String {
    if rate > 0.0 && rate < 0.005 {
        "<0.01".to_owned()
    } else {
        format!("{rate:.2}")
    }
}

/// Every cell of the grid in `view`, or `None` when there is nothing to show yet: no sweep has
/// finished for [`View::Hosts`], no minute has been watched for [`View::Rate`].
#[must_use]
pub fn cells(census: &Census, view: View) -> Option<Vec<Cell>> {
    let values: Vec<(f64, String)> = match view {
        View::Hosts => {
            let snapshot = census.snapshot()?;
            (0..BRACKET_COUNT)
                .map(|bracket| {
                    let count = snapshot.counts[bracket];
                    let floor = if snapshot.saturated[bracket] {
                        "\u{2265}"
                    } else {
                        ""
                    };
                    (count as f64, format!("{floor}{count}"))
                })
                .collect()
        }
        View::Rate => (0..BRACKET_COUNT)
            .map(|bracket| {
                let rate = census.arrivals.rate(bracket)?;
                Some((rate, rate_text(rate)))
            })
            .collect::<Option<_>>()?,
    };
    let peak = values.iter().map(|(value, _)| *value).fold(0.0, f64::max);
    Some(
        values
            .into_iter()
            .map(|(value, text)| Cell {
                text,
                weight: if peak > 0.0 {
                    (value / peak) as f32
                } else {
                    0.0
                },
            })
            .collect(),
    )
}

/// How long the rate has been watched, as the panel says it: `"42 min"`, `"3 h 05 min"`.
#[must_use]
pub fn watched_text(minutes: f64) -> String {
    let whole = minutes.max(0.0).round() as u64;
    if whole < 60 {
        format!("{whole} min")
    } else {
        format!("{} h {:02} min", whole / 60, whole % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(range: std::ops::Range<u64>) -> Vec<u64> {
        range.collect()
    }

    /// Answer every query of one sweep with `lobbies(query)`, returning the queries sent.
    fn run_sweep(
        census: &mut Census,
        now_ms: u64,
        lobbies: impl Fn(&Query) -> Vec<u64>,
    ) -> Vec<Query> {
        let mut sent = Vec::new();
        while let Some(query) = census.due(now_ms) {
            sent.push(query);
            census.answer(now_ms, &Answer::Lobbies(lobbies(&query)));
            if census.progress().is_none() {
                break;
            }
        }
        sent
    }

    #[test]
    fn the_brackets_are_seamless_values_and_levels() {
        assert_eq!(
            bracket_value(0),
            "ds2sc_yui_nexus_session_search_005_bimm_0_6"
        );
        assert_eq!(
            parse_bracket_value("ds2sc_yui_nexus_session_search_005_bimm_5_6"),
            Some(5)
        );
        assert_eq!(
            parse_bracket_value("ds2sc_yui_nexus_session_search_005_bimm_9_6"),
            None
        );
        assert_eq!(
            parse_bracket_value("ds2sc_yui_nexus_session_search_005_qmmm_1_6"),
            None
        );
        // The probes the scout and the owner's runs measured.
        for (level, bracket) in [
            (1, 0),
            (20, 0),
            (21, 1),
            (40, 1),
            (41, 2),
            (70, 2),
            (71, 3),
            (100, 3),
            (101, 4),
            (125, 4),
            (126, 5),
            (155, 5),
            (156, 6),
            (200, 6),
            (201, 7),
            (250, 7),
            (251, 8),
            (838, 8),
        ] {
            assert_eq!(bracket_of_level(level), bracket, "level {level}");
        }
        assert_eq!(bracket_label(0), "1-20");
        assert_eq!(bracket_label(5), "126-155");
        assert_eq!(bracket_label(BRACKET_COUNT - 1), "251+");
        for pair in BRACKET_FIRST_LEVEL.windows(2) {
            assert!(pair[0] < pair[1]);
        }
    }

    #[test]
    fn a_sweep_asks_every_bracket_once_and_records_the_counts() {
        let mut census = Census::new(Arrivals::default());
        let sent = run_sweep(&mut census, 0, |query| {
            if query.bracket == 2 {
                ids(0..7)
            } else {
                Vec::new()
            }
        });
        assert_eq!(sent.len(), BRACKET_COUNT);
        assert!(sent.iter().all(|query| query.split == Split::Whole));
        let snapshot = census.snapshot().expect("a finished sweep");
        assert_eq!(snapshot.counts[2], 7);
        assert_eq!(snapshot.total(), 7);
        assert!(!snapshot.saturated.iter().any(|floor| *floor));
        assert_eq!(census.sweeps, 1);
    }

    #[test]
    fn a_bracket_at_the_cap_is_asked_again_in_two_flag_halves() {
        let mut census = Census::new(Arrivals::default());
        let busy = 3;
        let sent = run_sweep(&mut census, 0, |query| {
            match (query.bracket == busy, query.split) {
                (true, Split::Whole) => ids(0..50),
                (true, Split::FlagsAtMost) => ids(0..30),
                (true, Split::FlagsAbove) => ids(100..129),
                _ => Vec::new(),
            }
        });
        assert_eq!(sent.len(), BRACKET_COUNT + 2);
        let snapshot = census.snapshot().expect("a finished sweep");
        assert_eq!(snapshot.counts[busy], 59);
        assert!(!snapshot.saturated[busy], "neither half hit the cap");

        let mut census = Census::new(Arrivals::default());
        run_sweep(&mut census, 0, |query| {
            match (query.bracket == busy, query.split) {
                (true, Split::FlagsAtMost) => ids(0..10),
                (true, _) => ids(200..250),
                _ => Vec::new(),
            }
        });
        let snapshot = census.snapshot().expect("a finished sweep");
        assert!(snapshot.saturated[busy], "a half at the cap is a floor");
        assert_eq!(snapshot.counts[busy], 60);
    }

    #[test]
    fn the_first_sweep_is_a_baseline_and_the_next_counts_new_ids() {
        let mut census = Census::new(Arrivals::default());
        let bracket = 4;
        run_sweep(&mut census, 0, |query| {
            if query.bracket == bracket {
                vec![1, 2, 3]
            } else {
                Vec::new()
            }
        });
        assert_eq!(census.arrivals.minutes, 0.0);
        assert_eq!(census.arrivals.rate(bracket), None);
        assert!(cells(&census, View::Rate).is_none());
        assert_eq!(census.due(1), None, "waits out the interval");
        let later = SWEEP_INTERVAL_MS * 2;
        run_sweep(&mut census, later, |query| {
            if query.bracket == bracket {
                vec![2, 3, 4, 5]
            } else {
                Vec::new()
            }
        });
        assert_eq!(census.arrivals.new[bracket], 2);
        assert_eq!(census.arrivals.minutes, 2.0);
        assert_eq!(census.arrivals.rate(bracket), Some(1.0));
        let rate = cells(&census, View::Rate).expect("a rate");
        assert_eq!(rate[bracket].text, "1.00");
        assert_eq!(rate[bracket].weight, 1.0);
        assert_eq!(rate[0].text, "0.00");
    }

    #[test]
    fn a_failure_drops_the_sweep_and_the_baseline() {
        let mut census = Census::new(Arrivals::default());
        run_sweep(&mut census, 0, |_| vec![9]);
        let at = SWEEP_INTERVAL_MS;
        assert!(census.due(at).is_some());
        census.answer(at, &Answer::Failed);
        assert_eq!(census.failures, 1);
        assert_eq!(census.progress(), None);
        assert_eq!(census.due(at + 1), None);
        let retry = at + RETRY_AFTER_FAILURE_MS;
        run_sweep(&mut census, retry, |_| vec![10]);
        // The sweep after the failure is a new baseline, so nothing counts as arrived.
        assert_eq!(census.arrivals.new, [0; BRACKET_COUNT]);
        assert_eq!(census.arrivals.minutes, 0.0);
        assert_eq!(census.snapshot().map(Snapshot::total), Some(BRACKET_COUNT));
    }

    #[test]
    fn a_sweep_that_stalled_sets_a_baseline_instead_of_counting() {
        let mut census = Census::new(Arrivals::default());
        run_sweep(&mut census, 0, |_| vec![1]);
        let start = SWEEP_INTERVAL_MS;
        let query = census.due(start).expect("due");
        census.answer(start, &Answer::Lobbies(vec![2]));
        assert_eq!(query.bracket, 0);
        let late = start + MAX_COHERENT_SWEEP_MS + 1;
        run_sweep(&mut census, late, |_| vec![2]);
        assert_eq!(census.arrivals.minutes, 0.0);
        assert_eq!(census.arrivals.new, [0; BRACKET_COUNT]);
    }

    #[test]
    fn the_arrivals_survive_a_round_trip_and_garbage_starts_over() {
        let mut arrivals = Arrivals::default();
        arrivals.new[3] = 12;
        arrivals.new[8] = 1;
        arrivals.minutes = 187.25;
        let text = arrivals.to_text();
        assert!(text.starts_with("# ds2-invadable-hosts"));
        let back = Arrivals::from_text(&text);
        assert_eq!(back.new, arrivals.new);
        assert!((back.minutes - 187.25).abs() < 1e-9);
        assert_eq!(
            Arrivals::from_text("minutes = 3\nnew = 1,2\n"),
            Arrivals::default()
        );
        assert_eq!(Arrivals::from_text("minutes = -1\n"), Arrivals::default());
        assert_eq!(Arrivals::from_text(""), Arrivals::default());
    }

    #[test]
    fn the_host_cells_mark_a_floor_and_shade_against_the_fullest() {
        let mut census = Census::new(Arrivals::default());
        assert!(cells(&census, View::Hosts).is_none());
        run_sweep(&mut census, 0, |query| match query.bracket {
            0 => ids(0..50),
            1 => ids(0..25),
            _ => Vec::new(),
        });
        let hosts = cells(&census, View::Hosts).expect("a sweep finished");
        assert_eq!(hosts[0].text, "\u{2265}50");
        assert_eq!(hosts[0].weight, 1.0);
        assert_eq!(hosts[1].text, "25");
        assert!((hosts[1].weight - 0.5).abs() < 1e-6);
        assert_eq!(hosts[2].weight, 0.0);
    }

    #[test]
    fn the_view_toggles_and_the_words_read_well() {
        assert_eq!(View::default(), View::Hosts);
        assert_eq!(View::Hosts.toggled(), View::Rate);
        assert_eq!(View::Rate.toggled(), View::Hosts);
        assert_eq!(rate_text(0.0), "0.00");
        assert_eq!(rate_text(0.001), "<0.01");
        assert_eq!(rate_text(0.426), "0.43");
        assert_eq!(watched_text(42.4), "42 min");
        assert_eq!(watched_text(185.0), "3 h 05 min");
    }
}
