# Sport Island

Explore Wuhu Island on foot or by plane in this partial Rust/Bevy rewrite of
Wii Sports Resort, with keyboard and modern gamepad controls.

Fly either aircraft, choose a livery, shoot balloons and discover all 80
sightseeing points. Read their original descriptions in five languages, explore
by day, evening or night, and keep your collection progress between sessions.

## Getting started

You need [Rust](https://www.rust-lang.org/tools/install) and your own disc image
of the **European Wii Sports Resort release (RZTP01)**. ISO, RVZ, WBFS, WIA, GCZ
and CISO images are supported.

From the repository folder, extract the game data once, then launch:

```sh
cargo run --release --locked -j 2 -p sport-island-extract -- "path/to/Wii Sports Resort.rvz" assets
cargo run --release --locked -j 2 -p sport-island
```

Choose your language, aircraft and color, then select **Explore**. Menus support
mouse clicks, arrow keys and gamepads. **Settings** includes flight/camera
sensitivity, pitch inversion, a speedometer and fullscreen; **Controls** saves your bindings.

## Controls

| Action | Keyboard / mouse | Gamepad |
|---|---|---|
| Walk / steer | W A S D | Left stick / D-pad |
| Look | Mouse / I J K L | Right stick |
| Run / boost | Left Shift | RT / R2 |
| Slow down | Left Ctrl | LT / L2 |
| Jump / shoot | Space | A / Cross |
| Cut engine | C | B / Circle |
| Flare | F | Y / Triangle |
| Board / parachute | E | X / Square |
| Pause | Escape | Start |
| Time of day | T | Back / Select |
| i-point list | Tab | LB / L1 |
| Speedometer | V | RB / R1 |
| Recenter camera | R | RS / R3 (right stick click) |

Pull back on the stick (S) to take off from the water. Boost and slowing are
temporary impulses: release and press again to repeat them. The optional
speedometer shows game units per second and the remaining impulse. Free look
holds briefly before returning to the flight camera. Hold ↑/↓ or use the wheel
to browse i-points; ←/→ changes pages. The paused ⓘ counter also opens the list.
Each point has a reading for each time of day. Progress saves automatically
to `SportIsland/save.json` in your system's application-data folder.
Switching apps or opening Windows capture tools pauses play and releases the
mouse. Select **Resume** to continue.

## Work in progress

Original scenery, messages and sounds come from your disc. Flight behavior is
being reconstructed from the game, but this is still an exploration demo:
wave/contact stabilization, some traffic steering, rarely used particle
features (stripes, indirect textures, some force fields) and parts of the
sequence synthesizer remain approximate. After updating, run the extractor
again: newer builds read data the old extraction did not write.

## About

This independent, non-commercial project contains only original code; no
Nintendo assets or original game code are distributed. Wii Sports Resort is
© Nintendo. Extracted data stays on your computer.
