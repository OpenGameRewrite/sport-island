# Sport Island

This repository is independent of the parent workspace's engine/mod/tracker
workflow. The user explicitly exempted Sport Island from that workflow and TDD;
there are no central TEST/IMPLEMENT issue or Haiku authorship gates here.

Keep changes on a `codex/` branch. Use Bevy's ECS and input directly, and prefer
small implementations with no unnecessary dependencies. Run appropriate checks;
on this machine use `cargo ... --locked -j 2` to avoid exhausting memory.

Only original Rust/source and our own documentation may be committed. Never add
ROMs, extracted models/textures/audio/messages/placements/collision, screenshots
of the original game, binaries, decompilation or disassembly. `assets/` and
scratch research outputs must remain ignored and local. Preserve the whitelist
in `.gitignore`; inspect staged paths before pushing. ROM inspection is for
recovering behavior and independently implementing it, not copying original code.

Use an isolated `SI_SAVE` for native smoke/collection checks to protect the
user's real progress. Keep README's fidelity limitations accurate; constants
recovered from the executable do not establish full behavioral equivalence.
