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

Choose a language on the title screen. The title and pause menus also offer
plane selection, colors and saved control bindings; pitch inversion is in the
pause menu. Navigate with arrow keys, D-pad or left stick; Enter / A confirms.

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

Pull back on the stick (S) to take off from the water. Release and press again
for another boost. Free look holds briefly before
returning to the flight camera. Each sightseeing point has a reading for each
time of day; see your collection in the pause menu. Progress saves automatically
to `SportIsland/save.json` in your system's application-data folder.

## Work in progress

Original scenery, messages and sounds come from your disc. Flight behavior is
being reconstructed from the game, but this is still an exploration demo:
some flight assistance, traffic, effects and sequenced audio remain approximate.

## About

This independent, non-commercial project contains only original code; no
Nintendo assets or original game code are distributed. Wii Sports Resort is
© Nintendo. Extracted data stays on your computer.
