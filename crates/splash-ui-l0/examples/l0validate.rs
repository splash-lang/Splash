//! TEMP soak-test validator (delete after use). `l0validate <card.splash>` →
//! JSON {ok, root, diagnostics[]} from the real realize() gate, so a correction
//! loop can feed the diagnostic messages back to the model.
use splash_ui_l0::{realize, RealizeLimits};

fn main() {
    let path = std::env::args().nth(1).expect("usage: l0validate <path>");
    let src = std::fs::read_to_string(&path).unwrap_or_default();
    let report = realize(&src, &serde_json::json!({}), RealizeLimits::default());
    let msgs: Vec<String> = report.diagnostics.iter().map(|d| d.message.clone()).collect();
    let ok = report.diagnostics.is_empty() && report.root.is_some();
    println!(
        "{}",
        serde_json::json!({"ok": ok, "root": report.root.is_some(), "diagnostics": msgs})
    );
}
