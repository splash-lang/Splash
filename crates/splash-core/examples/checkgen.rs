use splash_core::ui_l0::*;
fn main() {
    let src = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let r = check_ui_l0_named("generated", &src);
    println!("valid={}  level={:?}  diagnostics={}", r.valid, r.level, r.diagnostics.len());
    for d in r.diagnostics.iter().take(10) { println!("  {}:{}  {}", d.line, d.column, d.message); }
}
