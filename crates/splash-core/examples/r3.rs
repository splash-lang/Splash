use splash_core::ui_l0::*;
fn dump(n: &UiNode, d: usize) {
    println!("{}{} {:?}", "  ".repeat(d), n.kind, n.args);
    for c in &n.children {
        dump(c, d + 1);
    }
}
fn show(label: &str, src: &str) {
    let r = check_ui_l0_named("p", src);
    println!("  accepted={:<5} {label}", r.valid);
    for d in r.diagnostics.iter().take(1) {
        println!("        {}", d.message);
    }
}
fn main() {
    println!("#3 — does a literal reach a numeric position via a prop?");
    show("Bar(n: 41.2) -> TempBar(lo: n)",
      "component Bar(n: number) { view TempBar(lo: n, hi: n, min: n, max: n) }\nview root Bar(n: 41.2)");

    println!("\n#1 — copy with NO class at all:");
    show(
        "copy claim { en: \"Revenue: $41.2M\" }",
        "copy claim { en: \"Revenue: $41.2M\" }\nview root TextTitle(text: copy.claim)",
    );

    println!("\n#2 — model-copy laundered through a state initial:");
    show("state s { initial: copy.claim }",
      "copy claim { class: model-copy, en: \"Revenue: $41.2M\" }\ncomponent F() { state s { shape: text, initial: copy.claim } view TextTitle(text: s) }\nview root F()");

    println!("\n#6 — does an enum member overwrite the shape?");
    show("shape: enum[a, text] then set(\"arbitrary\")",
      "state mode { shape: enum[a, text], initial: .a }\nevent e { mode: set(\"arbitrary\") }\nview root Rule()");

    println!("\n#7 — misspelled shape bypasses set checking:");
    show(
        "shape: nubmer, set(\"oops\")",
        "state n { shape: nubmer, initial: 0 }\nevent e { n: set(\"oops\") }\nview root Rule()",
    );

    println!("\n#8 — token set() into a non-enum:");
    show(
        "shape: number, set(.c)",
        "state n { shape: number, initial: 0 }\nevent e { n: set(.c) }\nview root Rule()",
    );

    println!("\n#9 — dotted source authorizes its whole root:");
    show(
        "source env.locale, read env.theme.secret",
        "source env.locale sys.locale()\nview root TextRow(text: env.theme.secret)",
    );

    println!("\n#4 — enum token prop keeps its dot?");
    let src = "component F(unit: enum[c, f]) { view Col { when unit == .c { TextRow(text: \"MATCHED\") } } }\nview root F(unit: .c)";
    let out = realize(src, &serde_json::json!({}), RealizeLimits::default());
    print!("  ");
    dump(out.root.as_ref().unwrap(), 0);
}
