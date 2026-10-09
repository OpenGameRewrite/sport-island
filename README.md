# Sport Island

A partial Rust/Bevy rewrite of Wii Sports Resort's Wuhu Island and Island Flyover,
built for keyboard and modern gamepads.

**This repository contains no game data.** Every model, texture, sound and text
string is extracted at build time from *your own* Wii Sports Resort disc image.

## Quick start

```sh
# 1. Extract assets from your disc image (ISO, RVZ, WBFS, WIA, GCZ, CISO).
cargo run --release --locked -j 2 -p sport-island-extract -- "path/to/Wii Sports Resort.rvz" assets

# 2. Play.
cargo run --release --locked -j 2 -p sport-island
```

The extractor writes everything to `assets/` (git-ignored). The game looks for
`assets/` in the working directory, next to the executable, or at the repo root.

Supported: the European release (RZTP01). The executable palette lookup is
region-specific; other releases are currently unsupported.

After updating from an earlier demo, run the full extractor again: the runtime
now needs traffic paths, sound files, information-card thumbnails and plane colors.
Extraction asks for a disc path on stdin if no path argument is supplied. It marks
the directory complete only after a successful full run.

## Playing

Choose one of the five disc languages on the title screen and press Enter or the
gamepad's south button to start. The title and pause menus offer controls, plane
selection and original liveries. Plane selection applies when you next board;
changing the livery applies immediately. Arrow keys, D-pad or left-stick flicks
navigate menus. Enter / south confirms, Escape / east goes back.

| Action | Keyboard / mouse | Gamepad default |
|---|---|---|
| Walk / steer aircraft | W A S D | Left stick / D-pad |
| Look | Mouse / I J K L | Right stick |
| Run / boost impulse | Left Shift | RT / R2 |
| Slow impulse | Left Ctrl | LT / L2 |
| Jump / shoot | Space | South (A / Cross) |
| Cut engine | C | East (B / Circle) |
| Flare | F | North (Y / Triangle) |
| Board / parachute out | E | West (X / Square) |
| Pause | Escape | Start |
| Change time of day | T | Back / Select |

In Controls, select an action, confirm, then press a keyboard key, mouse button
or gamepad button. Overrides persist; Reset controls restores defaults. Escape
cancels capture. Menu navigation remains available regardless of bindings.
Look axes and their I J K L fallback are fixed. Pitch inversion is in the pause menu.

The first boarding of each session uses the original water departure for the
selected time of day. Later boarding spawns above your walking position. Boost
is a press impulse with a short re-arm timer and a decaying envelope; release
and press again to boost again. The flight camera follows pitch and heading.
After free look it holds for four seconds, then gradually returns.

Approaching a sightseeing point opens its original localized reading and
thumbnail, with approach/radio audio and a transmission effect. Passing within
50 game units collects that reading. Each of 80 points has three readings, one
for each time of day. The pause record tracks all 240. Settings and progress are
saved to `%APPDATA%/SportIsland/save.json` on Windows or
`$XDG_DATA_HOME/SportIsland/save.json` on Linux (with a home-directory fallback).

## Current scope and fidelity

The demo includes the extracted island, collision, day/evening/night lighting,
animated water and scenery, walking with a basic original character, both plane
models, shooting balloon targets, flares, moving information-point carriers and
positional ambient sounds. Cars use extracted motion clips; several boats and
aircraft use extracted paths. Night fireworks use the original model animation.

The flight solver runs at 60 Hz. Its steering forces, inertia, drag, damping,
lift/thrust parameters and boost envelope were recovered by inspecting the
original executable locally and reimplemented in Rust. Rendering and chase
camera interpolate between physics steps to avoid visible stepping.

This remains a partial rewrite: there is no frame-by-frame original-game oracle
comparison yet. Ground assistance and water takeoff are simplified, and original
reference-body stabilization is incomplete. Boat/aircraft route speeds, flock
steering and firework scheduling are approximations. The information-card layout,
spark, ripples and trails are recreated rather than the original UI/particle
engine; collected markers currently shrink instead of changing to grey checks.
Wave/stream sounds are decoded; sequenced sounds use a limited offline synthesizer,
so their envelopes and effects can differ. Gamepad defaults work through Bevy's
mapping; individual controller mappings still need hardware testing.

## Validation

```sh
cargo test --workspace --locked -j 2
cargo build --workspace --locked -j 2
```

Tests cover collision, placement orientation, steering/bank direction, pitch
inversion, boost retriggering, input overrides, save replacement, path traversal
and audio decoding/attenuation. Native smoke helpers require an isolated save:
`SI_SMOKE=1 SI_SAVE=<temporary-path>` exercises language selection, all 80 markers,
boarding, shooting, flare, pause freeze, ejection and time changes.
`SI_POINT=<0..79>` with `SI_PLAY=1` and `SI_SAVE` visits a point for collection
checks. `SI_SHOT=<absolute.png>` captures at `SI_WAIT` seconds and exits two
seconds later. `SI_CONTROLS=1` previews the binding menu.

## Layout

| Path | What |
|---|---|
| `extract/` | Disc reader (`nod`), Yaz0/U8 archives, BRRES (MDL0/TEX0/CHR0/SRT0) → glTF, BMG text, PMP placements, KCL collision, audio. |
| `game/` | The Bevy game. |

## Legal

Wii Sports Resort is © Nintendo. This project is an independent, non-commercial
reimplementation. Code here is written from scratch; it does not contain
decompiled code, disassembly or any extracted assets. You need to own the game.
