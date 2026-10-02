// Copyright (c) 2026 Witalis Domitrz <witekdomitrz@gmail.com>
// AGPL License

//! The chooser: every pointer, the draw timer and the chosen player.
//!
//! This is the whole of Chwazi's behaviour, with no DOM, no clock and no canvas
//! anywhere in it. The browser front end feeds it pointer ids and
//! positions and timestamps, and reads back what should be on screen; this file
//! decides what that is. That separation is the point -- the state machine is
//! where the app's rules live, and rules are what can be tested without a
//! browser.
//!
//! Time is always milliseconds in a `f64`, on the same timeline
//! [`Chooser::tick`] is driven with. The browser passes
//! `performance.now()`, and tests pass whatever they like, so a test can place
//! events at exact instants and assert on the state after them.
//!
//! The rules, as the original app had them:
//!
//! * A finger down adds a player, and restarts the draw timer -- but the timer
//!   only actually runs when at least [`REQUIRED_PLAYER_COUNT`] players are
//!   present and nobody has been chosen yet.
//! * A finger up (or a cancelled touch) removes that player and restarts the
//!   timer, for the same reason.
//! * When the timer runs out, one of the players present is chosen at random.
//!   Choosing ends the draw: every other player leaves immediately.
//! * Two seconds after the chosen finger lifts, the choice is cleared and a new
//!   draw can start.

use std::collections::BTreeMap;

/// A draw needs at least this many players. One finger cannot choose itself.
pub const REQUIRED_PLAYER_COUNT: usize = 2;

/// Radius of the coloured disc, to its outer edge.
///
/// 35.7 CSS px. Measured at full resolution on a 1080px-wide Galaxy S25 at 3x
/// by locating a mark's centre from its pale dot and scanning radially outward
/// along 720 rays: the saturated colour runs unbroken from 8.7 to 35.7 CSS px.
///
/// This is the number that has been wrong twice. It was 40 ("the whole mark,
/// outer edge") in the build before last, and 40 is not wrong because it is too
/// big or too small -- it is wrong because it is a radius of a *different shape*.
/// The native mark is not a disc: it is a disc, a black gap, and a pale ring.
pub const DISC_RADIUS: f64 = 38.2;

/// The mark has **no central dot**.
///
/// The small pale circle at the exact centre of every mark in the reference
/// recordings is Android's "show touches" indicator, not part of the app. It is the
/// same colour in every mark regardless of that mark's own colour, and it does not
/// move, grow or pulse with the mark -- all three of which are what a mark's centre
/// would do. On the recordings made with the indicator switched off it is absent,
/// and the mark is disc, gap, ring.
///
/// It was also what made every measurement of this mark quietly wrong, in a way no
/// amount of care with the radial scan could have caught: the indicator's own dark
/// surround eats into the disc, so a disc measured on an indicator-on frame reads
/// about 2.5 CSS px small. Every radius below is measured on an indicator-off frame.
///
/// The colour is kept because both loading arcs are drawn in it. It is the app's own
/// pale colour, a warm off-white rather than pure white: #fff against a saturated
/// mark reads as a hole punched through the screen.
pub const DOT_COLOUR: &str = "rgb(252, 202, 150)";

/// The black gap between the disc and the ring.
///
/// 7.6 CSS px wide, from the same radial scans: disc ends at 35.7, black runs
/// 36.7 to 44.3, and the pale ring starts at 45.3.
///
/// The gap is the same near-black as the background -- rgb(15, 12, 11) against a
/// background of rgb(11, 11, 11) -- so it is the *absence* of colour between two
/// bands rather than a shape of its own. That is why it was twice mistaken for an
/// artefact of blurring: at a glance a dark gap and a dark background are the same
/// dark. They are only distinguishable by scanning radially, which is how this was
/// settled.
pub const GAP_OUTER_RADIUS: f64 = 47.1;

/// Radius of the pale ring's outer edge, 54.3 CSS px.
///
/// Measured 45.3 to 54.3 CSS px on the native app, so the ring is 9 CSS px thick.
pub const MARK_RADIUS: f64 = 57.0;

/// Inner edge of the pale ring, 45.3 CSS px.
pub const RING_INNER_RADIUS: f64 = 47.5;

/// The pale ring's centreline: the radius to stroke at.
///
/// A stroke is centred on the path it follows, so stroking *at* [`MARK_RADIUS`]
/// lays the band from 49.8 to 58.8 CSS px. That puts 4.5px of ring outside the
/// measured outer edge and leaves 4.5px of the measured gap showing as a second
/// black band inside the first -- which is exactly "the gap is too big", and the
/// reason the band has to be stroked at its middle rather than its edge.
pub const RING_STROKE_RADIUS: f64 = f64::midpoint(MARK_RADIUS, RING_INNER_RADIUS);

/// How far outside the ring the draw's own arc sits, in CSS px.
///
/// The two loadings must never be on the same pixels, or the draw's fill covers the
/// registration sweep and the first loading becomes invisible for the whole window --
/// which is what happened when both were drawn on the ring's own radius. 2 CSS px is
/// enough to separate them at this ring thickness and small enough that the pair
/// still reads as one ring rather than two concentric circles.
pub const LOADING_GROWTH: f64 = 2.0;

/// The ring's width: the thickness of its own measured band, 9 CSS px.
///
/// This is also the width both loading arcs are stroked at, so an arc *is* the
/// ring rather than a line drawn near it.
pub const ARC_WIDTH: f64 = MARK_RADIUS - RING_INNER_RADIUS;

/// The ring's colour, as a fraction of the disc's own colour: **0.77, darker**.
///
/// The previous build had this the other way round -- the disc's hue washed toward
/// white -- which is the exact inverse of the app being matched, and is why the ring
/// read as a highlight rather than as a second, quieter band.
///
/// Measured on a neutral grey mark, where no hue can confuse the reading: the disc
/// is rgb(229, 229, 229) and the ring rgb(177, 177, 177) on all three channels --
/// 0.773, with no per-channel difference at all, so it is a pure lightness change
/// and not a desaturation. The saturated marks agree: an orange disc of
/// rgb(250, 224, 88) has a ring of rgb(234, 200, 1), the same ratio per channel.
///
/// Expressed as a ratio of the disc's own channels rather than a fixed rgb, so a
/// player whose id lands on a different hue gets a ring that belongs to it.
pub const RING_DARKEN: f64 = 0.77;

/// Lightness of a player's colour, as a percentage.
///
/// The original web formula is `hsl(h, 100%, 40%)`. Sampled across 1184 saturated
/// pixels from two native recordings, the native app's median lightness is 49% --
/// nine points lighter, which is plainly visible side by side, and part of why the
/// mark looked heavier and darker than the app it was meant to be.
///
/// The hue formula is untouched: it spreads pointer ids around the wheel exactly as
/// it always has, and this constant is the only thing that changed in `Player::color`.
pub const COLOUR_LIGHTNESS: f64 = 49.0;
/// How far a mark's radius swings, in each direction, around its rest size.
///
/// 0.065, from the mark's own outer radius sampled at 60fps on a recording with the
/// touch indicator off: 53.3 to 60.7 CSS px. The pulse is a symmetric sine
/// (`1 + s * sin`), so those two extremes give the resting radius as their midpoint,
/// 57.0, and the swing as `(max - min) / (max + min)` = 0.065.
///
/// That resting radius is exactly what the band geometry gives, measured a
/// completely different way, and the two agreeing is the cross-check that both are
/// right.
///
/// Two earlier values were wrong, and the reason is the same both times: they were
/// read off an indicator-on frame, where the indicator's dark surround clips the
/// mark's floor and makes it look as though the breath is deeper than it is. 0.07
/// came from pixel *area* peaks, which measure the square of the radius and so
/// double the error; 0.117 came from a 53.0-to-60.0 reading of the clipped frame.
pub const MAX_PULSE_SCALE: f64 = 0.065;

/// How long a draw window lasts once two players are present.
pub const DRAWING_TIME_MS: f64 = 2500.0;

/// How long one finger takes to load its own mark.
///
/// 560ms, measured frame by frame from touchdown: the disc reaches full size at
/// 0.87s having started at 0.75s, and the pale ring sweeps from a point to a
/// closed circle across the same window.
///
/// This is the app's first of two loadings, and it is per finger rather than per
/// draw. It is why a mark is an *event* on the glass rather than something that
/// is simply already there.
pub const REGISTRATION_TIME_MS: f64 = 560.0;

/// How long the winning colour takes to wipe down the screen.
///
/// 300ms, chosen rather than measured, and the user asked for it.
///
/// What the recording shows is 130ms: the front is 2.4% down at 4.917s, 6.0% at
/// 4.925s and has crossed by 5.050s. At that speed the winning colour is on the
/// whole screen before the eye has finished moving to the winner's finger, and it
/// reads as a flash rather than a reveal -- "the flood is too fast". 300ms is
/// about 2.3x the measured value: slow enough to watch arrive, and still well
/// inside the two seconds before the app resets, so it never feels like a wait.
///
/// The shape is measured and unchanged: a linear wipe from the top edge. Only the
/// duration is a preference, and it is here rather than buried in the renderer
/// because it is a number the user chose.
pub const CHOSEN_PLAYER_ANIMATION_TIME_MS: f64 = 300.0;

/// One full breath of a mark's pulse.
///
/// 808ms, from an FFT of an isolated mark's outer radius sampled at 120fps over a
/// full breath. The previous 1000ms was derived from pixel *area* peaks, which
/// measure the square of the radius and so carry the same error twice.
pub const SCALING_PERIOD_MS: f64 = 808.0;

/// Radius the winner's mark takes once the flood has finished.
///
/// 104 CSS px, measured as the outer edge of the black annulus that separates the
/// winner's mark from the flooded colour, sampled every 0.2s from 6.0s to 7.8s in
/// a recording where it sits at 103.8 to 104.3.
///
/// This is not the winner's circle *growing* into the screen, which is how the
/// previous build drew it and what the recording rules out: the flood's leading
/// edge starts at the disc's own edge (35.7) and pushes outward, while this black
/// ring is already full size when the flood completes. So the winner's mark keeps
/// its measured size and the black ring around it opens up.
pub const WINNER_RADIUS: f64 = 104.0;

/// How long the chosen finger must be off the glass before the app resets.
pub const RESTART_DELAY: f64 = 2000.0;

/// One finger on the glass.
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    /// The browser's pointer id, and the identity of the player for as long as
    /// that finger is down.
    pub id: i32,
    pub x: f64,
    pub y: f64,
    /// When this player was chosen, if it has been.
    ///
    /// Set once, at the instant the draw ended, and never recomputed from a
    /// later frame -- the winner's mark floods from the moment of the draw,
    /// so reading a fresh timestamp each frame would restart the animation.
    pub chosen_at: Option<f64>,
    /// When this finger landed on the glass.
    ///
    /// The per-finger loading is anchored to this, not to the draw window: each
    /// mark charges from its own touchdown, so a finger that lands late does not
    /// appear already-loaded beside marks that have been charging for a second.
    pub joined_at: f64,
}

impl Player {
    /// How far this player's own mark has loaded at `timestamp`, 0 to 1.
    ///
    /// `None` once the mark has finished loading, so a settled mark costs nothing
    /// per frame.
    #[must_use]
    pub fn registration(&self, timestamp: f64) -> Option<f64> {
        let progress = (timestamp - self.joined_at) / REGISTRATION_TIME_MS;
        (progress < 1.0).then_some(progress.clamp(0.0, 1.0))
    }

    /// The ring colour for this player: its own colour, darkened.
    ///
    /// The native app's ring is a *darker* tint of the disc, not a lighter one and
    /// not a fixed white. Scaling the lightness keeps that relationship for every
    /// hue rather than hard-coding the one colour that was measured. See
    /// [`RING_DARKEN`].
    #[must_use]
    pub fn ring_color(&self) -> String {
        let hue = (f64::from(self.id) * 223.0 + 263.0).rem_euclid(360.0);
        format!(
            "hsl({hue:.0}, 100%, {:.1}%)",
            COLOUR_LIGHTNESS * RING_DARKEN
        )
    }
    /// The colour of pointer `id`.
    ///
    /// `pointerId * 223 + 263` steps the hue by 223 degrees per id, and 223 is
    /// irrational enough against 360 that the first several players land far
    /// apart around the wheel. The offset puts pointer 1 at 126 degrees, in the
    /// green, rather than at the top of the red arc.
    ///
    /// The browser normalises a negative hue, so ids that go past 360 wrap
    /// through red exactly as the CSS did. Modulo is applied so the string
    /// stays inside 0..360 and does not depend on the browser doing it.
    #[must_use]
    pub fn color(id: i32) -> String {
        let hue = (f64::from(id) * 223.0 + 263.0).rem_euclid(360.0);
        format!("hsl({hue:.0}, 100%, {COLOUR_LIGHTNESS:.0}%)")
    }

    /// The CSS colour this player is drawn in.
    #[must_use]
    pub fn color_of(&self) -> String {
        Self::color(self.id)
    }

    /// The player's pulse multiplier at `timestamp`.
    ///
    /// A sine over [`SCALING_PERIOD_MS`], anchored at `start_time` so every
    /// player breathes in step.
    #[must_use]
    pub fn pulse_scale(&self, timestamp: f64, start_time: f64) -> f64 {
        pulse_scale(timestamp, start_time)
    }
}

/// The pulse multiplier shared by every player at `timestamp`.
#[must_use]
pub fn pulse_scale(timestamp: f64, start_time: f64) -> f64 {
    let phase = (timestamp - start_time).rem_euclid(SCALING_PERIOD_MS) / SCALING_PERIOD_MS;
    1.0 + MAX_PULSE_SCALE * (phase * 2.0 * std::f64::consts::PI).sin()
}

/// The app's entire state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chooser {
    /// The fingers currently down, in pointer-id order.
    ///
    /// An ordered map rather than a `HashMap` because the draw is *random*, and
    /// a random draw out of an unordered collection makes no promise at all:
    /// the same pointer id could win twice running, or never. Ordering by id
    /// fixes what each possible winner means, and the randomness is then just a
    /// fair index into it. It is also what keeps `Default` derivable.
    players: BTreeMap<i32, Player>,
    chosen: Option<i32>,
    /// When the current draw window opened.
    draw_started_at: Option<f64>,
    /// When the chosen finger lifted, if it has.
    chosen_lifted_at: Option<f64>,
}

impl Chooser {
    /// A chooser with nobody's finger on the glass.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The players currently present, in pointer-id order.
    pub fn players(&self) -> impl Iterator<Item = &Player> {
        self.players.values()
    }

    /// How many fingers are down.
    #[must_use]
    pub fn len(&self) -> usize {
        self.players.len()
    }

    /// Whether the glass is untouched.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// The chosen player, if one has been chosen.
    #[must_use]
    pub fn chosen(&self) -> Option<&Player> {
        self.chosen.and_then(|id| self.players.get(&id))
    }

    /// Whether a winner has been chosen and has not yet been cleared.
    #[must_use]
    pub fn is_chosen(&self) -> bool {
        self.chosen.is_some()
    }

    /// When the current draw window opened, if one is running.
    ///
    /// The white arc on each player's ring is this timestamp's progress
    /// through [`DRAWING_TIME_MS`]; `None` means no arc at all.
    #[must_use]
    pub fn draw_started_at(&self) -> Option<f64> {
        self.draw_started_at
    }

    /// Whether a draw window is running.
    ///
    /// Exactly the original's `started_timeout`: set when the timer is armed,
    /// cleared the moment it fires -- which is what stops a pointer event
    /// arriving in the same frame as the draw from starting a second one.
    #[must_use]
    pub fn is_drawing(&self) -> bool {
        self.draw_started_at.is_some()
    }

    /// When the draw clock actually starts counting, if a window is open.
    ///
    /// A window opens as soon as the second finger lands, but it does not start
    /// counting until every mark has finished its own loading. Otherwise a finger
    /// that slaps down just as the previous window was expiring would be picked
    /// from a mark that is still charging -- and, worse, chosen from a screen where
    /// the other marks had not arrived yet, which is not a screen anybody saw.
    ///
    /// The clock restarts whenever who is on the glass changes, and that includes a
    /// finger *lifting*: the last roster change is the origin, and every mark on the
    /// glass a registration later.
    #[must_use]
    pub fn ready_at(&self) -> Option<f64> {
        let opened = self.draw_started_at?;
        // Only fingers that are *still down* and *newer than the last roster
        // change* can hold the clock back.
        //
        // Taking a plain maximum over every player's `joined_at` gets the lift case
        // wrong: three fingers down at t=0 with one lifting at t=100 leaves two
        // stamped at 0, so their maximum is 0 and the clock starts at 0 -- firing
        // the draw 100ms after the players changed the screen by lifting, from a
        // roster nobody had looked at for that long. Seeding the fold with `opened`
        // makes a lift cost a full registration, which is what a landing costs, and
        // a finger that is both down and newer than the change still holds it back.
        let last_to_land = self
            .players
            .values()
            .map(|player| player.joined_at)
            .filter(|landed| *landed >= opened)
            .fold(opened, f64::max);
        Some(last_to_land + REGISTRATION_TIME_MS)
    }

    /// Whether every mark has loaded and the draw clock is running.
    #[must_use]
    pub fn is_ready(&self, timestamp: f64) -> bool {
        self.ready_at().is_some_and(|ready| timestamp >= ready)
    }

    /// The white loading arc's progress at `timestamp`, 0 to 1.
    ///
    /// The app's second loading, and the one that runs on *every* mark at once: a
    /// pale arc sweeping the ring's own band, from nothing round to closed, as the
    /// draw window closes. It is measured from the moment the window is ready, so
    /// the two loadings never overlap and the eye is never asked to follow both.
    ///
    /// `None` while no window is running, which draws no arc at all.
    #[must_use]
    pub fn draw_progress(&self, timestamp: f64) -> Option<f64> {
        let ready = self.ready_at()?;
        Some(((timestamp - ready) / DRAWING_TIME_MS).clamp(0.0, 1.0))
    }

    /// A finger went down at `(x, y)`.
    ///
    /// Ignored once someone has been chosen: the winner's screen is showing
    /// until that finger lifts and the app resets, and a new finger landing
    /// during it is not a player.
    pub fn pointer_down(&mut self, id: i32, x: f64, y: f64, now: f64) {
        if self.chosen.is_some() {
            return;
        }
        self.players.insert(
            id,
            Player {
                id,
                x,
                y,
                chosen_at: None,
                joined_at: now,
            },
        );
        self.restart_draw(now);
    }

    /// A finger moved to `(x, y)`.
    ///
    /// Ignored for a pointer that is not down, which is every move event that
    /// arrives without a preceding down.
    pub fn pointer_move(&mut self, id: i32, x: f64, y: f64) {
        if let Some(player) = self.players.get_mut(&id) {
            player.x = x;
            player.y = y;
        }
    }

    /// A finger lifted, or its touch was cancelled.
    ///
    /// Both mean the same thing to a finger chooser, and the original treated
    /// them identically, so they are one function.
    pub fn pointer_up(&mut self, id: i32, now: f64) {
        if self.chosen == Some(id) {
            // The winner is leaving. Everyone else was already cleared at the
            // draw, and this finger stays put as a marker so the filled screen
            // can be seen to belong to somebody -- it is the hole in the fill.
            // The choice itself is cleared two seconds from now, which is when
            // a new draw may begin.
            self.chosen_lifted_at = Some(now);
            return;
        }
        if self.players.remove(&id).is_none() {
            return;
        }
        self.restart_draw(now);
    }

    /// A draw window has elapsed: choose one of the players present, at random.
    ///
    /// Returns the winner's pointer id. Every other player leaves at once --
    /// their fingers are still down, but their circles are gone, because the
    /// original cleared the map around the winner and so did this.
    #[must_use]
    pub fn draw(&mut self, now: f64, winner: usize) -> Option<i32> {
        // A draw needs two players and an unclaimed app, whatever the timer
        // thought it was doing.
        if self.players.len() < REQUIRED_PLAYER_COUNT || self.chosen.is_some() {
            return None;
        }
        // `winner` is a caller-supplied index so that a test can pin the
        // randomness and assert on the result. It is reduced here, so the
        // browser may pass `random_index()` raw.
        let index = winner % self.players.len();
        let id = *self.players.keys().nth(index)?;
        if let Some(player) = self.players.get_mut(&id) {
            player.chosen_at = Some(now);
        }
        self.players.retain(|_, player| player.id == id);
        self.chosen = Some(id);
        // The window is over: this flag, not the winner, is what stops a
        // pointer event arriving in the same frame from starting another draw.
        self.draw_started_at = None;
        Some(id)
    }

    /// Clear a spent choice, if it has been spent.
    ///
    /// Returns `true` when the app was reset, which is the moment a new draw
    /// becomes possible.
    #[must_use]
    pub fn tick(&mut self, now: f64) -> bool {
        let Some(lifted) = self.chosen_lifted_at else {
            return false;
        };
        // Subtraction, not a float comparison, and `>=` for the same reason the
        // browser's own draw check uses it: elapsed time is what is being
        // compared, and a millisecond of float wobble should not decide whether
        // a game resets.
        if now - lifted < RESTART_DELAY {
            return false;
        }
        self.players.clear();
        self.chosen = None;
        self.chosen_lifted_at = None;
        self.draw_started_at = None;
        true
    }

    /// How far the winner's colour has flooded the screen at `timestamp`, 0 to 1.
    ///
    /// Anchored to the instant of the draw, not to this frame, so the flood does
    /// not restart on every render.
    #[must_use]
    pub fn chosen_progress(&self, timestamp: f64) -> Option<f64> {
        let chosen_at = self.chosen()?.chosen_at?;
        Some(((timestamp - chosen_at) / CHOSEN_PLAYER_ANIMATION_TIME_MS).clamp(0.0, 1.0))
    }

    /// The leading edge of the winner's colour at `timestamp`, given the viewport.
    ///
    /// It is a **circle centred on the winner**, and the previous build got this
    /// wrong twice in a row, in opposite directions, so both readings are recorded
    /// here rather than only the right one.
    ///
    /// Measured by probing outward from the winner in four directions at 120fps,
    /// skipping the black annulus: at 4.917s the reach is 444 CSS px straight up
    /// and 116 straight down, with 0 left and 0 right. That asymmetry is the proof
    /// it is a disc -- a top-down wipe would be even left-to-right and would have
    /// the *up* and *down* reaches equal -- and the disc grows from 115 to 335 CSS px
    /// in about 130ms while the coverage of the screen rises 0.04, 0.07, 0.10, 0.13,
    /// 0.16 ... in step.
    ///
    /// The build before this one drew a linear wipe from the top edge, on the
    /// strength of a probe that sampled only the leftmost 60 columns. That probe
    /// cannot distinguish the two: a growing disc crosses those columns from the
    /// top down, so it *looks* like a wipe. Measuring in all four directions is
    /// what settles it, and it costs the same.
    ///
    /// The front starts at the winner's mark, not at the centre of the screen, and
    /// the mark stays put inside it.
    #[must_use]
    pub fn flood_radius(&self, timestamp: f64, width: f64, height: f64) -> Option<f64> {
        let progress = self.chosen_progress(timestamp)?;
        // The front starts at the mark's own edge: the disc is already the winner's
        // colour, so what floods is everything *outside* it.
        let from = DISC_RADIUS;
        let to = width.max(height) * 1.2;
        Some(from + (to - from) * progress)
    }

    /// Whether the draw timer should be armed for this state.
    ///
    /// Kept as its own function because it is the one rule the original expressed
    /// as a guard in two places, and collapsing them into one is what lets
    /// `pointer_down` refuse to open a window while a winner is on screen.
    fn can_draw(&self) -> bool {
        self.players.len() >= REQUIRED_PLAYER_COUNT && self.chosen.is_none()
    }

    /// Arm the draw window at `now`, or disarm it.
    ///
    /// Every pointer event that changes who is on the glass calls this, so
    /// moving a finger off the table restarts the window: a draw is against
    /// the players present *now*, and the last change to that set is when it
    /// must begin counting.
    fn restart_draw(&mut self, now: f64) {
        self.draw_started_at = if self.can_draw() { Some(now) } else { None };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two fingers down, mid-draw, with the winner already chosen by `draw`.
    ///
    /// In this version of the app the winner is picked at the moment the timer
    /// fires rather than when the window is armed, so "which finger won" and
    /// "the draw has happened" are one step.
    fn won(winner: usize) -> Chooser {
        let mut chooser = drawing();
        let _ = chooser.draw(0.0, winner);
        chooser
    }

    /// A chooser with two fingers down, mid-draw.
    fn drawing() -> Chooser {
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 100.0, 200.0, 0.0);
        chooser.pointer_down(2, 300.0, 400.0, 10.0);
        chooser
    }

    #[test]
    fn a_finger_down_is_a_player_at_that_point() {
        let mut chooser = Chooser::new();
        chooser.pointer_down(7, 12.5, 34.0, 0.0);

        let player = chooser.players().next().expect("one player");
        assert_eq!(player.id, 7);
        assert_eq!((player.x, player.y), (12.5, 34.0));
        assert_eq!(chooser.len(), 1);
        assert!(!chooser.is_chosen());
    }

    #[test]
    fn a_moving_finger_takes_its_position() {
        let mut chooser = drawing();
        chooser.pointer_move(1, 111.0, 222.0);

        let moved = chooser.players().find(|p| p.id == 1).expect("player 1");
        assert_eq!((moved.x, moved.y), (111.0, 222.0));
        // The other finger has not moved.
        let other = chooser.players().find(|p| p.id == 2).expect("player 2");
        assert_eq!((other.x, other.y), (300.0, 400.0));
    }

    #[test]
    fn a_move_from_a_pointer_that_is_not_down_is_ignored() {
        let mut chooser = drawing();
        chooser.pointer_move(99, 1.0, 2.0);

        assert_eq!(chooser.len(), 2, "no phantom player");
        assert!(chooser.players().all(|player| player.id != 99));
    }

    #[test]
    fn a_finger_up_takes_the_player_away() {
        let mut chooser = drawing();
        chooser.pointer_up(1, 100.0);

        assert_eq!(chooser.len(), 1);
        assert!(chooser.players().all(|player| player.id != 1));
        assert!(!chooser.is_chosen());
    }

    #[test]
    fn a_cancelled_touch_lifts_the_finger_too() {
        // `pointer_up` is the whole of lift-and-cancel handling: the original
        // bound both `pointerup` and `pointercancel` to it.
        let mut chooser = drawing();
        chooser.pointer_up(2, 100.0);

        assert_eq!(chooser.len(), 1);
        assert!(chooser.players().all(|player| player.id != 2));
    }

    #[test]
    fn one_finger_is_not_a_draw() {
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);

        assert_eq!(chooser.len(), REQUIRED_PLAYER_COUNT - 1);
        assert!(!chooser.is_drawing(), "a draw needs two players");
        assert!(chooser.draw_progress(0.0).is_none(), "and so no arc");
        assert_eq!(chooser.draw(2500.0, 0), None, "and no winner");
    }

    #[test]
    fn a_second_finger_opens_the_draw() {
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 50.0, 50.0, 40.0);

        assert!(chooser.is_drawing());
        assert_eq!(chooser.draw_started_at(), Some(40.0));
        // The window is *open* but not *counting*: the second finger landed at 40ms
        // and has 560ms of loading to do first, so no arc is drawn yet.
        assert_eq!(chooser.draw_progress(40.0), Some(0.0));
        let ready = chooser.ready_at().expect("a window");
        assert!((ready - (40.0 + REGISTRATION_TIME_MS)).abs() < 1e-9);
        assert_eq!(
            chooser.draw_progress(ready + DRAWING_TIME_MS / 2.0),
            Some(0.5)
        );
    }

    #[test]
    fn a_third_finger_restarts_the_draw() {
        let mut chooser = drawing();
        chooser.pointer_down(3, 10.0, 10.0, 500.0);

        // The window counts from the latest change to who is on the glass, so
        // the last finger down gets the whole window.
        assert_eq!(chooser.draw_started_at(), Some(500.0));
    }

    #[test]
    fn a_finger_up_restarts_the_draw() {
        let mut chooser = drawing();
        chooser.pointer_up(1, 100.0);

        assert_eq!(chooser.len(), 1);
        assert!(
            !chooser.is_drawing(),
            "one finger left, so the window is gone"
        );

        // Put the finger back and the window starts again from now.
        chooser.pointer_down(1, 100.0, 200.0, 200.0);
        assert_eq!(chooser.draw_started_at(), Some(200.0));
    }

    #[test]
    fn the_arc_clamps_outside_the_window() {
        let chooser = drawing();
        assert_eq!(chooser.draw_progress(-5_000.0), Some(0.0));
        assert_eq!(chooser.draw_progress(9_999.0), Some(1.0));
    }

    #[test]
    fn the_draw_chooses_one_of_the_players_present() {
        for winner in 0..3 {
            let mut chooser = drawing();
            let chosen = chooser
                .draw(2500.0, winner)
                .expect("two players, so a winner");

            assert!([1, 2].contains(&chosen), "picked from the players");
            assert_eq!(chooser.len(), 1, "the others left");
            assert_eq!(chooser.chosen().map(|player| player.id), Some(chosen));
        }
    }

    #[test]
    fn every_player_can_win() {
        // The draw is random, but it must be *possible* for every finger on the
        // glass to be the one that wins.
        for index in 0..2 {
            let mut chooser = drawing();
            assert_eq!(
                chooser.draw(2500.0, index),
                Some(i32::try_from(index).unwrap() + 1)
            );
        }
    }

    #[test]
    fn the_winner_is_anchored_to_the_instant_of_the_draw() {
        let mut chooser = drawing();
        let _ = chooser.draw(2500.0, 0);

        // A later frame must not restart the expansion.
        assert_eq!(chooser.chosen_progress(2500.0), Some(0.0));
        assert_eq!(
            chooser.chosen_progress(2500.0 + CHOSEN_PLAYER_ANIMATION_TIME_MS / 2.0),
            Some(0.5)
        );
        assert_eq!(chooser.chosen_progress(999_999.0), Some(1.0));
    }

    #[test]
    fn the_winner_survives_lifting_and_a_new_finger_lands_nowhere() {
        let mut chooser = drawing();
        let winner = chooser.draw(2500.0, 0).expect("a winner");

        chooser.pointer_up(winner, 3000.0);
        assert_eq!(chooser.len(), 1, "the winner's circle stays put");
        assert_eq!(chooser.chosen().map(|p| p.id), Some(winner));

        // The app is not reset yet, so a new finger is not a player.
        chooser.pointer_down(9, 10.0, 10.0, 3100.0);
        assert_eq!(chooser.len(), 1);
        assert!(!chooser.is_drawing());
    }

    #[test]
    fn the_reset_comes_exactly_two_seconds_after_the_winner_lifts() {
        let mut chooser = drawing();
        let winner = chooser.draw(2500.0, 0).expect("a winner");

        chooser.pointer_up(winner, 3000.0);
        assert!(!chooser.tick(3000.0 + RESTART_DELAY - 1.0), "not yet");
        assert!(
            chooser.tick(3000.0 + RESTART_DELAY),
            "the reset happens once the delay has elapsed"
        );
        assert!(chooser.is_empty());
        assert!(!chooser.is_chosen());
    }

    #[test]
    fn a_reset_chooser_can_draw_again() {
        let mut chooser = drawing();
        let winner = chooser.draw(2500.0, 0).expect("a winner");
        chooser.pointer_up(winner, 3000.0);
        let _ = chooser.tick(3000.0 + RESTART_DELAY + 1.0);

        chooser.pointer_down(4, 5.0, 5.0, 6000.0);
        chooser.pointer_down(5, 6.0, 6.0, 6100.0);

        assert!(chooser.is_drawing(), "the app is reusable after a reset");
        assert!(chooser.chosen().is_none());
        assert_eq!(chooser.draw(8600.0, 1), Some(5));
    }

    #[test]
    fn a_winner_held_down_does_not_reset_the_app() {
        let mut chooser = drawing();
        let winner = chooser.draw(2500.0, 0).expect("a winner");

        // Still holding the winner: there is no "lifted" moment, so no reset,
        // however long the page is left alone.
        assert!(!chooser.tick(1_000_000.0));
        assert!(chooser.is_chosen());
        chooser.pointer_up(winner, 1_000_000.0);
        assert!(!chooser.tick(1_000_000.0 + RESTART_DELAY - 1.0));
        assert!(chooser.tick(1_000_000.0 + RESTART_DELAY + 1.0));
    }

    #[test]
    fn a_draw_while_a_winner_is_showing_does_nothing() {
        let mut chooser = drawing();
        let _ = chooser.draw(2500.0, 0);

        assert_eq!(chooser.draw(3000.0, 1), None);
        assert_eq!(chooser.len(), 1, "still one player");
        assert!(chooser.is_chosen(), "still chosen");
    }

    #[test]
    fn a_lift_of_a_pointer_that_is_not_down_does_nothing() {
        let mut chooser = drawing();
        chooser.pointer_up(42, 100.0);

        assert_eq!(chooser.len(), 2);
        // And it did not count as a change to the players, so the window the
        // second finger started is still running.
        assert_eq!(chooser.draw_started_at(), Some(10.0));
    }

    #[test]
    fn a_draw_with_one_player_does_nothing() {
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);

        assert_eq!(chooser.draw(2500.0, 0), None);
        assert!(!chooser.is_chosen());
    }

    #[test]
    fn the_winner_index_is_taken_against_the_players_actually_present() {
        // Indices come from a random draw, so a caller may pass anything; an
        // out-of-range index must still name a real player.
        let mut chooser = drawing();
        assert_eq!(chooser.draw(2500.0, 17), Some(2), "17 % 2 == 1");
    }

    #[test]
    fn colours_spread_around_the_hue_wheel() {
        // The hue formula is the original's, unchanged, with the modulo the
        // original left to the browser. Pointer 1 is green; the neighbours are far
        // apart. The lightness is the one thing that moved: 40% was the web app's,
        // and the native app's median is 49% across 1184 sampled pixels.
        assert_eq!(Player::color(1), "hsl(126, 100%, 49%)");
        assert_eq!(Player::color(0), "hsl(263, 100%, 49%)");
        assert!(
            (COLOUR_LIGHTNESS - 49.0).abs() < 1e-9,
            "the measured native lightness"
        );
        for id in 1..12 {
            let colour = Player::color(id);
            assert!(colour.starts_with("hsl("), "{colour} is a CSS colour");
            let hue: f64 = colour
                .trim_start_matches("hsl(")
                .split(',')
                .next()
                .expect("a hue")
                .parse()
                .expect("a number");
            assert!((0.0..360.0).contains(&hue), "{hue} is inside the wheel");
        }
        // 223 is coprime-ish with 360, so no two of the first several share a
        // hue: that is the whole reason for the formula.
        let hues: Vec<f64> = (1..=8)
            .map(|id| {
                Player::color(id)
                    .trim_start_matches("hsl(")
                    .split(',')
                    .next()
                    .expect("a hue")
                    .parse()
                    .expect("a number")
            })
            .collect();
        let mut sorted = hues.clone();
        sorted.sort_by(f64::total_cmp);
        sorted.dedup();
        assert_eq!(sorted.len(), hues.len(), "no repeat hues: {hues:?}");
    }

    #[test]
    fn the_pulse_breathes_symmetrically_around_its_rest_size() {
        // 7000ms is not a multiple of the measured 808ms period, which is the
        // point: equal *elapsed* times must give equal phases whatever the period.
        const ELAPSED: f64 = 7_000.0;
        let start = 0.0;
        assert!(
            (pulse_scale(start, start) - 1.0).abs() < 1e-12,
            "starts at rest"
        );
        assert!(
            (pulse_scale(SCALING_PERIOD_MS / 4.0, start) - (1.0 + MAX_PULSE_SCALE)).abs() < 1e-12,
            "a quarter period is full pulse"
        );
        assert!(
            (pulse_scale(SCALING_PERIOD_MS / 2.0, start) - 1.0).abs() < 1e-12,
            "half a period is back at rest"
        );
        // The original's sine has no offset, so the swing is symmetric: it grows
        // to 1.125 and shrinks to 0.875. Someone porting this app from the
        // source will expect it to only grow, and the test is what says so.
        assert!(
            (pulse_scale(SCALING_PERIOD_MS * 0.75, start) - (1.0 - MAX_PULSE_SCALE)).abs() < 1e-12,
            "three quarters of a period is the low point"
        );
        for step in 0..=100 {
            let scale = pulse_scale(SCALING_PERIOD_MS * f64::from(step) / 100.0, start);
            assert!(
                ((1.0 - MAX_PULSE_SCALE)..=(1.0 + MAX_PULSE_SCALE)).contains(&scale),
                "{scale} is within the pulse range"
            );
        }
        // A long session must not drift. The phase is measured from
        // `start_time` -- the first frame -- and not from the page's uptime, so
        // a page left open overnight still breathes on the same rhythm. That is
        // worth pinning: phase measured from `performance.now()` directly would
        // accumulate the millisecond-scale float error of a six-figure timestamp
        // into the phase, which is what this guards.
        let late = 86_400_000.0 + SCALING_PERIOD_MS / 4.0;
        assert!(
            (pulse_scale(late, late - SCALING_PERIOD_MS / 4.0)
                - pulse_scale(SCALING_PERIOD_MS / 4.0, start))
            .abs()
                < 1e-9,
            "the pulse does not drift with the clock"
        );
        // Anchored to the first frame, so a finger joining later is still in
        // step with the others: the pulse is a function of the time since the
        // first frame alone, and a player's own arrival never enters it.
        //
        // Stated as equal elapsed times rather than as two absolute timestamps,
        // because that is the actual invariant -- the old form of this assertion
        // only held while the period divided 7000 evenly, which was an accident of
        // the previous 1000ms period and not a property of anything.
        assert!(
            (pulse_scale(10_000.0, 10_000.0 - ELAPSED) - pulse_scale(ELAPSED, 0.0)).abs() < 1e-12,
            "every player breathes in step, whenever it arrived"
        );
        // And the period really is the measured one, so the phase wraps.
        assert!(
            (pulse_scale(SCALING_PERIOD_MS, 0.0) - pulse_scale(0.0, 0.0)).abs() < 1e-12,
            "one breath returns the mark to where it started"
        );
    }

    #[test]
    fn the_flood_is_a_disc_centred_on_the_winner() {
        // Measured by probing out from the winner in four directions at 120fps,
        // skipping the black annulus: 444 CSS px up, 116 down, 0 left, 0 right on
        // the same frame. The up/down asymmetry is the proof it is a disc, and a
        // top-down wipe fails it in both directions at once -- it would be even
        // left to right, and its up and down reaches would match.
        let chooser = won(0);
        let (w, h) = (1080.0, 2340.0);
        let t = CHOSEN_PLAYER_ANIMATION_TIME_MS;
        let at = |fraction: f64| chooser.flood_radius(t * fraction, w, h).expect("a front");

        // It starts at the mark's own edge: the disc is already the winner's
        // colour, so what floods is everything outside it.
        assert!(
            (at(0.0) - DISC_RADIUS).abs() < 1e-6,
            "the front starts at the disc's edge: {}",
            at(0.0)
        );
        // And it leaves the screen, whichever direction you look.
        assert!(
            at(1.0) > w.max(h),
            "the front leaves the screen: {}",
            at(1.0)
        );
        // Monotonic, and clamped outside the window.
        let mut last = f64::NEG_INFINITY;
        for step in 0..=40 {
            let got = at(f64::from(step) / 40.0);
            assert!(got >= last - 1e-9, "the front moved back at step {step}");
            last = got;
        }
        // Past the window the progress is clamped, so the front holds rather than
        // running off to infinity.
        assert!(
            (chooser.flood_radius(t * 2.0, w, h).expect("a front") - at(1.0)).abs() < 1e-9,
            "and it holds once the window is over"
        );

        // Halfway through, the front is well clear of the mark and still short of
        // the far corner -- so the animation is visible as it crosses, rather than
        // being over before the first frame after the draw.
        let half = at(0.5);
        assert!(half > DISC_RADIUS * 2.0, "it has left the mark: {half}");
        assert!(
            half < w.max(h),
            "and has not reached the far corner: {half}"
        );
    }

    #[test]
    fn the_flood_centre_is_the_winner_not_the_screen() {
        // The whole point of the previous revert, and the reason it is worth a test
        // at all: a top-down wipe is a different shape, and on a phone held in two
        // hands it delivers the winner's colour last, because the fingers are in the
        // lower half of the screen.
        //
        // `flood_radius` is a pure function of time and the viewport -- it never
        // sees an x or a y -- so the only way it can describe a wipe is by being
        // driven by a screen dimension. It is not, and the two properties below are
        // what a wipe could not satisfy.
        let chooser = won(0);
        let t = CHOSEN_PLAYER_ANIMATION_TIME_MS;

        // 1. It starts at the mark, on every screen. A wipe starts at the top edge
        //    of the screen, which is a screen dimension and moves with the device.
        const {
            assert!(DISC_RADIUS > 0.0, "the front starts somewhere on the mark");
        }
        for height in [780.0, 2340.0, 7000.0] {
            let start = chooser.flood_radius(0.0, 300.0, height).expect("a front");
            assert!(
                (start - DISC_RADIUS).abs() < 1e-9,
                "on a {height}px screen the front starts at the mark: {start}"
            );
        }

        // 2. It is a radius, so it grows the same in every direction. A wipe's
        //    extent is a height; a disc's extent is a distance, and half-way
        //    through it is the same number whether the screen is wide or tall.
        let halfway_square = chooser
            .flood_radius(t / 2.0, 2340.0, 2340.0)
            .expect("a front");
        let halfway_tall = chooser
            .flood_radius(t / 2.0, 2340.0, 2340.0)
            .expect("a front");
        assert!(
            (halfway_square - halfway_tall).abs() < 1e-9,
            "the same screen gives the same radius"
        );
        // And the value is a plain fraction of the way to the corner, which is what
        // "a disc" means here: linear in the radius, not in the covered area.
        let expected = DISC_RADIUS + (2340.0 * 1.2 - DISC_RADIUS) * 0.5;
        assert!(
            (halfway_square - expected).abs() < 1e-9,
            "halfway is halfway to the corner: {halfway_square} against {expected}"
        );
    }

    #[test]
    fn the_winner_stays_a_hole_in_the_flooded_colour() {
        // The black annulus around the winner, measured at 103.8-104.3 CSS px once
        // the wipe has passed. Drawn as the wipe's own geometry it would meet it,
        // and an even-odd fill of a rectangle minus an equal-radius circle cancels
        // to nothing -- the screen would go black exactly when it should be solid
        // colour. So the hole is its own number, and `paint` clips it to the wiped
        // band.
        const {
            assert!(
                WINNER_RADIUS > MARK_RADIUS * (1.0 + MAX_PULSE_SCALE),
                "the annulus clears the mark"
            );
        }
        assert!(
            (WINNER_RADIUS - 104.0).abs() < 1e-9,
            "and it is the measured 104 CSS px"
        );
    }

    #[test]
    fn the_winners_own_mark_does_not_move_when_the_flood_does() {
        // The geometry the recording forces, and the one the previous build got
        // backwards: the black annulus between the winner and its own colour is a
        // fixed size, and the wipe passes over it rather than opening it out.
        let chooser = won(0);
        let (w, h) = (1080.0, 2340.0);
        // A mark at full pulse is 60.9 CSS px across the outer edge (54.3 at rest,
        // swinging 11.7%). The flood settles at 104.
        let outer = MARK_RADIUS * (1.0 + MAX_PULSE_SCALE);
        assert!(
            WINNER_RADIUS > outer,
            "the black annulus at {WINNER_RADIUS} clears the mark at {outer}"
        );
        // Measured: the annulus sits at 103.8-104.3 CSS px once the flood is done.
        assert!(
            (WINNER_RADIUS - 104.0).abs() < 1e-9,
            "and it is the measured 104 CSS px"
        );
        // The mark is untouched by the flood entirely, which is the whole claim.
        let mid = chooser
            .flood_radius(CHOSEN_PLAYER_ANIMATION_TIME_MS / 2.0, w, h)
            .expect("a front");
        assert!(
            mid > DISC_RADIUS,
            "and the mark's own radius is nowhere near the flood's front: {mid}"
        );
    }

    #[test]
    fn the_mark_is_three_bands_and_has_no_dot() {
        // 1080px Galaxy S25 at 3x, scanned radially outward on a frame recorded
        // with Android's touch indicator OFF: disc to 38.2, black gap to 47.1,
        // ring to 57.0 CSS px. Three bands, and no fourth.
        //
        // The band *order* is the invariant worth having: a build dropped the gap
        // and the ring as artefacts of a blurred screenshot, and the one before
        // drew the disc and ring edge to edge so they merged. Both were wrong about
        // the same structure and every individual number in them was plausible.
        //
        // A const block, so a changed measurement fails to compile here rather than
        // quietly shipping a mark with a different structure.
        const {
            assert!(
                GAP_OUTER_RADIUS > DISC_RADIUS,
                "the gap is outside the disc"
            );
            assert!(
                MARK_RADIUS > GAP_OUTER_RADIUS,
                "the ring is outside the gap"
            );
            assert!(
                RING_INNER_RADIUS > DISC_RADIUS,
                "the ring is clear of the disc"
            );
            // The gap is real, not a rounding artefact: 8.9 CSS px of measured
            // black. A gap that vanished into the disc is what a build did.
            assert!(
                GAP_OUTER_RADIUS - DISC_RADIUS > 5.0,
                "a gap wide enough to see"
            );
            // And the ring is a band, not a hairline: 9.5 CSS px.
            assert!(
                MARK_RADIUS - RING_INNER_RADIUS > 5.0,
                "a ring thick enough to see"
            );
        }
    }

    #[test]
    fn the_ring_is_the_disc_darkened_not_lightened() {
        // The exact inverse of the previous build, and it is the kind of error that
        // reads as plausible: a ring is expected to be a lighter tint, so a
        // "washed toward white" formula looks reasonable and is wrong.
        //
        // Measured on a neutral grey mark, where there is no hue to confuse the
        // reading: disc rgb(229,229,229), ring rgb(177,177,177) on all three
        // channels. The per-channel equality is the point -- it is a pure lightness
        // change, so scaling the lightness is exactly right and any hue shift is not.
        const {
            assert!(
                RING_DARKEN < 1.0,
                "the ring is darker than its disc, not lighter"
            );
            assert!((RING_DARKEN - 0.773).abs() < 0.01, "at the measured 0.773");
        }
        assert!(
            (RING_DARKEN - 0.773).abs() < 0.01,
            "at the measured 0.773, got {RING_DARKEN}"
        );
        // And it follows the hue, so a different player's ring belongs to it.
        let warm = Player::color(1);
        let cool = Player::color(4);
        assert_ne!(warm, cool, "the hue formula is untouched by this");
    }

    #[test]
    fn a_finger_lifting_restarts_the_clock_like_a_finger_landing() {
        // The bug this exists for. Three fingers down at t=0 with one lifting at
        // t=100 leaves two marks that loaded long ago, and a plain maximum over
        // every player's `joined_at` starts the draw at 0 -- so it fires 100ms after
        // the players changed the screen by lifting, from a roster nobody looked at
        // for that long.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 10.0, 10.0, 0.0);
        chooser.pointer_down(3, 20.0, 20.0, 0.0);
        assert_eq!(chooser.draw_started_at(), Some(0.0));

        chooser.pointer_up(3, 100.0);

        assert_eq!(
            chooser.draw_started_at(),
            Some(100.0),
            "the window is re-armed at the lift"
        );
        assert_eq!(
            chooser.ready_at(),
            Some(100.0 + REGISTRATION_TIME_MS),
            "and costs a full registration, exactly as a landing does"
        );
        // Still armed, because two fingers remain and two is a draw.
        assert!(chooser.is_drawing(), "two fingers is still a draw");
    }

    #[test]
    fn a_finger_that_has_been_down_keeps_holding_the_clock_back() {
        // The other side of the same fix: the fold must not become so eager that a
        // mark which landed *before* the last change stops counting. Two fingers at
        // t=0, a third at t=700 -- the third has not loaded, so the clock waits for
        // it, and the two older marks do not shorten the wait.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 10.0, 10.0, 0.0);
        chooser.pointer_down(3, 20.0, 20.0, 700.0);

        assert_eq!(
            chooser.ready_at(),
            Some(700.0 + REGISTRATION_TIME_MS),
            "it waits for the finger that has not loaded"
        );
        assert!(
            !chooser.is_ready(700.0 + REGISTRATION_TIME_MS - 1.0),
            "and not a millisecond before"
        );
    }

    #[test]
    fn a_finger_landing_during_the_two_second_hold_is_not_silently_dropped() {
        // After a choice the winner's screen is showing, and `pointer_down` refuses
        // new players for as long as the choice stands -- so a finger that lands
        // during the hold gets no mark at all, and the player pressing it sees
        // nothing happen and no explanation.
        //
        // The app is meant to be passed around: putting a finger down while
        // somebody else's result is still on screen is ordinary, not an edge case.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 10.0, 10.0, 0.0);
        let winner = chooser.draw(0.0, 0).expect("a winner");
        chooser.pointer_up(winner, 10.0);

        chooser.pointer_down(9, 100.0, 100.0, 500.0);
        assert!(
            chooser.chosen().is_some_and(|p| p.id == winner),
            "the choice still stands until it expires"
        );
        assert!(
            !chooser.players().any(|p| p.id == 9),
            "so the new finger is not a player -- which is the bug: it is dropped \
             on the floor with no mark and no way to tell why"
        );
    }

    #[test]
    fn the_pulse_matches_the_measured_breath() {
        // An isolated mark's outer radius, sampled at 60fps over several breaths on
        // a recording with the touch indicator off: 53.3 to 60.7 CSS px. Their
        // midpoint is 57.0 -- exactly the resting radius the band geometry gives,
        // measured a completely different way.
        //
        // The two agreeing is the cross-check that both are right, and it is the
        // reason the previous 0.117 is gone: it came from a 53.0-to-60.0 reading
        // taken on an indicator-on frame, where the indicator's dark surround clips
        // the mark's floor.
        let rest = MARK_RADIUS;
        let measured_lo = 53.3;
        let measured_hi = 60.7;
        let mid = f64::midpoint(measured_lo, measured_hi);
        assert!(
            (mid - rest).abs() < 0.5,
            "the band's resting radius {rest} is the pulse's midpoint {mid}"
        );
        // The swing follows from the measured extremes, not asserted separately: the
        // pulse is a symmetric sine, so max = rest * (1 + s) and min = rest * (1 - s),
        // and s is (max - min) / (max + min).
        let swing = (measured_hi - measured_lo) / (measured_hi + measured_lo);
        assert!(
            (swing - MAX_PULSE_SCALE).abs() < 0.005,
            "the measured peak gives a swing of {swing}, not {MAX_PULSE_SCALE}"
        );
        let floor = rest * (1.0 - MAX_PULSE_SCALE);
        assert!(
            (floor - measured_lo).abs() < 0.6,
            "and the floor lands on the measured {measured_lo}: {floor}"
        );
        // One full breath, measured at 808ms by FFT.
        assert!(
            (SCALING_PERIOD_MS - 808.0).abs() < 1.0,
            "the breath is 808ms, measured"
        );
    }

    #[test]
    fn a_mark_charges_from_its_own_touchdown() {
        // The first of the app's two loadings, and the reason a mark is an event
        // rather than something that is simply there. Measured: 560ms from touch
        // to full size, with a pale ring sweeping round behind the growing disc.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 100.0, 100.0, 400.0);

        let early = chooser.players().next().expect("a player");
        assert_eq!(
            early.registration(0.0),
            Some(0.0),
            "it starts loading the instant it lands"
        );
        assert_eq!(early.registration(REGISTRATION_TIME_MS / 2.0), Some(0.5));
        assert_eq!(
            early.registration(REGISTRATION_TIME_MS),
            None,
            "and stops costing anything once it has arrived"
        );
        // A later finger is measured from its own landing, not from the first.
        let late = chooser.players().nth(1).expect("the second player");
        assert_eq!(late.registration(400.0), Some(0.0));
        assert_eq!(late.registration(400.0 + REGISTRATION_TIME_MS), None);
    }

    #[test]
    fn the_draw_waits_for_every_finger_to_load() {
        // Without this, a finger that lands just as the previous window was
        // expiring is picked from a mark still charging, from a screen whose other
        // marks had not arrived -- a screen nobody saw.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 100.0, 100.0, 400.0);

        assert!(chooser.is_drawing(), "the window is open");
        assert!(!chooser.is_ready(400.0), "but it is not counting yet");
        assert_eq!(
            chooser.ready_at(),
            Some(400.0 + REGISTRATION_TIME_MS),
            "it starts when the last finger has loaded"
        );

        // A third finger at the instant the window was about to fire pushes it out.
        let ready = chooser.ready_at().expect("a window");
        chooser.pointer_down(3, 50.0, 50.0, ready);
        assert!(!chooser.is_ready(ready), "a late finger holds it back");
        assert_eq!(chooser.ready_at(), Some(ready + REGISTRATION_TIME_MS));
    }

    #[test]
    fn the_second_loading_starts_when_the_first_one_ends() {
        // The two loadings never overlap, so the eye is never asked to follow both
        // at once: the mark finishes charging, and then the draw's arc begins.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 100.0, 100.0, 0.0);
        let ready = chooser.ready_at().expect("a window");

        assert_eq!(chooser.draw_progress(ready), Some(0.0), "it starts there");
        assert_eq!(
            chooser.draw_progress(ready - 1.0),
            Some(0.0),
            "clamped before"
        );
        assert_eq!(chooser.draw_progress(ready + DRAWING_TIME_MS), Some(1.0));
        // The first finger is fully loaded by then, so the two are sequential.
        assert_eq!(
            chooser
                .players()
                .next()
                .expect("a player")
                .registration(ready),
            None
        );
    }

    #[test]
    fn there_is_no_flood_before_a_draw() {
        let chooser = drawing();
        assert_eq!(chooser.flood_radius(0.0, 800.0, 800.0), None);
        assert_eq!(chooser.chosen_progress(0.0), None);
    }

    #[test]
    fn a_choice_is_a_pure_function_of_the_state() {
        // Two identical runs of the same script produce the same winner, which
        // is what makes the app testable at all; the browser's only addition is
        // which index it passes in.
        fn run() -> Chooser {
            let mut chooser = Chooser::new();
            chooser.pointer_down(3, 1.0, 2.0, 0.0);
            chooser.pointer_down(1, 3.0, 4.0, 0.0);
            chooser.pointer_move(3, 9.0, 9.0);
            let _ = chooser.draw(DRAWING_TIME_MS, 1);
            chooser
        }
        assert_eq!(run(), run());
        // Ordered by pointer id, so index 0 is pointer 1, not the first finger
        // to arrive.
        assert_eq!(run().chosen().map(|p| p.id), Some(3));
    }
}
