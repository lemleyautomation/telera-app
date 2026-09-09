/// Compile-time arithmetic (`` `calc` `` functions and expression-valued
/// `` `set-numeric` `` declarations) used by [`layout_runner`].
mod calc;
/// Pan/zoom state for `` `canvas` `` elements.
pub mod canvas;
pub mod layout_runner;
// The renderer itself lives in `ui_renderer.rs`; the enclosing module keeps the
// same name so the crate path reads `ui_renderer::UIRenderer`.
#[allow(clippy::module_inception)]
pub mod ui_renderer;

pub use telera_layout;
