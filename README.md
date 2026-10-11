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
sensitivity, ground assist, pitch inversion, a speedometer and fullscreen; **Controls** saves your bindings.

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
| Time of day | T | — (pause menu) |
| i-point list | Tab | — (pause menu) |
| Speedometer | V | — (Settings) |
| Recenter camera | R | RS / R3 (right stick click) |

On a gamepad the time of day, i-point list and speedometer are unbound by
default so they never clash with flying; **Controls** can give them buttons.

Pull back on the stick (S) to take off from the water. Boost and slowing are
temporary impulses: release and press again to repeat them. The optional
speedometer shows game units per second and the remaining impulse. Free look
holds briefly before returning to the flight camera. Hold ↑/↓ or use the wheel
to browse i-points; ←/→ changes pages. The paused ⓘ counter also opens the list.
Each point has a reading for each time of day. Progress saves automatically
to `SportIsland/save.json` in your system's application-data folder.
Switching apps or opening Windows capture tools pauses play and releases the
mouse. Select **Resume** to continue.

## Optional: Pilotwings Resort add-on

With your own **decrypted** dump of Pilotwings Resort (3DS, Europe), run the
extractor a second time on it, into the same `assets` folder:

```sh
cargo run --release --locked -j 2 -p sport-island-extract -- "path/to/Pilotwings Resort.3ds" assets
```

The game works the same without it. With it, the aircraft choice adds the
3DS vehicles, painted in any of the 12 Mii favourite colours (**Livery**):

| Vehicle | Flight | Controls beyond steering |
|---|---|---|
| Plane, Turbo Jet | The 3DS flight model: bank to turn, the plane levels out when you let go | RT / Shift: turbo (2.5 s, then it recharges), LT / Ctrl: brake; LB / Z and RB / X: lean left or right, both: fly upside down, double-tap one: barrel roll |
| Hang Glider | The 3DS glider: ride thermals (the swirling columns) to climb | LT / Ctrl or B / C: brake |
| Pedal Glider | Hang glider plus pedals | Tap RT / Shift (or A / Space) to pedal; pedalling too fast drains the hearts |
| Rocket Belt, Super Rocket Belt | The 3DS jets; the pilot stays upright and turns on the spot | RT / Shift: full jets, A / Space: light throttle, LT / Ctrl: air brake; the stick leans the jets |

Each vehicle uses its 3DS sounds and moving parts: control surfaces, the pedal
glider's crank and propellers, swivelling rocket nozzles and their flames, and a
pilot who leans and pedals. Settings → **3DS plane handling** switches the plane and jet to the Wii plane's
physics (Wii-style) if you prefer that feel; leans and barrel rolls then do
nothing. The island also gains the 3DS floating landing pads,
tugboat decks and the cruising landing-pad blimp (all solid: land or walk on
them, and the blimp carries you along), the Boost Gate (fly through it for a
burst of speed) and the i-points Pilotwings Resort added.

Each vehicle class also has its 3DS free-flight collectibles, shown only while
you fly it: 60 Gold Rings for the gliders, 60 Stunt Rings for the plane and jet
(fly through while leaning, upside down or barrel-rolling as the ring asks;
spinning rings want a roll that way) and 60 Mii Trophies for the rocket belts,
plus 40 balloons each. The counts show under the i-point counter while you
fly, all of them are listed at the end of the i-point list (Tab, or the pause menu), and they
are saved. The 3DS shows your Mii inside each Stunt Ring, posed to show the
stunt; there is no Mii here, so the rings are open and the stunt it asks for
(and its buttons) appears at the top of the screen as one comes up ahead.
Collectibles placed right at the ground or water are lifted to rest on it
rather than sink half into it. Shared i-points keep the Wii wording; build with
`--features pwr-text` to read the 3DS wording instead.

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
