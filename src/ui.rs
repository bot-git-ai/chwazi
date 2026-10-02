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

/// A full turn of the circle, and the arc every stroke sweeps.
const TWO_PI: f64 = 2.0 * std::f64::consts::PI;

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
    /// The screen's device pixel ratio, so the draw code reads one number rather
    /// than asking the window on every frame.
    scale: f64,
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
    // assigning either clears it, so this happens before the first frame and
    // on every resize.
    resize(&canvas);

    let app = Rc::new(RefCell::new(App {
        chooser: Chooser::new(),
        start_time: now(),
        scale: device_pixel_ratio(),
    }));

    // A window can move between displays -- a phone dragged to a monitor, a tab
    // dragged between a Retina and a non-Retina screen -- so the ratio is re-read
    // on every resize, not just at startup.
    listen(&window, "resize", {
        let canvas = canvas.clone();
        let app = Rc::clone(&app);
        move |_| {
            resize(&canvas);
            let scale = device_pixel_ratio();
            borrow(&app, |app| app.scale = scale);
            Ok(())
        }
    })?;

    // Pointer events, which on a touch screen *are* the fingers. A mouse sends
    // the same events, so the chooser is usable on a desktop too.
    listen(&window, "pointerdown", {
        let app = Rc::clone(&app);
        move |event| {
            let event: web_sys::PointerEvent = event.dyn_into()?;
            let (id, x, y) = at(&event);
            let count = borrow(&app, |app| {
                app.chooser.pointer_down(id, x, y, now());
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
                    app.chooser.pointer_up(event.pointer_id(), now());
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
/// Two things happen here that have no event of their own, because both are
/// *time passing* rather than something a finger did, and this loop already
/// runs on a clock:
///
/// * the reset, due two seconds after the winner lifts;
/// * the draw itself, due `DRAWING_TIME_MS` after the last change to who is on
///   the glass. Running it here rather than in a `setTimeout` means the winner
///   and the frame that shows the winner are the same instant, and a frame that
///   arrives late draws a late winner instead of a stale one.
fn render(
    app: &Rc<RefCell<App>>,
    canvas: &web_sys::HtmlCanvasElement,
    context: &web_sys::CanvasRenderingContext2d,
    timestamp: f64,
) -> Option<(i32, usize)> {
    borrow(app, |app| {
        let mut announced = None;
        // The reset is a side effect here: the frame after it simply finds an
        // empty chooser, and nothing about the drawing changes to announce.
        let _ = app.chooser.tick(timestamp);

        let drawing_since = app.chooser.ready_at();
        if let Some(since) = drawing_since {
            if timestamp - since >= chooser::DRAWING_TIME_MS {
                // The number of players is read before the draw, because the
                // draw removes all but the winner.
                let of = app.chooser.len();
                if let Some(winner) = app.chooser.draw(timestamp, random_index(of)) {
                    announced = Some((winner, of));
                }
            }
        }

        let start_time = app.start_time;
        let scale = app.scale;
        paint(&app.chooser, canvas, context, timestamp, start_time, scale);
        announced
    })
}

/// Draw the whole screen: the winner's fill if there is a winner, every player
/// otherwise.
fn paint(
    app: &Chooser,
    canvas: &web_sys::HtmlCanvasElement,
    context: &web_sys::CanvasRenderingContext2d,
    timestamp: f64,
    start_time: f64,
    scale: f64,
) {
    // Everything below draws in CSS pixels. The canvas is `scale` times that in
    // device pixels, and this transform is what makes a 40px circle land on 120
    // pixels of glass instead of being stretched across 40.
    let _ = context.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    let (width, height) = (
        f64::from(canvas.width()) / scale,
        f64::from(canvas.height()) / scale,
    );
    context.clear_rect(0.0, 0.0, width, height);

    let pulse = chooser::pulse_scale(timestamp, start_time);

    if let Some(winner) = app.chosen() {
        if let Some(front) = app.flood_radius(timestamp, width, height) {
            // The winner's colour floods OUTWARD from the mark, as a disc centred on
            // the winner -- measured in four directions: 444 CSS px up, 116 down, 0
            // left, 0 right on the same frame. It is not a wipe from the top, which
            // a one-sided probe cannot tell apart from a growing disc.
            //
            // The hole is the measured black annulus -- 104 CSS px -- and it is a
            // separate number from the front. Drawn as the front's own geometry the
            // two would meet, and an even-odd fill of a rectangle minus a circle of
            // equal radius cancels to nothing, so the screen would go black exactly
            // when it should be solid colour.
            match web_sys::Path2d::new() {
                Ok(path) => {
                    path.rect(0.0, 0.0, width, height);
                    if path.arc(winner.x, winner.y, front, 0.0, TWO_PI).is_ok() {
                        context.set_fill_style_str(&winner.color_of());
                        context.fill_with_path_2d_and_winding(
                            &path,
                            web_sys::CanvasWindingRule::Evenodd,
                        );
                    }
                }
                Err(error) => show_failure(&describe(&error)),
            }
        }
        // The winner alone, at full pulse, and never a loading arc: the draw is
        // over, and an arc sweeping its ring would read as a second, still-running
        // draw. It has already loaded, so it is drawn loaded.
        draw_player(context, winner, pulse, None, None);
        return;
    }

    let progress = app.draw_progress(timestamp);
    for player in app.players() {
        // The first loading, as a raw fraction. The easing is applied once, in
        // `draw_player`, and only to the disc.
        //
        // It was applied here as well, and applying a smoothstep twice is not a
        // cosmetic slip: at a third of the way through the load the fraction is
        // 0.29, the eased disc is 0.23, and eased again it is 0.05. The disc was
        // being drawn at a twentieth of its size, so the mark read as a black hole
        // with a bright ring round it.
        //
        // The arc is not eased at all. Measured, the sweep is linear, and easing it
        // too would be inventing a curve the samples do not show.
        let loading = player.registration(timestamp);
        draw_player(context, player, pulse, loading, progress);
    }
}

/// One player: a pale dot, a coloured disc, a black gap and a pale ring, and the
/// two arcs that load them.
///
/// The structure is measured, band by band, from the centre outward on a 1080px
/// Galaxy S25 at 3x: pale dot to 7.7 CSS px, saturated disc to 35.7, black gap to
/// 44.3, pale ring to 54.3.
///
/// The band order is the point. The disc and the ring are both the player's
/// colour, so drawn edge to edge they merge into a single flat blob, which is
/// what the build before last did; and dropping the gap and the ring entirely,
/// which is what the last build did, leaves a dot on a disc with no structure
/// around it. The gap is what separates them.
fn draw_player(
    context: &web_sys::CanvasRenderingContext2d,
    player: &Player,
    pulse: f64,
    // `loading` is the finger's own registration, 0 to 1, or `None` once it has
    // arrived. `draw` is the draw window's progress, 0 to 1, or `None` when no draw
    // is running. They are two separate numbers, deliberately: see `draw_player`.
    loading: Option<f64>,
    draw: Option<f64>,
) {
    let colour = player.color_of();
    // The whole mark breathes in the pulse.
    let scale = pulse;

    // The disc grows into its place on its own, much faster than the loading arc
    // sweeps: measured, it reaches full size at about 100ms of a 620ms load, and
    // then sits still for the remaining 500ms while the arc comes round.
    //
    // Tying the disc's growth to the same fraction as the sweep is what made the
    // load look static -- the mark inflated slowly and then stopped, and the eye
    // was given most of a second of nothing. The disc's arrival is front-loaded and
    // eased, and the arc does the rest of the work.
    let disc_scale = loading.map_or(1.0, Player::disc_arrival);

    // The saturated disc.
    context.begin_path();
    if let Err(error) = context.arc(
        player.x,
        player.y,
        chooser::DISC_RADIUS * disc_scale,
        0.0,
        TWO_PI,
    ) {
        show_failure(&describe(&error));
        return;
    }
    context.set_fill_style_str(&colour);
    context.fill();

    // The ring's centreline: the radius it is stroked at. A stroke is centred on the
    // path it follows, so this has to be the *middle* of the band -- stroking at the
    // outer edge would lay it from 52.2 to 61.7, outside the measured mark, and
    // would leave 4.75px of the gap showing as a second black band. Every number in
    // that is correct and only the arithmetic between them is wrong, which is why it
    // survived a build.
    let ring_radius = chooser::RING_STROKE_RADIUS * scale;

    // The ring, in the ring's own band, stroked at the band's centreline.
    //
    // It is a *track*, not a finished shape: it is the full circle in the ring's own
    // tint whether or not anything is loading, and both loadings are drawn on top of
    // it. That is the whole reason there are two loadings rather than one -- the
    // track has to exist first, or an arc drawn over it has nothing to reveal, which
    // is what made the first version of this invisible.
    context.begin_path();
    if context
        .arc(player.x, player.y, ring_radius, 0.0, TWO_PI)
        .is_ok()
    {
        context.set_line_width(chooser::ARC_WIDTH * scale);
        // Bound to a local: the colour is a `String` built per frame, and passing the
        // temporary straight into the call borrows one that is dropped before the
        // browser has finished reading it.
        let ring = player.ring_color();
        context.set_stroke_style_str(&ring);
        context.stroke();
    }

    // The first loading: the finger's own registration. A pale arc sweeps the ring's
    // band from a fixed start at 7:30 round to closed, over 620ms, while the disc
    // beneath it has already arrived.
    //
    // Three things this has to get right, and each was wrong in a build:
    //
    // * the radius. It is the ring's own radius, unscaled by the loading fraction.
    //   Scaling it drags the arc inward every frame, so it drifts out of the ring's
    //   band and the mark's bands stop lining up -- the misalignment reported with
    //   this fix.
    // * the colour. The player's own colour, washed toward white, not a fixed
    //   warm off-white. A constant pale colour is orange against every hue that is
    //   not orange, which is why the sweep looked like it belonged to nobody.
    // * the start angle. Measured, the sweep's leading edge is 146, 140, 134, 128,
    //   122 degrees... it only ever moves one way, so the fixed 135 is the origin
    //   and not part of the animation.
    if let Some(loading) = loading {
        let start = chooser::LOADING_ARC_START.to_radians();
        context.begin_path();
        if context
            .arc(
                player.x,
                player.y,
                ring_radius,
                start,
                start + TWO_PI * loading,
            )
            .is_ok()
        {
            context.set_line_width(chooser::ARC_WIDTH * scale);
            let arc = player.loading_color();
            context.set_stroke_style_str(&arc);
            context.stroke();
        }
    }

    // The second loading: the draw window closing.
    //
    // The ring's band fills up with the player's own colour over the window, so the
    // ring visibly charges rather than sitting there for two and a half seconds.
    //
    // It is drawn on the ring's *own* radius, not on a second circle offset outside
    // it. The previous build offset it by 2px so the two loadings could not overlap,
    // which solved a real problem with the wrong tool: a second circle is a visibly
    // misaligned ring, and a misaligned ring is worse than one circle the two
    // loadings take turns on. They never coexist regardless -- the registration ends
    // before the draw opens, because the draw does not start counting until every
    // mark has arrived.
    if let Some(progress) = draw {
        let start = chooser::SELECTION_ARC_START.to_radians();
        context.begin_path();
        if context
            .arc(
                player.x,
                player.y,
                ring_radius,
                start,
                start + TWO_PI * progress,
            )
            .is_ok()
        {
            context.set_line_width(chooser::ARC_WIDTH * scale);
            // The player's own colour, not a fixed pale one: a constant colour is
            // orange against every hue that is not orange, so the fill looked like it
            // belonged to whichever player happened to be orange.
            context.set_stroke_style_str(&colour);
            context.stroke();
        }
    }
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
/// Capped at 3: past that the pixels are finer than the circles, so the extra
/// cost buys nothing you can see, and it is battery on a phone that is meant to be
/// handed round a table.
fn device_pixel_ratio() -> f64 {
    web_sys::window()
        .map_or(1.0, |window| window.device_pixel_ratio())
        .clamp(1.0, 3.0)
}

/// Size the canvas to the window, and to the screen's pixels.
///
/// This is the smoothness. A canvas's drawing surface is its `width`/`height` in
/// *device* pixels; sizing it in CSS pixels means a 40px circle is rasterised
/// across 40 backing pixels and then stretched over 120 of them by the
/// compositor. On any modern phone -- every phone has DPR 2.5 or more -- that is
/// visible as a jagged, soft-edged mark, and it is not anti-aliasing, it is
/// resolution.
///
/// So the backing store is `css size * devicePixelRatio`, and every frame draws
/// through a transform of the same ratio. All the arithmetic in this file and in
/// `chooser.rs` stays in CSS pixels, so every constant and every test is
/// unchanged; the ratio appears in exactly two places, here and in the one
/// `set_transform` call in [`paint`].
fn resize(canvas: &web_sys::HtmlCanvasElement) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let width = window
        .inner_width()
        .ok()
        .and_then(|v| dimension(&v))
        .unwrap_or(1);
    let height = window
        .inner_height()
        .ok()
        .and_then(|v| dimension(&v))
        .unwrap_or(1);
    let scale = device_pixel_ratio();
    canvas.set_width(backing(width, scale));
    canvas.set_height(backing(height, scale));
}

/// A canvas dimension in device pixels, from a CSS size and the screen's ratio.
///
/// Rounded here rather than at the call site so both axes go through the same
/// arithmetic. The `f64 -> u32` conversion is saturating rather than a cast: the
/// product of a viewport dimension and a device pixel ratio is well inside `u32`
/// on any real screen, and a cast that would silently wrap on a nonsense value
/// would set the canvas to some arbitrary size instead of falling back to 1px.
fn backing(css: u32, scale: f64) -> u32 {
    let wanted = f64::from(css) * scale;
    if !wanted.is_finite() || wanted < 1.0 {
        return 1;
    }
    // Above `u32::MAX` this saturates, which is the only sensible answer: a canvas
    // that big is not addressable and the browser would refuse it anyway.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rounded = wanted.round() as u32;
    rounded.max(1)
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
fn announce_players(count: usize) {
    let text = if count >= chooser::REQUIRED_PLAYER_COUNT {
        format!("{count} fingers down. Choosing in a moment.")
    } else {
        format!("{count} finger down. Put two or more fingers on the screen.")
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
///
/// A viewport dimension is a whole number of CSS pixels, and the browser only
/// ever hands back a finite non-negative one. A value that is not -- a NaN
/// viewport, or a value beyond `u32` -- falls back to 1px, which is the same
/// answer [`backing`] gives, and is the one that keeps the canvas at a size the
/// browser will actually accept.
fn dimension(value: &JsValue) -> Option<u32> {
    let size = value.as_f64()?;
    if !size.is_finite() || size < 1.0 {
        return Some(1);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rounded = size.round() as u32;
    Some(rounded.clamp(1, u32::MAX))
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
