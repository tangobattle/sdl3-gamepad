//! One gamepad-input API over two very different sources: SDL3 natively,
//! and the browser's [Gamepad API] in a wasm build.
//!
//! Callers see only [`Button`], [`Axis`], [`Id`] and
//! [`Event`], drive input with [`init`] + [`next_event`], and stay
//! oblivious to which backend is underneath.
//!
//! [Gamepad API]: https://developer.mozilla.org/en-US/docs/Web/API/Gamepad_API
//!
//! # Event model
//!
//! Following `gilrs`, input is a pull-based stream rather than a
//! callback: [`next_event`] pops one [`Event`] at a time (loop
//! `while let Some(ev) = next_event()` to drain a frame), and every
//! event is tagged with the [`Id`] it came from. Connect and
//! disconnect surface as their own [`EventKind`] variants. The
//! crate does **not** coalesce multiple pads into one logical
//! controller — that's the caller's call to make, keyed on `id`.
//!
//! Pull is what makes one API over both backends possible. SDL has a
//! real event queue; the browser has none for buttons and axes — you
//! read a snapshot and compare it to the last one. Both can answer "give
//! me the next thing that changed", so that is what the API asks for.
//!
//! # Threading
//!
//! [`init`] and [`next_event`] must run on the same thread, and on
//! native that thread must be the one SDL was initialized on (SDL's
//! handles are `!Send` and it checks). Neither is a real constraint for
//! a UI thread, which is where input belongs anyway.
//!
//! # Backends
//!
//! Chosen by target and feature, not by the caller:
//!
//! * wasm32 → the browser Gamepad API. Always; there is nothing else.
//! * anything else with the default `sdl3` feature → SDL3.
//! * anything else without it → a stub that reports no gamepads, so a
//!   host that doesn't want to link SDL still builds and runs.

#[cfg(all(feature = "sdl3", not(target_arch = "wasm32")))]
mod sdl3;

#[cfg(target_arch = "wasm32")]
mod web;

/// A gamepad button.
///
/// Mirrors SDL3's standard layout 1:1, which the browser's "standard
/// gamepad" mapping also follows — beyond the usual Xbox/PS
/// face/shoulder/d-pad set this covers the extras on fancier pads: the
/// `Misc*` share/capture-style buttons, the four back paddles, and the
/// touchpad click. A browser only ever reports the first seventeen of
/// these, since its standard mapping stops at `Guide`.
///
/// Triggers are **not** buttons here: SDL reports them as axes, so they
/// come through [`Axis::TriggerLeft`] / [`Axis::TriggerRight`]. The web
/// backend converts — the browser calls them buttons 6 and 7, but they
/// carry an analog `value`, so they are forwarded as axis motion to keep
/// one contract across backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    South, // A on Xbox, X on PS
    East,  // B on Xbox, Circle on PS
    West,  // X on Xbox, Square on PS
    North, // Y on Xbox, Triangle on PS
    Back,  // Select / Share
    Start,
    Guide, // Guide / PS button
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Misc1,
    Misc2,
    Misc3,
    Misc4,
    Misc5,
    Misc6,
    RightPaddle1,
    LeftPaddle1,
    RightPaddle2,
    LeftPaddle2,
    Touchpad,
}

/// A gamepad analog axis, mirroring SDL3's naming. Values are
/// pre-normalized to `f32` in `[-1, 1]` in SDL's own convention
/// (stick-up is negative Y), which the browser's standard mapping shares
/// — so no backend has to flip anything. Triggers run `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    TriggerLeft,
    TriggerRight,
}

/// Opaque, per-connection identifier for a gamepad. Stable while a pad
/// stays plugged in, and reusable for a different physical pad after a
/// disconnect — so treat it as meaningful only between a [`Connected`]
/// and its matching [`Disconnected`]. Callers key their per-device state
/// on this.
///
/// Natively this is SDL's joystick instance id; in a browser it is the
/// gamepad's `index`.
///
/// [`Connected`]: EventKind::Connected
/// [`Disconnected`]: EventKind::Disconnected
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub u32);

/// One gamepad event, tagged with the device it came from. Mirrors
/// `gilrs`'s `Event { id, event }` split so callers can route or
/// coalesce per device however they like.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub id: Id,
    pub kind: EventKind,
}

/// The narrow slice of gamepad input this crate emits. Keeping the
/// surface this small is what lets one API cover both backends.
#[derive(Clone, Copy, Debug)]
pub enum EventKind {
    /// A controller became available. Pads already attached when
    /// [`init`] ran are adopted silently on the native backend, with no
    /// event; in a browser a pad is invisible until the user presses
    /// something on it, so first input always produces a `Connected`
    /// first.
    Connected,
    /// A controller went away. Callers should drop any held state keyed
    /// on this device's `id` so its buttons don't read as still-down.
    Disconnected,
    ButtonDown(Button),
    ButtonUp(Button),
    AxisMotion {
        axis: Axis,
        value: f32,
    },
}

/// Initialize gamepad input. Call once at startup, on the thread that
/// will later call [`next_event`].
///
/// `app_name` identifies the application to the platform where it can
/// (SDL passes it to D-Bus); a browser has nowhere to put it and ignores
/// it. Any failure is logged and turns subsequent [`next_event`] calls
/// into no-ops — the app keeps running without gamepad support rather
/// than taking the process down.
pub fn init(app_name: &str) {
    backend::init(app_name);
}

/// Pop the next gamepad event, or `None` once there is nothing left for
/// now. Callers pull in a loop — `while let Some(ev) = next_event() { … }`
/// — to consume a frame's worth of input. Device add/remove is handled
/// internally *and* surfaced as [`EventKind::Connected`] /
/// [`EventKind::Disconnected`]. Always `None` if [`init`] never
/// succeeded.
///
/// Must run on the thread that called [`init`].
pub fn next_event() -> Option<Event> {
    backend::next_event()
}

#[cfg(all(feature = "sdl3", not(target_arch = "wasm32")))]
use sdl3 as backend;

#[cfg(target_arch = "wasm32")]
use web as backend;

/// No gamepads, ever. What a native build without the `sdl3` feature
/// gets: the API stays callable so a host can drop SDL from its link
/// without touching any call sites.
#[cfg(not(any(
    target_arch = "wasm32",
    all(feature = "sdl3", not(target_arch = "wasm32"))
)))]
mod backend {
    pub fn init(_app_name: &str) {
        log::info!("gamepad support is not compiled in (no `sdl3` feature)");
    }

    pub fn next_event() -> Option<super::Event> {
        None
    }
}
