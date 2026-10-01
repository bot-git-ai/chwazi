// Copyright (c) 2026 Witalis Domitrz <witekdomitrz@gmail.com>
// AGPL License

//! The browser layer: events in, pixels out, [`crate::chooser`] in between.
//!
//! This file holds no rules. It translates pointer events into chooser calls,
//! drives one `requestAnimationFrame` loop, and draws what the chooser says is
//! on screen. Every number it draws comes out of [`crate::chooser`], so the
//! values the unit tests assert on are the values that reach the canvas.
//!
//! Nothing here is application JavaScript either. The page's one module script
//! does a dynamic import of the generated bindings and this start function runs
//! when they load; there is no manual wasm ABI and no handwritten app code.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::Event;

use crate::chooser::{self, Chooser, Player};
use crate::wheel;

/// A full turn of the circle, and the arc every stroke sweeps.
const TWO_PI: f64 = 2.0 * std::f64::consts::PI;

/// The white loading arc, at the alpha the original used.
const LOADING_COLOR: &str = "rgba(255, 255, 255, 0.34)";

/// The scope this app's worker is registered for.
///
/// Stated rather than inherited, and it is the one string that has to agree
/// with `src/service-worker.js`: the worker resolves its own directory the same
/// way, from `self.location`. See [`register_service_worker`] for why this is
/// not optional.
const SCOPE: &str = "./";

/// Everything the render loop needs.
struct App {
    chooser: Chooser,
    /// `performance.now()` at the first frame, which anchors the pulse so every
    /// circle breathes in step.
    start_time: f64,
    /// When the last draw was won, and by whom, for as long as the winner's
    /// colour is on screen.
    ///
    /// A separate record rather than the chooser's own winner, because the
    /// chooser forgets a player the instant their finger lifts -- and the colour
    /// that floods the screen has to stay that colour and stay centred on where
    /// that finger *was*, for the full second and a bit that the reveal lasts.
    /// Without this the circle would re-centre on the last finger position
    /// recorded, or vanish mid-reveal.
    last_winner: Option<Winner>,
    /// The screen's device pixel ratio, so the canvas can be drawn at full
    /// resolution.
    scale: f64,
}

/// A won draw, kept after the finger has gone.
#[derive(Debug, Clone, Copy)]
struct Winner {
    id: i32,
    x: f64,
    y: f64,
    at: f64,
}

/// The animation-frame closure.
///
/// Held in a `RefCell` because a frame schedules the next one from inside the
/// closure it is stored in, and parked in a thread-local for the life of the
/// page: it is never dropped, and never replaced.
struct Frame {
    closure: RefCell<Closure<dyn FnMut(f64)>>,
}

thread_local! {
    static FRAME: RefCell<Option<Rc<Frame>>> = const { RefCell::new(None) };
}

/// Start the app.
///
/// Registered as a wasm-bindgen start function, so it runs as the module is
/// instantiated and the page needs nothing but `import('./app.js')`.
#[wasm_bindgen(start)]
pub fn start() {
    if let Err(error) = run() {
        show_failure(&describe(&error));
    }
}

/// Set up the canvas, the listeners, the loop and the service worker.
fn run() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let canvas: web_sys::HtmlCanvasElement = element("main")?;
    let context = canvas
        .get_context("2d")?
        .ok_or("no 2d canvas context")?
        .dyn_into::<web_sys::CanvasRenderingContext2d>()?;

    // The canvas's drawing surface is its width and height in device pixels, and
    // assigning either clears it, so this happens before the first frame and on
    // every resize.
    resize(&canvas);
    let scale = device_pixel_ratio();

    let app = Rc::new(RefCell::new(App {
        chooser: Chooser::new(),
        start_time: now(),
        last_winner: None,
        scale,
    }));
    {
        let app = Rc::clone(&app);
        // A window can move between displays -- a phone dragged to a monitor, a
        // browser tab dragged between a Retina and a non-Retina screen. The scale
        // is part of the app's state so that the draw code reads one value rather
        // than reaching for the window every frame.
        listen(&window, "resize", {
            let canvas = canvas.clone();
            move |_| {
                resize(&canvas);
                let scale = device_pixel_ratio();
                borrow(&app, |app| app.scale = scale);
                Ok(())
            }
        })?;
    }

    // Pointer events, which on a touch screen *are* the fingers. A mouse sends
    // the same events, so the chooser is usable on a desktop too.
    listen(&window, "pointerdown", {
        let app = Rc::clone(&app);
        move |event| {
            let event: web_sys::PointerEvent = event.dyn_into()?;
            let (id, x, y) = at(&event);
            let count = borrow(&app, |app| {
                app.chooser
                    .pointer_down(id, x, y, now(), random_index(16));
                app.chooser.len()
            });
            announce_players(count);
            Ok(())
        }
    })?;
    listen(&window, "pointermove", {
        let app = Rc::clone(&app);
        move |event| {
            let event: web_sys::PointerEvent = event.dyn_into()?;
            let (id, x, y) = at(&event);
            borrow(&app, |app| app.chooser.pointer_move(id, x, y));
            Ok(())
        }
    })?;
    // Lift and cancel are the same thing to a finger chooser: the finger is no
    // longer on the glass. The original bound both to one handler.
    for name in ["pointerup", "pointercancel"] {
        listen(&window, name, {
            let app = Rc::clone(&app);
            move |event| {
                let event: web_sys::PointerEvent = event.dyn_into()?;
                let count = borrow(&app, |app| {
                    app.chooser
                        .pointer_up(event.pointer_id(), now(), random_index(16));
                    app.chooser.len()
                });
                announce_players(count);
                Ok(())
            }
        })?;
    }

    // No scrolling, no rubber-banding and no pull-to-refresh under a finger that
    // is choosing a winner. `touch-action: none` in the shell says the same
    // thing in CSS; this is the one that works when a browser ignores it.
    // `passive: false` is what makes `preventDefault` allowed at all, and its
    // absence is the single most common way this ends up scrolling instead.
    listen(&window, "touchmove", |event| {
        event.prevent_default();
        Ok(())
    })?;

    register_service_worker();
    // Nothing between the first frame and the canvas: no splash to take down, no
    // prompt to show. The app opens straight into being the thing you touch.
    start_loop(Rc::clone(&app), canvas, context);
    Ok(())
}

/// Start the one animation-frame loop that draws everything.
fn start_loop(
    app: Rc<RefCell<App>>,
    canvas: web_sys::HtmlCanvasElement,
    context: web_sys::CanvasRenderingContext2d,
) {
    // One JS function object for the whole session, not a fresh one per frame:
    // building a `Closure` allocates a JS function, and a loop that forgets one
    // every frame leaks sixty of them a second until the tab dies.
    //
    // The closure holds a `Weak` to its own `Rc`, so the cycle is broken and the
    // loop is still "alive as long as the page is". `new_cyclic` is what makes
    // that self-reference expressible at all.
    let frame = Rc::new_cyclic(|weak: &Weak<Frame>| {
        let weak = weak.clone();
        Frame {
            closure: RefCell::new(Closure::new(move |timestamp| {
                let winner = render(&app, &canvas, &context, timestamp);
                if let Some((winner, of)) = winner {
                    announce_winner(winner, of);
                }
                if let Some(frame) = weak.upgrade() {
                    schedule(&frame);
                }
            })),
        }
    });
    FRAME.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&frame)));
    schedule(&frame);
}

/// Ask for the next frame, if the loop is still installed.
fn schedule(frame: &Rc<Frame>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = frame.closure.borrow();
    let _ = window.request_animation_frame(callback.as_ref().unchecked_ref());
}

/// Draw one frame. Returns the winner, if this frame drew one.
///
/// Three things happen here that have no event of their own, because all three
/// are *time passing* rather than something a finger did, and this loop already
/// runs on a clock:
///
/// * the reset, due two seconds after the winner lifts;
/// * the draw itself, due `DRAWING_TIME_MS` after the last change to who is on
///   the glass. Running it here rather than in a `setTimeout` means the winner
///   and the frame that shows the winner are the same instant, and a frame that
///   arrives late draws a late winner instead of a stale one;
/// * the haptics, on the one frame the winner is announced.
///
/// The winner is committed here, at the end of the window, and the spinning
/// animation before it was laid out to land on the player already chosen in
/// [`Chooser::pending_winner`] -- so the animation reveals a result rather than
/// producing one, and a finger landing or lifting mid-spin cannot change who
/// wins.
fn render(
    app: &Rc<RefCell<App>>,
    canvas: &web_sys::HtmlCanvasElement,
    context: &web_sys::CanvasRenderingContext2d,
    timestamp: f64,
) -> Option<(i32, usize)> {
    borrow(app, |app| {
        let mut announced = None;
        app.chooser.tick(timestamp);

        let ready_since = app.chooser.ready_at();
        if let Some(since) = ready_since {
            if timestamp - since >= chooser::DRAWING_TIME_MS {
                // The number of players is read before the draw, because the
                // draw removes all but the winner.
                let of = app.chooser.len();
                if let Some(winner) = app.chooser.draw(timestamp) {
                    // The one moment the whole app is about, so the one moment it
                    // speaks: a buzz, for whoever is not looking at the screen.
                    buzz();
                    // The chooser keeps only the winner now; where they were and
                    // when is the reveal's business, so it is recorded here.
                    let won = app.chooser.chosen().map(|player| Winner {
                        id: player.id,
                        x: player.x,
                        y: player.y,
                        at: timestamp,
                    });
                    app.last_winner = won;
                    announced = Some((winner, of));
                }
            }
        }

        let start_time = app.start_time;
        let last_winner = app.last_winner;
        paint(
            &app.chooser,
            last_winner,
            app.scale,
            canvas,
            context,
            timestamp,
            start_time,
        );
        announced
    })
}

/// Draw the whole screen.
///
/// Three states, in the order the eye meets them:
///
/// * **nothing yet** -- the hint, so the first finger has something to arrive at;
/// * **drawing** -- the players on the wheel, spinning, with the white arc
///   closing on each of them;
/// * **won** -- the winner's colour flooding the screen.
///
/// The reveal reads from [`App::last_winner`] rather than the chooser's live
/// winner, so it keeps its position and colour after the winning finger lifts.
fn paint(
    app: &Chooser,
    last_winner: Option<Winner>,
    scale: f64,
    canvas: &web_sys::HtmlCanvasElement,
    context: &web_sys::CanvasRenderingContext2d,
    timestamp: f64,
    start_time: f64,
) {
    // Everything below draws in CSS pixels. The canvas is `scale` times that in
    // device pixels, so the transform is what makes a circle 51 CSS pixels across
    // land on 51 * device pixels of glass instead of being stretched over 51.
    let _ = context.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    let (width, height) = (
        f64::from(canvas.width()) / scale,
        f64::from(canvas.height()) / scale,
    );
    context.clear_rect(0.0, 0.0, width, height);

    let pulse = chooser::pulse_scale(timestamp, start_time);

    // The reveal: a colour flooding the screen from the winner's circle, and the
    // winner still visible inside it as a hole.
    if let Some(winner) = last_winner.filter(|_| app.chosen().is_some()) {
        draw_reveal(context, &winner, app, width, height, timestamp, pulse);
        return;
    }
    if let Some(winner) = app.chosen() {
        let won = Winner {
            id: winner.id,
            x: winner.x,
            y: winner.y,
            at: app.chosen_at(timestamp).unwrap_or(timestamp),
        };
        draw_reveal(context, &won, app, width, height, timestamp, pulse);
        return;
    }

    let players: Vec<(i32, f64, f64)> = app.players().map(|p| (p.id, p.x, p.y)).collect();

    // Drawing. With fewer than two fingers there is no wheel to show, but the
    // circles still register: a player putting their finger down sees it arrive,
    // which is the whole of the app's feedback before there is a choice to make.
    if players.len() < chooser::REQUIRED_PLAYER_COUNT {
        for (index, (id, x, y)) in players.iter().enumerate() {
            let loaded = app.registration(index, timestamp).unwrap_or(1.0);
            let here = Player {
                id: *id,
                x: *x,
                y: *y,
                joined_at: f64::NEG_INFINITY,
                chosen_at: None,
            };
            draw_player(context, &here, pulse, 0.0, loaded);
        }
        return;
    }

    // Drawing. The wheel is a function of the players and the clock alone, so it
    // needs no state: the same hands on the glass and the same millisecond always
    // give the same wheel, which is what lets it stay in step while fingers move
    // underneath it.
    let window = chooser::DRAWING_TIME_MS;
    let elapsed = timestamp - app.ready_at().unwrap_or(timestamp);
    let gather = wheel::gather_progress(elapsed, window);
    let slots = wheel::slots(&players, gather, width, height);
    // `elapsed` is measured from the moment the app is ready, so it is zero while
    // fingers are still charging and the whole window is spent spinning. That is
    // the same instant `ready_at` returns, so the pointer, the arc and the draw
    // cannot disagree about when the draw began.
    let spin = wheel::spin_progress(elapsed, window);
    let turns = wheel::turns_for(players.len());
    let arc = app.draw_progress(timestamp).unwrap_or(0.0);

    // Faint spokes first, so the circles sit on something. This is the one piece
    // of chrome the app has ever had, and it earns its place: without it the
    // circles drift together mid-spin and the eye loses track of which is which.
    draw_spokes(context, &players, &slots);

    for (index, (id, _, _)) in players.iter().enumerate() {
        let slot = slots[index];
        let loaded = app.registration(index, timestamp).unwrap_or(1.0);
        let landing = app.pending_winner() == Some(index);
        let here = Player {
            id: *id,
            x: slot.x,
            y: slot.y,
            joined_at: f64::NEG_INFINITY,
            chosen_at: None,
        };
        draw_player(
            context,
            &here,
            // The winner's circle grows slightly as the pointer closes on it, so
            // the eye is pulled to the right place before the result is announced.
            pulse * if landing { landing_grow(spin) } else { 1.0 },
            arc,
            loaded,
        );
    }

    draw_pointer(
        context,
        &players,
        &slots,
        elapsed,
        window,
        &View { width, height, turns },
    );
}

/// The winner's colour taking the screen, with the winner as a hole in it.
fn draw_reveal(
    context: &web_sys::CanvasRenderingContext2d,
    winner: &Winner,
    app: &Chooser,
    width: f64,
    height: f64,
    timestamp: f64,
    pulse: f64,
) {
    let progress = app
        .chosen_progress(timestamp)
        .unwrap_or_else(|| {
            ((timestamp - winner.at) / chooser::CHOSEN_PLAYER_ANIMATION_TIME_MS).clamp(0.0, 1.0)
        });

    // The winner's finger, if it is still down. The chooser keeps only the winner
    // after a draw, and its `pointer_move` still updates that player, so the live
    // position is one lookup -- and the whole reveal follows it.
    let live = app
        .chosen()
        .filter(|player| player.id == winner.id)
        .map_or((winner.x, winner.y), |player| (player.x, player.y));
    let (cx, cy) = live;
    // The same radius curve as the original: from off-screen down to just
    // clearing the winner's own ring.
    let from = width.max(height).max(chooser::WINNER_RADIUS);
    let radius = progress * chooser::WINNER_RADIUS + (1.0 - progress) * from;

    match web_sys::Path2d::new() {
        Ok(path) => {
            path.rect(0.0, 0.0, width, height);
            if path.arc(cx, cy, radius, 0.0, TWO_PI).is_ok() {
                context.set_fill_style_str(&Player::color(winner.id));
                context.fill_with_path_2d_and_winding(&path, web_sys::CanvasWindingRule::Evenodd);
            }
        }
        Err(error) => show_failure(&describe(&error)),
    }
    // The winner alone, at full pulse, and never a loading arc: the draw is over,
    // and an arc sweeping its ring would read as a second, still-running draw.
    // The winner's circle follows their finger for the whole reveal. `winner` was
    // recorded at the draw, and this is the same pointer id, so if that finger is
    // still down its live position wins: the colour expands from wherever the
    // winner's finger actually is, and drags with it.
    let here = Player {
        id: winner.id,
        x: cx,
        y: cy,
        joined_at: f64::NEG_INFINITY,
        chosen_at: None,
    };
    draw_player(context, &here, pulse, 1.0, 1.0);
}

/// How much larger the landing player's circle is at the end of the spin.
///
/// Only in the last stretch, and only by a little: enough to pull the eye, not
/// so much that it looks like a second highlight. Eased out, so it grows with the
/// pointer's own deceleration rather than against it.
fn landing_grow(spin: f64) -> f64 {
    const MAX: f64 = 1.12;
    ((spin - 0.7) / 0.3).clamp(0.0, 1.0).powi(2) * (MAX - 1.0) + 1.0
}

/// The faint lines from the centre to each circle.
///
/// `alpha` is deliberately low and the width hairline: this is a hint that the
/// circles are in a wheel, not decoration, and anything stronger competes with
/// the colours it is meant to organise.
fn draw_spokes(
    context: &web_sys::CanvasRenderingContext2d,
    players: &[(i32, f64, f64)],
    slots: &[wheel::Slot],
) {
    context.save();
    context.set_line_width(1.0);
    context.set_stroke_style_str("rgba(255, 255, 255, 0.10)");
    for ((_, px, py), slot) in players.iter().zip(slots) {
        let (slot_x, slot_y) = (slot.x, slot.y);
        // From the circle to where it is going, not to the centre: a spoke to the
        // centre under a circle on the ring would be entirely hidden.
        let (dx, dy) = (slot_x - px, slot_y - py);
        let length = dx.hypot(dy);
        if length < 1.0 {
            continue;
        }
        context.begin_path();
        context.move_to(px + dx * 0.35, py + dy * 0.35);
        context.line_to(slot_x - dx / length * 8.0, slot_y - dy / length * 8.0);
        context.stroke()
    }
    context.restore();
}

/// The pointer sweeping the wheel.
///
/// A single small white dot riding the ring, which is all it needs to be: the
/// wheel's own rotation is the spectacle, and a pointer that drew attention to
/// itself would compete with the landing.
/// The screen's size, in CSS pixels, with the wheel's turn count.
struct View {
    width: f64,
    height: f64,
    turns: f64,
}

fn draw_pointer(
    context: &web_sys::CanvasRenderingContext2d,
    players: &[(i32, f64, f64)],
    slots: &[wheel::Slot],
    elapsed: f64,
    window: f64,
    view: &View,
) {
    let Some(landed) = wheel::landed_on(slots, elapsed, window, view.turns) else {
        return;
    };
    let colour = Player::color(players[landed].0);
    let (px, py) = wheel::pointer_position(elapsed, window, view.turns, view.width, view.height);

    // The landing player's colour as a halo, so the dot is legible over a circle
    // of the same colour and does not vanish into it.
    context.begin_path();
    let _ = context.arc(px, py, 16.0, 0.0, TWO_PI);
    context.set_fill_style_str("rgba(0, 0, 0, 0.55)");
    context.fill();

    context.begin_path();
    let _ = context.arc(px, py, 9.0, 0.0, TWO_PI);
    context.set_fill_style_str("#ffffff");
    context.fill();

    // And a thin ring in the winner's colour just inside it, which is the moment
    // the eye reads as "this one" rather than "a dot stopped somewhere".
    context.begin_path();
    let _ = context.arc(px, py, 22.0, 0.0, TWO_PI);
    context.set_line_width(3.0);
    context.set_fill_style_str(&colour);
    context.set_stroke_style_str(&colour);
    context.stroke()
}

/// One player: a filled inner disc, a ring around it, and the white arc that
/// counts the draw down.
/// `loaded` is the first of the app's two loadings: how far this circle's ring
/// has filled, 0 at the moment the finger lands and 1 when it is fully charged.
///
/// The ring fills with the player's own colour rather than a white halo. The
/// original filled the ring from light to dark; loading *towards the colour the
/// player will be* is the same idea with the destination moved to where it
/// belongs, so the ring is both the progress indicator and the first thing drawn
/// in that player's colour.
fn draw_player(
    context: &web_sys::CanvasRenderingContext2d,
    player: &Player,
    pulse: f64,
    loading: f64,
    loaded: f64,
) {
    let colour = player.color_of();
    let ring_radius = (chooser::INNER_RADIUS + chooser::OUTER_RADIUS) * pulse;
    let ring_width = chooser::OUTER_CIRCLE_WIDTH * pulse;

    context.begin_path();
    // `arc` on the 2d context is fallible in web-sys's bindings and infallible in
    // the browser -- a radius that is not finite throws there. `arc` is the one
    // call whose failure would end the frame, so it is checked; the others are
    // not, because no number that reaches them can be non-finite.
    if let Err(error) = context.arc(
        player.x,
        player.y,
        chooser::INNER_RADIUS * pulse,
        0.0,
        TWO_PI,
    ) {
        show_failure(&describe(&error));
        return;
    }
    // The disc fades in over the second half of the registration: the ring is the
    // thing that loads, and the disc arrives as the ring fills.
    context.set_global_alpha((loaded * loaded).clamp(0.0, 1.0));
    context.set_fill_style_str(&colour);
    context.fill();
    context.set_global_alpha(1.0);

    // The ring, loading: transparent at the moment the finger lands, the player's
    // own colour when it is charged. `globalAlpha` rather than a colour function
    // so the alpha composites with whatever is behind the canvas instead of being
    // baked into one string.
    context.save();
    context.set_global_alpha(loaded.clamp(0.0, 1.0));
    context.begin_path();
    if context.arc(player.x, player.y, ring_radius, 0.0, TWO_PI).is_ok() {
        context.set_line_width(ring_width);
        context.set_stroke_style_str(&colour);
        context.stroke();
    }
    context.restore();

    // The arc spans from `2*PI*(1-loading)/2` to `2*PI*(1-loading)*3/2`: a gap of
    // a quarter turn at the start of the window, closing to nothing by the end,
    // and the whole ring when no window is running at all.
    let remaining = 1.0 - loading;
    context.begin_path();
    let _ = context.arc(
        player.x,
        player.y,
        ring_radius,
        TWO_PI * remaining / 2.0,
        TWO_PI * remaining * 3.0 / 2.0,
    );
    context.set_line_width(ring_width);
    context.set_stroke_style_str(LOADING_COLOR);
    context.stroke();
}

/// One unbiased index into `len` players.
///
/// `getrandom` fills from the browser's own CSPRNG. The rejection loop is what
/// keeps it fair: taking the remainder of a draw over the whole 32-bit range
/// would favour the first players whenever `len` does not divide it evenly,
/// which is exactly the wrong bias in an app whose only job is choosing fairly.
fn random_index(len: usize) -> usize {
    let Ok(range) = u32::try_from(len) else {
        return 0;
    };
    if range < 2 {
        return 0;
    }
    // The largest multiple of `range` that fits, minus one: the draws above this
    // are the incomplete final block and are thrown away.
    let bound = u32::MAX - (u32::MAX % range) - 1;
    loop {
        match getrandom::u32() {
            Ok(value) if value <= bound => return usize::try_from(value % range).unwrap_or(0),
            // A failed draw is not a reason to stop choosing: ask again.
            _ => {}
        }
    }
}

/// One pointer event's id and position, in the units the chooser uses.
///
/// The call sites each read `pointerId`, `clientX` and `clientY` off an event,
/// and web-sys types those two getters differently depending on whether
/// `web_sys_unstable_apis` is set: `i32` normally, `f64` behind the cfg. So
/// there is no single spelling of a conversion here that is both correct and
/// lint-clean in both configurations:
///
/// * `f64::from(x)` is right normally and `clippy::useless_conversion` behind
///   the cfg, because `From<f64> for f64` is the identity impl and clippy
///   reaches the resolved one through the concrete getter's return type;
/// * `x as f64` is right behind the cfg and `clippy::unnecessary_cast` normally.
///
/// Both were tried here and both failed one side, which is the evidence for the
/// attribute below rather than an argument against it.
///
/// `Into` is the one that satisfies both, because a `From` impl makes
/// `Into` resolve to it without pinning a source type for inference the way
/// `f64::from` does. The values are CSS pixels, so they sit well inside `f64`'s
/// exact-integer range and no precision is at stake.
#[allow(clippy::useless_conversion)]
fn at(event: &web_sys::PointerEvent) -> (i32, f64, f64) {
    let (id, x, y) = (event.pointer_id(), event.client_x(), event.client_y());
    (id, x.into(), y.into())
}

/// `performance.now()`, the clock the chooser runs on.
fn now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

/// The screen's device pixel ratio, never less than 1.
///
/// Capped at 3: a 4x ratio would quadruple the pixels for no visible gain on a
/// screen whose pixels are already smaller than the circles, and costs battery on
/// a phone that is meant to be handed round.
fn device_pixel_ratio() -> f64 {
    web_sys::window()
        .map_or(1.0, |window| window.device_pixel_ratio())
        .clamp(1.0, 3.0)
}

/// Size the canvas to the window, and to the screen's pixels.
///
/// The canvas's drawing surface is its `width`/`height` in *device* pixels. The
/// original set those to `innerWidth`/`innerHeight` and drew in CSS pixels, so on
/// any high-density screen -- every modern phone -- a 51px circle was being drawn
/// across about 51 physical pixels and upscaled by the compositor. That is the
/// single biggest reason the edges looked soft: not anti-aliasing, resolution.
///
/// So the backing store is now `css size * devicePixelRatio`, and everything is
/// drawn through a transform of the same ratio, which keeps all the arithmetic in
/// this file in CSS pixels. A CSS pixel is still a CSS pixel in every constant and
/// every test; the device ratio only ever appears here and in the one
/// `set_transform` call.
fn resize(canvas: &web_sys::HtmlCanvasElement) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let width = window.inner_width().ok().and_then(dimension).unwrap_or(1);
    let height = window.inner_height().ok().and_then(dimension).unwrap_or(1);
    let scale = device_pixel_ratio();
    canvas.set_width((f64::from(width) * scale).round().max(1.0) as u32);
    canvas.set_height((f64::from(height) * scale).round().max(1.0) as u32);
}

/// Borrow the app for the length of `body`.
///
/// One helper, so that the render loop -- which borrows it from inside an
/// animation-frame closure -- is the only place a borrow can overlap.
fn borrow<T>(app: &Rc<RefCell<App>>, body: impl FnOnce(&mut App) -> T) -> T {
    body(&mut app.borrow_mut())
}

/// Add a listener that lives as long as the page does.
fn listen(
    target: &web_sys::EventTarget,
    name: &str,
    mut handler: impl FnMut(Event) -> Result<(), JsValue> + 'static,
) -> Result<(), JsValue> {
    let callback = Closure::<dyn FnMut(Event)>::new(move |event| {
        if let Err(error) = handler(event) {
            show_failure(&describe(&error));
        }
    });
    target.add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())?;
    // A fixed handful of handlers for the life of the document. Nothing is ever
    // removed and none are added per finger or per frame, so nothing accumulates.
    callback.forget();
    Ok(())
}

/// Ask the browser to keep the app installable and working offline.
///
/// Best effort, and the failure is logged rather than shown: the chooser is
/// fully usable without it, and a chooser with an error on screen has stopped
/// being a chooser. A browser that refuses the registration outright (no HTTPS,
/// no service workers) is a normal thing to happen.
fn register_service_worker() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let container = window.navigator().service_worker();

    // The scope is stated rather than inherited. Left to itself, a
    // registration's scope is the directory of the page that registered it,
    // which is right today and silently wrong the moment the app is published
    // somewhere else, or opened through a path that resolves higher up the
    // origin. A worker registered for the whole origin does not serve just its
    // own pages -- it answers for every page on that origin, including the
    // ones that have nothing to do with it. Naming the scope keeps that claim
    // as small as the app.
    let options = web_sys::RegistrationOptions::new();
    options.set_scope(SCOPE);
    let registration = container.register_with_options("./service-worker.js", &options);

    wasm_bindgen_futures::spawn_local(async move {
        match JsFuture::from(registration).await {
            Ok(_) => {}
            Err(error) => {
                let unavailable = JsValue::from_str("Chwazi: offline install unavailable.");
                web_sys::console::warn_1(&unavailable);
                let detail = JsValue::from_str(&describe(&error));
                web_sys::console::warn_1(&detail);
            }
        }
        release_stale_registrations(&window).await;
    });
}

/// Hand this app's own URLs back to the current worker.
///
/// A service worker is a registration, and a registration outlives the page
/// that made it: it is kept by the browser, not by the tab, and it keeps
/// answering for its scope until something explicitly unregisters it. That is
/// how a page on this origin can come to be served by a worker installed for a
/// *different* page, long after the app that installed it was closed. A stale
/// registration is not corrected by a reload, by a newer version of the app, or
/// by a newer worker installing itself -- the newer worker only takes control
/// where its own scope reaches, and a wider stale one is still in the way.
///
/// So the repair is explicit: find any registration whose scope covers this
/// app's directory but is not this app's directory, and unregister it. This
/// app's own registration is left alone, and so is every other app on the
/// origin -- each is scoped to its own directory, and a sibling that never
/// covered us is not ours to remove.
///
/// Failures are ignored on purpose. This is best-effort cleanup of state this
/// app did not create, and a browser that refuses leaves the user no worse
/// off: the app still runs and still caches its own assets.
async fn release_stale_registrations(window: &web_sys::Window) {
    let container = window.navigator().service_worker();
    let Ok(registrations) = JsFuture::from(container.get_registrations()).await else {
        return;
    };
    let Ok(array) = registrations.dyn_into::<js_sys::Array>() else {
        return;
    };

    // This app's own directory, as an absolute URL with a trailing slash. The
    // app is served from a subdirectory and every URL of ours is inside it.
    let Ok(home) = window.location().href() else {
        return;
    };
    let Ok(ours) = web_sys::Url::new_with_base(&home, "./") else {
        return;
    };
    let ours = ours.href();

    for entry in array.iter() {
        let Ok(registration) = entry.dyn_into::<web_sys::ServiceWorkerRegistration>() else {
            continue;
        };
        let scope = registration.scope();
        // Leave alone any scope that is this app's own, or narrower: a sibling
        // app mounted inside this directory is legitimate and separate, and
        // nothing there can intercept us. One test, because the two cases are
        // the same one: `ours` begins with `scope`.
        if ours.starts_with(&scope) {
            continue;
        }
        // What is left is a scope that is a *strict* prefix of ours: a worker
        // that would be consulted for this app's URLs while being registered
        // for more than this app. A worker is consulted for a URL exactly when
        // its scope is a prefix of that URL, which is the test above inverted.
        //
        // Of those, only our own worker qualifies: a different app's worker
        // lives in a different directory, so unregistering it would break the
        // app it belongs to.
        let script = registration
            .active()
            .map(|worker| worker.script_url())
            .unwrap_or_default();
        if script_belongs_to_app(&script, &ours) {
            match registration.unregister() {
                Ok(promise) => {
                    let _ = JsFuture::from(promise).await;
                }
                Err(error) => {
                    let released = JsValue::from_str("stale service worker could not be released");
                    web_sys::console::warn_2(&released, &error);
                }
            }
        }
    }
}

/// Whether a worker script at `script` is this app's own worker, registered for
/// more of the origin than this app's directory.
///
/// A wider scope means the script sits at the root of this app's own directory
/// rather than anywhere below it: a sibling app's worker is in a sibling
/// directory and does not match. `strip_suffix`, not `trim_end_matches` -- the
/// latter strips a *set of characters*, so it would happily eat a directory
/// named `...e-worker.js` and call it ours.
fn script_belongs_to_app(script: &str, ours: &str) -> bool {
    match web_sys::Url::new(script) {
        Ok(url) => url.href().strip_suffix("service-worker.js") == Some(ours),
        Err(_) => false,
    }
}

/// Say how many fingers are down, for anyone who cannot see the canvas.
///
/// This is a screen-reader live region and nothing else -- it is clipped off the
/// screen for sighted players, so the "one more" prompt this used to carry is
/// gone from the app entirely. The count itself is the useful part: it is how a
/// non-sighted player knows whether they are alone on the glass.
fn announce_players(count: usize) {
    let text = if count >= chooser::REQUIRED_PLAYER_COUNT {
        format!("{count} fingers down. Choosing in a moment.")
    } else if count == 1 {
        "1 finger down.".to_string()
    } else {
        "No fingers down.".to_string()
    };
    status(&text);
}

/// Say who won, out of how many.
fn announce_winner(winner: i32, of: usize) {
    status(&format!(
        "Finger {} wins, out of {of}.",
        winner_position(winner)
    ));
}

/// Buzz once when a winner is announced.
///
/// The native Chwazi app vibrates here, and it is worth keeping: the phone is in
/// somebody's hand and being passed around a table, so the buzz is the one piece
/// of the result that reaches the person who is not looking at the screen -- the
/// one who has to go first.
///
/// One short pulse, not a pattern: a pattern would read as an app being chatty
/// rather than a result being announced, and the screen already says what
/// happened.
fn buzz() {
    let Some(window) = web_sys::window() else {
        return;
    };
    window.navigator().vibrate_with_duration(WINNER_BUZZ_MS);
}

/// How long the winner's buzz lasts, in milliseconds.
const WINNER_BUZZ_MS: u32 = 40;

/// The winner's ordinal among the players it chose from.
///
/// The chooser keeps players in pointer-id order, so the finger that happened to
/// be pointer 1 is not necessarily the one that touched down first and there is
/// no insertion order left to report. Counting from the highest pointer id is at
/// least stable within a draw, which is what makes the announcement mean
/// something.
fn winner_position(winner: i32) -> i32 {
    winner
}

/// Show a message in the shell's live region.
fn status(text: &str) {
    if let Ok(element) = element::<web_sys::HtmlElement>("status") {
        element.set_inner_text(text);
    }
}

/// Show the shell's failure UI, which is hidden until something goes wrong.
fn show_failure(detail: &str) {
    status("Chwazi could not start.");
    let Ok(error) = element::<web_sys::HtmlElement>("error") else {
        return;
    };
    error.set_hidden(false);
    let Some(detail_element) = error.last_element_child() else {
        return;
    };
    detail_element.set_text_content(Some(&format!(
        "Chwazi could not start. Reload the page, or check your connection. ({detail})"
    )));
}

/// The element with this id, typed as the caller needs it.
fn element<T>(id: &str) -> Result<T, JsValue>
where
    T: JsCast,
{
    let document = web_sys::window()
        .ok_or("no window")?
        .document()
        .ok_or("no document")?;
    let found = document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("the page has no element #{id}")))?;
    Ok(found.unchecked_into())
}

/// `innerWidth`/`innerHeight`, which web-sys exposes as untyped properties.
fn dimension(value: JsValue) -> Option<u32> {
    value.as_f64().map(|size| size as u32)
}

/// A `JsValue` as text, without throwing on anything.
///
/// `JsValue` is a `Debug` wrapper, not something to hand to a user: its
/// formatting differs between release and debug builds. A thrown string or
/// error object has to read as itself.
fn describe(error: &JsValue) -> String {
    if let Some(text) = error.as_string() {
        return text;
    }
    js_sys::JsString::from(error.clone()).to_string().into()
}
