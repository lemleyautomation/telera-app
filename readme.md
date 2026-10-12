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
- [`docs/tml-spec.md`](docs/tml-spec.md) — the TML markdown layout language (`.tmd` files).


## Web (WebAssembly)

Apps compile to `wasm32-unknown-unknown` and run in a browser on **WebGPU**
(current Chrome/Edge; Firefox and Safari where WebGPU is enabled). One-time setup:

```
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <wasm-bindgen version in Cargo.lock> --locked
```

plus a `clang` that can target wasm32 (it compiles clay's C source). Then:

```
scripts/build-web layout              # or basic, effects; add --release for a small, fast build
python3 -m http.server -d target/web/layout 8000   # open http://localhost:8000
```

`--serve [port]` does both steps. `--all` builds every example into
`target/web/` with an index page linking them, which is a quick way to check
everything still compiles for wasm32:

```
scripts/build-web layout --serve       # build + serve on :8000
scripts/build-web --all --serve 8080   # all examples, index at http://localhost:8080
```

Panics, GPU errors, and a missing WebGPU are shown in a bar at the bottom of
the page as well as in the console. Only examples that embed their assets
(currently `basic`, `layout`, `effects`) load layouts in a browser. The others
build but log a load error.

What differs in a browser:

- **No filesystem.** Ship layouts, and the images, fonts and shaders their
  directives name, inside the binary with `RunType::Embedded`. Each bundle is an
  `include_dir!` directory standing in for a path, so directive paths resolve
  unchanged (see `layouts()` in `examples/layout.rs`). `RunType::Watch` and
  `RunType::Once` can't read their directory in a browser, so they load nothing
  (and log why). There is no hot reload. See `docs/telera_api.md` §6 for a full
  walkthrough.
- **No system fonts.** A `font` directive (or `API::load_font`) works as usual.
  If no *text* font is registered by the end of `onload` (an emoji-only font
  doesn't count), the default Noto Sans is downloaded from jsDelivr. Change or
  disable this with `API::set_web_fallback_font_url`.
- **Time:** use `telera_app::web_time::Instant`, because `std::time::Instant::now()`
  panics in a browser.
- The agent access port (sockets, threads) is native-only and ignored on the web.
  Every window becomes a `<canvas>` appended to the page.
- Errors, including GPU validation and WGSL the browser rejects, go to the
  browser console. Browsers enforce WGSL's uniformity rules, which native naga
  doesn't, so call `textureSample` outside branches that depend on per-fragment
  values. Use `textureSampleLevel` when it has to be inside one.


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
