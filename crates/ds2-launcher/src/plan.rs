//! Turning `[launcher]` into an ordered list of DLLs to inject, before any process exists.
//!
//! Everything here is host-testable on purpose. The Windows half of this crate cannot be run on
//! this workspace's own machine, so the half that makes the decisions is kept free of it: the
//! config text and a game directory go in, an ordered and validated plan comes out, and
//! `cargo test` on Linux reaches all of it.
//!
//! # The section
//!
//! ```text
//! [launcher]
//! exe = "DarkSoulsII.exe"
//! dlls = ["SeamlessCoop/ds2sc.dll", "SomeOtherMod/other.dll"]
//! ```
//!
//! The list is injected in the order written, each one fully loaded before the next is asked
//! for. Order is part of the contract, because two mods that hook the same function resolve in
//! load order, and a list whose order did not survive would make that unfixable from the config.
//!
//! # Why `[seamless]` is folded in here rather than listed twice
//!
//! `[seamless] enabled` already exists and already means "load that mod". Making a player write
//! the same path a second time under `dlls` to keep it working would be a config that lies about
//! what it controls. So when `[seamless] enabled = true` its `dll` is prepended to the list --
//! first, which is where its own launcher puts it -- and deduplicated if it also appears in
//! `dlls`.
//!
//! Loading one DLL twice is not harmless. The second `LoadLibraryW` returns the same module with
//! a bumped reference count and does not re-run `DllMain`, so a mod that counts its own
//! initialisations would be told something untrue.

use std::path::{Path, PathBuf};

use ds2_hotkey_config::kv::KeyValues;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "launcher";

/// Which DLLs to inject, as a list of paths relative to the game directory.
pub const KEY_DLLS: &str = "dlls";

/// Which executable to start. Present so a future build can be pointed somewhere else.
pub const KEY_EXE: &str = "exe";

/// What `exe` means when it is absent, which is the only thing SOTFS ships.
pub const DEFAULT_EXE: &str = "DarkSoulsII.exe";

/// Log prefix, in the shape every other feature in this workspace uses.
pub const LOG_PREFIX: &str = "ds2-launcher:";

/// One DLL to inject, with the reason it is in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Injection {
    /// The path exactly as the config spelled it, for messages a human has to match up.
    pub spelled: String,
    /// That path resolved against the game directory.
    pub resolved: PathBuf,
    /// Which key put it here, so a refusal can name the line to edit.
    pub source: Source,
}

/// Where an entry in the plan came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Named in `[launcher] dlls`.
    List,
    /// Implied by `[seamless] enabled = true`.
    Seamless,
}

impl Source {
    /// The config line a person would edit to remove this entry.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::List => "[launcher] dlls",
            Self::Seamless => "[seamless] enabled",
        }
    }
}

/// What to start and what to put inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The executable to create suspended.
    pub exe: PathBuf,
    /// The DLLs to inject, in load order.
    pub injections: Vec<Injection>,
}

/// A reason a plan cannot be carried out, decided before the game is started.
///
/// Every one of these is checked up front rather than as the injection runs. A failure halfway
/// through would leave a running game with some of its mods in it and no way to tell from the
/// inside which ones; refusing before `CreateProcessW` means the only two outcomes are a session
/// with the whole list, or no session at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The executable named by `exe` is not there.
    MissingExe(PathBuf),
    /// A DLL in the list is not there.
    MissingDll {
        /// The path as written in the config.
        spelled: String,
        /// Where that resolved to.
        resolved: PathBuf,
        /// Which key named it.
        source: Source,
    },
}

impl Refusal {
    /// A single line naming the file and the config key that asked for it.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::MissingExe(path) => {
                format!("no executable at {} -- `[launcher] exe`", path.display())
            }
            Self::MissingDll {
                spelled,
                resolved,
                source,
            } => format!(
                "no DLL at {} (`{spelled}`, from `{}`)",
                resolved.display(),
                source.describe()
            ),
        }
    }
}

/// Split a `["a", "b"]` list into its items.
///
/// The same shape `ds2-loader`'s `menu_row` reads, and deliberately the same forgiving parse:
/// this workspace's config files are a TOML-shaped subset read by a dependency-free key-value
/// reader, not by a TOML parser, so a list is text between brackets and nothing here validates
/// quoting.
fn split_list(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Resolve one config path against the game directory.
///
/// An absolute path is taken as written; anything else is relative to the game directory, which
/// is what every other path key in this workspace means and what Seamless's own launcher does.
fn resolve(game_dir: &Path, spelled: &str) -> PathBuf {
    // Backslashes are what a Windows-facing config is likely to carry, and `\` is an ordinary
    // filename character to a Linux `Path` -- so `SeamlessCoop\ds2sc.dll` would resolve to one
    // file with a slash in its name rather than to a file in a subdirectory. Normalising here
    // keeps both spellings meaning the same thing on both sides of the prefix.
    let normalised = spelled.replace('\\', "/");
    let candidate = Path::new(&normalised);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        game_dir.join(candidate)
    }
}

/// Read one quoted scalar out of a section, treating an empty value as absent.
fn scalar<'a>(parsed: &'a KeyValues, section: &str, key: &str) -> Option<&'a str> {
    parsed
        .get(section, key)
        .map(|raw| raw.trim().trim_matches('"'))
        .filter(|raw| !raw.is_empty())
}

impl Plan {
    /// Read a plan out of config text, without touching the filesystem.
    ///
    /// Resolution against `game_dir` happens here; existence is not checked, so this stays usable
    /// in a test with a directory that does not exist. [`Plan::refusals`] is the check.
    #[must_use]
    pub fn from_text(text: &str, game_dir: &Path) -> Self {
        let parsed = KeyValues::parse(text);

        let exe = scalar(&parsed, CONFIG_SECTION, KEY_EXE).unwrap_or(DEFAULT_EXE);

        let mut injections: Vec<Injection> = Vec::new();

        // Seamless leads, because that is where its own launcher puts it and because a mod that
        // renames the save container should be in before anything that might read the name.
        let seamless_on = scalar(
            &parsed,
            ds2_seamless::CONFIG_SECTION,
            ds2_seamless::KEY_ENABLED,
        )
        .is_some_and(|raw| raw.eq_ignore_ascii_case("true"));
        if seamless_on {
            let spelled = scalar(&parsed, ds2_seamless::CONFIG_SECTION, ds2_seamless::KEY_DLL)
                .unwrap_or(ds2_seamless::DEFAULT_DLL)
                .to_owned();
            injections.push(Injection {
                resolved: resolve(game_dir, &spelled),
                spelled,
                source: Source::Seamless,
            });
        }

        if let Some(raw) = parsed.get(CONFIG_SECTION, KEY_DLLS) {
            for spelled in split_list(raw) {
                let resolved = resolve(game_dir, &spelled);
                // The comparison is on the resolved path, not the spelling: `SeamlessCoop/x.dll`
                // and `SeamlessCoop\x.dll` are one file and must not be loaded twice.
                if injections.iter().any(|seen| seen.resolved == resolved) {
                    continue;
                }
                injections.push(Injection {
                    spelled,
                    resolved,
                    source: Source::List,
                });
            }
        }

        Self {
            exe: resolve(game_dir, exe),
            injections,
        }
    }

    /// Everything about this plan that cannot be carried out, checked against the filesystem.
    ///
    /// Empty means the plan is runnable. A non-empty list is meant to be printed in full and the
    /// launch abandoned -- see [`Refusal`] for why it is all-or-nothing.
    #[must_use]
    pub fn refusals(&self) -> Vec<Refusal> {
        let mut refusals = Vec::new();
        if !self.exe.is_file() {
            refusals.push(Refusal::MissingExe(self.exe.clone()));
        }
        for injection in &self.injections {
            if !injection.resolved.is_file() {
                refusals.push(Refusal::MissingDll {
                    spelled: injection.spelled.clone(),
                    resolved: injection.resolved.clone(),
                    source: injection.source,
                });
            }
        }
        refusals
    }
}

/// Flags this launcher reads for itself. Everything else on its command line is the game's.
pub const OWN_FLAGS: [&str; 2] = ["--dry-run", "--selftest"];

/// The arguments this launcher was given that belong to the game.
///
/// Steam's `%command%` is the game's own command line, so a launch option that puts this
/// launcher in front of it hands over the path to `DarkSoulsII.exe` first and any arguments the
/// player added after. That leading path is dropped -- the plan already names the executable,
/// and [`command_line`] puts it back quoted -- and the rest is passed through in order, so an
/// argument the player gave the game is not silently eaten by the thing in front of it.
#[must_use]
pub fn game_arguments(arguments: &[String], exe: &Path) -> Vec<String> {
    let mut rest: Vec<&String> = arguments
        .iter()
        .filter(|argument| !OWN_FLAGS.contains(&argument.as_str()))
        .collect();
    let leading_is_the_game = rest.first().is_some_and(|first| {
        // Steam on Windows spells it with backslashes, which a Linux `Path` would not split on.
        let normalised = first.replace('\\', "/");
        let named = Path::new(&normalised).file_name();
        match (named, exe.file_name()) {
            (Some(named), Some(exe)) => named
                .to_string_lossy()
                .eq_ignore_ascii_case(&exe.to_string_lossy()),
            _ => false,
        }
    });
    if leading_is_the_game {
        rest.remove(0);
    }
    rest.into_iter().cloned().collect()
}

/// Quote one argument so `CommandLineToArgvW` in the game reads back exactly this string.
///
/// The executable path is quoted too. It has spaces in it on a default install
/// (`Dark Souls II Scholar of the First Sin`), and an unquoted one is found at all only because
/// `CreateProcessW` retries each space-separated prefix until one is a file -- after which the
/// game's own `argv[0]` is still split in pieces.
#[must_use]
pub fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '\n', '\u{b}', '"']) {
        return argument.to_owned();
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                // A quote after a run of backslashes: every one of them doubles, and the quote
                // gets one more of its own.
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    // Backslashes before the closing quote double, or the last one would escape it.
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// The full command line the game is started with: its path, then its arguments.
#[must_use]
pub fn command_line(exe: &Path, arguments: &[String]) -> String {
    std::iter::once(exe.to_string_lossy().into_owned())
        .chain(arguments.iter().cloned())
        .map(|argument| quote(&argument))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = "/games/ds2";

    fn plan(text: &str) -> Plan {
        Plan::from_text(text, Path::new(GAME))
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn steams_command_drops_the_game_path_and_keeps_what_follows() {
        let exe = Path::new(GAME).join(DEFAULT_EXE);
        let given = strings(&[
            r"C:\Games\Dark Souls II Scholar of the First Sin\Game\DarkSoulsII.exe",
            "-windowed",
        ]);
        assert_eq!(game_arguments(&given, &exe), strings(&["-windowed"]));
    }

    #[test]
    fn the_game_path_is_matched_by_name_whatever_the_case() {
        let exe = Path::new(GAME).join(DEFAULT_EXE);
        let given = strings(&["z:/elsewhere/darksoulsii.EXE"]);
        assert!(game_arguments(&given, &exe).is_empty());
    }

    #[test]
    fn no_arguments_is_no_arguments() {
        let exe = Path::new(GAME).join(DEFAULT_EXE);
        assert!(game_arguments(&[], &exe).is_empty());
    }

    #[test]
    fn the_launchers_own_flags_never_reach_the_game() {
        let exe = Path::new(GAME).join(DEFAULT_EXE);
        let given = strings(&["--dry-run", "DarkSoulsII.exe", "a"]);
        assert_eq!(game_arguments(&given, &exe), strings(&["a"]));
    }

    #[test]
    fn an_argument_that_is_not_the_game_is_kept_even_first() {
        let exe = Path::new(GAME).join(DEFAULT_EXE);
        let given = strings(&["-windowed"]);
        assert_eq!(game_arguments(&given, &exe), given);
    }

    #[test]
    fn quoting_follows_the_rules_the_game_parses_with() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote(""), r#""""#);
        assert_eq!(quote("has space"), r#""has space""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(r"C:\dir with space\"), r#""C:\dir with space\\""#);
        assert_eq!(quote(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quote(r"no\space"), r"no\space");
    }

    #[test]
    fn the_command_line_quotes_the_install_path() {
        let exe = Path::new("/games/Dark Souls II/Game/DarkSoulsII.exe");
        assert_eq!(
            command_line(exe, &strings(&["-x"])),
            r#""/games/Dark Souls II/Game/DarkSoulsII.exe" -x"#
        );
    }

    /// The config file that ships in the download, read here by the reader that will read it
    /// there.
    ///
    /// A comment could not hold this rule, which is why it is a test. The file is TOML-shaped and
    /// is read by a line-based key-value parser, so an array written the way every TOML formatter
    /// writes one --
    ///
    /// ```text
    /// rows = [
    ///   "quit-to-desktop",
    /// ]
    /// ```
    ///
    /// -- parses as `rows = "["`, discards each continuation line as a non-assignment, and reads
    /// as "no rows at all": the whole pause menu gone, from a file that looks right in an editor
    /// and produces no error anywhere. It was written that way once, in the change that added it.
    const SHIPPED_CONFIG: &str = include_str!("../../../.github/dist-ds2-mods.toml");

    #[test]
    fn the_config_in_the_download_has_no_line_the_reader_throws_away() {
        let parsed = ds2_hotkey_config::KeyValues::parse(SHIPPED_CONFIG);
        let rejected: Vec<String> = parsed
            .rejected()
            .iter()
            .map(|entry| format!("line {}: {}", entry.line, entry.text))
            .collect();
        assert!(
            rejected.is_empty(),
            ".github/dist-ds2-mods.toml has lines the reader discards:\n{}",
            rejected.join("\n")
        );
    }

    #[test]
    fn the_config_in_the_download_is_the_defaults_it_claims_to_be() {
        // Seamless staying off is the one that matters. The file ships beside a launcher that
        // would otherwise refuse every launch on a machine where Seamless is not installed, and
        // injecting somebody else's binary is not something a shipped default may do unasked.
        let plan = plan(SHIPPED_CONFIG);
        assert!(
            plan.injections.is_empty(),
            "the shipped config must inject nothing until a player asks for it"
        );
        assert_eq!(plan.exe, Path::new(GAME).join(DEFAULT_EXE));
    }

    #[test]
    fn an_empty_config_starts_the_game_and_injects_nothing() {
        let plan = plan("");
        assert_eq!(plan.exe, Path::new(GAME).join(DEFAULT_EXE));
        assert!(plan.injections.is_empty(), "nothing was asked for");
    }

    #[test]
    fn the_list_keeps_the_order_it_was_written_in() {
        let plan = plan("[launcher]\ndlls = [\"a/one.dll\", \"b/two.dll\", \"c/three.dll\"]\n");
        let spelled: Vec<&str> = plan
            .injections
            .iter()
            .map(|item| item.spelled.as_str())
            .collect();
        assert_eq!(
            spelled,
            ["a/one.dll", "b/two.dll", "c/three.dll"],
            "load order is part of the contract -- two mods hooking one function resolve in it"
        );
    }

    #[test]
    fn seamless_leads_when_it_is_enabled() {
        let plan = plan("[launcher]\ndlls = [\"other.dll\"]\n[seamless]\nenabled = true\n");
        assert_eq!(plan.injections[0].source, Source::Seamless);
        assert_eq!(plan.injections[0].spelled, ds2_seamless::DEFAULT_DLL);
        assert_eq!(plan.injections[1].spelled, "other.dll");
    }

    #[test]
    fn seamless_disabled_is_not_injected_even_with_a_dll_key() {
        let plan = plan("[seamless]\nenabled = false\ndll = \"SeamlessCoop/ds2sc.dll\"\n");
        assert!(plan.injections.is_empty(), "`enabled` is the switch");
    }

    #[test]
    fn naming_seamless_twice_loads_it_once() {
        // A second `LoadLibraryW` would return the same module with a bumped reference count and
        // would not re-run `DllMain`, so a mod counting its own initialisations would be told
        // something untrue.
        let plan =
            plan("[launcher]\ndlls = [\"SeamlessCoop/ds2sc.dll\"]\n[seamless]\nenabled = true\n");
        assert_eq!(plan.injections.len(), 1, "one file, one load");
        assert_eq!(plan.injections[0].source, Source::Seamless);
    }

    #[test]
    fn a_backslash_spelling_is_the_same_file_as_a_slash_one() {
        let plan = plan(
            "[launcher]\ndlls = [\"SeamlessCoop\\\\ds2sc.dll\"]\n[seamless]\nenabled = true\n",
        );
        assert_eq!(
            plan.injections.len(),
            1,
            "a Windows-spelled path must not slip past the deduplication"
        );
    }

    #[test]
    fn an_absolute_path_is_taken_as_written() {
        let plan = plan("[launcher]\ndlls = [\"/elsewhere/mod.dll\"]\n");
        assert_eq!(plan.injections[0].resolved, Path::new("/elsewhere/mod.dll"));
    }

    #[test]
    fn a_relative_path_hangs_off_the_game_directory() {
        let plan = plan("[launcher]\ndlls = [\"Mods/mod.dll\"]\n");
        assert_eq!(
            plan.injections[0].resolved,
            Path::new(GAME).join("Mods/mod.dll")
        );
    }

    #[test]
    fn an_empty_list_is_not_an_entry() {
        let plan = plan("[launcher]\ndlls = []\n");
        assert!(plan.injections.is_empty());
    }

    #[test]
    fn a_missing_file_is_refused_by_name_and_by_the_key_that_asked_for_it() {
        let plan = plan("[launcher]\ndlls = [\"nope.dll\"]\n");
        let refusals = plan.refusals();
        // The game directory does not exist either, so the exe is refused as well.
        assert!(matches!(refusals[0], Refusal::MissingExe(_)));
        let dll = refusals
            .iter()
            .find(|refusal| matches!(refusal, Refusal::MissingDll { .. }))
            .expect("the missing DLL is refused");
        let described = dll.describe();
        assert!(described.contains("nope.dll"), "{described}");
        assert!(described.contains("[launcher] dlls"), "{described}");
    }

    #[test]
    fn the_exe_key_can_move_the_executable() {
        let plan = plan("[launcher]\nexe = \"Other.exe\"\n");
        assert_eq!(plan.exe, Path::new(GAME).join("Other.exe"));
    }
}
