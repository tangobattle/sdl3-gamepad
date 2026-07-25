//! The browser backend, over the [Gamepad API].
//!
//! [Gamepad API]: https://developer.mozilla.org/en-US/docs/Web/API/Gamepad_API
//!
//! # Why this polls and diffs
//!
//! The browser has no event for a button press or a stick movement. It
//! offers `navigator.getGamepads()`, which hands back a *snapshot* of
//! every pad's current buttons and axes, and that is the whole input
//! surface. (`gamepadconnected` / `gamepaddisconnected` exist, but they
//! are not needed here: a pad appears in and vanishes from the snapshot,
//! which is the same information without a listener to install.)
//!
//! So this backend keeps the previous snapshot, and when the caller asks
//! for an event and the queue is empty it takes a new one and emits the
//! differences. That reproduces SDL's edge-triggered stream from
//! level-triggered data, which is what lets both backends answer the
//! same [`crate::next_event`].
//!
//! One consequence worth knowing: pads are invisible to a page until the
//! user presses something on one ("gamepad gesture" gating, for
//! fingerprinting reasons). So unlike the native backend, there is no
//! such thing as a pad that was already attached at [`init`] — the first
//! input on a pad is what reveals it, and it always gets a `Connected`
//! before anything else.
//!
//! # Threading
//!
//! No globals to protect and no `Send` to worry about: state lives in a
//! thread-local, so [`init`] and [`next_event`] naturally have to be
//! called from the one thread, which is what the crate's contract
//! already says.

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::JsCast as _;

use crate::{Axis, Button, Event, EventKind, Id};

/// The buttons of the browser's ["standard gamepad"] mapping, by index.
///
/// Indices 6 and 7 are deliberately absent: those are the triggers,
/// which the browser reports as buttons carrying an analog `value`. This
/// crate's contract (following SDL) makes triggers axes, so they are
/// handled separately in [`diff_pad`] and emitted as
/// [`Axis::TriggerLeft`] / [`Axis::TriggerRight`].
///
/// ["standard gamepad"]: https://w3c.github.io/gamepad/#remapping
const STANDARD_BUTTONS: &[(usize, Button)] = &[
    (0, Button::South),
    (1, Button::East),
    (2, Button::West),
    (3, Button::North),
    (4, Button::LeftShoulder),
    (5, Button::RightShoulder),
    (8, Button::Back),
    (9, Button::Start),
    (10, Button::LeftStick),
    (11, Button::RightStick),
    (12, Button::DPadUp),
    (13, Button::DPadDown),
    (14, Button::DPadLeft),
    (15, Button::DPadRight),
    (16, Button::Guide),
];

/// Browser button index of each trigger, and the axis it becomes.
const TRIGGER_BUTTONS: &[(usize, Axis)] = &[(6, Axis::TriggerLeft), (7, Axis::TriggerRight)];

/// The standard mapping's four stick axes, by index. Sign convention
/// matches SDL's (up and left are negative), so values pass through
/// untouched.
const STANDARD_AXES: &[(usize, Axis)] = &[
    (0, Axis::LeftX),
    (1, Axis::LeftY),
    (2, Axis::RightX),
    (3, Axis::RightY),
];

/// How much an axis must move before it counts as motion. The browser
/// reports full `f64` precision and a resting stick jitters in the last
/// digits, which would otherwise emit an event on every single poll.
/// SDL's own event queue coalesces at a coarser granularity than this.
const AXIS_EPSILON: f32 = 1.0 / 512.0;

/// Last-seen state of one pad, as the snapshot reported it.
#[derive(Default)]
struct PadState {
    /// Pressed-ness per browser button index.
    buttons: Vec<bool>,
    /// Value per browser axis index.
    axes: Vec<f32>,
    /// Value per trigger button index, kept apart from `buttons`
    /// because triggers are analog here.
    triggers: Vec<f32>,
}

#[derive(Default)]
struct Backend {
    /// `false` until `init` ran, so `next_event` stays a no-op like the
    /// native backend's does when SDL failed to start.
    live: bool,
    pads: HashMap<u32, PadState>,
    /// Events synthesized by the last poll and not yet handed out.
    queue: std::collections::VecDeque<Event>,
    /// Whether a non-standard mapping has already been complained about,
    /// so a pad the browser can't map doesn't log once per poll forever.
    warned_nonstandard: bool,
}

thread_local! {
    static BACKEND: RefCell<Backend> = RefCell::new(Backend::default());
}

pub fn init(_app_name: &str) {
    // Nothing to spin up — `getGamepads` is available on any Navigator,
    // and there is nowhere to put an app name. Just check we can reach a
    // Navigator at all, so a host running outside a browsing context
    // (say, a bare worker with no `window`) degrades to "no gamepads"
    // rather than failing on every poll.
    if navigator().is_none() {
        log::warn!("gamepad: no Navigator available; gamepad input disabled");
        return;
    }
    BACKEND.with(|b| b.borrow_mut().live = true);
}

pub fn next_event() -> Option<Event> {
    BACKEND.with(|b| {
        let mut b = b.borrow_mut();
        if !b.live {
            return None;
        }
        if let Some(ev) = b.queue.pop_front() {
            return Some(ev);
        }
        poll(&mut b);
        b.queue.pop_front()
    })
}

fn navigator() -> Option<web_sys::Navigator> {
    Some(web_sys::window()?.navigator())
}

/// Take a snapshot, diff it against the last one, and fill the queue.
fn poll(b: &mut Backend) {
    let Some(navigator) = navigator() else {
        return;
    };
    let Ok(list) = navigator.get_gamepads() else {
        return;
    };

    let mut seen: Vec<u32> = Vec::new();

    for entry in list.iter() {
        // The list is sparse: a disconnected slot is a hole (null).
        let Ok(pad) = entry.dyn_into::<web_sys::Gamepad>() else {
            continue;
        };
        if !pad.connected() {
            continue;
        }
        let index = pad.index();
        seen.push(index);

        if pad.mapping() != web_sys::GamepadMappingType::Standard && !b.warned_nonstandard {
            b.warned_nonstandard = true;
            log::warn!(
                "gamepad {index} ({}) has no standard mapping; buttons and axes are being read by raw index and may be wrong",
                pad.id()
            );
        }

        let fresh = !b.pads.contains_key(&index);
        if fresh {
            b.pads.insert(index, PadState::default());
            b.queue.push_back(Event {
                id: Id(index),
                kind: EventKind::Connected,
            });
        }
        diff_pad(b, index, &pad);
    }

    // Anything we knew about and didn't see is gone.
    let dropped: Vec<u32> = b
        .pads
        .keys()
        .copied()
        .filter(|i| !seen.contains(i))
        .collect();
    for index in dropped {
        b.pads.remove(&index);
        b.queue.push_back(Event {
            id: Id(index),
            kind: EventKind::Disconnected,
        });
    }
}

/// Emit an event for every button and axis that changed since the last
/// snapshot of this pad.
fn diff_pad(b: &mut Backend, index: u32, pad: &web_sys::Gamepad) {
    let buttons = pad.buttons();
    let axes = pad.axes();
    let id = Id(index);
    let mut events: Vec<Event> = Vec::new();

    let state = b.pads.entry(index).or_default();

    // Buttons: a plain pressed/released edge.
    let button_count = buttons.length() as usize;
    state.buttons.resize(button_count, false);
    for &(idx, button) in STANDARD_BUTTONS {
        if idx >= button_count {
            continue;
        }
        let Some(pressed) = button_pressed(&buttons, idx) else {
            continue;
        };
        if pressed != state.buttons[idx] {
            state.buttons[idx] = pressed;
            events.push(Event {
                id,
                kind: if pressed {
                    EventKind::ButtonDown(button)
                } else {
                    EventKind::ButtonUp(button)
                },
            });
        }
    }

    // Triggers: buttons in the browser, axes in this API.
    state.triggers.resize(button_count, 0.0);
    for &(idx, axis) in TRIGGER_BUTTONS {
        if idx >= button_count {
            continue;
        }
        let Some(value) = button_value(&buttons, idx) else {
            continue;
        };
        if (value - state.triggers[idx]).abs() >= AXIS_EPSILON {
            state.triggers[idx] = value;
            events.push(Event {
                id,
                kind: EventKind::AxisMotion { axis, value },
            });
        }
    }

    // Sticks.
    let axis_count = axes.length() as usize;
    state.axes.resize(axis_count, 0.0);
    for &(idx, axis) in STANDARD_AXES {
        if idx >= axis_count {
            continue;
        }
        let value = axes.get(idx as u32).as_f64().unwrap_or(0.0) as f32;
        let value = value.clamp(-1.0, 1.0);
        if (value - state.axes[idx]).abs() >= AXIS_EPSILON {
            state.axes[idx] = value;
            events.push(Event {
                id,
                kind: EventKind::AxisMotion { axis, value },
            });
        }
    }

    b.queue.extend(events);
}

fn button_at(buttons: &js_sys::Array, idx: usize) -> Option<web_sys::GamepadButton> {
    buttons
        .get(idx as u32)
        .dyn_into::<web_sys::GamepadButton>()
        .ok()
}

fn button_pressed(buttons: &js_sys::Array, idx: usize) -> Option<bool> {
    Some(button_at(buttons, idx)?.pressed())
}

fn button_value(buttons: &js_sys::Array, idx: usize) -> Option<f32> {
    Some(button_at(buttons, idx)?.value().clamp(0.0, 1.0) as f32)
}
