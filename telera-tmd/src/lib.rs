//! Shared grammar for the Telera markdown layout language (`.tmd`).
//!
//! This crate holds the pieces of the TML grammar that must stay identical
//! between the runtime ([`telera-app`](https://docs.rs/telera-app)), the derive
//! macros (`telera_macros`), and editor tooling (`telera-lsp`):
//!
//! - [`calc`] — the compile-time arithmetic sub-language used by
//!   `` `set-numeric` `` values and `` `calc` `` header directives.
//! - [`prepass`] — the keyword backtick pre-pass + [`prepass::LEADING_KEYWORDS`],
//!   the canonical lexical keyword set.
//! - [`classify`] — Rust field-type → TML-value-kind classification (what
//!   `#[derive(LayoutRunnerReflection)]` / `#[derive(FieldAccess)]` expose).
//! - [`schema`] — a declarative table describing every keyword: where it is
//!   valid, its argument shape, a one-line doc, and whether it is a
//!   known-incomplete path. Single source of truth for completion / hover /
//!   diagnostics.
//! - [`tree`] *(feature `mdast`)* — a position-aware structural classifier over
//!   `markdown`'s AST, for the language server. It does **not** reproduce the
//!   runtime command stream — it records spans and grammar roles, malformed
//!   nodes included.
//!
//! Nothing here touches wgpu / winit / the renderer; the runtime `Layout`
//! command stream and its resolution stay in `telera-app`.

pub mod calc;
pub mod classify;
pub mod prepass;
pub mod schema;

#[cfg(feature = "mdast")]
pub mod tree;
