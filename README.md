# tf-demo-parser (tf2jump fork)

This repository is a fork of the TF2 demo parser from demostf.

Upstream:

- Codeberg: https://codeberg.org/demostf/parser
- GitHub mirror: https://github.com/demostf/parser

This fork:

- https://github.com/Hona/parser

This fork exists to power tf2jump.xyz (dribble.tf fork) with a fast Rust/WASM parser that emits viewer-ready typed-array caches.

## What's different from upstream

- Adds a `wasm-bindgen` API (`parse_demo_cache`, `parse_demo_cache_with_progress`) that returns a cache object tailored for the web viewer.
- Uses server ticks (~66.67Hz) as the internal timeline and fills SourceTV snapshot gaps by interpolation so caches have no holes.
- Exports additional per-player caches used by the viewer:
  - `connected` (from player resource `m_bConnected`)
  - `duck` (from `DT_BasePlayer::m_fFlags` using `FL_DUCKING` / `FL_ANIMDUCKING`)
- Preserves inline TF2 chat color markup in SayText2/TextMessage output:
  - TextColor control codes `\x01..\x09`
  - Hex colors `\x07RRGGBB` and `\x08RRGGBBAA` (commonly used by server plugins)
- Exposes analyser accessors on `DemoHandler` to support the custom packet loop used by the wasm cache builder.

## Building

Rust:

```bash
cargo build --release
```

WASM (for dribble.tf):

```bash
wasm-pack build --target web --out-dir "C:\\Repositories\\dribble.tf\\src\\libs\\parser2" --out-name parser2 --release
```

Note: dribble.tf commits the generated wasm artifacts under `src/libs/parser2`, so CI/Pages doesn't need to run `wasm-pack`.

## Syncing with upstream

This repo has an `upstream` remote pointing at `demostf/parser`.

```bash
git fetch upstream
git merge upstream/master
```

## API notes

The wasm output shape is not intended as a stable public API; it's tailored for the tf2jump viewer.
See `C:\\Repositories\\dribble.tf\\src\\components\\Analyse\\Data\\ParseWorker.ts` for the exact JS types.
