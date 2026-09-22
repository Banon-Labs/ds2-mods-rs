//! A direction to every other player in your session, drawn over the world.
//!
//! Press the key (default `;`) during an invasion or a co-op session and every other player in it
//! gets a coloured arrow leaving your body and pointing at them -- in three dimensions, so a
//! player forty metres straight down reads as *down* rather than as five metres away.
//!
//! | situation | what you see |
//! |---|---|
//! | another player in the session | an arrow out of your body, pointing exactly at them |
//! | N players | N arrows, N colours, each colour stuck to one player |
//! | they are closer than 30 m | nothing -- you already know where they are |
//!
//! The closer a player is, the bolder and more opaque their arrow. A player who is merely far
//! away still draws at the faintest weight rather than vanishing, so "distant" and "not there"
//! never look the same.
//!
//! # What this is a port of, and what is missing from the port
//!
//! `../er-mods-rs/crates/er-invasion-path` draws a **walkable route** along the navmesh, and
//! falls back to an arrow when the navmesh says there is no way to walk there. So does this, now.
//!
//! `docs/PORTING.md` filed the Elden Ring crate under "no DS2 analogue (Havok-AI navmesh)". Half
//! of that is right -- there is no Havok AI in this engine -- and the conclusion is wrong. DARK
//! SOULS II ships its own navigation stack, RTTI-named throughout, with the same request/poll
//! shape: `crate::navquery` snaps both ends of the line to navigation-graph ids and asks
//! `NvRoutePlanner` for a path, `crate::gametick` polls it on the game's own tick, and
//! `crate::navpath` decodes what comes back.
//!
//! This module doc used to say the request could not be made, because turning a world position
//! into a graph id was an asynchronous engine job. **It is not**: `0x14037be30` does the whole
//! snap synchronously in twenty-eight instructions. bd `ds2-mods-rs-4yd` was filed on the wrong
//! premise and has been corrected.
//!
//! **The arrow has not gone anywhere**, and it is not a placeholder. It is what you get when the
//! planner answers "there is no way to walk there" -- which happens whenever the other player is
//! on the other side of a fog gate, in another area, or off the navmesh entirely -- and it is
//! what every player past the nearest one gets, because one route is one planner and the engine's
//! own AI allocates one per agent.
//!
//! # The four things this does per frame, in order
//!
//! 1. **Re-read the config** if the file changed -- by content, not by timestamp.
//! 2. **Sample the toggle key**, only when the game has focus.
//! 3. **Read the roster** from `CharacterManager`, keeping objects whose vtable is `PlayerCtrl`'s
//!    and which are not the local player. See `crate::census`.
//! 4. **Find the camera** among a fixed list of candidates, keeping the one whose projection
//!    matrix has the shape the engine's own builder emits AND that agrees with where the game
//!    says the player is. See `crate::camera`.
//!
//! Any of the four failing means fewer lines or none, never an exception: all of this runs inside
//! `IDXGISwapChain::Present`, on the game's render thread, while someone is being invaded.
//!
//! # What has actually been run
//!
//! Seven live runs on 2026-09-22, under Proton, with the game's own log as the testimony and two
//! screenshots as the correction. The chain is proved end to end: the shader compiles against the
//! game's own device, the pipeline state is saved and restored, eighteen vertices -- three
//! segments of six, a shaft and two barbs -- reach the back buffer, and the game goes on
//! rendering.
//!
//! **What the screenshots proved that the log could not.** Both showed an orange line hanging in
//! the sky with nothing under it, while the log said `drew 1 arrow(s), 18 vertices` and looked
//! entirely healthy. The cause was the same in both camera paths and is worth naming once: each
//! vetted a camera when it found one and then reused it without re-checking. A camera is only
//! the camera for the frame it was taken from, so the arrow was correct at capture and wrong
//! from the next turn of the player's head onwards -- a line in the WRONG PLACE, which the log
//! cannot distinguish from a line in the right one.
//!
//! Two things follow, and both are in the code rather than in this comment. `crate::capture`
//! remembers the PLACE the matrix was found and re-reads it every frame. And the arrow's base is
//! no longer projected at all: `emit` pins it to the middle of the frame, where a third-person
//! camera keeps the character, so the camera decides which way the shaft POINTS and nothing
//! else. A stale camera now draws an arrow that is a little off true instead of an arrow that has
//! come away from the player's body.
//!
//! **Still unmeasured: whether the overlay is USEFUL.** Every run so far has been offline and
//! solo, so the `remotes=1` in them is an NPC phantom or a bloodstain replay -- DARK SOULS II
//! builds both from the same class (see `crate::census`) -- rather than an invader.
//!
//! # What it does to the game
//!
//! **No param edits, no writes to game memory, no input injection, no network traffic.** It reads
//! a roster, reads two matrices, and appends triangles to a frame that was already finished.
//!
//! Two things it does beyond drawing, both only while the overlay is on and someone is in your
//! session, and both through the game's own functions rather than by writing memory:
//!
//! - **It asks the navigation system for a route.** That means creating an `NvRoutePlanner` --
//!   with `0x140bae8d0`, the engine's own factory -- and letting `NvNavigationSystem::Update`
//!   step it, which is what every AI in the map is already doing. Nothing is created until a
//!   route is actually wanted.
//! - **It spawns Prism Stone effects** along that route, if `marker_effect_id` is not `0`, and
//!   extinguishes them again as you walk past. `marker_effect_id` is `0` by default, so this is
//!   off unless the file asks for it.
//!
//! Both happen on `NvNavigationSystem::Update`, not in the `Present` detour -- `crate::gametick`
//! has the reason, and it is not the one the repo used to give.
//!
//! Two detours are installed. `IDXGISwapChain::Present` lives in `dxgi.dll` and is therefore
//! outside everything this workspace has learned about Arxan; `NvNavigationSystem::Update` is in
//! the game image but is not one of its 286 redirected entries, and its prologue is checked
//! before the detour goes in.

#![cfg_attr(not(windows), allow(dead_code))]

pub mod camera_yaw;
pub mod config;
pub mod frame_hook;
pub mod geometry;
pub mod log;
pub mod navpath;
pub mod routes;

pub(crate) mod lines;
pub(crate) mod selfcheck;
pub(crate) mod trail;

#[cfg(windows)]
pub(crate) mod camera;
#[cfg(windows)]
pub(crate) mod capture;
#[cfg(windows)]
pub(crate) mod census;
#[cfg(windows)]
pub(crate) mod gametick;
#[cfg(windows)]
pub(crate) mod navquery;
#[cfg(windows)]
pub(crate) mod render;
#[cfg(windows)]
pub(crate) mod sfx;

pub use log::{LOG_PREFIX, LogFn, set_logger};

#[cfg(windows)]
pub use windows_impl::{Report, Request, install};

#[cfg(windows)]
mod windows_impl {
    use core::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use ds2_hotkey_config::keys::{MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_SHIFT};
    use ds2_hotkey_config::reload::{FileChange, HotFile};
    use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
    use windows::Win32::Graphics::Dxgi::IDXGISwapChain;

    use crate::camera::{Found, Tracker};
    use crate::config::PathConfig;
    use crate::geometry::{self, Camera};
    use crate::lines::{VERTICES_PER_SEGMENT, Vertex, push_segment};
    use crate::log::log;
    use crate::routes::{Palette, Route, RouteShape};
    use crate::{census, render};

    unsafe extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetForegroundWindow() -> *mut c_void;
        fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
        fn GetCurrentProcessId() -> u32;
    }

    /// `VK_CONTROL`, `VK_MENU`, `VK_SHIFT` -- the three modifiers a chord can carry.
    const VK_CONTROL: i32 = 0x11;
    const VK_MENU: i32 = 0x12;
    const VK_SHIFT: i32 = 0x10;

    /// What the loader asks for.
    #[derive(Clone, Debug, Default)]
    pub struct Request {
        /// The config file to read and watch. `None` leaves the built-in defaults in force and
        /// the key unrebindable, which is a degraded mode rather than the normal one.
        pub config_path: Option<PathBuf>,
    }

    /// What [`install`] managed to do.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Report {
        /// The `Present` detour is live and the overlay can draw.
        pub installed: bool,
    }

    /// Everything the per-frame path owns.
    ///
    /// Behind a [`Mutex`] rather than in atomics because it is a config, a palette and a camera
    /// tracker rather than three numbers -- and behind `try_lock` rather than `lock`, so a second
    /// thread calling `Present` skips a frame instead of blocking the renderer. Skipping a frame
    /// of an overlay is invisible; stalling the game's present is not.
    struct State {
        config: PathConfig,
        file: Option<HotFile>,
        palette: Palette,
        tracker: Tracker,
        enabled: bool,
        /// The toggle key's state last frame, so a held key is one toggle and not sixty.
        toggle_was_down: bool,
        /// Whether the captured-camera line has been written. See `crate::capture`.
        said_captured: bool,
        /// Whether the first frame has been reported. See the line it writes.
        said_first_frame: bool,
        /// Whether the last pass found a camera, so the refusal is logged on the edge rather
        /// than every frame.
        ///
        /// **Starts `true`, which is a lie that makes the first refusal audible.** Started
        /// `false` these two logged nothing until a success had happened first, so a session
        /// that never found a camera said nothing at all about it -- and "no camera" is the one
        /// state a reader most needs named.
        had_camera: bool,
        /// Whether the last pass found the world, likewise, and `true` for the same reason.
        had_world: bool,
        /// The last roster counts logged, so the line is written on change rather than per frame.
        last_census: Option<(usize, usize, usize, usize)>,
        /// The last (arrows, vertices) pair logged, likewise.
        last_drawn: Option<(usize, usize)>,
        /// Whether the markers-requested line has been written. One line per session: the file
        /// is re-read every second and a per-read complaint would be a log full of it.
        said_markers: bool,
        /// The `CharacterCtrl` the self-check latched onto, so a walking NPC stays the target
        /// while it walks and two NPCs milling about do not swap the destination every frame.
        /// See `census::self_check_target`.
        self_check_target: Option<usize>,
    }

    static STATE: Mutex<Option<State>> = Mutex::new(None);

    /// Install the overlay.
    ///
    /// # Safety
    ///
    /// Installs a native code detour on `IDXGISwapChain::Present`. Call once, from the loader's
    /// install position, before the game has presented a frame.
    pub unsafe fn install(request: &Request) -> Report {
        let (config, file) = match request.config_path.as_ref() {
            Some(path) => {
                let mut file = HotFile::new(path.clone());
                // The first poll always reads, so this is the file's contents rather than a
                // change against them. A missing file is the defaults, not a failure.
                let config = match file.poll() {
                    Some(FileChange::Text(text)) => PathConfig::parse(&text),
                    _ => PathConfig::default(),
                };
                (config, Some(file))
            }
            None => (PathConfig::default(), None),
        };

        if config.toggle_fell_back {
            log(format_args!(
                "config: \"{}\" is not a key name -- falling back to {}",
                config.toggle_text,
                crate::config::DEFAULT_TOGGLE_KEY_NAME
            ));
        }
        log(format_args!(
            "toggle key {} -- suppressing inside {:.0}m, at most {} players",
            config.toggle_text, config.near_suppress_meters, config.max_targets
        ));

        let enabled = config.start_enabled;
        if let Ok(mut state) = STATE.lock() {
            *state = Some(State {
                config,
                file,
                palette: Palette::default(),
                tracker: Tracker::default(),
                enabled,
                toggle_was_down: false,
                said_captured: false,
                said_first_frame: false,
                had_camera: true,
                had_world: true,
                last_census: None,
                last_drawn: None,
                said_markers: false,
                self_check_target: None,
            });
        }

        // SAFETY: the caller's contract, forwarded -- one detour, from the install position.
        let installed = unsafe { render::install() };
        // THE SECOND SEAM, AND IT IS NOT OPTIONAL FOR THE ROUTE. `Present` may ask for a route;
        // only the game's own tick may fetch one. `crate::gametick` says why that is a property
        // of the frame rather than of the thread -- DARK SOULS II presents from the simulation
        // thread, with two engine locks held, after `EndDraw`.
        //
        // Failing here costs the route and the trail and nothing else: the arrow needs neither.
        // SAFETY: the caller's contract, forwarded -- one detour, from the install position.
        let ticking = unsafe { crate::gametick::install() };
        if !ticking {
            log(format_args!(
                "no game tick -- the arrow still draws, but no route will be asked for and no \
                 marker placed"
            ));
        }
        Report { installed }
    }

    /// Is the game the window the keyboard is talking to?
    ///
    /// Without this the overlay toggles while the player is typing in another application, which
    /// is exactly the complaint that made the sibling workspace's hotkeys configurable in the
    /// first place.
    fn game_has_focus() -> bool {
        // SAFETY: three `user32`/`kernel32` calls with no pointers into our memory except one
        // owned local.
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return false;
            }
            let mut process = 0u32;
            GetWindowThreadProcessId(window, &mut process);
            process == GetCurrentProcessId()
        }
    }

    /// Is `key` down right now?
    fn key_down(vk: i32) -> bool {
        if vk == 0 {
            return false;
        }
        // SAFETY: `GetAsyncKeyState` takes a virtual-key code and returns a bitfield.
        (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
    }

    /// Is the configured chord held?
    fn chord_down(config: &PathConfig) -> bool {
        let chord = config.toggle;
        if !key_down(chord.vk as i32) {
            return false;
        }
        // Modifiers are matched exactly rather than as a minimum: `;` must not fire while the
        // player is holding Ctrl for something else.
        let wanted = |bit: u8, vk: i32| (chord.modifiers & bit != 0) == key_down(vk);
        wanted(MODIFIER_CTRL, VK_CONTROL)
            && wanted(MODIFIER_ALT, VK_MENU)
            && wanted(MODIFIER_SHIFT, VK_SHIFT)
    }

    /// Re-read the config if the file changed, and say what moved.
    fn reload(state: &mut State) {
        let Some(file) = state.file.as_mut() else {
            return;
        };
        // `Missing` is deliberately NOT a reset to the defaults. A config file that vanishes for
        // a moment -- an editor writing through a temporary and renaming over it is exactly that
        // -- would otherwise rebind the key twice in a second while the player watched.
        let Some(FileChange::Text(text)) = file.poll() else {
            return;
        };
        let fresh = PathConfig::parse(&text);
        if fresh == state.config {
            return;
        }
        if fresh.toggle != state.config.toggle {
            log(format_args!(
                "config: reloaded -- toggle key {} -> {}",
                state.config.toggle_text, fresh.toggle_text
            ));
            // A key that happens to be held at the moment the binding changes must not count as
            // a press of the new one.
            state.toggle_was_down = true;
        } else {
            log(format_args!("config: reloaded"));
        }
        if fresh.toggle_fell_back && !state.config.toggle_fell_back {
            log(format_args!(
                "config: \"{}\" is not a key name -- keeping {}",
                fresh.toggle_text,
                crate::config::DEFAULT_TOGGLE_KEY_NAME
            ));
        }
        state.config = fresh;
    }

    /// Build one frame's worth of vertices. Called from the `Present` detour.
    ///
    /// Never panics and never propagates: every failure is an empty frame. A `try_lock` that
    /// fails is also an empty frame, which is the correct answer to "another thread is already
    /// in here".
    pub(crate) fn frame(swap_chain: &IDXGISwapChain) -> Vec<Vertex> {
        // THE CLOCK, AND IT RUNS BEFORE ANYTHING HERE CAN DECLINE TO. `ds2-input-harness`
        // advances its whole state machine from this call: every countdown, every command
        // dispatch and the closed camera loop. It used to ride a device poll, and a live session
        // proved why that was wrong -- the game stopped calling the poll it had elected and the
        // harness went deaf while the process was still running. `Present` cannot be unplugged.
        //
        // Above the `try_lock` and above the `enabled` check on purpose: a consumer's clock must
        // not stop because the overlay had nothing to draw this frame.
        crate::frame_hook::run_frame_hook();

        let Ok(mut guard) = STATE.try_lock() else {
            return Vec::new();
        };
        let Some(state) = guard.as_mut() else {
            return Vec::new();
        };

        // ONE LINE, THE FIRST TIME THE DETOUR EVER RUNS. Without it "the overlay drew nothing"
        // and "the detour is not being called at all" are the same silence -- which is exactly
        // what the first live run produced, and it cost a second launch to tell them apart.
        // `overlay: Present hooked` says the hook went in; only this says it fires.
        if !state.said_first_frame {
            state.said_first_frame = true;
            log(format_args!(
                "first frame: the Present detour is running, overlay {}",
                if state.enabled { "on" } else { "off" }
            ));
        }

        reload(state);

        if game_has_focus() {
            let down = chord_down(&state.config);
            if down && !state.toggle_was_down {
                state.enabled = !state.enabled;
                log(format_args!(
                    "overlay {}",
                    if state.enabled { "on" } else { "off" }
                ));
                if !state.enabled {
                    // A map change while the overlay is off would otherwise leave a camera
                    // candidate pointing at a freed object.
                    state.tracker.forget();
                    state.palette.retain(&[]);
                }
            }
            state.toggle_was_down = down;
        } else {
            state.toggle_was_down = false;
        }

        if !state.enabled {
            // THE OFF SWITCH HAS TO REACH THE OTHER SEAM TOO, and this is the only place both
            // are in scope. Without it the tick keeps the last thing it was asked for forever:
            // it would go on planning a route half a second at a time, and the Prism Stones
            // behind you would stay lit for the rest of the session, for a feature the player
            // has switched off. `ask(None)` is what `crate::gametick` reads as "stand down" --
            // it puts the trail out and drops the stale answer.
            crate::gametick::ask(None);
            return Vec::new();
        }

        let Some(screen) = back_buffer_size(swap_chain) else {
            return Vec::new();
        };

        let Some(local) = census::local_position() else {
            if state.had_world {
                log(format_args!(
                    "no live character -- nothing to draw from until one exists"
                ));
                state.had_world = false;
            }
            // No world means no route and no trail either. Not the same as the camera failing to
            // frame you -- that is a view problem and the trail should go on being laid through
            // it; this is "there is nobody standing anywhere".
            crate::gametick::ask(None);
            return Vec::new();
        };
        state.had_world = true;

        // Tell the capture what to judge a candidate against, then prefer whatever it caught.
        // A matrix taken out of the renderer's own upload is the matrix the frame was drawn
        // with; one found by searching memory is a matrix that resembles it. The search stays as
        // the fallback because the capture needs an upload to happen and a player to exist, and
        // neither is true on the first frames.
        crate::capture::set_subject(local, screen);
        if let Some(camera) = crate::capture::camera() {
            if !state.said_captured {
                state.said_captured = true;
                log(format_args!("camera: using the captured view-projection"));
            }
            state.had_camera = true;
            return draw_for(state, &camera, local, screen);
        }

        let Some((camera, found)) = state.tracker.acquire(local, screen) else {
            if state.had_camera {
                let probe = state.tracker.last_probe;
                log(format_args!(
                    "no camera: {} tried, {} shaped like a projection, {} with a camera pose -- drawing \
                     nothing rather than guessing",
                    probe.tried, probe.shaped, probe.posed
                ));
                state.had_camera = false;
            }
            return Vec::new();
        };
        if let Found::Chose(candidate) = found {
            log(format_args!("camera: drawing through {candidate}"));
        }
        state.had_camera = true;
        draw_for(state, &camera, local, screen)
    }

    /// Everything downstream of having a camera: read the roster, build the arrows, project them.
    ///
    /// Split out because there are now two ways to get a camera -- captured from the renderer's
    /// own upload, or found by searching memory -- and only the getting differs.
    fn draw_for(
        state: &mut State,
        camera: &Camera,
        local: [f32; 3],
        screen: [f32; 2],
    ) -> Vec<Vertex> {
        // PUBLISH THE HEADING BEFORE ANY EARLY RETURN. This is the one point both ways of
        // getting a camera meet, and `ds2-input-harness` closes its camera-turn loop on what is
        // published here -- so it measures the same camera this overlay is drawing through
        // rather than a second resolution that could disagree. Two relaxed stores; nothing in
        // this crate reads it back. Above the framing guard on purpose: a camera pointed
        // somewhere the player is not is still the camera, and a turn that is halfway through
        // needs its readings to keep arriving while it swings past.
        crate::camera_yaw::publish(camera.yaw_degrees());

        // THERE IS NO PER-FRAME FRAMING GATE, and deleting the one that used to stand here is
        // the fix for "I've seen the base off the player more times than I've seen it on the
        // player".
        //
        // The gate was a way of not drawing when the base would land somewhere wrong, which
        // presumes the base is something projection can get wrong. It is not, any more: `emit`
        // pins it to the middle of the frame in pixels. The camera decides which way the shaft
        // POINTS and nothing else, so a camera that is a few pixels stale now draws an arrow
        // that is a few pixels off true -- rather than an arrow whose base has walked off the
        // character, which was the only symptom ever reported.
        //
        // Measured cost of keeping it: at one pixel it refused every frame of a live run, with
        // the head between 8.7 and 315.6 pixels from centre, and the overlay drew nothing at
        // all. `Camera::frames_the_character` still exists and still earns its keep in
        // `crate::camera`, where it separates a candidate matrix from the rendered one.

        // KEEP THE PROMISE THE FILE MAKES. `marker_effect_id` is parsed, and now it is also
        // acted on -- by `crate::gametick`, on the game's own tick, because this detour is the
        // wrong place to spawn anything. All this does is forward the setting; the line below
        // says once that it was forwarded, so an edit that appears to do nothing still has an
        // audible reason when the tick seam is the thing that failed.
        crate::gametick::set_markers(state.config.markers_requested().then_some(
            crate::gametick::Markers {
                effect_id: state.config.marker_effect_id,
                spacing: crate::trail::Spacing {
                    meters: state.config.marker_spacing_meters,
                    keep_behind_meters: state.config.marker_keep_behind_meters,
                    max_markers: state.config.max_markers,
                    per_pass: state.config.markers_per_pass,
                },
            },
        ));
        if state.config.markers_requested() && !state.said_markers {
            state.said_markers = true;
            log(format_args!(
                "markers: effect {} requested -- handed to the game tick, which places them \
                 along the route. The Prism Stone's own ids are {:?}; anything else is whatever \
                 effect carries that number.",
                state.config.marker_effect_id,
                ds2_rva::PRISM_STONE_SFX_IDS
            ));
        }

        let Some((players, census)) = census::remotes(state.config.max_targets) else {
            crate::gametick::ask(None);
            return Vec::new();
        };
        let present: Vec<u64> = players.iter().map(|player| player.ctrl as u64).collect();
        state.palette.retain(&present);

        // THE ROSTER LINE IS THE DIAGNOSTIC, and both halves of it are load-bearing.
        //
        // `remotes=0 characters=4` is "you are alone" and is the ordinary solo case. `remotes=0`
        // with a fat `characters` is a bug in THIS crate -- the roster was walked, hundreds of
        // objects were seen, and the vtable comparison matched none of them. Without the second
        // number those two are the same empty overlay. The Elden Ring crate learned this from a
        // log line it could not interpret; the difference here is that a `players` count of 1
        // narrows it further still, to "the local player was found and nobody else was".
        //
        // Logged on change rather than per frame, because this runs sixty times a second.
        let fingerprint = (
            census.characters,
            census.players,
            census.remotes,
            census.skipped,
        );
        if state.last_census != Some(fingerprint) {
            state.last_census = Some(fingerprint);
            log(format_args!(
                "roster: characters={} players={} remotes={} skipped={} nearest={}",
                census.characters,
                census.players,
                census.remotes,
                census.skipped,
                // The nearest distance is what says whether `near_suppress_meters` is the reason
                // nothing is on screen. Without it, "suppressed because they are close" and
                // "found nobody" are the same absence of a line.
                players.first().map_or_else(
                    || "-".to_string(),
                    |player| format!("{:.0}m", player.distance)
                )
            ));
        }

        // ONE ROUTE, TO THE NEAREST PERSON, AND THE REST KEEP THEIR ARROWS.
        //
        // Not a shortcut: a route costs an `NvRoutePlanner` linked into the engine's own update
        // list and an A* search it runs for you, and the engine's AI allocates exactly one of
        // those per agent. N planners for N phantoms would multiply the one thing in this crate
        // that touches engine state, to draw four walkable lines nobody can follow at once --
        // and the trail, which is the reason the route exists at all, can only follow one path.
        //
        // The ask is published every frame and answered whenever the tick gets to it, so the
        // route lags the roster by up to half a second. `answer_for` refuses an answer about a
        // different player rather than drawing it, because a confident line to the wrong person
        // is worse than the arrow it replaced.
        let mut routed = players
            .iter()
            .find(|player| player.distance >= state.config.near_suppress_meters)
            .copied();

        // A TARGET EVEN WHEN YOU ARE ALONE, and this no longer stops when the diagnostic does.
        //
        // It used to. `self_check_done` gated the nomination, so thirty seconds after the check
        // gave up -- which it does whenever the route never arrives, and the route is not
        // arriving -- there was no target, no route, no arrow and no dot. A solo session got
        // `drew 0 arrow(s), 0 vertices` and a player looking at the screen got nothing at all,
        // with nothing on screen to say whether the overlay was even running. That is the
        // diagnostic switching OFF THE FEATURE it was written to observe.
        //
        // Two things were conflated and are now separate. The TRAIL diagnostic finishes, says
        // what it found and stops laying stones -- that is what `self_check_done` means, and the
        // tick still honours it. WHO TO POINT AT is a question about the overlay, and the answer
        // when nobody else is in your session is "the nearest character", for as long as you are
        // alone. A real player always wins; see `config::DEFAULT_NPC_SELF_CHECK`.
        crate::gametick::set_self_check(state.config.npc_self_check);
        if !state.config.npc_self_check {
            // Switched off in the file: stop bypassing `near_suppress_meters` for the old
            // target, or an NPC standing next to you keeps an arrow after the key says no.
            state.self_check_target = None;
        }
        if state.config.npc_self_check && routed.is_none() {
            let latched = state.self_check_target;
            if let Some(npc) = census::self_check_target(latched) {
                if latched != Some(npc.ctrl) {
                    state.self_check_target = Some(npc.ctrl);
                    // THE PICK IS LOGGED, so a bad one is visible rather than inferred. An
                    // address, a distance and a position: if the route then fails, the position
                    // is what says whether the thing picked was somewhere a route could start.
                    log(format_args!(
                        "self-check: picked character 0x{:016x} at {:.1},{:.1},{:.1}, {:.1} m \
                         away -- routing to it",
                        npc.ctrl, npc.position[0], npc.position[1], npc.position[2], npc.distance
                    ));
                }
                routed = Some(npc);
            } else if state.self_check_target.take().is_some() {
                log(format_args!(
                    "self-check: the character it was routing to has left the roster -- picking \
                     another"
                ));
            }
        }

        crate::gametick::ask(routed.as_ref().map(|player| crate::gametick::Wanted {
            from: local,
            to: player.position,
            target: player.ctrl as u64,
        }));

        // DRAW THE SELF-CHECK'S TARGET TOO. The user is at the keyboard looking at the screen,
        // and a diagnostic whose whole output is in a file gives them nothing to look at. The
        // NPC is appended rather than substituted, so a session that has both a real player and
        // the check running shows both.
        let mut drawn = players.clone();
        if let Some(npc) = routed.filter(|target| !drawn.iter().any(|p| p.ctrl == target.ctrl)) {
            drawn.push(npc);
        }

        let mut snapshot = Vec::new();
        for player in &drawn {
            // `near_suppress_meters` exists so the overlay stops shouting about someone standing
            // next to you. It must NOT apply to the self-check's target: an NPC thirty metres
            // away is the normal case in Majula, and a diagnostic the user cannot see because it
            // worked too close to them is a diagnostic that reports nothing.
            let is_check_target = state.self_check_target == Some(player.ctrl);
            if player.distance < state.config.near_suppress_meters && !is_check_target {
                continue;
            }
            let slot = state.palette.slot_for(player.ctrl as u64);
            // `Some(Some(points))` is a walkable route; `Some(None)` is the planner having
            // answered "there is no way to walk there"; `None` is no answer yet. The last two
            // both draw the arrow, and deliberately look the same on screen -- the difference
            // between them is in the log, not in what the player needs to do about it.
            if let Some(Some(points)) = crate::gametick::answer_for(player.ctrl as u64)
                && points.len() >= 2
            {
                snapshot.push(Route::new(
                    RouteShape::Walk(points),
                    slot,
                    player.distance,
                    state.config.bold_at_meters,
                    state.config.faint_at_meters,
                ));
                continue;
            }
            // THE DESTINATION, NOT A SHAPE. The arrow is built in pixels at draw time, so there
            // is nothing to construct here and nothing that can fail: a target directly behind
            // the camera, at the same position as the player, or fifty metres below all produce
            // an arrow, because the direction is read out of clip space rather than projected.
            snapshot.push(Route::new(
                RouteShape::Arrow(player.position),
                slot,
                player.distance,
                state.config.bold_at_meters,
                state.config.faint_at_meters,
            ));
        }

        let mut vertices = Vec::with_capacity(snapshot.len() * 3 * VERTICES_PER_SEGMENT);
        for route in &snapshot {
            emit(&mut vertices, route, camera, screen);
        }

        // THE ONLY LINE THAT SAYS ANYTHING WAS DRAWN. `camera:` proves a camera was found and
        // `roster:` proves players were seen; neither proves a triangle reached the vertex
        // buffer, and the three steps in between -- suppression, the arrow's direction, the
        // near-plane clip -- can each legitimately eat everything. Logged on change so a run
        // with a steady overlay writes it once.
        let drawn = (snapshot.len(), vertices.len());
        if state.last_drawn != Some(drawn) {
            state.last_drawn = Some(drawn);
            log(format_args!(
                "drew {} arrow(s), {} vertices",
                drawn.0, drawn.1
            ));
        }
        vertices
    }

    /// The shortest an arrow's shaft may appear, as a fraction of the viewport height.
    ///
    /// **A world-space arrow pointing away from the camera foreshortens to a dot**, and that is
    /// not a corner case -- it is what happens whenever you are running towards the person you
    /// are pointing at, which is most of the time. The first live look at this feature was the
    /// user asking whether the orange thing on screen was an arrow at all; it was, three metres
    /// long, aimed almost straight down the view axis, and it projected to a handful of pixels.
    ///
    /// A FRACTION AND NOT A PIXEL COUNT. Eighty pixels was chosen against 1080p and looks like a
    /// legible glyph there; on the 2561x1440 back buffer this actually runs on it is a stub about
    /// as long as the character is wide, which is how the arrow came to be sitting ON the player
    /// and still be unreadable as an arrow. A share of the viewport is the same size to the eye
    /// whatever the display is.
    /// How long the shaft is, as a share of the viewport's height. **A fixed size, not a
    /// minimum.**
    ///
    /// It used to be a floor under a world-space length that was then grown towards the target
    /// until it looked long enough. That cannot work when the direction has no extent on screen:
    /// a live run produced a twelve-pixel stub with both barbs on the same pixel, pointing at an
    /// NPC 84.6 m away that happened to be near the view axis -- which is the ordinary case, not
    /// a rare one, because you are usually facing roughly towards whoever you are looking for.
    /// A compass needle does not foreshorten, so neither does this.
    const ARROW_SHARE: f32 = 0.15;

    /// How long each barb is, as a share of the viewport's height. A third of the shaft.
    const ARROW_BARB_SHARE: f32 = 0.05;

    /// The last ten-degree band of heading the arrow pointed in, and the frames left before
    /// another sample may be written.
    ///
    /// SAMPLED ON MOVEMENT, NOT ON A COUNT. A fixed number of samples runs out while the game
    /// sits on a loaded save with nobody at the controls, which is exactly when the arrow cannot
    /// move and the log therefore proves nothing. Keying the sample to a CHANGE of heading makes
    /// a still camera silent and a turning one talkative, which is the right way round.
    static ARROW_BAND: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(usize::MAX);
    static ARROW_COUNTDOWN: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(0);

    /// Frames between arrow samples, so a swinging camera writes a readable trail rather than one
    /// line per frame.
    const ARROW_SAMPLE_FRAMES: usize = 15;

    /// Half the width of the dot that marks the arrow's base, in pixels.
    ///
    /// The base is a fixed point on the screen, so it is drawn rather than implied: "there should
    /// ALWAYS be an orange dot at the very centre of the screen". A visible dot is also the only
    /// thing that distinguishes "the overlay is running and nobody is being pointed at" from "the
    /// overlay is not running", which no line in a file can tell you while you are playing.
    const BASE_DOT_RADIUS_PX: f32 = 4.0;

    /// Project one route and append its triangles.
    fn emit(out: &mut Vec<Vertex>, route: &Route, camera: &Camera, screen: [f32; 2]) {
        let color = [route.color[0], route.color[1], route.color[2], route.alpha];
        match &route.shape {
            RouteShape::Arrow(target) => {
                // THE WHOLE ARROW IS IN PIXELS. Base on the centre, fixed length, direction from
                // the camera -- see `Camera::screen_arrow` for why it stopped being a world-space
                // object. Nothing below can collapse, clip, or land behind the lens, so there is
                // no case in which this branch draws nothing.
                let arrow = camera.screen_arrow(
                    geometry::head(*target),
                    screen,
                    screen[1] * ARROW_SHARE,
                    screen[1] * ARROW_BARB_SHARE,
                );
                let ready = ARROW_COUNTDOWN
                    .fetch_update(
                        core::sync::atomic::Ordering::Relaxed,
                        core::sync::atomic::Ordering::Relaxed,
                        |left| Some(left.saturating_sub(1)),
                    )
                    .is_ok_and(|left| left == 0);
                // Sampled on a CHANGE of heading, so a still camera is silent and a turning one
                // is talkative. The band is the tip's angle in whole tens of degrees.
                let turn = (arrow.tip[1] - arrow.base[1]).atan2(arrow.tip[0] - arrow.base[0]);
                let band = (turn.to_degrees() / 10.0) as i32 as usize;
                if ready && ARROW_BAND.swap(band, core::sync::atomic::Ordering::Relaxed) != band {
                    ARROW_COUNTDOWN
                        .store(ARROW_SAMPLE_FRAMES, core::sync::atomic::Ordering::Relaxed);
                    log(format_args!(
                        "arrow: base {:.0},{:.0} -> tip {:.0},{:.0} ({:.0} deg) | target \
                         {:.1},{:.1},{:.1} | screen={:.0}x{:.0}",
                        arrow.base[0],
                        arrow.base[1],
                        arrow.tip[0],
                        arrow.tip[1],
                        turn.to_degrees(),
                        target[0],
                        target[1],
                        target[2],
                        screen[0],
                        screen[1]
                    ));
                }
                // The base is drawn as a dot because it is a fixed point on the dial, and a
                // shaft alone leaves nothing under the reticle when the arrow points straight at
                // you and foreshortens -- which it cannot now, but the dot is also the only
                // thing on screen that says the overlay is running at all.
                push_segment(
                    out,
                    [arrow.base[0] - BASE_DOT_RADIUS_PX, arrow.base[1]],
                    [arrow.base[0] + BASE_DOT_RADIUS_PX, arrow.base[1]],
                    BASE_DOT_RADIUS_PX * 2.0,
                    color,
                );
                push_segment(out, arrow.base, arrow.tip, route.stroke_px, color);
                push_segment(out, arrow.tip, arrow.left_barb, route.stroke_px, color);
                push_segment(out, arrow.tip, arrow.right_barb, route.stroke_px, color);
            }
            RouteShape::Walk(points) => {
                // NOT PINNED. A walkable route is a path over the ground and every point of it
                // belongs where the world puts it; dragging the whole polyline so its first node
                // sits under the reticle would draw a path nobody can follow. Only the arrow is
                // a dial.
                for pair in points.windows(2) {
                    if let Some((a, b)) = camera.project_segment(pair[0], pair[1], screen) {
                        push_segment(out, a, b, route.stroke_px, color);
                    }
                }
            }
        }
    }

    /// The back buffer's size in pixels, which is the coordinate space the overlay draws in.
    fn back_buffer_size(swap_chain: &IDXGISwapChain) -> Option<[f32; 2]> {
        // THE TEXTURE, NOT THE SWAP CHAIN'S DESCRIPTION, and the two are not the same number.
        //
        // This used to read `GetDesc().BufferDesc`, which is the MODE the chain was created
        // with. `crate::render` has always sized its viewport and its shader's `inverse_size`
        // from the back buffer texture instead. Two sources of truth for one number is a scale
        // and offset error between where a point is projected and where it is drawn, and the
        // live log caught them disagreeing: the same display reported `2561x1440` on one run and
        // `2560x1441` on the next, neither of which is a back buffer any hardware produces.
        //
        // A pixel of disagreement moves the arrow by a pixel; a game rendering at a different
        // internal resolution from its window moves it across the screen. Both paths now ask the
        // texture that is actually about to be presented.
        //
        // SAFETY: slot 0 is the back buffer of any swap chain, and `swap_chain` is the game's
        // own, borrowed by the detour.
        let back_buffer = unsafe { swap_chain.GetBuffer::<ID3D11Texture2D>(0) }.ok()?;
        let mut description = Default::default();
        // SAFETY: `back_buffer` is a live texture; `GetDesc` fills the out-parameter.
        unsafe { back_buffer.GetDesc(&mut description) };
        let width = description.Width as f32;
        let height = description.Height as f32;
        if width > 0.0 && height > 0.0 {
            Some([width, height])
        } else {
            None
        }
    }
}

/// The per-frame entry point the `Present` detour calls.
#[cfg(windows)]
pub(crate) fn frame(
    swap_chain: &windows::Win32::Graphics::Dxgi::IDXGISwapChain,
) -> Vec<lines::Vertex> {
    windows_impl::frame(swap_chain)
}
