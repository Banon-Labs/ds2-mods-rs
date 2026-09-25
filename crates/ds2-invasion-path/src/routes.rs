//! What the game thread hands to the draw thread, and the colour bookkeeping behind it.
//!
//! Pure data and pure logic, so the part that decides *which player is which colour* is proven by
//! `cargo test` rather than by squinting at two similar oranges in a screenshot.

// Windows-only in practice; ungated so the assignment logic below stays host-testable.
// DEBT: ds2-mods-rs-2rs -- module-wide dead_code, reason not yet recorded.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::geometry;

/// What to draw for one player.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RouteShape {
    /// A walkable route: world-space points along the ground, start first.
    ///
    /// Produced by `crate::gametick`, which asks `NvRoutePlanner` for one on the game's own tick
    /// and decodes the answer with `crate::navpath`. The two ends are world positions snapped to
    /// navigation-graph ids by `crate::navquery::snap` -- the step this variant used to be
    /// blocked on, and which turned out to be an ordinary synchronous call rather than the
    /// asynchronous job bd `ds2-mods-rs-4yd` was filed on.
    ///
    /// Every target past `near_suppress_meters` gets one, up to `max_routes` of them, each from
    /// its own planner. See the comment at the `far_enough` binding in
    /// `crate::windows_impl::draw_for`, and the lane header in `crate::gametick`.
    Walk(Vec<[f32; 3]>),
    /// The planner has answered that there is no way to walk to this target, so an arrow leaves
    /// the player's body pointing at them. Same fallback the Elden Ring crate uses when its
    /// navmesh says the same thing.
    ///
    /// **Only on a proven refusal**, which is a narrowing, not a detail. This was also drawn for
    /// any target the planner had not been asked about -- and back when one `NvRoutePlanner`
    /// served one target at a time, that was most of them. Arrows appeared beside a perfectly
    /// good line because "I have not looked" was being drawn as "there is no way". A target
    /// nobody has pathed to draws nothing now; see the shape decision in
    /// `crate::windows_impl::draw_for`.
    ///
    /// **The target's world position, not a shape.** The arrow is built in pixels at draw time
    /// by [`crate::geometry::Camera::screen_arrow`], because a world-space arrow pointing near
    /// the view axis foreshortens to a stub -- and that is the usual case, not a rare one.
    Arrow([f32; 3]),
}

/// One player's overlay, ready to project.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Route {
    pub(crate) shape: RouteShape,
    pub(crate) color: [f32; 3],
    pub(crate) alpha: f32,
    pub(crate) stroke_px: f32,
    /// How far the target actually is, in metres.
    ///
    /// Carried past the colour ramp because the draw path needs it as a **length budget**: an
    /// arrow may grow to stay legible, and the one thing it must never do is grow past the
    /// person it is pointing at. See `crate::windows_impl::emit`.
    pub(crate) distance_meters: f32,
}

impl Route {
    /// Build a route with its weight derived from how far away the target is.
    pub(crate) fn new(
        shape: RouteShape,
        color_slot: usize,
        distance_meters: f32,
        bold_at: f32,
        faint_at: f32,
    ) -> Self {
        let boldness = geometry::boldness(distance_meters, bold_at, faint_at);
        Self {
            shape,
            color: geometry::path_color(color_slot),
            alpha: geometry::alpha(boldness),
            stroke_px: geometry::stroke_px(boldness),
            distance_meters,
        }
    }
}

/// Hands each player a colour slot and keeps it for as long as they are in the session.
///
/// # Why this is not just an enumeration index
///
/// Colouring by position in the roster means the roster's order decides the colour, and the
/// roster is sorted by distance -- so two players swapping places as they run would swap colours
/// mid-fight. The whole point of "N players, N colours" is that a colour identifies a person, so
/// a slot is bound to an identity and released only when that player is gone.
///
/// # What identifies a player here
///
/// The address of their `PlayerCtrl`. Elden Ring has a `FieldInsHandle` -- a stable id the engine
/// assigns -- and this is not that: an address is only as stable as the allocation, and a player
/// who leaves and rejoins may land on the freed memory of someone else. The consequence is
/// bounded and visible: a colour is reused for a different person. It is not a safety problem,
/// because the pointer is re-read from the roster every pass and never held across one.
///
/// A real id exists -- both player factories format a name from a `%06u` -- and swapping to it is
/// a small change here and an offset in `ds2-rva`. It is not done on a guess about where that
/// number lands in the object.
#[derive(Debug, Default)]
pub(crate) struct Palette {
    /// `slots[i]` is the player currently holding colour `i`, or `None` when free.
    slots: Vec<Option<u64>>,
}

impl Palette {
    /// The colour slot for `player`, allocating the lowest free one on first sight.
    pub(crate) fn slot_for(&mut self, player: u64) -> usize {
        if let Some(index) = self.slots.iter().position(|held| *held == Some(player)) {
            return index;
        }
        if let Some(index) = self.slots.iter().position(Option::is_none) {
            self.slots[index] = Some(player);
            return index;
        }
        self.slots.push(Some(player));
        self.slots.len() - 1
    }

    /// Release every slot whose player is no longer in `present`.
    ///
    /// Called once per roster read. Without it a session that churned through phantoms would keep
    /// allocating new hues until the colours started repeating.
    pub(crate) fn retain(&mut self, present: &[u64]) {
        for slot in &mut self.slots {
            if let Some(player) = *slot
                && !present.contains(&player)
            {
                *slot = None;
            }
        }
        // Trailing free slots are dropped so the next player gets a low, well-separated hue
        // rather than the next index past a long-dead one.
        while self.slots.last().is_some_and(Option::is_none) {
            self.slots.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrow() -> RouteShape {
        RouteShape::Arrow([0.0, 0.0, 1.0])
    }

    #[test]
    fn a_player_keeps_their_colour_across_frames() {
        let mut palette = Palette::default();
        let first = palette.slot_for(0xaaaa);
        let second = palette.slot_for(0xbbbb);
        assert_ne!(first, second);
        // Re-reading the roster in the other order must not swap them.
        assert_eq!(palette.slot_for(0xbbbb), second);
        assert_eq!(palette.slot_for(0xaaaa), first);
    }

    #[test]
    fn distance_order_does_not_decide_colour() {
        let mut palette = Palette::default();
        let host = palette.slot_for(0x1111);
        let phantom = palette.slot_for(0x2222);
        // The host runs past the phantom; the roster order flips, the colours must not.
        palette.retain(&[0x2222, 0x1111]);
        assert_eq!(palette.slot_for(0x2222), phantom);
        assert_eq!(palette.slot_for(0x1111), host);
    }

    #[test]
    fn a_departed_player_frees_their_colour_for_the_next_arrival() {
        let mut palette = Palette::default();
        let first = palette.slot_for(0xaaaa);
        let second = palette.slot_for(0xbbbb);
        palette.retain(&[0xbbbb]);
        // The newcomer takes the freed low slot rather than a third hue.
        assert_eq!(palette.slot_for(0xcccc), first);
        assert_eq!(palette.slot_for(0xbbbb), second);
    }

    #[test]
    fn an_emptied_session_does_not_grow_the_palette_forever() {
        let mut palette = Palette::default();
        for player in 0..32u64 {
            palette.slot_for(player);
        }
        palette.retain(&[]);
        assert_eq!(palette.slot_for(0xffff), 0, "slots leaked across a session");
    }

    #[test]
    fn a_nearer_route_is_bolder_than_a_distant_one() {
        let near = Route::new(arrow(), 0, 5.0, 20.0, 150.0);
        let far = Route::new(arrow(), 0, 140.0, 20.0, 150.0);
        assert!(near.stroke_px > far.stroke_px);
        assert!(near.alpha > far.alpha);
        assert_eq!(near.color, far.color, "weight must not change the hue");
    }

    #[test]
    fn two_players_do_not_share_a_hue() {
        let mut palette = Palette::default();
        let mut seen: Vec<[f32; 3]> = Vec::new();
        for player in 0..6u64 {
            let colour = geometry::path_color(palette.slot_for(player));
            for previous in &seen {
                let difference = (colour[0] - previous[0]).abs()
                    + (colour[1] - previous[1]).abs()
                    + (colour[2] - previous[2]).abs();
                assert!(difference > 0.25, "{colour:?} is too close to {previous:?}");
            }
            seen.push(colour);
        }
    }
}
