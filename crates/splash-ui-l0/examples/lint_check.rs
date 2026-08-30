//! Report theme-portability lints for a card.
//!
//! Usage: cargo run -p splash-ui-l0 --example lint_check -- <card>...
fn main() {
    for p in std::env::args().skip(1) {
        let Ok(src) = std::fs::read_to_string(&p) else {
            continue;
        };
        let r = splash_ui_l0::check_ui_l0_named("card.l0", &src);
        let name = p.rsplit('/').next().unwrap_or(&p);
        if r.lints.is_empty() {
            println!("{name}: portable");
            continue;
        }
        println!("{name}: {} lint(s), valid={}", r.lints.len(), r.valid);
        for l in &r.lints {
            println!("  {}:{}  {}", l.line, l.column, l.message);
        }
    }
}
