# Telera Application Framework

a GUI app framework designed for performance, and modularity.

## Features:
- build UI elements in a custom script or directly in code
- UI elements are modular and reusable
- UI is immediate mode and definitionaly reactive.
    - Only the layout is stored in script, all the data lives in *your* user application
- UI layout is similar to CSS's flex layout system.
- Hardware rendering of the UI
    - rendering can automatically be switched to CPU if no graphics card is found
    - small layouts can render on the order of microseconds
- Included convenience macros remove most of the Rust boilerplate
    - go from empty folder to running application in minutes!
    - complexity can scale with requirements


## Roadmap:
- v0.5.0:
    - build ui with custom script
        - recursive reusable components
        - access application data with macros simply by matching the variable name in your script
        - hot reloading of UI
        - bundle UI script in application binary or specify user directory for text files
    - build application code with Safe and Performant Rust
        - cross platform:
            - Windows
            - Arch Linux
        - multi window support
        - able to compile and run with < 100 lines of Rust
        - startup immediately from template project
    - 3D rendering capabilities built in
        - convenient api to load/manipulate GLTF models
- v0.6.0: In progress
  - allow user to load and render custom shaders for scene and ui
  - allow multiple 3d scenes
  - window interaction improvements (dbl/tpl clicks, drag and drop)
  - cross platform testing (webassembly?)
  - performance improvements
- v0.7.0 ~ v0.9.0: feature set not decided
- v1.0
    - rich set of highly customizable UI widgets
    - reactive animations for any/all UI configuration settings
    - fully cross-platform (no native mobile)
    - batch rendering of UI for performance
    - expose api for user created UI/3D Shaders
    - expose api for user Graphics middleware


## Docs

- [`docs/telera_api.md`](docs/telera_api.md) — the Rust API: `App`, `run`, the
  macros, and every `API` method, with a trivial app to start from.
- [`docs/tml-spec.md`](docs/tml-spec.md) — the TML markdown layout language.


## Profiling

`[profile.release]` in `Cargo.toml` sets `debug = "line-tables-only"`, so
optimized builds carry enough debug info for a profiler to name frames across
the whole graph (telera-layout / clay, wgpu, lyon, glyphon). No runtime cost,
~10-15% larger binary. The first build after a fresh checkout recompiles all
deps once.

### samply (recommended)

Statistical sampler that opens the recording in the Firefox Profiler web UI
(flamegraph + inverted call tree + per-thread timeline). Pure Rust, no `perf`
package, works without root.

One-time host setup:

```
cargo install samply
# ~/.cargo/bin must be on PATH (the script also falls back to it directly)
# samply needs perf_event_paranoid <= 1 for a non-root user:
echo 'kernel.perf_event_paranoid = 1' | sudo tee /etc/sysctl.d/10-perf.conf && sudo sysctl --system
```

Then:

```
scripts/profile stress        # build + record examples/stress, quit the window to view
```

`scripts/profile [example] [args…]` wraps `cargo build --release --example`
+ `samply record`; the example name defaults to `stress`, extra args pass to the
example. To record without a browser:
`samply record --save-only -o profile.json.gz ./target/release/examples/stress`,
then `samply load profile.json.gz`.

### cargo-flamegraph (single SVG)

When you want one self-contained file to attach to an issue. Needs `perf`.

```
sudo pacman -S perf           # or the distro equivalent
cargo install flamegraph      # one-time
cargo flamegraph --release --example stress   # writes flamegraph.svg; Ctrl-C the app to finish
```
