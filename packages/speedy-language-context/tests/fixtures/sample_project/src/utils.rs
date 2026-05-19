/// Public helper callable from other modules.
pub fn helper() -> String {
    "hello".to_string()
}

/// Crate-internal helper — not part of the public API.
pub(crate) fn internal_fn() -> bool {
    true
}

/// Fully private — should be hidden in Minimal skeleton output.
fn private_fn() {
    let _ = internal_fn();
}
