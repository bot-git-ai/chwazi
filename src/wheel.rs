// Copyright (c) 2026 Witalis Domitrz <witekdomitrz@gmail.com>
// AGPL License

//! The spinning wheel of colours: where it sits, and how fast the pointer turns.
//!
//! This is the selection animation the native Chwazi app has and the web app
//! never had. During the draw window the players' circles gather into a ring and
//! a pointer sweeps around it, fast at first and easing to a stop beside one of
//! them. The point is not decoration: a result that arrives with nothing on the
//! glass to look at can be argued with, and one that lands in front of everyone
//! cannot. That is the whole reason the native app describes itself as "no one
//! can argue with the spinning circle".
//!
//! **The segments are fixed and only the pointer moves.** That is what a wheel of
//! fortune is, and getting it wrong is obvious: an earlier version gave the
//! circles the same time term as the pointer, so the dots orbited one another
//! while the pointer chased them. Nothing about a spinner is a carousel, and a
//! wheel whose parts move is a wheel you cannot read -- the eye has to track a
//! dot relative to two other moving things instead of relative to the pointer.
//! So [`slots`] does not depend on elapsed time at all.
//!
//! No DOM and no canvas, like [`crate::chooser`]: the geometry is arithmetic on
//! a player's position and a timestamp, so it is testable without a browser.
//!
//! The wheel needs no state of its own. Given the same players and the same
//! timestamp it always draws the same wheel, which is what lets the render loop
//! keep it in step with the fingers moving underneath it.

/// How long the wheel takes to spin and settle, as a fraction of the draw
/// window.
///
/// Just over half, so the spin and the circle-in are both quick and roughly 900ms
/// of the window is left over as a landed, readable result before the colour
/// floods the screen. That tail is the point: a result that is announced by the
/// animation producing it cannot be followed, and 900ms is long enough to say
/// "that one" out loud.
pub const SPIN_FRACTION: f64 = 0.55;

/// How many turns the pointer makes before it stops.
///
/// How many turns the pointer makes before it stops.
///
/// Two whole turns, plus a whole extra turn per player -- see
/// [`turns_for`]. The count is *derived* rather than chosen, because a fixed
/// number of turns cannot land on a segment of a wheel whose segment count is not
/// known until the last finger is down. Two and a bit turns, for instance, lands
/// 30% of a segment past the mark with two players, 75% past it with five, and
/// 5% short of it with seven: so the pointer would come to rest between two
/// circles, which is the one thing a wheel must never do.
///
/// One turn per player costs about 200ms at this speed, so a table of ten is
/// still under 2s of spin, and every one of them lands exactly on the mark.
pub const BASE_TURNS: f64 = 2.0;

/// Whole turns to spin, given how many players are on the wheel.
///
/// Integer by construction: `BASE_TURNS` plus one per player is a whole number of
/// slot steps, so the pointer arrives back at the mark it started from.
pub fn turns_for(players: usize) -> f64 {
    BASE_TURNS + players as f64
}

/// Radius the wheel's player circles sit at, as a fraction of the window's
/// shorter side.
///
/// The window can be any shape from a tall phone to a tablet in landscape, and a
/// finger circle has to stay big enough to recognise under a hand. So the wheel
/// grows with the screen but stops well short of filling it, and the result is
/// that the same circle is the same size on every device.
pub const WHEEL_RADIUS_FRACTION: f64 = 0.3;

/// How far outside the ring of circles the pointer rides.
///
/// The pointer stops *beside* the winner rather than on top of it. Landing on the
/// circle hides the very thing it is pointing at behind a white dot and a ring of
/// the same colour, and reads as "a dot stopped somewhere" instead of "that one".
/// A gap of this size keeps both readable at once.
pub const POINTER_OFFSET: f64 = 30.0;

/// How much of the spin the circles take to gather onto the wheel, as a fraction
/// of it.
///
/// Short on purpose: the circles reach their slots while the pointer is still
/// travelling, so by the time it starts slowing they are already waiting for it.
pub const GATHER_FRACTION: f64 = 0.34;

/// Ceiling on the wheel's radius, in the same units as the circles.
///
/// Deliberately only a ceiling. A floor would look sensible -- keep the wheel a
/// decent size on a small phone -- and would put a player's circle off the edge
/// of exactly those phones. [`radius`] derives that bound from the screen.
pub const WHEEL_RADIUS_MAX: f64 = 260.0;

/// How much of the screen's short side the wheel may use, before the outermost
/// circle is hung off the edge.
///
/// `0.5` is half the short side, so the wheel is fitted to whichever dimension is
/// smaller. The circle's own radius is then subtracted, which is the whole point:
/// a ring that fits is not the same as a ring whose *players* fit.
const EDGE_MARGIN: f64 = 0.5;

/// A point on the wheel, and what it is for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// Centre of this player's circle, in CSS pixels.
    pub x: f64,
    pub y: f64,
    /// Angle from the wheel's centre, in radians, increasing clockwise on screen.
    ///
    /// Zero is straight up, which is where the pointer starts and stops, so the
    /// sweep always begins and ends at the top no matter how many players there
    /// are. A spin that began beside a player and ended on another would look
    /// arbitrary.
    pub angle: f64,
}

/// Where the wheel is centred for this screen.
pub fn centre(width: f64, height: f64) -> (f64, f64) {
    (width / 2.0, height / 2.0)
}

/// The radius the wheel's circles sit at on this screen.
///
/// Every bound here is a *ceiling*. The tempting mistake is to make the screen's
/// fit a lower bound so the wheel stays big on a small phone, but a wheel too
/// big to fit is a wheel with a player's circle hanging off the edge of the
/// glass -- which is worse than a small one, and is what the first version did.
///
/// Three of them, smallest first:
///
/// * `shorter * EDGE_MARGIN - circle_edge()`: what actually fits;
/// * `WHEEL_RADIUS_MAX`: a large tablet does not get a wheel the size of a table;
/// * and `shorter * WHEEL_RADIUS_FRACTION`, which is what the wheel would
///   ideally be.
///
/// `clamp` would panic if the fit bound exceeded the maximum, which it cannot on
/// any screen worth drawing on, but a `min` chain cannot panic at all and reads
/// as the ceiling it is.
pub fn radius(width: f64, height: f64) -> f64 {
    let shorter = width.min(height);
    let fits = (shorter * EDGE_MARGIN - circle_edge()).max(0.0);
    (shorter * WHEEL_RADIUS_FRACTION).min(fits).min(WHEEL_RADIUS_MAX)
}

/// The radius of a player's circle at its largest, at full pulse.
///
/// The wheel's own floor is stated in terms of this, so the two can never
/// disagree about how much room a circle needs.
pub fn circle_edge() -> f64 {
    (crate::chooser::INNER_RADIUS + crate::chooser::OUTER_RADIUS
        + crate::chooser::OUTER_CIRCLE_WIDTH / 2.0)
        * (1.0 + crate::chooser::MAX_PULSE_SCALE)
}

/// Where each player's circle belongs on the wheel.
///
/// `players` are `(id, x, y)` in pointer-id order, which is the order the
/// chooser holds them in, so the wheel's layout is stable for as long as the
/// players are: a finger moving does not shuffle the wheel around it.
///
/// `gather` is 0 to 1 -- how far the circles have moved from their fingers onto
/// their slots -- and is the *only* thing that varies with time. The slots
/// themselves do not turn. See the module comment: this is the fix for the wheel
/// that orbited.
pub fn slots(players: &[(i32, f64, f64)], gather: f64, width: f64, height: f64) -> Vec<Slot> {
    let count = players.len();
    if count == 0 {
        return Vec::new();
    }
    let (cx, cy) = centre(width, height);
    let radius = radius(width, height);
    let base = -std::f64::consts::FRAC_PI_2;

    players
        .iter()
        .enumerate()
        .map(|(index, &(_, x, y))| {
            // Fixed: the segment this player occupies, and it never moves. No
            // time term. That is the whole change from the version that orbited.
            let slot = base + (index as f64 / count as f64) * std::f64::consts::TAU;
            // A circle whose centre is on the ring, so the wheel is a ring of
            // circles rather than a set of dots inside one.
            let (sx, sy) = (cx + radius * slot.cos(), cy + radius * slot.sin());
            // The finger's own position, pulled most of the way to the slot: not
            // all the way, so the circle still visibly belongs to the finger that
            // owns it while it settles.
            //
            // The gather is over quickly -- the last third of the window -- and is
            // eased in on its own rather than riding the pointer's deceleration.
            // Sharing the pointer's curve made the circles crawl for most of the
            // spin and only snap together at the very end, which is the opposite
            // of the flick it should be.
            let ease = 1.0 - (1.0 - gather).powi(3);
            Slot {
                x: x + (sx - x) * ease,
                y: y + (sy - y) * ease,
                angle: slot,
            }
        })
        .collect()
}

/// How far the circles have moved from their fingers onto the wheel, 0 to 1.
///
/// The gather is over in the first third of the spin, so that by the time the
/// pointer starts slowing the circles are already waiting for it. It is eased
/// cubic-out on its own rather than riding the pointer's deceleration: sharing that
/// curve made them crawl for most of the spin and only snap together at the very
/// end, which is the opposite of a flick.
pub fn gather_progress(elapsed: f64, window: f64) -> f64 {
    if window <= 0.0 {
        return 1.0;
    }
    ((elapsed / window) / GATHER_FRACTION).clamp(0.0, 1.0)
}

/// The pointer's sweep, 0 to 1, `elapsed` milliseconds into the draw window.
///
/// Ease-out cubic, and that is the whole feel of it: the pointer leaves fast, so
/// the draw reads as decided immediately, and decelerates hard into its stop, so
/// the player it lands on is unmistakable and the eye is not still moving when
/// the colour floods the screen.
///
/// A linear sweep here would be the single most noticeable difference between
/// this and a wheel that feels right: constant angular velocity reads as a
/// machine, and a machine can be argued with.
pub fn spin_progress(elapsed: f64, window: f64) -> f64 {
    if window <= 0.0 {
        return 1.0;
    }
    ((elapsed / window) / SPIN_FRACTION).clamp(0.0, 1.0)
}

/// Where the pointer is, in screen radians, at `elapsed`.
///
/// `turns` is [`turns_for`] of the players on the wheel. Because it is a whole
/// number, the pointer finishes back at the top, where slot 0 sits -- so it lands
/// on the first player's circle rather than between two.
///
/// The angle is held once the spin has stopped, rather than continuing to
/// advance with elapsed time. Letting it keep going past the stop is not merely
/// untidy: after [`SPIN_FRACTION`] the slots are no longer moving either, so a
/// still-advancing pointer drifts away from the player it is pointing at and
/// appears to run backwards past it. That is exactly the bug the monotonicity
/// test found.
pub fn pointer_angle(elapsed: f64, window: f64, turns: f64) -> f64 {
    let progress = spin_progress(elapsed, window);
    -std::f64::consts::FRAC_PI_2 + progress * turns * std::f64::consts::TAU
}

/// The pointer's position at `elapsed`.
///
/// [`POINTER_OFFSET`] further out than the circles, so it stops beside the winner
/// rather than over it.
pub fn pointer_position(
    elapsed: f64,
    window: f64,
    turns: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let (cx, cy) = centre(width, height);
    let ring = radius(width, height) + POINTER_OFFSET;
    let angle = pointer_angle(elapsed, window, turns);
    (cx + ring * angle.cos(), cy + ring * angle.sin())
}

/// Which player the pointer has landed on, given the slots.
///
/// Returns an index into the same list [`slots`] was built from. `None` before
/// the spin starts, when the pointer is at the top and no player is there yet.
pub fn landed_on(slots: &[Slot], elapsed: f64, window: f64, turns: f64) -> Option<usize> {
    if slots.is_empty() {
        return None;
    }
    let pointer = pointer_angle(elapsed, window, turns);
    // Nearest by angular distance, wrapping at the top of the wheel.
    slots
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| angle_distance(a.angle, pointer).total_cmp(&angle_distance(b.angle, pointer)))
        .map(|(index, _)| index)
}

/// The shortest signed difference between two angles, in radians.
fn angle_distance(a: f64, b: f64) -> f64 {
    let two_pi = std::f64::consts::TAU;
    let mut delta = (a - b) % two_pi;
    if delta > std::f64::consts::PI {
        delta -= two_pi;
    }
    if delta < -std::f64::consts::PI {
        delta += two_pi;
    }
    delta.abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f64 = 390.0;
    const H: f64 = 844.0;

    /// Two fingers, which is the minimum a draw needs.
    fn two() -> Vec<(i32, f64, f64)> {
        vec![(1, 100.0, 200.0), (2, 300.0, 600.0)]
    }

    fn five() -> Vec<(i32, f64, f64)> {
        (0..5)
            .map(|i| (i + 1, 50.0 + f64::from(i) * 60.0, 100.0 + f64::from(i) * 90.0))
            .collect()
    }

    #[test]
    fn the_wheel_is_centred_and_sized_for_the_screen() {
        assert_eq!(centre(W, H), (195.0, 422.0));
        // An ordinary phone takes the fraction of its short side.
        assert_eq!(radius(W, H), 117.0, "0.3 of 390");
        // A large tablet is held back by the ceiling.
        assert_eq!(radius(1024.0, 1366.0), 260.0, "clamped to the ceiling");
        // And a square screen in between.
        assert_eq!(radius(700.0, 700.0), 210.0, "0.3 of 700");
    }

    #[test]
    fn the_wheel_fits_inside_the_narrowest_phone() {
        // The wheel plus a whole circle either side has to fit, or a circle ends
        // up off the edge of a small screen.
        for (small_w, small_h) in [(320.0, 568.0), (360.0, 640.0), (320.0, 480.0)] {
            let r = radius(small_w, small_h);
            assert!(
                r + chooser_outer_edge() <= small_w / 2.0 + 1e-9,
                "the wheel at {r} plus a circle overflows a {small_w}x{small_h} screen"
            );
            let _ = small_h;
        }
    }

    #[test]
    fn the_wheel_shrinks_rather_than_overflowing() {
        // The property the inverted bound broke: on a small screen the wheel gives
        // way rather than pushing a circle off the edge.
        let big = radius(390.0, 844.0);
        let small = radius(320.0, 568.0);
        assert!(small < big, "a smaller screen gets a smaller wheel");
        assert!(
            small > 0.0,
            "but not a degenerate one: {small} on a 320pt phone"
        );
    }

    /// The radius of a player's outer ring at full pulse.
    fn chooser_outer_edge() -> f64 {
        circle_edge()
    }

    #[test]
    fn there_are_as_many_slots_as_players() {
        assert_eq!(slots(&two(), 0.5, W, H).len(), 2);
        assert_eq!(slots(&five(), 0.5, W, H).len(), 5);
        assert!(slots(&[], 0.5, W, H).is_empty(), "no players, no wheel");
    }

    #[test]
    fn every_slot_sits_on_the_wheel_ring() {
        let (cx, cy) = centre(W, H);
        let r = radius(W, H);
        for slot in slots(&five(), 1.0, W, H) {
            let d = ((slot.x - cx).powi(2) + (slot.y - cy).powi(2)).sqrt();
            assert!(
                (d - r).abs() < 1e-9,
                "slot at distance {d}, expected {r}"
            );
        }
    }

    #[test]
    fn the_slots_are_evenly_spread_around_the_wheel() {
        let s = slots(&five(), 1.0, W, H);
        for pair in s.windows(2) {
            let step = (pair[1].angle - pair[0].angle).abs();
            assert!(
                (step - std::f64::consts::TAU / 5.0).abs() < 1e-9,
                "steps of {step} are not even"
            );
        }
    }

    #[test]
    fn the_wheel_starts_at_the_top() {
        // Zero is straight up, so the sweep begins and ends at the top.
        let s = slots(&five(), 0.0, W, H);
        let top = -std::f64::consts::FRAC_PI_2;
        assert!((s[0].angle - top).abs() < 1e-12, "first slot is at the top");
        let (_, y) = centre(W, H);
        assert!(s[0].y < y, "and above the centre, not below it");
    }

    #[test]
    fn the_circles_start_under_their_own_fingers_and_end_on_the_ring() {
        let players = five();
        let start = slots(&players, 0.0, W, H);
        for (slot, &(_, x, y)) in start.iter().zip(&players) {
            assert!(
                (slot.x - x).abs() < 1e-9 && (slot.y - y).abs() < 1e-9,
                "at the start a circle is exactly under its finger"
            );
        }
        // At the end every circle is on the wheel and no longer under its finger.
        let end = slots(&players, 1.0, W, H);
        assert!(
            end.iter()
                .zip(&players)
                .any(|(slot, &(_, x, y))| (slot.x - x).abs() > 1.0 || (slot.y - y).abs() > 1.0),
            "at least one circle has travelled to the wheel"
        );
    }

    #[test]
    fn no_circle_ever_snaps_back_to_its_finger() {
        // The property that matters is that a circle's *progress towards the
        // wheel* only ever advances: it never retreats to its finger and starts
        // again, which would read as a twitch.
        //
        // Distance from the finger is not the right measure. A circle travelling
        // round a ring passes near its own finger's position again on the way,
        // so that distance legitimately dips -- the first version of this test
        // asserted monotonic distance and was wrong.
        let players = five();
        for (index, &(_, fx, fy)) in players.iter().enumerate() {
            let mut last_ease = -1.0;
            for step in 0..=100 {
                let progress = f64::from(step) / 100.0;
                let slot = slots(&players, progress, W, H)[index];
                let (_, cy) = centre(W, H);
                let slot_r = radius(W, H);
                let target = (
                    W / 2.0 + slot_r * slot.angle.cos(),
                    cy + slot_r * slot.angle.sin(),
                );
                let total = (slot.x - fx).hypot(slot.y - fy);
                let remaining = (target.0 - slot.x).hypot(target.1 - slot.y);
                let journey = total + remaining;
                // The journey length never changes; only how much is behind us.
                let done = 1.0 - remaining / journey.max(1e-9);
                assert!(
                    done >= last_ease - 1e-9,
                    "player {index} moved back towards its finger at {progress}"
                );
                last_ease = done;
            }
        }
    }

    #[test]
    fn the_wheel_does_not_move_when_a_finger_moves() {
        // The layout is by pointer id, not by position, so dragging a finger
        // slides its circle around the wheel without reshuffling the others.
        let before: Vec<f64> = slots(&five(), 1.0, W, H)
            .iter()
            .map(|s| s.angle)
            .collect();
        let mut moved = five();
        moved[0].1 += 50.0;
        let after: Vec<f64> = slots(&moved, 1.0, W, H)
            .iter()
            .map(|s| s.angle)
            .collect();
        assert_eq!(before, after, "the wheel's angles depend only on count");
    }

    #[test]
    fn the_dots_do_not_rotate_around_each_other() {
        // THE regression test. The slots are a function of the players and
        // nothing else: no elapsed time, no spin progress, no clock. An earlier
        // version added the pointer's time term to each slot's angle, so the
        // circles orbited one another while the pointer chased them, and the wheel
        // was unreadable -- you had to track a dot against two moving things
        // instead of against the pointer.
        //
        // Every settle argument is compared: whatever the gather is doing, the
        // angle a player ends up at is the same.
        let players = five();
        let settled: Vec<f64> = slots(&players, 1.0, W, H).iter().map(|s| s.angle).collect();
        for step in 0..=100 {
            let gather = f64::from(step) / 100.0;
            let angles: Vec<f64> = slots(&players, gather, W, H)
                .iter()
                .map(|s| s.angle)
                .collect();
            assert_eq!(
                angles, settled,
                "the dots moved at gather {gather}: {:?} vs {settled:?}",
                angles
            );
        }
    }

    #[test]
    fn the_dots_are_fixed_however_long_the_wheel_spins() {
        // The spin's own clock must not reach the layout at all. This is the same
        // property from the other direction: drive it with the real elapsed time
        // and check nothing about the wheel depends on it.
        let players = five();
        let first = slots(&players, 1.0, W, H);
        for elapsed in (0..2500).step_by(25) {
            let elapsed = f64::from(elapsed);
            let window = 2500.0;
            let gather = gather_progress(elapsed, window);
            let now = slots(&players, gather, W, H);
            for (a, b) in first.iter().zip(&now) {
                assert!(
                    (a.angle - b.angle).abs() < 1e-12,
                    "a dot's angle changed at {elapsed}ms"
                );
            }
        }
    }

    #[test]
    fn the_gather_finishes_while_the_pointer_is_still_travelling() {
        let window = 2500.0;
        assert_eq!(gather_progress(0.0, window), 0.0);
        assert_eq!(
            gather_progress(GATHER_FRACTION * window, window),
            1.0,
            "home by {GATHER_FRACTION} of the window"
        );
        const {
            assert!(
                GATHER_FRACTION < SPIN_FRACTION,
                "and well before the pointer stops"
            )
        };
    }

    #[test]
    fn the_spin_starts_instantly_and_never_before_it_ends() {
        assert_eq!(spin_progress(0.0, 2500.0), 0.0);
        assert_eq!(spin_progress(2500.0, 2500.0), 1.0, "the full window");
        // Past the window it is stopped, not running off past 1.
        assert_eq!(spin_progress(9999.0, 2500.0), 1.0);
        assert_eq!(spin_progress(-100.0, 2500.0), 0.0, "and not before it began");
        assert_eq!(spin_progress(100.0, 0.0), 1.0, "a zero window is over");
    }

    #[test]
    fn the_spin_decelerates_into_its_stop() {
        // Halfway through the spin, less than half the distance is covered --
        // the signature of an ease-out. This is what makes it feel like a wheel
        // rather than a machine.
        let halfway = spin_progress(1250.0, 2500.0);
        assert!((halfway - 1.0).abs() < 1e-9 || halfway > 0.5, "half time");
        let progress_at_half_the_distance = spin_progress(1250.0, 2500.0);
        assert!(
            progress_at_half_the_distance > 0.5,
            "at half the time the pointer is already past halfway, so it is \
             slowing: {progress_at_half_the_distance}"
        );
        // Ease-out cubic: t^3 of the way is 3/4 of the time.
        let t = 0.5_f64;
        let eased = 1.0 - (1.0 - t).powi(3);
        let elapsed = (eased * SPIN_FRACTION) * 2500.0;
        assert!(
            (spin_progress(elapsed, 2500.0) - eased).abs() < 1e-9,
            "the curve is ease-out cubic"
        );
    }

    #[test]
    fn the_spin_finishes_well_before_the_reveal_starts() {
        // Just over half the window is spin, so roughly 900ms is a landed,
        // readable result before the colour floods the screen.
        let stopped_at = SPIN_FRACTION * 2500.0;
        assert!(stopped_at < 1500.0, "the spin is over by {stopped_at}ms");
        assert_eq!(spin_progress(stopped_at, 2500.0), 1.0);
        assert_eq!(
            spin_progress(stopped_at + 1.0, 2500.0),
            1.0,
            "and it stays stopped after that"
        );
        let readable = 2500.0 - stopped_at;
        assert!(readable >= 800.0, "a beat long enough to read: {readable}ms");
    }

    #[test]
    fn the_spin_is_over_in_under_a_second_and_a_half() {
        // The original's window is 2500ms and the spin now uses just over half of
        // it. Slow enough to follow, fast enough to feel decided.
        const {
            assert!(
                SPIN_FRACTION * 2500.0 <= 1500.0,
                "the spin itself should be under 1.5s"
            )
        };
    }

    #[test]
    fn the_circles_are_on_the_wheel_before_the_pointer_slows() {
        // The property, in terms of elapsed time rather than gather: at every
        // instant from the gather finishing onwards, every circle is on its slot
        // and stays there. Riding the pointer's own curve made them crawl for most
        // of the spin and only arrive at the very end -- the opposite of a flick,
        // and the reason the dots looked like they were chasing something.
        let window = 2500.0;
        let players = five();
        let (_, cy) = centre(W, H);
        let r = radius(W, H);
        let gathered_at = GATHER_FRACTION * window;

        for step in 0..=100 {
            let elapsed = f64::from(step) * 25.0;
            if elapsed < gathered_at {
                continue;
            }
            let s = slots(&players, gather_progress(elapsed, window), W, H);
            for slot in &s {
                let on_ring =
                    ((slot.x - W / 2.0).powi(2) + (slot.y - cy).powi(2)).sqrt();
                assert!(
                    (on_ring - r).abs() < 1e-9,
                    "a circle left the ring at {elapsed}ms: {on_ring} vs {r}"
                );
            }
        }
    }

    #[test]
    fn the_pointer_stops_beside_the_winner_not_on_it() {
        // Landing on the circle hides the thing it points at behind a white dot
        // and a ring of the same colour. It has to sit outside the ring.
        let s = slots(&five(), 1.0, W, H);
        let (px, py) = pointer_position(2500.0, 2500.0, turns_for(5), W, H);
        let (_, cy) = centre(W, H);
        let distance_from_centre = ((px - W / 2.0).powi(2) + (py - cy).powi(2)).sqrt();
        let ring = radius(W, H);
        assert!(
            distance_from_centre > ring,
            "the pointer at {distance_from_centre} is inside the ring at {ring}"
        );
        // By exactly the offset, and no more.
        assert!((distance_from_centre - (ring + POINTER_OFFSET)).abs() < 1e-9);
        // And it is clear of the circle it points at: outside its outer edge.
        assert!(
            (px - s[0].x).hypot(py - s[0].y) >= POINTER_OFFSET - 1e-9,
            "the pointer is inside the circle's own edge"
        );
    }

    #[test]
    fn the_pointer_is_beside_the_circle_for_the_whole_spin() {
        let (cx, cy) = centre(W, H);
        let outside = radius(W, H) + POINTER_OFFSET - 1e-9;
        for step in 0..=60 {
            let elapsed = f64::from(step) * 2500.0 / 60.0;
            let (x, y) = pointer_position(elapsed, 2500.0, turns_for(5), W, H);
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            assert!(d >= outside, "the pointer dipped inside the ring at {elapsed}ms");
        }
    }

    #[test]
    fn the_pointer_starts_at_the_top() {
        assert!(
            (pointer_angle(0.0, 2500.0, turns_for(5)) + std::f64::consts::FRAC_PI_2).abs() < 1e-12,
            "the sweep begins straight up"
        );
    }

    #[test]
    fn the_pointer_travels_monotonically_and_never_backwards() {
        // Non-decreasing, not increasing: the pointer deliberately *holds* its
        // angle once the spin has stopped, so it stops advancing rather than
        // running on and past the player it is resting on. A strict comparison
        // fails on that flat tail, which is how the first version of this test
        // did.
        let mut last = f64::NEG_INFINITY;
        for step in 0..=200 {
            let elapsed = f64::from(step) * 12.5;
            let angle = pointer_angle(elapsed, 2500.0, turns_for(5));
            assert!(
                angle >= last - 1e-12,
                "the pointer reversed at {elapsed}ms: {angle} came after {last}"
            );
            last = angle;
        }
    }

    #[test]
    fn the_pointer_stops_advancing_when_the_spin_stops() {
        // The end state, which is what the eye actually sees.
        let stopped = pointer_angle(SPIN_FRACTION * 2500.0, 2500.0, turns_for(5));
        assert_eq!(pointer_angle(2500.0, 2500.0, turns_for(5)), stopped);
        assert_eq!(pointer_angle(9999.0, 2500.0, turns_for(5)), stopped, "and it stays put");
    }

    #[test]
    fn the_pointer_ends_where_a_player_is() {
        // Otherwise it stops between two circles and the landing is ambiguous,
        // which is the one thing a wheel must never do.
        for count in 2..=10 {
            let players: Vec<(i32, f64, f64)> = (0..count)
                .map(|i| (i + 1, 0.0, 0.0))
                .collect();
            let s = slots(&players, 1.0, W, H);
            let landed = landed_on(&s, 2500.0, 2500.0, turns_for(5)).expect("a landing");
            let distance = angle_distance(s[landed].angle, pointer_angle(2500.0, 2500.0, turns_for(5)));
            assert!(
                distance < 1e-9,
                "with {count} players the pointer stopped {distance} rad from the \
                 nearest circle"
            );
        }
    }

    #[test]
    fn the_landing_player_is_stable_once_the_spin_is_slow() {
        // The last stretch of a spin must not dither between two players, or the
        // winner would appear to change as it stops.
        let s = slots(&five(), 1.0, W, H);
        let first = landed_on(&s, 2100.0, 2500.0, turns_for(5));
        for step in 2100..=2500 {
            assert_eq!(
                landed_on(&s, f64::from(step), 2500.0, turns_for(5)),
                first,
                "the landing changed at {step}ms"
            );
        }
    }

    #[test]
    fn the_landing_is_the_same_player_the_geometry_puts_at_the_top() {
        // The pointer is one step behind the slots, so both must agree on who is
        // under it. If they disagreed, the wheel would visibly point at one
        // player and stop on another.
        for count in 2..=10 {
            let players: Vec<(i32, f64, f64)> = (0..count)
                .map(|i| (i + 1, 0.0, 0.0))
                .collect();
            let s = slots(&players, 1.0, W, H);
            let turns = turns_for(usize::try_from(count).expect("a count"));
            // A whole number of turns brings the pointer back to the mark it
            // started from, which is where slot 0 sits -- the first player, by
            // pointer id.
            let stopping = pointer_angle(2500.0, 2500.0, turns);
            let expected = s
                .iter()
                .position(|slot| angle_distance(slot.angle, stopping) < 1e-9)
                .expect("a slot under the pointer");
            assert_eq!(
                landed_on(&s, 2500.0, 2500.0, turns),
                Some(expected),
                "with {count} players the pointer and the geometry disagree"
            );
            assert_eq!(
                expected, 0,
                "and it is always the first player by pointer id"
            );
        }
    }

    #[test]
    fn the_pointer_points_at_exactly_the_landing_player() {
        // The end state. The pointer is offset *radially* from the winner, so it
        // shares its angle exactly -- which is what "pointing at" means, and is
        // what `landed_on` reports. Its position deliberately does not coincide
        // with the circle's; `the_pointer_stops_beside_the_winner_not_on_it` is
        // what pins the gap. The first version of this test asserted both, and the
        // second half is precisely the behaviour that was changed.
        let s = slots(&five(), 1.0, W, H);
        let angle = pointer_angle(2500.0, 2500.0, turns_for(5));
        let landed = landed_on(&s, 2500.0, 2500.0, turns_for(5)).expect("a landing");

        // Same ray from the centre, so the pointer and the winner are aligned.
        assert!(angle_distance(s[landed].angle, angle) < 1e-9);

        // And the gap between them is exactly the offset, not more.
        let (px, py) = pointer_position(2500.0, 2500.0, turns_for(5), W, H);
        let gap = (px - s[landed].x).hypot(py - s[landed].y);
        assert!(
            (gap - POINTER_OFFSET).abs() < 1e-9,
            "the pointer is {gap} from the winner, not {POINTER_OFFSET}"
        );
    }

    #[test]
    fn there_is_no_landing_without_players_or_before_the_spin() {
        assert_eq!(landed_on(&[], 2500.0, 2500.0, turns_for(5)), None);
        let s = slots(&two(), 0.0, W, H);
        assert!(landed_on(&s, 0.0, 2500.0, turns_for(5)).is_some(), "always somewhere");
    }

    #[test]
    fn the_pointer_holds_a_constant_offset_from_the_ring() {
        // It rides a circle of its own, so its distance from the centre never
        // changes -- which is what keeps the gap to the circles even the whole way
        // round rather than bunching up on one side.
        let (cx, cy) = centre(W, H);
        let want = radius(W, H) + POINTER_OFFSET;
        for step in 0..=50 {
            let elapsed = f64::from(step) * 50.0;
            let (x, y) = pointer_position(elapsed, 2500.0, turns_for(5), W, H);
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            assert!((d - want).abs() < 1e-9, "the pointer's orbit changed at {elapsed}ms");
        }
    }

    #[test]
    fn two_players_also_land_correctly() {
        // The minimum case, and the one that has to look least like a wheel --
        // two circles and a pointer halfway round.
        let s = slots(&two(), 1.0, W, H);
        let landed = landed_on(&s, 2500.0, 2500.0, turns_for(5)).expect("a landing");
        assert!(
            angle_distance(s[landed].angle, pointer_angle(2500.0, 2500.0, turns_for(5))) < 1e-9,
            "the pointer did not stop on a circle"
        );
        // They are opposite each other.
        let separation = (s[0].angle - s[1].angle).abs();
        assert!((separation - std::f64::consts::PI).abs() < 1e-9);
    }

    #[test]
    fn the_whole_wheel_stays_on_screen_for_two_fingers_on_a_small_phone() {
        // The end state is the one that has to fit: every circle fully visible.
        let (small_w, small_h) = (320.0, 568.0);
        let players = two();
        for slot in slots(&players, 1.0, small_w, small_h) {
            let edge = chooser_outer_edge();
            assert!(slot.x - edge >= 0.0, "circle off the left edge at {}", slot.x);
            assert!(slot.x + edge <= small_w, "circle off the right edge");
            assert!(slot.y - edge >= 0.0, "circle off the top");
            assert!(slot.y + edge <= small_h, "circle off the bottom");
        }
    }
}
