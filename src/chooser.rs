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
/// Radius of the solid inner disc.
pub const INNER_RADIUS: f64 = 36.0;
/// Gap between the inner disc and the outer ring's centreline.
pub const OUTER_RADIUS: f64 = 16.0;
/// Stroke width of the outer ring.
pub const OUTER_CIRCLE_WIDTH: f64 = 12.0;
/// How far a player circle's radius swings, in each direction, around its rest
/// size.
///
/// The pulse is `1 + MAX_PULSE_SCALE * sin(...)`, so the swing is symmetric: the
/// circle grows to 1.125 and shrinks to 0.875. Growing and shrinking is what
/// makes it read as a breath rather than a throb, and it is the author's own
/// formula. Anyone porting this app from the source will half-expect it to only
/// grow -- `the_pulse_breathes_symmetrically_around_its_rest_size` is what says
/// otherwise.
pub const MAX_PULSE_SCALE: f64 = 0.125;
/// How long a draw window lasts once two players are present.
pub const DRAWING_TIME_MS: f64 = 2500.0;
/// How long the winner's circle takes to expand across the screen.
pub const CHOSEN_PLAYER_ANIMATION_TIME_MS: f64 = 1000.0;
/// One full breath of a player circle's pulse.
pub const SCALING_PERIOD_MS: f64 = 1500.0;
/// Clearance between the winner's ring and the edge of the filled screen.
pub const CHOSEN_SEPARATION: f64 = 8.0;
/// How long the chosen finger must be off the glass before the app resets.
pub const RESTART_DELAY: f64 = 2000.0;

/// Radius of the winner's circle when its expansion has finished.
///
/// The original's own constant, carried over arithmetic for arithmetic: 74.25.
/// Its evident intent holds exactly. The winner's ring is stroked at a
/// centreline radius of 52 with a 12px stroke, so its outer edge is 58, which
/// the pulse swings to 65.25 at its largest -- and the fill stops at 74.25,
/// leaving precisely the 8px `CHOSEN_SEPARATION` the author asked for at the
/// tightest point of the breath, and 16.25 at rest. The winner is a hole in the
/// colour and never touches it.
///
/// It is kept rather than "corrected" because a rewrite keeps the author's
/// numbers, and here they were right. `the_winner_radius_clears_the_winners_own_ring`
/// pins all of it.
pub const MIN_WINNER_RADIUS: f64 = (INNER_RADIUS
    + OUTER_RADIUS
    + OUTER_CIRCLE_WIDTH / 2.0
    + CHOSEN_SEPARATION)
    * (1.0 + MAX_PULSE_SCALE);

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
    /// later frame -- the winner's circle expands from the moment of the draw,
    /// so reading a fresh timestamp each frame would restart the animation.
    pub chosen_at: Option<f64>,
}

impl Player {
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
    pub fn color(id: i32) -> String {
        let hue = (f64::from(id) * 223.0 + 263.0).rem_euclid(360.0);
        format!("hsl({hue:.0}, 100%, 40%)")
    }

    /// The CSS colour this player is drawn in.
    pub fn color_of(&self) -> String {
        Self::color(self.id)
    }

    /// The player's pulse multiplier at `timestamp`.
    ///
    /// A sine over [`SCALING_PERIOD_MS`], anchored at `start_time` so every
    /// player breathes in step.
    pub fn pulse_scale(&self, timestamp: f64, start_time: f64) -> f64 {
        pulse_scale(timestamp, start_time)
    }
}

/// The pulse multiplier shared by every player at `timestamp`.
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
    pub fn new() -> Self {
        Self::default()
    }

    /// The players currently present, in pointer-id order.
    pub fn players(&self) -> impl Iterator<Item = &Player> {
        self.players.values()
    }

    /// How many fingers are down.
    pub fn len(&self) -> usize {
        self.players.len()
    }

    /// Whether the glass is untouched.
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// The chosen player, if one has been chosen.
    pub fn chosen(&self) -> Option<&Player> {
        self.chosen.and_then(|id| self.players.get(&id))
    }

    /// Whether a winner has been chosen and has not yet been cleared.
    pub fn is_chosen(&self) -> bool {
        self.chosen.is_some()
    }

    /// When the current draw window opened, if one is running.
    ///
    /// The white arc on each player's ring is this timestamp's progress
    /// through [`DRAWING_TIME_MS`]; `None` means no arc at all.
    pub fn draw_started_at(&self) -> Option<f64> {
        self.draw_started_at
    }

    /// Whether a draw window is running.
    ///
    /// Exactly the original's `started_timeout`: set when the timer is armed,
    /// cleared the moment it fires -- which is what stops a pointer event
    /// arriving in the same frame as the draw from starting a second one.
    pub fn is_drawing(&self) -> bool {
        self.draw_started_at.is_some()
    }

    /// The white loading arc's progress at `timestamp`, 0 to 1.
    ///
    /// `None` while no draw window is running, which draws no arc at all.
    pub fn draw_progress(&self, timestamp: f64) -> Option<f64> {
        let started = self.draw_started_at?;
        Some(((timestamp - started) / DRAWING_TIME_MS).clamp(0.0, 1.0))
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

    /// How far the winner's circle has grown at `timestamp`, 0 to 1.
    ///
    /// Anchored to the instant of the draw, not to this frame, so the
    /// expansion does not restart on every render.
    pub fn chosen_progress(&self, timestamp: f64) -> Option<f64> {
        let chosen_at = self.chosen()?.chosen_at?;
        Some(
            ((timestamp - chosen_at) / CHOSEN_PLAYER_ANIMATION_TIME_MS).clamp(0.0, 1.0),
        )
    }

    /// The winner's circle radius at `timestamp`, given the viewport size.
    ///
    /// Grows from zero to [`MIN_WINNER_RADIUS`] over the animation, starting
    /// from the longest screen dimension so the fill sweeps in from off-screen
    /// and the win reads as the screen being claimed rather than a circle
    /// swelling.
    pub fn chosen_radius(&self, timestamp: f64, width: f64, height: f64) -> Option<f64> {
        let progress = self.chosen_progress(timestamp)?;
        let from = width.max(height).max(MIN_WINNER_RADIUS);
        Some(progress * MIN_WINNER_RADIUS + (1.0 - progress) * from)
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
    fn a_second_finger_starts_the_draw() {
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 50.0, 50.0, 40.0);

        assert!(chooser.is_drawing());
        assert_eq!(chooser.draw_started_at(), Some(40.0));
        assert_eq!(chooser.draw_progress(40.0), Some(0.0));
        assert_eq!(chooser.draw_progress(40.0 + DRAWING_TIME_MS / 2.0), Some(0.5));
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
            assert_eq!(chooser.draw(2500.0, index), Some(index as i32 + 1));
        }
    }

    #[test]
    fn the_winner_is_anchored_to_the_instant_of_the_draw() {
        let mut chooser = drawing();
        chooser.draw(2500.0, 0);

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
        chooser.tick(3000.0 + RESTART_DELAY + 1.0);

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
        chooser.draw(2500.0, 0);

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
        // hsl() of the hue the original computed, with the modulo the original
        // left to the browser. Pointer 1 is green; the neighbours are far apart.
        assert_eq!(Player::color(1), "hsl(126, 100%, 40%)");
        assert_eq!(Player::color(0), "hsl(263, 100%, 40%)");
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
        sorted.sort_by(|a, b| a.total_cmp(b));
        sorted.dedup();
        assert_eq!(sorted.len(), hues.len(), "no repeat hues: {hues:?}");
    }

    #[test]
    fn the_pulse_breathes_symmetrically_around_its_rest_size() {
        let start = 0.0;
        assert!((pulse_scale(start, start) - 1.0).abs() < 1e-12, "starts at rest");
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
            let scale = pulse_scale(SCALING_PERIOD_MS * step as f64 / 100.0, start);
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
        // step with the others.
        assert!(
            (pulse_scale(10_000.0, 3_000.0) - pulse_scale(1_000.0, 0.0)).abs() < 1e-12,
            "every player breathes in step, whenever it arrived"
        );
    }

    #[test]
    fn the_winner_radius_starts_off_screen_and_settles_at_the_minimum() {
        let mut chooser = drawing();
        chooser.draw(0.0, 0);

        let (width, height) = (800.0, 1600.0);
        assert_eq!(chooser.chosen_radius(0.0, width, height), Some(1600.0));
        assert_eq!(
            chooser.chosen_radius(CHOSEN_PLAYER_ANIMATION_TIME_MS, width, height),
            Some(MIN_WINNER_RADIUS)
        );
        assert_eq!(
            chooser.chosen_radius(CHOSEN_PLAYER_ANIMATION_TIME_MS / 2.0, width, height),
            Some((MIN_WINNER_RADIUS + 1600.0) / 2.0)
        );
        assert_eq!(chooser.chosen_radius(0.0, width, height), Some(1600.0));
    }

    #[test]
    fn the_winner_radius_never_collapses_on_a_tiny_screen() {
        // The starting radius is at least `MIN_WINNER_RADIUS`, so a very small
        // viewport does not make the circle shrink as it grows in.
        let mut chooser = drawing();
        chooser.draw(0.0, 0);
        assert_eq!(
            chooser.chosen_radius(0.0, 10.0, 10.0),
            Some(MIN_WINNER_RADIUS)
        );
    }

    #[test]
    fn there_is_no_winner_radius_before_a_draw() {
        let chooser = drawing();
        assert_eq!(chooser.chosen_radius(0.0, 800.0, 800.0), None);
        assert_eq!(chooser.chosen_progress(0.0), None);
    }

    #[test]
    fn the_winner_radius_clears_the_winners_own_ring() {
        // The one geometric claim the constant exists for: at full pulse the
        // winner's ring sits strictly inside the finished fill, so the winner
        // reads as a hole in the colour rather than a ring painted over it.
        // The ring is stroked at a *centreline* radius of `INNER + OUTER`, half
        // the stroke width to either side, so its outer edge is 6px further out:
        // 58 at rest.
        let centreline = INNER_RADIUS + OUTER_RADIUS;
        let outer_edge = centreline + OUTER_CIRCLE_WIDTH / 2.0;
        assert_eq!(outer_edge, 58.0);
        assert_eq!(MIN_WINNER_RADIUS, 74.25);

        // Clearance beyond the ring's outer edge at the pulse's lowest point --
        // which is where the pulse is smallest and the ring is closest to the
        // edge of the fill -- and a good deal more at the top of the swing.
        assert_eq!(MIN_WINNER_RADIUS - outer_edge, 16.25, "at rest");
        assert_eq!(
            MIN_WINNER_RADIUS - outer_edge * (1.0 + MAX_PULSE_SCALE),
            CHOSEN_SEPARATION * (1.0 + MAX_PULSE_SCALE),
            "and the original's constant is dimensioned so the winner's ring clears \
             the fill by exactly CHOSEN_SEPARATION at the top of the pulse -- the \
             tightest it is ever, and still a gap"
        );
        assert_eq!(MIN_WINNER_RADIUS - outer_edge * (1.0 + MAX_PULSE_SCALE), 9.0);
        // The winner's inner disc is always well inside the fill, at either end
        // of the pulse, so the winner stays visible once the fill has settled.
        // A compile-time check: this is arithmetic on constants, and a constant
        // that stopped holding would be a change to what the app looks like.
        const { assert!(INNER_RADIUS * (1.0 - MAX_PULSE_SCALE) < MIN_WINNER_RADIUS) };
        assert_eq!(MIN_WINNER_RADIUS, 74.25, "the original's own constant");
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
            chooser.draw(DRAWING_TIME_MS, 1);
            chooser
        }
        assert_eq!(run(), run());
        // Ordered by pointer id, so index 0 is pointer 1, not the first finger
        // to arrive.
        assert_eq!(run().chosen().map(|p| p.id), Some(3));
    }
}
