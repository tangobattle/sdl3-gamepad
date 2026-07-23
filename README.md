# sdl3-gamepad

Minimal SDL3 gamepad support, decoupled from the rest of SDL3.

The vendored SDL3 this crate links is built from source and trimmed to
the joystick subsystem (which the gamepad API sits on) plus HIDAPI — no
audio, video, render, or GPU. Dropping video removes the X11/Wayland
build dependency on Linux and the Metal/Cocoa link on macOS. See
`Cargo.toml` for the exact feature trim.

The API never surfaces an `sdl3` type, so callers depend on this crate
instead of `sdl3`:

```rust
// On the main thread, once at startup:
sdl3_gamepad::init("MyApp");

// Then, on the same thread, drain input whenever you want to poll:
sdl3_gamepad::pump(|event| match event {
    sdl3_gamepad::GamepadEvent::ButtonDown(button) => { /* ... */ }
    sdl3_gamepad::GamepadEvent::ButtonUp(button) => { /* ... */ }
    sdl3_gamepad::GamepadEvent::AxisMotion { axis, value } => { /* ... */ }
    sdl3_gamepad::GamepadEvent::DeviceRemoved => { /* clear held state */ }
});
```

`init` and `pump` must run on the same thread — SDL3's handles are
`!Send` and enforce main-thread init. Controllers are opened and
hotplugged automatically; callers only ever see the narrow
[`GamepadEvent`] surface.

## License

MPL-2.0
