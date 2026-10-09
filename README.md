# Sport Island

A partial Rust/Bevy rewrite of Wii Sports Resort's Wuhu Island and Island Flyover,
built for keyboard and modern gamepads.

**This repository contains no game data.** Every model, texture, sound and text
string is extracted at build time from *your own* Wii Sports Resort disc image.

## Quick start

```sh
# 1. Extract assets from your disc image (ISO, RVZ, WBFS, WIA, GCZ, CISO).
cargo run --release -p sport-island-extract -- "path/to/Wii Sports Resort.rvz" assets

# 2. Play.
cargo run --release -p sport-island
```

The extractor writes everything to `assets/` (git-ignored). The game looks for
`assets/` in the working directory, next to the executable, or at the repo root.

Supported: the European release (RZTP01). Other regions may work but are untested.

## Layout

| Path | What |
|---|---|
| `extract/` | Disc reader (`nod`), Yaz0/U8 archives, BRRES (MDL0/TEX0/CHR0/SRT0) → glTF, BMG text, PMP placements, KCL collision, audio. |
| `game/` | The Bevy game. |

## Legal

Wii Sports Resort is © Nintendo. This project is an independent, non-commercial
reimplementation. Code here is written from scratch; it does not contain
decompiled code, disassembly or any extracted assets. You need to own the game.
