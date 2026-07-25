# gamepad-facade

One gamepad-input API over two very different sources: SDL3 natively, and
the browser [Gamepad API] in a wasm build. Callers see neither.

[Gamepad API]: https://developer.mozilla.org/en-US/docs/Web/API/Gamepad_API

```rust
// On the thread that will poll, once at startup:
gamepad_facade::init("MyApp");

// Then, on that same thread, drain input whenever you want to poll:
while let Some(ev) = gamepad_facade::next_event() {
    use gamepad_facade::EventKind as K;
    match ev.kind {
        K::ButtonDown(button) => { /* ... */ }
        K::ButtonUp(button) => { /* ... */ }
        K::AxisMotion { axis, value } => { /* ... */ }
        K::Connected => { /* ... */ }
        K::Disconnected => { /* drop any state keyed on ev.id */ }
    }
}
```

Every event is tagged with the `Id` it came from; the crate does
not coalesce multiple pads into one logical controller — that is the
caller's call to make.

`init` and `next_event` must run on the same thread, and on native that
thread must be the one SDL was initialized on (its handles are `!Send`
and it checks).

## Why the API is pull-based

Following `gilrs`, input is a stream you pop from rather than a callback
you install — and that is what makes one API over both backends possible.

SDL has a real event queue. The browser has none for buttons and axes:
`navigator.getGamepads()` hands back a *snapshot* of current state, and
that is the entire input surface. What both backends can answer is "give
me the next thing that changed", so that is what the API asks for. The
web backend keeps the previous snapshot and diffs it, turning
level-triggered data back into SDL's edge-triggered stream.

## Backends

Chosen by target and feature, never by the caller:

| target | feature | backend |
| --- | --- | --- |
| `wasm32` | (n/a) | browser Gamepad API |
| anything else | `sdl3` (default) | SDL3 |
| anything else | `--no-default-features` | stub: no gamepads, nothing linked |

The stub exists so a host can drop the vendored SDL build from its link
without touching any call site.

The vendored SDL3 is built from source and trimmed to the joystick
subsystem (which the gamepad API sits on) plus HIDAPI — no audio, video,
render, or GPU. Dropping video removes the X11/Wayland build dependency
on Linux and the Metal/Cocoa link on macOS. See `Cargo.toml` for the
exact feature trim.

## Backend differences worth knowing

* **Triggers are axes**, following SDL. The browser calls them buttons 6
  and 7, but they carry an analog value, so the web backend forwards them
  as `Axis::TriggerLeft` / `Axis::TriggerRight` to keep one contract.
* **Pads attached before `init`** are adopted silently by SDL, with no
  `Connected` event. A browser hides pads from a page until the user
  presses something on one, so there they always produce a `Connected`
  first.
* **`Button` covers SDL's full layout** (paddles, `Misc*`, touchpad
  click). A browser's standard mapping stops at `Guide`, so the rest
  never arrive there.
* **Non-standard browser mappings** are read by raw index anyway, with a
  one-time warning — it is all the browser gives us to work with.

## License

MPL-2.0
