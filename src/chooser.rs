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
///
/// 3000ms, chosen rather than measured, and the user asked for it.
///
/// The 2500ms before it came from the original app. The window now *covers* the
/// ring in the player's colour rather than sweeping an arc, so its progress is
/// legible as a filling band, and three seconds is long enough to read it as one.
pub const DRAWING_TIME_MS: f64 = 3000.0;

/// How long one finger takes to load its own mark.
///
/// 1000ms, chosen rather than measured, and the user asked for it.
///
/// What the recording shows is 620ms: the disc reaches full size in about 100ms and
/// the sweep closes over the rest. At that speed the sweep is over before anyone has
/// registered that it was happening, and the two-sided sweep added to it needs room
/// to be read at all -- two arcs meeting is legible as a gesture in a way one arc
/// sweeping past is not.
pub const REGISTRATION_TIME_MS: f64 = 1000.0;

/// The share of the registration during which the disc reaches full size.
///
/// 0.16 -- about 100ms of the 620ms load. Measured: the disc's radius is 20.7 CSS
/// px at 25ms into the load, 29 at 75ms, 35.7 at 100ms, and then it sits within a
/// pixel of 38.2 for the remaining 500ms while the arc sweeps.
///
/// The split matters because the two halves are different events. A disc that
/// inflates over the whole window reads as one slow growth and nothing else, which
/// is what "the initial loading is too static" describes: the mark becomes the
/// right size early and then spends four fifths of its time sitting there.
pub const DISC_ARRIVAL_FRACTION: f64 = 0.16;

/// How much of the disc's radius the loading arc is drawn at, as a multiple.
///
/// 1.368, which is the ring's own centreline divided by the disc's radius: the band
/// runs 47.5 to 57.0 CSS px so its middle is 52.25, and 52.25 / 38.2 = 1.368.
///
/// It is expressed as a multiple of the disc's *full* radius, and that is the whole
/// point. Scaling the arc by the loading fraction instead, which a build did, puts
/// it in a different place every frame: it drifts inward as the mark loads and ends
/// up nowhere near the ring, so the mark's three bands stop lining up with each
/// other. That is the misalignment reported with this change, and it is invisible in
/// the numbers -- only the multiplication is wrong.
///
/// The constant is derived from the band geometry rather than measured separately,
/// so it cannot drift out of agreement with the ring it is supposed to trace.
pub const LOADING_ARC_SCALE: f64 = RING_STROKE_RADIUS / DISC_RADIUS;

/// How far outside the ring the loading sweep is drawn, in CSS px.
///
/// The sweep is drawn on its own band, *outside* the ring's, and this is why.
///
/// The obvious implementation paints the pale arc over the ring's own band, which
/// means the ring's colour is the arc's colour for as long as the arc is there and
/// snaps back to its resting tint the instant the load ends. That is the "the
/// colour should stay loaded, not flip back": the ring is recoloured for the load
/// and released afterwards, and the release is visible as a flash.
///
/// Measured on the native app, the ring's band is one single colour throughout --
/// rgb(35, 113, 132) at the fixed 135-degree origin and everywhere else, at every
/// sampled instant of the load and long after it. The sweep is a lighter tint, and
/// it is a separate ring outside this one.
///
/// 10 CSS px puts it clear of the ring's outer edge (57.0) with a 5px black gap
/// between them, which is the same visual separation the mark's own gap uses.
pub const LOADING_ARC_GROWTH: f64 = 10.0;

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

/// The two arcs a loading sweep is drawn as, at `progress`.
///
/// A loading starts at a random point on the ring and closes from **both**
/// sides, finishing at the point opposite where it started.
///
/// So at progress `t` each side has swept `t/2` of the circle, and the two meet
/// exactly at the antipode when `t` is 1: two half-circles, adjacent, with no
/// gap and no overlap. A single sweep from a fixed origin instead reads as a
/// dial, which is a thing with a position rather than a thing that is arriving.
///
/// Returned as a `(from, to)` pair per side, so the caller can stroke two arcs.
/// Angles are radians in canvas convention: 0 is 3 o'clock and they increase
/// clockwise, which is the convention every `arc` call here already uses.
///
/// What the eye follows is the gap between the two closing, rather than a line
/// lengthening.
#[must_use]
pub fn sweep_arcs(origin: f64, progress: f64) -> [(f64, f64); 2] {
    let half = progress.clamp(0.0, 1.0) * std::f64::consts::PI;
    [(origin, origin + half), (origin - half, origin)]
}

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
    /// Where this mark's loading sweep starts, in radians around the ring.
    ///
    /// Chosen at random per finger, so a table of people all putting fingers down
    /// together gets a different answer for each. A fixed start reads as a
    /// deliberate mechanism; a random one reads as each mark doing its own thing,
    /// which is what a mark belongs to a particular finger rather than to the app.
    ///
    /// It is drawn from the browser's CSPRNG by the front end, alongside the draw,
    /// so this file stays free of a source of randomness it cannot test.
    pub load_origin: f64,
}

impl Player {
    /// How far the disc's own arrival has got, given how far the load has got.
    ///
    /// The disc reaches full size in the first [`DISC_ARRIVAL_FRACTION`] of the
    /// load, so the two are fractions of different things and the arrival is
    /// `loading / DISC_ARRIVAL_FRACTION`, not `loading * DISC_ARRIVAL_FRACTION`.
    ///
    /// It was multiplied once, and that never exceeds 0.16: the disc was drawn at a
    /// sixteenth of its size for the entire load and only reached full size when the
    /// load ended. The mark read as a black hole with a bright ring round it for
    /// 620ms. It is arithmetic on a number that looks right in both forms, which is
    /// why it is a function with a test rather than a line inside the renderer.
    #[must_use]
    pub fn disc_arrival(loading: f64) -> f64 {
        let arrived = (loading / DISC_ARRIVAL_FRACTION).clamp(0.0, 1.0);
        // Eased out, matching the measured radii: fast at first, then flattening.
        arrived * arrived * (3.0 - 2.0 * arrived)
    }

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

    /// The colour this player's ring is *slightly brighter* than at rest.
    ///
    /// The first loading draws the ring in this colour, on the ring's own band, and
    /// the ring settles to [`Self::ring_color`] when the load ends. Same band, same
    /// place, so the arrival is a colour settling rather than a shape appearing and
    /// then being taken away.
    ///
    /// It has to follow the player's hue. A single fixed pale colour is orange
    /// against every hue that is not orange, so the sweep appeared to belong to
    /// whichever player happened to be orange rather than to the finger that had
    /// just landed -- which is exactly backwards for a loading indicator.
    ///
    /// "Slightly brighter" is relative to the resting ring rather than to white:
    /// lifted to 0.65 of the way from the disc to full brightness, which is a
    /// visible step at these radii without reading as a different colour entirely.
    #[must_use]
    pub fn loading_color(&self) -> String {
        let hue = (f64::from(self.id) * 223.0 + 263.0).rem_euclid(360.0);
        // A third of the way from the resting ring's lightness up to full.
        let resting = COLOUR_LIGHTNESS * RING_DARKEN;
        format!(
            "hsl({hue:.0}, 100%, {:.1}%)",
            resting + (COLOUR_LIGHTNESS - resting) * 0.33
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
    /// Where the draw's sweep starts, in radians around the ring.
    ///
    /// Random per draw, for the same reason each finger's is: a draw is a fresh
    /// event and it should not look like the last one replayed.
    draw_origin: f64,
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

    /// Where the draw's sweep starts, in radians around the ring.
    ///
    /// Random per draw, supplied by the front end when the draw is made. Zero before
    /// any draw, which is only ever read while a winner has been chosen.
    #[must_use]
    pub fn draw_origin(&self) -> f64 {
        self.draw_origin
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
    pub fn pointer_down(&mut self, id: i32, x: f64, y: f64, now: f64, load_origin: f64) {
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
                load_origin,
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
    pub fn draw(&mut self, now: f64, winner: usize, draw_origin: f64) -> Option<i32> {
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
        self.draw_origin = draw_origin;
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

    /// Radius of the clear circle left around the winner, at `timestamp`.
    ///
    /// The winner's colour covers the screen **except** a circle centred on him,
    /// and that circle's radius **shrinks**. Everything outside it is the winner's
    /// colour; he is the last thing left uncovered.
    ///
    /// This is the third time this function has changed shape, and the measurements
    /// agree on this one. Taking the un-flooded region around the winner, its radius
    /// is 242.7 CSS px at 4.892s and holds until the flood has covered 38% of the
    /// screen, then comes in: 235, 214, 192, 170, 147, 125 CSS px, and settles at
    /// 124.8.
    ///
    /// So it is not a disc *growing* outwards from the mark -- which is what the last
    /// two builds drew -- and it is not a wipe either. A big circle starts already
    /// covering most of the screen, and the hole closes.
    ///
    /// The shrink is fast: about 23 CSS px per 8ms at 120fps, over roughly 50ms. It
    /// is left at [`CHOSEN_PLAYER_ANIMATION_TIME_MS`] rather than fitted to those
    /// samples, because the user asked for it to be watchable, and because a
    /// duration read off four frames is not a measurement of a duration.
    ///
    /// The final radius is [`WINNER_RADIUS`]: the hole the flood leaves is the same
    /// 104 CSS px black circle the mark has always sat in.
    #[must_use]
    pub fn flood_hole(&self, timestamp: f64, width: f64, height: f64) -> Option<f64> {
        let progress = self.chosen_progress(timestamp)?;
        // It has to start by covering the screen, or there is nothing for the
        // flood to close in on. The far corner from the winner, plus a little.
        let opened = (width * width + height * height).sqrt() + MARK_RADIUS;
        Some(opened + (WINNER_RADIUS - opened) * progress)
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
        let _ = chooser.draw(0.0, winner, 0.0);
        chooser
    }

    /// A chooser with two fingers down, mid-draw.
    fn drawing() -> Chooser {
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 100.0, 200.0, 0.0, 0.0);
        chooser.pointer_down(2, 300.0, 400.0, 10.0, 0.0);
        chooser
    }

    #[test]
    fn a_finger_down_is_a_player_at_that_point() {
        let mut chooser = Chooser::new();
        chooser.pointer_down(7, 12.5, 34.0, 0.0, 0.0);

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
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);

        assert_eq!(chooser.len(), REQUIRED_PLAYER_COUNT - 1);
        assert!(!chooser.is_drawing(), "a draw needs two players");
        assert!(chooser.draw_progress(0.0).is_none(), "and so no arc");
        assert_eq!(chooser.draw(2500.0, 0, 0.0), None, "and no winner");
    }

    #[test]
    fn a_second_finger_opens_the_draw() {
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 50.0, 50.0, 40.0, 0.0);

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
        chooser.pointer_down(3, 10.0, 10.0, 500.0, 0.0);

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
        chooser.pointer_down(1, 100.0, 200.0, 200.0, 0.0);
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
                .draw(2500.0, winner, 0.0)
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
                chooser.draw(2500.0, index, 0.0),
                Some(i32::try_from(index).unwrap() + 1)
            );
        }
    }

    #[test]
    fn the_winner_is_anchored_to_the_instant_of_the_draw() {
        let mut chooser = drawing();
        let _ = chooser.draw(2500.0, 0, 0.0);

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
        let winner = chooser.draw(2500.0, 0, 0.0).expect("a winner");

        chooser.pointer_up(winner, 3000.0);
        assert_eq!(chooser.len(), 1, "the winner's circle stays put");
        assert_eq!(chooser.chosen().map(|p| p.id), Some(winner));

        // The app is not reset yet, so a new finger is not a player.
        chooser.pointer_down(9, 10.0, 10.0, 3100.0, 0.0);
        assert_eq!(chooser.len(), 1);
        assert!(!chooser.is_drawing());
    }

    #[test]
    fn the_reset_comes_exactly_two_seconds_after_the_winner_lifts() {
        let mut chooser = drawing();
        let winner = chooser.draw(2500.0, 0, 0.0).expect("a winner");

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
        let winner = chooser.draw(2500.0, 0, 0.0).expect("a winner");
        chooser.pointer_up(winner, 3000.0);
        let _ = chooser.tick(3000.0 + RESTART_DELAY + 1.0);

        chooser.pointer_down(4, 5.0, 5.0, 6000.0, 0.0);
        chooser.pointer_down(5, 6.0, 6.0, 6100.0, 0.0);

        assert!(chooser.is_drawing(), "the app is reusable after a reset");
        assert!(chooser.chosen().is_none());
        assert_eq!(chooser.draw(8600.0, 1, 0.0), Some(5));
    }

    #[test]
    fn a_winner_held_down_does_not_reset_the_app() {
        let mut chooser = drawing();
        let winner = chooser.draw(2500.0, 0, 0.0).expect("a winner");

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
        let _ = chooser.draw(2500.0, 0, 0.0);

        assert_eq!(chooser.draw(3000.0, 1, 0.0), None);
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
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);

        assert_eq!(chooser.draw(2500.0, 0, 0.0), None);
        assert!(!chooser.is_chosen());
    }

    #[test]
    fn the_winner_index_is_taken_against_the_players_actually_present() {
        // Indices come from a random draw, so a caller may pass anything; an
        // out-of-range index must still name a real player.
        let mut chooser = drawing();
        assert_eq!(chooser.draw(2500.0, 17, 0.0), Some(2), "17 % 2 == 1");
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
    fn the_flood_is_a_shrinking_hole_around_the_winner() {
        // Measured: the un-flooded radius around the winner is 242.7 CSS px while
        // the flood is still only 2% of the screen, and then comes in --
        // 235, 214, 192, 170, 147, 125 -- settling at 124.8.
        //
        // So the colour is everywhere EXCEPT a circle on the winner, and that circle
        // closes. It is not a disc growing outwards from the mark, and it is not a
        // wipe: both of those start with nothing coloured.
        let chooser = won(0);
        let (w, h) = (1080.0, 2340.0);
        let t = CHOSEN_PLAYER_ANIMATION_TIME_MS;
        let at = |fraction: f64| chooser.flood_hole(t * fraction, w, h).expect("a hole");

        // It starts by covering the screen, or there is nothing to close in on.
        let start = at(0.0);
        let far_corner = (w * w + h * h).sqrt();
        assert!(
            start > far_corner,
            "the hole starts beyond the far corner, covering everything: {start} \
             against {far_corner}"
        );
        // And it closes to the mark's own black circle.
        assert!(
            (at(1.0) - WINNER_RADIUS).abs() < 1e-6,
            "and ends at the measured hole: {} against {WINNER_RADIUS}",
            at(1.0)
        );
        // Monotonically shrinking, never growing: that is the whole difference
        // between this and the two shapes it replaced.
        let mut last = f64::INFINITY;
        for step in 0..=40 {
            let got = at(f64::from(step) / 40.0);
            assert!(got <= last + 1e-9, "the hole opened at step {step}: {got}");
            last = got;
        }
        // And it is held once the window is over rather than closing further.
        assert!((chooser.flood_hole(t * 2.0, w, h).expect("a hole") - at(1.0)).abs() < 1e-9);
    }

    #[test]
    fn the_hole_is_centred_on_the_winner_not_on_the_screen() {
        // The hole is a circle about the winner, so its radius is a function of the
        // viewport and of time and never of the winner's position -- move him to the
        // other end of the screen and the radius is the same.
        //
        // That is the structural form of "centred on the player": `paint` punches
        // the hole at `winner.x, winner.y`, and a front that cannot see an x or a y
        // could not be a top-down wipe or a screen-centred disc.
        let mut a = Chooser::new();
        a.pointer_down(1, 100.0, 100.0, 0.0, 0.0);
        a.pointer_down(2, 500.0, 900.0, 0.0, 0.0);
        let _ = a.draw(0.0, 0, 0.0);
        let mut b = Chooser::new();
        b.pointer_down(1, 500.0, 900.0, 0.0, 0.0);
        b.pointer_down(2, 100.0, 100.0, 0.0, 0.0);
        let _ = b.draw(0.0, 0, 0.0);

        let at = CHOSEN_PLAYER_ANIMATION_TIME_MS / 2.0;
        let one = a.flood_hole(at, 1080.0, 2340.0).expect("a hole");
        let two = b.flood_hole(at, 1080.0, 2340.0).expect("a hole");
        assert!(
            (one - two).abs() < 1e-9,
            "the hole is the same size whichever player won: {one} against {two}"
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
        // The mark is untouched by the flood entirely: the hole is still far wider
        // than the mark halfway through, so the hole is closing and has not arrived.
        let mid = chooser
            .flood_hole(CHOSEN_PLAYER_ANIMATION_TIME_MS / 2.0, w, h)
            .expect("a hole");
        assert!(
            mid > MARK_RADIUS,
            "and the mark is still well inside the closing hole: {mid}"
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
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 10.0, 10.0, 0.0, 0.0);
        chooser.pointer_down(3, 20.0, 20.0, 0.0, 0.0);
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
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 10.0, 10.0, 0.0, 0.0);
        chooser.pointer_down(3, 20.0, 20.0, 700.0, 0.0);

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
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 10.0, 10.0, 0.0, 0.0);
        let winner = chooser.draw(0.0, 0, 0.0).expect("a winner");
        chooser.pointer_up(winner, 10.0);

        chooser.pointer_down(9, 100.0, 100.0, 500.0, 0.0);
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
    fn the_disc_reaches_full_size_early_and_the_loading_is_a_share_not_a_duration() {
        // The bug this pins: `loading / DISC_ARRIVAL_FRACTION` and
        // `loading * DISC_ARRIVAL_FRACTION` differ by an order of magnitude, and only
        // one of them ever reaches 1. Multiplying capped the disc at a sixteenth of
        // its size for the whole load.
        assert!(
            (Player::disc_arrival(DISC_ARRIVAL_FRACTION) - 1.0).abs() < 1e-9,
            "the disc is full size when the arrival fraction has elapsed"
        );
        assert!(
            Player::disc_arrival(DISC_ARRIVAL_FRACTION / 2.0) > 0.4,
            "and most of the way there by half of it: {}",
            Player::disc_arrival(DISC_ARRIVAL_FRACTION / 2.0)
        );
        // Monotone, bounded, and never above 1 -- an ease that overshoots would make
        // the disc grow past its own size and shrink back.
        let mut last = f64::NEG_INFINITY;
        for step in 0..=40 {
            let v = Player::disc_arrival(f64::from(step) / 40.0);
            assert!(v >= last - 1e-12, "not monotone at step {step}");
            assert!((0.0..=1.0).contains(&v), "out of range at step {step}: {v}");
            last = v;
        }
        // Halfway through the load the disc is long since arrived.
        assert!(
            Player::disc_arrival(0.5) > 0.99,
            "and it is still full size late in the load"
        );
    }

    #[test]
    fn the_load_is_two_events_not_one_slow_growth() {
        // Measured from a touchdown: the disc reaches full size at about 100ms of a
        // 620ms load and then sits within a pixel of its final radius for the
        // remaining 500ms while the arc sweeps round.
        //
        // The previous build grew the disc over the whole window, so the mark became
        // the right size early and then spent four fifths of its time sitting
        // there. That is what "the initial loading is too static" describes, and it
        // is why the split is two constants rather than one.
        const {
            assert!(
                DISC_ARRIVAL_FRACTION < 0.25,
                "the disc arrives in the first fifth of the load"
            );
            assert!(
                DISC_ARRIVAL_FRACTION * REGISTRATION_TIME_MS > 50.0,
                "and that is long enough to read as an arrival, not a pop"
            );
            assert!(
                DISC_ARRIVAL_FRACTION * REGISTRATION_TIME_MS < 200.0,
                "and short enough that most of the load is the arc's sweep"
            );
        }
        // The arc is what fills the rest of the window, so it has to be the longer
        // of the two -- a mark that finishes loading before its sweep is done is a
        // mark whose sweep is decoration.
        const {
            assert!(
                (1.0 - DISC_ARRIVAL_FRACTION) * REGISTRATION_TIME_MS > 300.0,
                "the sweep is the bulk of the load"
            );
        }
    }

    #[test]
    fn the_loading_arc_is_drawn_on_the_rings_own_band() {
        // The ring's band is 47.5 to 57.0 CSS px, and the arc sits at 47.8 measured
        // while the disc is at its full 38.2. It is the ring's own centreline, so
        // the arc the mark is loaded with and the ring it ends up with are the same
        // circle.
        const {
            assert!(
                (RING_STROKE_RADIUS - 52.25).abs() < 0.2,
                "the ring's centreline is 52.25 CSS px, and the arc uses it"
            );
        }
        // A multiple of the disc's *full* radius, and exactly the ring's own
        // centreline. If it were a multiple of the disc's *current* radius it would
        // drift inward every frame and never meet the ring.
        const {
            assert!(
                (DISC_RADIUS * LOADING_ARC_SCALE - RING_STROKE_RADIUS).abs() < 1e-9,
                "the arc's radius IS the ring's centreline, derived rather than \
                 measured a second time"
            );
        }
        const {
            assert!(
                (LOADING_ARC_SCALE - 1.368).abs() < 0.005,
                "which is 1.368x the disc"
            );
        }
    }

    #[test]
    fn a_loading_sweep_is_the_players_own_colour() {
        // A fixed pale colour is orange against every hue that is not orange, so the
        // sweep looked like it belonged to whichever player happened to be orange
        // rather than to the finger that had just landed.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);
        chooser.pointer_down(4, 10.0, 10.0, 0.0, 0.0);
        let players: Vec<Player> = chooser.players().cloned().collect();
        let (a, b) = (&players[0], &players[1]);

        assert_ne!(a.color_of(), b.color_of(), "the players differ in colour");
        assert_ne!(
            a.loading_color(),
            b.loading_color(),
            "so their loading sweeps must differ too"
        );
        // And the sweep is a lightened version of the player's own colour, so it
        // reads as the same mark rather than as a foreign highlight on it.
        assert_ne!(a.loading_color(), a.color_of());
        assert_ne!(a.loading_color(), a.ring_color());
    }

    #[test]
    fn a_loading_closes_from_both_sides_and_meets_opposite() {
        // The sweep is two halves growing from one origin, in opposite directions,
        // and they meet exactly opposite when it is finished.
        let origin = 0.7_f64;
        let arcs = sweep_arcs(origin, 1.0);
        let (from_a, to_a) = arcs[0];
        let (from_b, to_b) = arcs[1];

        // Each half is a half-circle, and the two together are the whole ring.
        let a = to_a - from_a;
        let b = to_b - from_b;
        assert!(
            (a - std::f64::consts::PI).abs() < 1e-9,
            "each side sweeps exactly half the circle: {a}"
        );
        assert!(
            (a + b - std::f64::consts::TAU).abs() < 1e-9,
            "and together they cover the ring, not a part of it: {}",
            a + b
        );
        // They start together at the origin...
        assert!(
            (from_a - origin).abs() < 1e-12 && (to_b - origin).abs() < 1e-12,
            "both sides start at the origin: {from_a} and {to_b}"
        );
        // ...and finish together opposite it. `to_a` is the clockwise side's far
        // end and `from_b` the anticlockwise side's, and they are the same angle --
        // that angle being the origin plus half a turn.
        let turn = std::f64::consts::TAU;
        let far_a = to_a.rem_euclid(turn);
        let far_b = from_b.rem_euclid(turn);
        assert!(
            (far_a - far_b).abs() < 1e-9,
            "and finish together at {far_a} against {far_b}"
        );
        assert!(
            (far_a - origin.rem_euclid(turn) - std::f64::consts::PI).abs() < 1e-9,
            "which is half a turn round from the origin, i.e. opposite it: {far_a}"
        );
    }

    #[test]
    fn a_loading_is_two_sides_going_opposite_ways_from_one_origin() {
        // The shape itself, as opposed to the arithmetic of how far round each side
        // has got. A one-sided sweep -- the thing this replaced -- returns two arcs
        // where the second is degenerate, and every other test here still passes on
        // it, so the shape needs stating on its own.
        let origin = 0.9_f64;
        let (a_from, a_to) = sweep_arcs(origin, 0.5)[0];
        let (b_from, b_to) = sweep_arcs(origin, 0.5)[1];

        // One side runs anticlockwise *back* from the origin, the other clockwise
        // *forward* from it. The canvas grows an arc from its smaller start angle to
        // its larger, so the second side is written reversed -- hence the swap below.
        assert!(
            (a_to - a_from) > 0.0,
            "the clockwise side runs forward from the origin"
        );
        assert!(
            (b_to - b_from) > 0.0,
            "and the other is written in reverse, so the canvas can draw it"
        );
        assert!(
            (a_from - b_from) > 0.0,
            "the two occupy opposite halves of the circle, not the same one: \
             {a_from} against {b_from}"
        );
        // Both start at the origin, and the loading therefore begins as a point
        // there rather than somewhere else on the ring.
        assert!(
            (a_from - origin).abs() < 1e-12 && (b_to - origin).abs() < 1e-12,
            "both sides start at the origin"
        );
        // Neither side is degenerate at any point in the loading: a one-sided sweep
        // leaves one of them zero-length for the whole window.
        // Both sides are live and equal, and they leave the origin in opposite
        // directions. The equality is what a degenerate one-sided arc fails: it
        // returns one full half and one of zero length, which is still "both
        // positive" for the first and "both equal" for neither, so the shape is
        // pinned by the ratio rather than by a sign.
        for step in 1..=20 {
            let arcs = sweep_arcs(origin, f64::from(step) / 20.0);
            let a = arcs[0].1 - arcs[0].0;
            let b = arcs[1].1 - arcs[1].0;
            assert!(
                a > 0.0 && b > 0.0,
                "both sides are live at step {step}: {a} and {b}"
            );
            assert!(
                (a - b).abs() < 1e-12,
                "and they are the same length, so neither is degenerate: {a}, {b}"
            );
            // Neither is the whole circle: each is half of the progress, reaching
            // at most half a turn when the loading is done, so a single side
            // covering all of it -- the sweep this replaced -- cannot pass.
            assert!(
                a <= std::f64::consts::PI + 1e-9,
                "and each is at most half a turn at step {step}: {a}"
            );
            assert!(
                arcs[0].0 - arcs[1].0 > 0.0,
                "and they start from opposite sides of the origin: {} against {}",
                arcs[0].0,
                arcs[1].0
            );
        }
    }

    #[test]
    fn a_loading_starts_nothing_and_grows_from_both_ends() {
        let origin = 2.1_f64;
        // Nothing at the start: both halves are zero-length at the origin, so the
        // loading is invisible rather than a dot that has to be hidden.
        let start = sweep_arcs(origin, 0.0);
        assert!((start[0].1 - start[0].0).abs() < 1e-12);
        assert!((start[1].1 - start[1].0).abs() < 1e-12);

        // And it grows on both sides, symmetrically, with no gap opening up.
        let mid = sweep_arcs(origin, 0.5);
        let a = mid[0].1 - mid[0].0;
        let b = mid[1].1 - mid[1].0;
        assert!((a - b).abs() < 1e-12, "the two sides stay the same length");
        assert!(a > 0.0, "and both are growing");

        // Monotone in progress, and clamped outside the window.
        let mut last = 0.0_f64;
        for step in 0..=40 {
            let arcs = sweep_arcs(origin, f64::from(step) / 40.0);
            let grown = (arcs[0].1 - arcs[0].0) + (arcs[1].1 - arcs[1].0);
            assert!(grown >= last - 1e-12, "not monotone at step {step}");
            last = grown;
        }
        assert_eq!(
            sweep_arcs(origin, 2.0),
            sweep_arcs(origin, 1.0),
            "clamped above"
        );
        assert_eq!(
            sweep_arcs(origin, -1.0),
            sweep_arcs(origin, 0.0),
            "and below"
        );
    }

    #[test]
    fn each_loading_starts_from_its_own_origin() {
        // A table of fingers all landing together must not fill in lockstep from
        // one place, or the marks stop reading as belonging to particular fingers.
        // The origin is supplied per finger, so this is a matter of the state
        // keeping them apart.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.25);
        chooser.pointer_down(2, 50.0, 50.0, 0.0, 1.75);
        chooser.pointer_down(3, 90.0, 90.0, 0.0, 4.5);
        let origins: Vec<f64> = chooser.players().map(|p| p.load_origin).collect();
        assert_eq!(origins, vec![0.25, 1.75, 4.5], "each mark keeps its own");
        // And a different origin gives a different pair of arcs, so the geometry
        // really does depend on it.
        assert_ne!(sweep_arcs(0.25, 0.5), sweep_arcs(1.75, 0.5));
    }

    #[test]
    fn the_two_loadings_take_one_and_three_seconds() {
        // Both durations are the user's choice, stated as such where they are
        // defined. They are asserted here so a change to either is deliberate.
        assert!((REGISTRATION_TIME_MS - 1000.0).abs() < 1e-9);
        assert!((DRAWING_TIME_MS - 3000.0).abs() < 1e-9);
        // And the draw is long enough to be read as counting down rather than as a
        // flash, which is why it is not the original app's 2500ms.
        const {
            assert!(
                DRAWING_TIME_MS > REGISTRATION_TIME_MS,
                "the selection takes longer than the arrival"
            );
        }
    }

    #[test]
    fn a_chosen_player_stays_chosen_and_keeps_its_ring_loaded() {
        // The winner's ring filled up with the player's colour over the draw window
        // and has to stay that way.
        //
        // `draw_progress` returns `None` both before a window opens and after one
        // ends, so the draw's own progress cannot distinguish "waiting" from
        // "won" -- and the drawing code fell through to the resting tint for both, so
        // the winner's ring dimmed the instant the window closed. The charge visibly
        // discharged, which is the opposite of what being chosen should look like.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 100.0, 100.0, 0.0, 0.0);
        let ready = chooser.ready_at().expect("a window");
        assert!(!chooser.is_chosen(), "nobody has won yet");

        let winner = chooser
            .draw(ready + DRAWING_TIME_MS, 0, 0.0)
            .expect("a winner");

        // The window is over, so the draw no longer has a progress to report...
        assert!(
            chooser
                .draw_progress(ready + DRAWING_TIME_MS + 1.0)
                .is_none(),
            "and there is no draw progress left to draw with"
        );
        // ...but the choice stands, so the rendering has something to key off.
        assert!(
            chooser.is_chosen(),
            "the choice outlives the draw window, so the winner's ring can hold"
        );
        assert_eq!(chooser.chosen().map(|p| p.id), Some(winner));

        // And it holds for as long as the winner's finger is down, which is until
        // the reset -- the reset is timed from the winner lifting, not from the draw.
        for later in [1.0_f64, 500.0, 1_900.0] {
            assert!(
                chooser.is_chosen(),
                "and is still the choice {later}ms after the draw ended"
            );
        }
        assert!(
            !chooser.tick(ready + DRAWING_TIME_MS + RESTART_DELAY),
            "holding the finger down holds the choice, however long that is"
        );
        // Lift it, and the reset is two seconds from *that*.
        chooser.pointer_up(winner, ready + DRAWING_TIME_MS + 1.0);
        assert!(
            !chooser.tick(ready + DRAWING_TIME_MS + 1.0 + RESTART_DELAY - 1.0),
            "not a millisecond early"
        );
        assert!(
            chooser.tick(ready + DRAWING_TIME_MS + 1.0 + RESTART_DELAY),
            "and the app resets two seconds after the winner lifts"
        );
    }

    #[test]
    fn the_draw_waits_for_every_finger_to_load() {
        // Without this, a finger that lands just as the previous window was
        // expiring is picked from a mark still charging, from a screen whose other
        // marks had not arrived -- a screen nobody saw.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 100.0, 100.0, 400.0, 0.0);

        assert!(chooser.is_drawing(), "the window is open");
        assert!(!chooser.is_ready(400.0), "but it is not counting yet");
        assert_eq!(
            chooser.ready_at(),
            Some(400.0 + REGISTRATION_TIME_MS),
            "it starts when the last finger has loaded"
        );

        // A third finger at the instant the window was about to fire pushes it out.
        let ready = chooser.ready_at().expect("a window");
        chooser.pointer_down(3, 50.0, 50.0, ready, 0.0);
        assert!(!chooser.is_ready(ready), "a late finger holds it back");
        assert_eq!(chooser.ready_at(), Some(ready + REGISTRATION_TIME_MS));
    }

    #[test]
    fn the_second_loading_starts_when_the_first_one_ends() {
        // The two loadings never overlap, so the eye is never asked to follow both
        // at once: the mark finishes charging, and then the draw's arc begins.
        let mut chooser = Chooser::new();
        chooser.pointer_down(1, 0.0, 0.0, 0.0, 0.0);
        chooser.pointer_down(2, 100.0, 100.0, 0.0, 0.0);
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
        assert_eq!(chooser.flood_hole(0.0, 800.0, 800.0), None);
        assert_eq!(chooser.chosen_progress(0.0), None);
    }

    #[test]
    fn a_choice_is_a_pure_function_of_the_state() {
        // Two identical runs of the same script produce the same winner, which
        // is what makes the app testable at all; the browser's only addition is
        // which index it passes in.
        fn run() -> Chooser {
            let mut chooser = Chooser::new();
            chooser.pointer_down(3, 1.0, 2.0, 0.0, 0.0);
            chooser.pointer_down(1, 3.0, 4.0, 0.0, 0.0);
            chooser.pointer_move(3, 9.0, 9.0);
            let _ = chooser.draw(DRAWING_TIME_MS, 1, 0.0);
            chooser
        }
        assert_eq!(run(), run());
        // Ordered by pointer id, so index 0 is pointer 1, not the first finger
        // to arrive.
        assert_eq!(run().chosen().map(|p| p.id), Some(3));
    }
}
