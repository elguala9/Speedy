pub mod utils;
pub mod models;

/// Top-level entry point that delegates to the utils module.
pub fn top_level_fn() -> String {
    utils::helper()
}
