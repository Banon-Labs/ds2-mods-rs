//! A direction to every other player in your session, drawn over the world.
//!
//! Press the key (default `;`) during an invasion or a co-op session and every other player in it
//! gets a coloured arrow leaving your body and pointing at them -- in three dimensions, so a
//! player forty metres straight down reads as *down* rather than as five metres away.
//!
//! | situation | what you see |
//! |---|---|
//! | another player in the session | a walkable path of Prism Stones, or an arrow when there is no way to walk there |
//! | N players | N paths and N arrows, N colours, each colour stuck to one player |
//! | more than `max_routes` of them | the nearest four get paths; the rest draw nothing until a lane frees up |
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
//! planner answers "there is no way to walk there" -- whenever the other player is on the other
//! side of a fog gate, in another area, or off the navmesh entirely -- and that answer is the
//! only thing it means. Somebody past `max_routes` has not been asked about, which is not the
//! same claim, so they draw nothing rather than an arrow that would be asserting something
//! nobody looked up.
//!
//! # Several paths at once
//!
//! Every target past `near_suppress_meters`, up to `max_routes`, gets a path of its own: its own
//! `NvRoutePlanner`, its own search in flight, its own trail of stones in its own colour. Two
//! invaders are two lines, and the colour that identifies a person on their arrow is the colour of
//! the stones under their path.
//!
//! This drew exactly one, to the nearest person, on the argument that a planner is engine state
//! and the game's own AI allocates one per agent. It does -- per AGENT, so an area with a dozen
//! hollows in it is already stepping a dozen, and four more is a rounding error against that.
//! `crate::gametick` holds the rest of the reasoning, including which budgets are divided between
//! the paths rather than multiplied by them.
//!
//! # How often a path is re-planned
//!
//! When either end has walked far enough to make the old one wrong -- `replan_move_meters` -- with
//! a floor under it so a fall cannot ask every frame and a ceiling over it so a fog gate opening
//! is eventually noticed. Not on a timer; `crate::trail::Cadence` has the argument and the
//! measurements. When a fresh route no longer runs under the stones already down, the whole trail
//! is put out and laid again from your feet rather than left forking down a corridor nobody is
//! walking.
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
//! solo, and the `remotes` in them were never invaders. A 2026-09-24 run laid stone paths to two
//! of them and named them when asked: `Npc_c741000` and `Npc_c761000`. DARK SOULS II builds
//! humanoid NPCs, bloodstain replays and wandering ghosts out of the same class as a person, and
//! this crate counted all of them. It no longer does -- `crate::census::is_person` keeps only the
//! name the remote-player factory formats, and the roster line names everything it threw out --
//! but that means no run so far has routed to a human being, and the thing the overlay exists to
//! do remains untested in the only way that would settle it.
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

// DEBT: ds2-mods-rs-2rs -- module-wide dead_code, reason not yet recorded.
#![cfg_attr(not(windows), allow(dead_code))]

pub mod camera_yaw;
pub mod config;
pub mod frame_hook;
pub mod geometry;
pub mod log;
pub mod navpath;
pub mod routes;

pub(crate) mod lines;
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
        ///
        /// The whole [`census::Census`] rather than a tuple of its fields: a tuple has to be
        /// widened by hand every time a count is added, and the compiler only notices because the
        /// arity stops matching, not because the line went stale.
        last_census: Option<census::Census>,
        /// The last (routes, arrows, vertices) triple logged, likewise.
        last_drawn: Option<(usize, usize, usize)>,
        /// The effect id last handed to the tick, so a change of target writes one line rather
        /// than sixty a second. `0` is "nothing laid yet" and is not a real id.
        last_marker_id: u32,
        /// Consecutive frames the roster counts have not changed. See `ROSTER_SETTLE_FRAMES`.
        roster_still: u32,
        /// Whether the markers-requested line has been written. One line per session: the file
        /// is re-read every second and a per-read complaint would be a log full of it.
        said_markers: bool,
        /// Whether the camera matrix has been dumped. Once per session; see its use in `draw_for`.
        said_matrix: bool,
        /// The `CharacterCtrl` the self-check latched onto, so a walking NPC stays the target
        /// while it walks and two NPCs milling about do not swap the destination every frame.
        /// See `census::self_check_target`.
        self_check_target: Option<usize>,
        /// Who the planner last said it could not walk to, so the arrow's reason is written once
        /// per target rather than sixty times a second.
        unreachable: Option<usize>,
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
                last_marker_id: 0,
                roster_still: 0,
                said_markers: false,
                said_matrix: false,
                self_check_target: None,
                unreachable: None,
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
            crate::gametick::ask(Vec::new());
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
            crate::gametick::ask(Vec::new());
            return Vec::new();
        };
        state.had_world = true;

        // There is a world now, so the capture may start hunting, and here is somebody standing
        // in it. A matrix taken out of the renderer's own upload is the matrix the frame was
        // drawn with; one found by searching memory is a matrix that RESEMBLES it, and
        // resembling is not enough to draw a line through the world.
        //
        // The character is handed over because the shape of the matrix is not sufficient on its
        // own -- see `capture::grounds`. It is not used to judge where they land on screen.
        crate::capture::arm(local);

        // Resolve the camera to an `Option` and hand that on, rather than returning early when
        // there is none. Not a tidy-up: the early return used to skip the roster, the target
        // nomination and the ask along with the projection, so a run whose camera was never
        // recognised also never asked for a route and never laid a stone -- and said nothing in
        // the log about either. See `draw_for`, which now decides everything a camera has no
        // bearing on before it looks at one.
        let camera = if let Some(camera) = crate::capture::camera() {
            if !state.said_captured {
                state.said_captured = true;
                log(format_args!("camera: using the captured view-projection"));
            }
            state.had_camera = true;
            Some(camera)
        } else {
            // THE SEARCH RUNS AND DOES NOT DRAW, and the split is measured rather than tasteful.
            //
            // It used to draw whenever the capture had nothing, and a live run on 2026-09-24 put
            // 1835 of about 1840 drawn frames through it against ONE through a captured matrix.
            // What that produced on screen was a straight green line across the sky that touched
            // neither the player nor the target -- because the search does not return one answer,
            // it returns whichever of two unrelated camera objects last passed its oracles, and
            // it changed its mind on nearly every frame it logged.
            //
            // `crate::capture`'s own header already settled this: the oracles are sound, nothing
            // in `CameraManager` passes them, and the conclusion is not "search harder" but that
            // the renderer's matrix is not reachable from there at all. A fallback that draws a
            // near-miss is worse than no overlay, because a wrong line is read as a wrong ROUTE.
            //
            // So `acquire` is still called -- its probe counts are the only evidence of what was
            // tried, and the `no camera` line below is assembled from them -- and its answer is
            // dropped. When the capture has nothing, this draws nothing.
            let searched = state.tracker.acquire(local, screen);
            if let Some((_, Found::Chose(candidate))) = &searched {
                log(format_args!(
                    "camera: the memory search settled on {candidate} -- NOT drawn through; only \
                     a matrix caught in the renderer's own upload is"
                ));
            }
            if state.had_camera {
                // The capture's own counts, not the memory search's. The search no longer draws,
                // so what it tried says nothing about why there is no line on screen -- and the
                // four numbers below separate the three ways this fails: the detours never fired,
                // they fired and nothing was walked, or everything was walked and no window was
                // ever the right shape.
                let seen = crate::capture::probe();
                log(format_args!(
                    "no camera: {} upload(s) seen, {} walked, {} window(s) tested, {} shaped like \
                     a view-projection -- drawing nothing rather than guessing. The route and the \
                     trail carry on without it.",
                    seen.uploads, seen.walked, seen.windows, seen.shaped
                ));
                state.had_camera = false;
            }
            None
        };
        draw_for(state, camera.as_ref(), local, screen)
    }

    /// Read the roster, decide who to point at, ask the tick for a route, and -- if there is a
    /// camera -- project the result.
    ///
    /// Split out because there are two ways to get a camera (captured from the renderer's own
    /// upload, or found by searching memory) and only the getting differs.
    ///
    /// The camera is optional, and making it optional is a bug fix, measured 2026-09-24.
    ///
    /// This took a `&Camera` and was reached only once one had been found. A session where the
    /// capture recognised nothing and the fallback search agreed -- `no camera: 98 tried, 2
    /// shaped like a projection, 9 with a camera pose` -- therefore never got here at all:
    /// `set_self_check` was never called, no target was ever nominated, no ask was ever
    /// published, and the log said nothing whatsoever about routing. "The route failed" and "the
    /// route was never requested" looked identical from the outside, and the run that prompted
    /// this spent its whole length being the second one.
    ///
    /// The file already said this was wrong in two places. The no-world guard in [`frame`]
    /// separates "there is nobody standing anywhere" from "the camera is not framing you -- that
    /// is a view problem and the trail should go on being laid through it", and the comment on
    /// the nomination below says in as many words that a diagnostic must not switch off the
    /// feature it was written to observe. Only the control flow disagreed.
    ///
    /// So everything above the projection runs whether or not there is a camera. A camera
    /// decides what can be drawn and nothing else: it has no say in where a path goes, and the
    /// Prism Stone trail is laid by the game tick, which never needed one.
    fn draw_for(
        state: &mut State,
        camera: Option<&Camera>,
        local: [f32; 3],
        screen: [f32; 2],
    ) -> Vec<Vertex> {
        // THE MATRIX ITSELF, ONCE, BECAUSE NOBODY HAS EVER LOOKED AT IT.
        //
        // Every direction this crate draws comes out of these sixteen floats, and three hours of
        // wrong arrows were spent reasoning about what they must contain instead of reading
        // them. A left-handed projection built by `0x140001a90` has a POSITIVE x scale; if the
        // product's x column is negated, every horizontal offset this crate computes is mirrored
        // and both the projection and the needle are mirrored together -- which is exactly the
        // symptom, and is invisible to any test that compares those two against each other.
        //
        // Printed once per camera acquisition, not per frame.
        if let Some(camera) = camera
            && !state.said_matrix
        {
            state.said_matrix = true;
            let m = camera.view_projection;
            log(format_args!(
                "camera matrix (row-major): [{:.4} {:.4} {:.4} {:.4} | {:.4} {:.4} {:.4} {:.4} \
                 | {:.4} {:.4} {:.4} {:.4} | {:.1} {:.1} {:.1} {:.1}] | player {:.1},{:.1},{:.1}",
                m[0],
                m[1],
                m[2],
                m[3],
                m[4],
                m[5],
                m[6],
                m[7],
                m[8],
                m[9],
                m[10],
                m[11],
                m[12],
                m[13],
                m[14],
                m[15],
                local[0],
                local[1],
                local[2]
            ));
        }

        // PUBLISH THE HEADING BEFORE ANY EARLY RETURN. This is the one point both ways of
        // getting a camera meet, and `ds2-input-harness` closes its camera-turn loop on what is
        // published here -- so it measures the same camera this overlay is drawing through
        // rather than a second resolution that could disagree. Two relaxed stores; nothing in
        // this crate reads it back. Above the framing guard on purpose: a camera pointed
        // somewhere the player is not is still the camera, and a turn that is halfway through
        // needs its readings to keep arriving while it swings past.
        if let Some(camera) = camera {
            crate::camera_yaw::publish(camera.yaw_degrees());
        }

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
            crate::gametick::ask(Vec::new());
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
        // Captured BEFORE the line below overwrites it: the settle gate further down needs to
        // know whether the roster CHANGED this frame, and by then `last_census` always matches.
        let roster_changed = state.last_census != Some(census);
        if roster_changed {
            state.last_census = Some(census);
            log(format_args!(
                "roster: characters={} players={} remotes={} not_people={} skipped={} nearest={}",
                census.characters,
                census.players,
                census.remotes,
                census.not_people,
                census.skipped,
                // The nearest distance is what says whether `near_suppress_meters` is the reason
                // nothing is on screen. Without it, "suppressed because they are close" and
                // "found nobody" are the same absence of a line.
                players.first().map_or_else(
                    || "-".to_string(),
                    |player| format!("{:.0}m", player.distance)
                )
            ));
            // Name the recordings that were thrown out, so "the overlay is ignoring that figure
            // over there" is a claim the log can settle. Only when there are any, and only on the
            // same change that wrote the line above -- it walks the roster again.
            if census.not_people > 0 {
                log(format_args!(
                    "roster: not people -- {}",
                    census::describe_not_people(state.config.max_targets)
                ));
            }
        }

        // A ROUTE AND A TRAIL PER PERSON, UP TO `max_routes`, AND THE REST KEEP THEIR ARROWS.
        //
        // This drew exactly one, to the nearest person, and the argument for that was real: a
        // route costs an `NvRoutePlanner` linked into the engine's own update list and an A*
        // search it runs for you, and the engine's AI allocates one of those per agent. What the
        // argument missed is the other half of that sentence -- one per AGENT, so an area with a
        // dozen hollows in it is already stepping a dozen, and four more is a rounding error
        // against what the engine does to itself. What it cost was the case the feature exists
        // for: two invaders, and a line to one of them.
        //
        // So every target past `near_suppress_meters` gets its own lane -- its own planner, its
        // own search in flight, its own trail in its own colour -- and `max_routes` bounds how
        // many. `crate::gametick::lay_markers` divides the stone budget between them, so a fourth
        // target shortens the trails rather than quadrupling the spawns.
        //
        // The ask is published every frame and answered whenever the tick gets to it. `answer_for`
        // refuses an answer about a different player rather than drawing it, because a confident
        // line to the wrong person is worse than the arrow it replaced.
        let far_enough: Vec<census::Player> = players
            .iter()
            .filter(|player| player.distance >= state.config.near_suppress_meters)
            .take(state.config.max_routes)
            .copied()
            .collect();
        // The nearest of them, for the self-check's "is anybody real being routed to" question.
        let mut routed = far_enough.first().copied();

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
        // DO NOT LATCH ONTO A ROSTER THAT IS STILL LOADING.
        //
        // The target is held until it leaves the roster, the way you keep pointing at one invader
        // until they are gone -- so the FIRST pick is the only one that matters, and making it
        // early is making it wrong for the rest of the session. Live, the roster went
        // `characters=1`, then 3, then 5, then 6 over the first seconds of a load, and the pick
        // landed on something 84.6 m away that the player could not see, while the Emerald Herald
        // ten metres in front of them had not been added yet.
        //
        // So the count has to hold still first. Once it does, the nearest character is the one a
        // player would name, and it stays the target until it dies or despawns.
        let settled = if !roster_changed {
            state.roster_still = state.roster_still.saturating_add(1);
            state.roster_still >= ROSTER_SETTLE_FRAMES
        } else {
            state.roster_still = 0;
            false
        };
        if state.config.npc_self_check
            && routed.is_none()
            && (settled || state.self_check_target.is_some())
        {
            let latched = state.self_check_target;
            if let Some(npc) = census::self_check_target(latched) {
                if latched != Some(npc.ctrl) {
                    state.self_check_target = Some(npc.ctrl);
                    // THE PICK IS LOGGED, so a bad one is visible rather than inferred. An
                    // address, a distance and a position: if the route then fails, the position
                    // is what says whether the thing picked was somewhere a route could start.
                    log(format_args!(
                        "self-check: picked character 0x{:016x} at {:.1},{:.1},{:.1}, {:.1} m \
                         away -- routing to it. The whole roster, nearest first: {}",
                        npc.ctrl,
                        npc.position[0],
                        npc.position[1],
                        npc.position[2],
                        npc.distance,
                        // NAME THEM ALL, because the count alone cannot say why the arrow points
                        // somewhere the player can see nothing. `characters=6` with one visible
                        // NPC is five objects that are not what anybody means by a character.
                        census::describe_characters(8)
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

        // ONE COLOUR PER PERSON, AND THE STONES WEAR IT TOO.
        //
        // The trail used to lay `marker_effect_id` for everybody, so two targets got two arrows in
        // two colours and two trails in the same one. On screen that undoes the thing the palette
        // exists for: a colour is supposed to identify a person, and a trail that ignores it makes
        // the player work out which line belongs to which stone.
        //
        // So the stone is chosen from the same slot the route line is drawn from. A slot is bound
        // to a `PlayerCtrl` until they leave (see `routes::Palette`), so a target's colour does not
        // change as people move around them, and the seven Prism Stone ids wrap -- the eighth
        // person in one session repeats the first, which is the same compromise the line colours
        // already make.
        //
        // AN ID THAT IS NOT A PRISM STONE IS LEFT ALONE. `marker_effect_id` is documented as
        // "anything else is whatever effect carries that number", and somebody who put a specific
        // number in the file asked for THAT effect rather than for a palette. Only a value that is
        // already one of the seven opts into the sweep.
        //
        // With several trails on screen this stopped being a nicety. One shared colour for three
        // routes is one trail with three branches to anybody looking at it, and it is exactly the
        // picture a player would misread as "the path forks here".
        let stone_for = |state: &mut State, ctrl: usize| {
            if ds2_rva::PRISM_STONE_SFX_IDS.contains(&state.config.marker_effect_id) {
                let slot = state.palette.slot_for(ctrl as u64);
                ds2_rva::PRISM_STONE_SFX_IDS[slot % ds2_rva::PRISM_STONE_SFX_IDS.len()]
            } else {
                state.config.marker_effect_id
            }
        };

        // EVERY TARGET WORTH A ROUTE, PUBLISHED TOGETHER. The tick opens a lane per entry and
        // closes the ones that stop appearing, so this list is the whole statement of who is
        // being routed to -- there is nothing to withdraw separately.
        let mut wanted: Vec<crate::gametick::Wanted> = Vec::with_capacity(far_enough.len() + 1);
        for player in &far_enough {
            let effect_id = stone_for(state, player.ctrl);
            wanted.push(crate::gametick::Wanted {
                from: local,
                to: player.position,
                target: player.ctrl as u64,
                effect_id,
            });
        }
        // The self-check's character, when it nominated one and it is not already in the list.
        if let Some(npc) = routed.filter(|npc| !wanted.iter().any(|w| w.target == npc.ctrl as u64))
        {
            let effect_id = stone_for(state, npc.ctrl);
            wanted.push(crate::gametick::Wanted {
                from: local,
                to: npc.position,
                target: npc.ctrl as u64,
                effect_id,
            });
        }

        crate::gametick::set_markers(state.config.markers_requested().then_some(
            crate::gametick::Markers {
                spacing: crate::trail::Spacing {
                    meters: state.config.marker_spacing_meters,
                    keep_behind_meters: state.config.marker_keep_behind_meters,
                    max_markers: state.config.max_markers,
                    per_pass: state.config.markers_per_pass,
                },
            },
        ));
        // THE RE-PLAN RULE REACHES THE TICK WHETHER OR NOT THERE ARE STONES. It rides its own
        // call rather than `Markers`, because `Markers` is `None` when the trail is switched off
        // and the routes still have to be kept fresh for the lines.
        crate::gametick::set_cadence(crate::trail::Cadence {
            move_meters: state.config.replan_move_meters,
            min_seconds: state.config.replan_min_seconds,
            max_seconds: state.config.replan_max_seconds,
        });
        let marker_id = wanted.first().map_or(0, |first| first.effect_id);
        if marker_id != state.last_marker_id {
            state.last_marker_id = marker_id;
            log(format_args!(
                "markers: laying effect {marker_id} on the nearest of {} route(s) -- one colour \
                 per target, taken from the palette slot their line is drawn in",
                wanted.len()
            ));
        }

        crate::gametick::ask(wanted);

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
            // THE ARROW IS WHAT THE TRAIL SAYS WHEN IT CANNOT GET THERE, and nothing else.
            //
            // Three answers, and they used to collapse into two. `Some(Some(points))` is a
            // walkable route, so the stones are going down along it. `Some(None)` is the planner
            // having answered "there is no way to walk there" -- a complete answer, and the only
            // thing an arrow is entitled to mean. `None` is no answer yet, or a target past
            // `max_routes` that has no lane and will never be asked about.
            //
            // The last two both drew an arrow. That is why arrows were on screen beside a
            // perfectly good line: every target the planner had not been asked about got one, and
            // "I have not looked" was being drawn as "there is no way". A target nobody has
            // pathed to is not known to be unreachable, so it now draws nothing at all and the
            // arrow means what it says.
            //
            // A target past `max_routes` therefore draws NOTHING rather than an arrow, which is
            // the one place this rule costs something: the fifth player in a session is invisible
            // until a lane frees up. Drawing them an arrow would be drawing "there is no way to
            // walk there" about somebody nobody has looked for, which is the confident falsehood
            // this whole distinction exists to stop.
            match crate::gametick::answer_for(player.ctrl as u64) {
                Some(Some(points)) if points.len() >= 2 => {
                    snapshot.push(Route::new(
                        RouteShape::Walk(points),
                        slot,
                        player.distance,
                        state.config.bold_at_meters,
                        state.config.faint_at_meters,
                    ));
                }
                // THE DESTINATION, NOT A SHAPE. The arrow is built in pixels at draw time, so
                // there is nothing to construct here and nothing that can fail: a target directly
                // behind the camera, at the same position as the player, or fifty metres below
                // all produce an arrow, because the direction is read out of clip space rather
                // than projected.
                Some(None) => {
                    if state.unreachable.replace(player.ctrl) != Some(player.ctrl) {
                        log(format_args!(
                            "arrow: the planner found no way to walk to {:#x} -- no stones can \
                             reach them, so an arrow points at them instead",
                            player.ctrl
                        ));
                    }
                    snapshot.push(Route::new(
                        RouteShape::Arrow(player.position),
                        slot,
                        player.distance,
                        state.config.bold_at_meters,
                        state.config.faint_at_meters,
                    ));
                }
                // A route that decoded to fewer than two points is not a path either, and an
                // answer about a different target is not an answer about this one. Neither is
                // evidence that the stones cannot reach them, so neither draws.
                _ => {}
            }
        }

        // The last thing that actually needs a camera, and the first thing that stops without
        // one. Everything above has already run: the roster was read, the target nominated, the
        // ask published and the marker settings forwarded -- so the tick goes on planning the
        // route and laying the trail on the ground while the overlay has nothing to draw with.
        // The stones are in the world; only the arrow over them is missing.
        let Some(camera) = camera else {
            return Vec::new();
        };

        let mut vertices = Vec::with_capacity(snapshot.len() * 3 * VERTICES_PER_SEGMENT);
        for route in &snapshot {
            emit(&mut vertices, route, camera, screen, local);
        }

        // THE ONLY LINE THAT SAYS ANYTHING WAS DRAWN. `camera:` proves a camera was found and
        // `roster:` proves players were seen; neither proves a triangle reached the vertex
        // buffer, and the three steps in between -- suppression, the arrow's direction, the
        // near-plane clip -- can each legitimately eat everything. Logged on change so a run
        // with a steady overlay writes it once.
        // ROUTES, NOT ARROWS, AND THE TWO STOPPED BEING THE SAME NUMBER. `snapshot` counts
        // targets with something to say about them; only the ones the planner refused produce an
        // arrow, and a walkable route now produces no geometry at all because the stones carry
        // it. `1 route(s), 0 vertices` is the ordinary healthy line for a target being walked to,
        // and reading it as "one arrow was drawn" is how a working overlay looks broken.
        let arrows = snapshot
            .iter()
            .filter(|route| matches!(route.shape, RouteShape::Arrow(_)))
            .count();
        let drawn = (snapshot.len(), arrows, vertices.len());
        if state.last_drawn != Some(drawn) {
            state.last_drawn = Some(drawn);
            log(format_args!(
                "drew {} route(s), {} of them an arrow, {} vertices -- a walkable route draws \
                 nothing here; its path is the stones",
                drawn.0, drawn.1, drawn.2
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

    /// Frames the roster counts must hold still before the target may be latched.
    ///
    /// A load fills the roster in stages -- measured live as `characters=1`, then 3, 5, 6 over
    /// the first seconds -- and the target is held until it dies, so a pick made during that
    /// window is wrong for the whole session. Thirty frames is half a second at 60 Hz: long
    /// enough that a staged load has finished arriving, short enough that nobody notices the
    /// arrow appearing late.
    const ROSTER_SETTLE_FRAMES: u32 = 30;

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

    /// Project one route and append its triangles.
    fn emit(
        out: &mut Vec<Vertex>,
        route: &Route,
        camera: &Camera,
        screen: [f32; 2],
        local: [f32; 3],
    ) {
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
                    // THE TWO NUMBERS THAT SEPARATE A BAD CAMERA FROM BAD ARITHMETIC, and
                    // without them "the arrow points in random directions" is unfalsifiable.
                    //
                    // `head` is where the PLAYER'S OWN head projects through the same matrix the
                    // arrow's direction came from. A third-person camera puts it within a few
                    // pixels of the middle of the frame; anything else means the matrix is not
                    // the one this frame was rendered with, and then the direction is garbage
                    // for a reason that has nothing to do with the arrow.
                    //
                    // `target px` is where the thing being pointed at projects. If that lands on
                    // the character you can see and the arrow points somewhere else, the fault
                    // is here. If it lands nowhere near them, the fault is the camera. One line
                    // now answers which, instead of a screenshot and another launch.
                    let at = |world: [f32; 3]| {
                        camera.project(world, screen).map_or_else(
                            || "behind".to_string(),
                            |p| format!("{:.0},{:.0}", p[0], p[1]),
                        )
                    };
                    // THE CHECK THE CODE SHOULD HAVE BEEN DOING, and its absence is why a
                    // mirrored needle survived a whole session of launches: the log carried
                    // `tip` and `target px` side by side for hours and nothing ever compared
                    // them. A human had to notice, from a screenshot, that the NPC was on the
                    // other side.
                    //
                    // Now the mod notices. When the target projects far enough from the middle
                    // of the frame to HAVE a side, the needle must lean the same way. It cannot
                    // fail while the direction is taken from clip space -- which is the point:
                    // if this ever prints, the thing that was assumed to be impossible happened,
                    // and that is worth a loud line rather than a silent wrong arrow.
                    let verdict = camera
                        .project(geometry::head(*target), screen)
                        .map_or("", |p| {
                            let side = p[0] - screen[0] * 0.5;
                            let needle = arrow.tip[0] - arrow.base[0];
                            if side.abs() > 40.0 && side.signum() != needle.signum() {
                                " *** NEEDLE DISAGREES WITH THE PROJECTION ***"
                            } else {
                                ""
                            }
                        });
                    log(format_args!(
                        "arrow: base {:.0},{:.0} -> tip {:.0},{:.0} ({:.0} deg) | head px {} | \
                         target px {}{verdict} | target {:.1},{:.1},{:.1} | screen={:.0}x{:.0}",
                        arrow.base[0],
                        arrow.base[1],
                        arrow.tip[0],
                        arrow.tip[1],
                        turn.to_degrees(),
                        at(geometry::head(local)),
                        at(geometry::head(*target)),
                        target[0],
                        target[1],
                        target[2],
                        screen[0],
                        screen[1]
                    ));
                }
                push_segment(out, arrow.base, arrow.tip, route.stroke_px, color);
                push_segment(out, arrow.tip, arrow.left_barb, route.stroke_px, color);
                push_segment(out, arrow.tip, arrow.right_barb, route.stroke_px, color);
            }
            RouteShape::Walk(_) => {
                // THE STONES ARE THE PATH. NOTHING IS DRAWN OVER THE GROUND HERE.
                //
                // This projected the route as a polyline and pushed a segment per pair of
                // points, which is where the green line on the ground came from. The line was
                // how the route was proven while the trail could not be seen; it is not the
                // product. DARK SOULS II already has a way to mark a path -- the Prism Stone,
                // which ELDEN RING renamed to Rainbow Stone -- and `crate::trail` lays one every
                // few metres along exactly these points, in the colour bound to this target.
                //
                // So a walkable route draws no overlay at all. It still has to exist as a shape,
                // because what the draw path needs to know about it is that it EXISTS: a target
                // the stones can reach is a target that does not get an arrow. See the shape
                // decision in `draw_for`.
                //
                // Nothing to project, and nothing that can be mis-projected -- which also means
                // a route drawn against a wrong camera can no longer put a line across the sky.
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
