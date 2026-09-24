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
//! falls back to an arrow when the navmesh says there is no way to walk there. This draws the
//! arrow, always, and that is the honest state of it rather than a design choice.
//!
//! `docs/PORTING.md` filed the Elden Ring crate under "no DS2 analogue (Havok-AI navmesh)". Half
//! of that is right -- there is no Havok AI in this engine -- and the conclusion is wrong. DARK
//! SOULS II ships its own navigation stack, RTTI-named throughout, with the same request/poll
//! shape. `crate::navpath` implements and tests the half of it that could be derived: reading a
//! finished route. What it cannot do yet is *ask* for one, because the request takes
//! navigation-graph ids rather than world positions and the conversion is an asynchronous engine
//! job driven from state this crate does not have. The full account is in that module.
//!
//! So the route is not here. Everything else is, and everything else is what makes the route
//! worth having: the roster, the positions, the colours, the ramp, and a place to draw.
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
//! remembers the PLACE the matrix was found and re-reads it every frame. And `draw_for` applies
//! the acceptance test EVERY FRAME to whichever camera it was handed, so a camera that cannot
//! currently put the player in frame draws nothing instead of drawing somewhere wrong.
//!
//! **Still unmeasured: whether the overlay is USEFUL.** Every run so far has been offline and
//! solo, so the `remotes=1` in them is an NPC phantom or a bloodstain replay -- DARK SOULS II
//! builds both from the same class (see `crate::census`) -- rather than an invader.
//!
//! # What it does to the game
//!
//! **Nothing except drawing.** No param edits, no writes to game memory, no input injection, no
//! network traffic. It reads a roster, reads two matrices, and appends triangles to a frame that
//! was already finished. The one detour it installs is on `IDXGISwapChain::Present`, which lives
//! in `dxgi.dll` and is therefore outside everything this workspace has learned about Arxan.

#![cfg_attr(not(windows), allow(dead_code))]

pub mod config;
pub mod geometry;
pub mod log;
pub mod navpath;
pub mod routes;

pub(crate) mod lines;

#[cfg(windows)]
pub(crate) mod camera;
#[cfg(windows)]
pub(crate) mod capture;
#[cfg(windows)]
pub(crate) mod census;
#[cfg(windows)]
pub(crate) mod render;

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
        /// Whether the camera framed the character last frame. `true` for the same reason as
        /// [`State::had_camera`]: the first refusal has to be audible.
        framed: bool,
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
                framed: true,
            });
        }

        // SAFETY: the caller's contract, forwarded -- one detour, from the install position.
        let installed = unsafe { render::install() };
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
        // THE GUARD BOTH WAYS OF GETTING A CAMERA NEEDED, and the reason two screenshots in a row
        // showed an orange line hanging in the sky with nothing under it.
        //
        // Each source vets its camera once and then goes on using it: `crate::camera` remembers
        // the offsets that passed its oracles, `crate::capture` remembers the buffer that held a
        // matrix that passed its own. Both were *right when accepted* and neither was re-checked
        // afterwards, so the first turn of the player's head left the overlay aiming with a
        // camera the game had stopped using -- which is a line drawn in the wrong place, not a
        // line that is missing, and therefore the failure that reaches the screen.
        //
        // So the acceptance test is applied EVERY FRAME, at the one place both paths meet. A
        // camera that cannot currently put the player in frame is not the camera being rendered,
        // whatever it was when it was found, and the honest response is to draw nothing.
        if !camera.frames_the_character(local, screen) {
            if state.framed {
                state.framed = false;
                let at = camera.project(local, screen).map_or_else(
                    || "behind the lens".to_string(),
                    |p| format!("{:.0},{:.0}", p[0], p[1]),
                );
                log(format_args!(
                    "camera no longer frames the character (player at {at} on {:.0}x{:.0}) -- \
                     drawing nothing rather than drawing it somewhere wrong",
                    screen[0], screen[1]
                ));
            }
            return Vec::new();
        }
        state.framed = true;

        let Some((players, census)) = census::remotes(state.config.max_targets) else {
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

        let mut snapshot = Vec::new();
        for player in &players {
            if player.distance < state.config.near_suppress_meters {
                continue;
            }
            let slot = state.palette.slot_for(player.ctrl as u64);
            let Some(arrow) = geometry::arrow(
                local,
                player.position,
                state.config.arrow_meters,
                camera.up(),
            ) else {
                continue;
            };
            snapshot.push(Route::new(
                RouteShape::Arrow(arrow),
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

    /// The shortest an arrow's shaft may appear, in pixels.
    ///
    /// **A world-space arrow pointing away from the camera foreshortens to a dot**, and that is
    /// not a corner case -- it is what happens whenever you are running towards the person you
    /// are pointing at, which is most of the time. The first live look at this feature was the
    /// user asking whether the orange thing on screen was an arrow at all; it was, three metres
    /// long, aimed almost straight down the view axis, and it projected to a handful of pixels.
    ///
    /// Eighty is a legible glyph at 1080p without being a thing you have to look past.
    const MIN_ARROW_PX: f32 = 80.0;

    /// The last hundred-pixel band the arrow's tail landed in, and the frames left before another
    /// sample may be written.
    ///
    /// SAMPLED ON MOVEMENT, NOT ON A COUNT. A fixed number of samples runs out while the game
    /// sits on a loaded save with nobody at the controls, which is exactly when the arrow cannot
    /// move and the log therefore proves nothing: a latched camera and a live one write identical
    /// lines until something turns. Keying the sample to a CHANGE in where the tail lands makes a
    /// still camera silent and a turning one talkative, which is the right way round -- and it is
    /// motion that separates the two failures, because only a latched camera lets the tail walk
    /// away from the character.
    static ARROW_BAND: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(usize::MAX);
    static ARROW_COUNTDOWN: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(0);

    /// Frames between arrow samples, so a swinging camera writes a readable trail rather than one
    /// line per frame.
    const ARROW_SAMPLE_FRAMES: usize = 15;

    /// How wide a band of tail-from-centre counts as the same place, in pixels.
    const ARROW_BAND_PX: f32 = 100.0;

    /// Project one route and append its triangles.
    fn emit(out: &mut Vec<Vertex>, route: &Route, camera: &Camera, screen: [f32; 2]) {
        let color = [route.color[0], route.color[1], route.color[2], route.alpha];
        let mut segment = |from: [f32; 3], to: [f32; 3]| {
            if let Some((a, b)) = camera.project_segment(from, to, screen) {
                push_segment(out, a, b, route.stroke_px, color);
            }
        };
        match &route.shape {
            RouteShape::Arrow(arrow) => {
                let arrow = &lengthen(arrow, route.distance_meters, camera, screen);
                // A FEW ARROWS, SPACED. The first screenshot of this feature showed an orange
                // stripe floating in the air with no head and no connection to the player, which
                // is at least two different bugs wearing one appearance -- a tail that is not
                // the body, or barbs collapsing onto the shaft. Theorising about which costs a
                // game launch per theory; four projected points settle it. The second screenshot
                // showed the same stripe again, and the one shot this used to take could not
                // distinguish the remaining cause -- a camera read once and never again -- from
                // a correct one, because both are right in the frame they are sampled. Hence
                // several samples, seconds apart: `tail-from-centre` staying small across them
                // is the overlay tracking, and growing is it drifting.
                // The number that answers "is it on the character": how far the tail lands from
                // the middle of the frame, where a third-person camera keeps them.
                let drift = camera.project(arrow.tail, screen).map(|p| {
                    ((p[0] - screen[0] * 0.5).powi(2) + (p[1] - screen[1] * 0.5).powi(2)).sqrt()
                });
                let ready = ARROW_COUNTDOWN
                    .fetch_update(
                        core::sync::atomic::Ordering::Relaxed,
                        core::sync::atomic::Ordering::Relaxed,
                        |left| Some(left.saturating_sub(1)),
                    )
                    .is_ok_and(|left| left == 0);
                // `usize::MAX` is the never-sampled state and has to differ from a real band, so
                // a tail that is off screen gets its own value rather than sharing that one.
                let band = drift.map_or(usize::MAX - 1, |drift| (drift / ARROW_BAND_PX) as usize);
                if ready && ARROW_BAND.swap(band, core::sync::atomic::Ordering::Relaxed) != band {
                    ARROW_COUNTDOWN
                        .store(ARROW_SAMPLE_FRAMES, core::sync::atomic::Ordering::Relaxed);
                    let px = |world| {
                        camera.project(world, screen).map_or_else(
                            || "off".to_string(),
                            |p| format!("{:.0},{:.0}", p[0], p[1]),
                        )
                    };
                    let drift =
                        drift.map_or_else(|| "off".to_string(), |drift| format!("{drift:.0}px"));
                    log(format_args!(
                        "arrow: tail-from-centre {drift} | tail {:.1},{:.1},{:.1} -> tip {:.1},{:.1},{:.1} | screen tail={} tip={} barbs={} {} | screen={:.0}x{:.0}",
                        arrow.tail[0],
                        arrow.tail[1],
                        arrow.tail[2],
                        arrow.tip[0],
                        arrow.tip[1],
                        arrow.tip[2],
                        px(arrow.tail),
                        px(arrow.tip),
                        px(arrow.left_barb),
                        px(arrow.right_barb),
                        screen[0],
                        screen[1]
                    ));
                }
                segment(arrow.tail, arrow.tip);
                // NO BARBS ON A TIP THAT IS BEHIND THE LENS. `project_segment` trims a crossing
                // segment to the camera plane, where dividing by a near-zero `w` throws the
                // trimmed end thousands of pixels out -- the live log has barbs at `6138,-525`
                // on a 2132x1200 screen. For the shaft that is harmless and correct: the line
                // leaves the character and runs off the edge towards a player who is behind you,
                // which is the information wanted. For the head it is a huge V flung across the
                // frame from a vertex that is not on screen at all, which is noise drawn over
                // the game. An arrowhead is only meaningful where its point is.
                if camera.project(arrow.tip, screen).is_some() {
                    segment(arrow.tip, arrow.left_barb);
                    segment(arrow.tip, arrow.right_barb);
                }
            }
            RouteShape::Walk(points) => {
                for pair in points.windows(2) {
                    segment(pair[0], pair[1]);
                }
            }
        }
    }

    /// Grow an arrow until its shaft is at least [`MIN_ARROW_PX`] long on screen, or until it
    /// reaches the person it points at -- whichever comes first.
    ///
    /// # Why the length is a minimum rather than a size
    ///
    /// The configured `arrow_meters` is a WORLD length, and a world length says nothing about how
    /// big the thing looks. An arrow pointing across your view is three metres of clearly visible
    /// line; the same arrow pointing away from you is three metres of nearly nothing, because
    /// almost all of it is depth. That is the case that matters most -- you are usually running
    /// towards the player you are pointing at.
    ///
    /// **Bounded by the real distance**, which is the part that keeps this honest. An arrow is a
    /// claim about direction, and one that extended past its target would be a claim about
    /// distance as well, and a false one. A player far away and dead ahead gets a long line that
    /// stops short of them; a player beside you keeps the short arrow that was already legible.
    ///
    /// Returns the arrow unchanged when it cannot be projected at all, which is the case when
    /// both ends are behind the lens -- there is nothing to make legible.
    fn lengthen(
        arrow: &geometry::Arrow,
        distance_meters: f32,
        camera: &Camera,
        screen: [f32; 2],
    ) -> geometry::Arrow {
        let Some(direction) = geometry::normalize(geometry::sub(arrow.tip, arrow.tail)) else {
            return *arrow;
        };
        let current = geometry::length(geometry::sub(arrow.tip, arrow.tail));
        let Some((tail_px, tip_px)) = camera.project_segment(arrow.tail, arrow.tip, screen) else {
            return *arrow;
        };
        let shaft_px = geometry::length([tip_px[0] - tail_px[0], tip_px[1] - tail_px[1], 0.0]);
        // Already legible, or so foreshortened that the scale factor would be meaningless -- a
        // shaft of a fraction of a pixel is a target almost exactly along the view axis, and
        // dividing by it produces a number rather than an answer.
        const DEGENERATE_PX: f32 = 0.5;
        if shaft_px >= MIN_ARROW_PX || shaft_px < DEGENERATE_PX || current <= f32::EPSILON {
            return *arrow;
        }
        // BISECT, DO NOT SCALE. The obvious `length * wanted_px / shaft_px` is wrong twice over:
        // foreshortening is not linear in length, and a shaft that projected to about a pixel
        // makes that ratio enormous. The first version did exactly that, clamped the result to
        // the target's distance, and so grew every arrow to the full fifty metres -- which put
        // the arrowhead off the top of the screen and left a bare orange stripe. The screenshot
        // that found it is the reason this is a search.
        //
        // Six halvings over [current, distance] land within a couple of percent, which is far
        // finer than the eye needs, and every step re-projects rather than extrapolating.
        let projected_px = |length: f32| {
            let tip = geometry::add_scaled(arrow.tail, direction, length);
            camera
                .project_segment(arrow.tail, tip, screen)
                .map_or(0.0, |(a, b)| {
                    geometry::length([b[0] - a[0], b[1] - a[1], 0.0])
                })
        };
        let (mut low, mut high) = (current, distance_meters.max(current));
        // The far end is not long enough either: nothing more can be done, and stopping short of
        // the target is still better than drawing past them.
        if projected_px(high) > MIN_ARROW_PX {
            for _ in 0..6 {
                let middle = 0.5 * (low + high);
                if projected_px(middle) < MIN_ARROW_PX {
                    low = middle;
                } else {
                    high = middle;
                }
            }
        }
        let length = high;
        let up = camera.up();
        let target = geometry::add_scaled(arrow.tail, direction, length);
        geometry::arrow(arrow.tail, target, length, up).unwrap_or(*arrow)
    }

    /// The back buffer's size in pixels, which is the coordinate space the overlay draws in.
    fn back_buffer_size(swap_chain: &IDXGISwapChain) -> Option<[f32; 2]> {
        // SAFETY: `swap_chain` is the game's own, borrowed by the detour.
        let description = unsafe { swap_chain.GetDesc() }.ok()?;
        let width = description.BufferDesc.Width as f32;
        let height = description.BufferDesc.Height as f32;
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
