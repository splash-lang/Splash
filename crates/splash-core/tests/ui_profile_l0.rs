//! The L0 profile checked against the reference cards.
//!
//! These three cards are the evidence for the profile: any construct they need
//! that the grammar lacks is a defect in `docs/ui-profile-l0.md`, not in the
//! cards. They are the same files as `octos-one/docs/l0/*.card`.

use splash_core::ui_l0::{catalog, check_ui_l0_named, Level};

const WEATHER: &str = include_str!("fixtures/weather.card");
const NEWS: &str = include_str!("fixtures/news.card");
const STOCK: &str = include_str!("fixtures/stock.card");

fn accepts(name: &str, source: &str) {
    let report = check_ui_l0_named(name, source);
    assert!(
        report.valid,
        "{name} should be L0, but: {:#?}",
        report.diagnostics
    );
    assert_eq!(report.level, Level::L0);
}

fn rejects_at(name: &str, source: &str, level: Level) -> String {
    let report = check_ui_l0_named(name, source);
    assert!(!report.valid, "{name} should have been rejected");
    assert_eq!(
        report.level, level,
        "wrong level for {name}: {:#?}",
        report.diagnostics
    );
    report
        .diagnostics
        .first()
        .map(|d| d.message.clone())
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────── the reference cards ──

#[test]
fn the_three_reference_cards_are_l0() {
    accepts("weather", WEATHER);
    accepts("news", NEWS);
    accepts("stock", STOCK);
}

/// Every constructor the cards use must be catalogued, or the checker is
/// accepting names it cannot check the arguments of.
#[test]
fn every_constructor_the_cards_use_is_catalogued_or_declared() {
    for (name, source) in [("weather", WEATHER), ("news", NEWS), ("stock", STOCK)] {
        let declared: Vec<&str> = source
            .lines()
            .filter_map(|l| l.trim().strip_prefix("component "))
            .filter_map(|l| l.split(['(', ' ']).next())
            .collect();

        for line in source.lines() {
            let line = line.split('#').next().unwrap_or("");
            for word in line.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                let is_ctor = word.chars().next().is_some_and(|c| c.is_uppercase())
                    && line.contains(&format!("{word}("));
                if is_ctor && catalog::lookup(word).is_none() && !declared.contains(&word) {
                    panic!("{name}: {word:?} is neither catalogued nor declared");
                }
            }
        }
    }
}

// ──────────────────────────────────────────────────────────── level classification ──

#[test]
fn a_function_is_l2() {
    let msg = rejects_at("fn", "fn tick() { }\nview root Panel", Level::L2);
    assert!(msg.contains("unbounded work"), "{msg}");
}

#[test]
fn an_imperative_widget_command_is_l2() {
    let msg = rejects_at(
        "ui",
        "view root Panel { Row(on_tap: go) }\nevent go { ui.map.set_text(1) }",
        Level::L2,
    );
    assert!(msg.contains("imperative"), "{msg}");
}

#[test]
fn module_access_is_l2() {
    let msg = rejects_at("mod", "source x mod.fs.read(\"/etc/passwd\")", Level::L2);
    assert!(msg.contains("host surface is empty"), "{msg}");
}

#[test]
fn arithmetic_is_l1() {
    let msg = rejects_at(
        "arith",
        "view hero TextHero(value: now.temp * 2)",
        Level::L1,
    );
    assert!(msg.contains("fabricated fact"), "{msg}");
}

/// The correction that mattered: `expanded: !expanded` is computation, however
/// small, and L0 has `toggle` precisely so it need not be written.
#[test]
fn negation_in_a_transition_is_l1() {
    let msg = rejects_at(
        "neg",
        "state open { shape: bool, initial: false }\nevent t { open: !open }",
        Level::L1,
    );
    assert!(msg.contains("toggle"), "{msg}");
}

#[test]
fn a_ternary_is_l1() {
    rejects_at(
        "ternary",
        "state u { shape: enum[c, f], initial: .c }\nevent t { u: u == .c ? .f : .c }",
        Level::L1,
    );
}

// ────────────────────────────────────────────────────────────────── grammar rules ──

/// Identity may never come from position — profile §6.3.
#[test]
fn a_loop_without_a_key_is_rejected() {
    let report = check_ui_l0_named(
        "nokey",
        "source week sys.weather()\nview f Panel { for d in week { Row() } }",
    );
    assert!(!report.valid);
    assert!(
        report.diagnostics.iter().any(|d| d.message.contains("key")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_unknown_constructor_is_rejected() {
    let report = check_ui_l0_named("unknown", "view root Blockchain(hype: 11)");
    assert!(!report.valid);
    assert!(
        report.diagnostics[0]
            .message
            .contains("not an L0 constructor"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_unknown_argument_is_rejected() {
    let report = check_ui_l0_named("badarg", "view root Panel { Rule(sparkle: 3) }");
    assert!(!report.valid);
    assert!(
        report.diagnostics[0].message.contains("no argument"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_illegal_token_is_rejected_with_the_legal_set() {
    let report = check_ui_l0_named("badtoken", "view root Surface(pad: .enormous)");
    assert!(!report.valid);
    let m = &report.diagnostics[0].message;
    assert!(
        m.contains(".page"),
        "the diagnostic should name the legal tokens: {m}"
    );
}

/// The distinction lexical marking exists to make: a bare name is a path, and a
/// path cannot stand in for a token.
#[test]
fn a_bare_name_where_a_token_belongs_is_rejected() {
    let report = check_ui_l0_named("bare", "view root Surface(pad: page)");
    assert!(!report.valid);
    let m = &report.diagnostics[0].message;
    assert!(m.contains("leading dot"), "{m}");
}

#[test]
fn an_undeclared_name_is_rejected() {
    let report = check_ui_l0_named("undeclared", "view root TextTitle(text: nowhere.name)");
    assert!(!report.valid);
    assert!(
        report.diagnostics[0]
            .message
            .contains("not a declared name"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_undeclared_event_is_rejected() {
    let report = check_ui_l0_named("noevent", "view root Card(on_tap: nope)");
    assert!(!report.valid);
    assert!(
        report.diagnostics[0]
            .message
            .contains("not a declared event"),
        "{:#?}",
        report.diagnostics
    );
}

// ──────────────────────────────────────────────────────────── component contracts ──

/// Profile §5.3: card state must arrive as a prop, or the dependency is
/// invisible to the tracker.
#[test]
fn a_component_may_not_read_card_state() {
    let report = check_ui_l0_named(
        "scope",
        "state units { shape: enum[c, f], initial: .c }\n\
         component Row2(item: record) { view TextValue(value: item.hi, unit: units) }\n\
         view root Panel { }",
    );
    assert!(!report.valid);
    let m = &report.diagnostics[0].message;
    assert!(m.contains("pass it as a prop"), "{m}");
}

/// A component reading a card-scope source is fine — sources are immutable and
/// already declared dependencies.
#[test]
fn a_component_may_read_a_card_source() {
    let report = check_ui_l0_named(
        "srcread",
        "source week sys.weather()\n\
         component Bar(item: record) { view TempBar(lo: item.lo, hi: item.hi, min: week.min, max: week.max) }\n\
         view root Panel { }",
    );
    assert!(report.valid, "{:#?}", report.diagnostics);
}

/// Profile §6, condition 1. Without it an L0 realization need not terminate.
#[test]
fn a_recursive_component_is_rejected() {
    let report = check_ui_l0_named(
        "rec",
        "component Node(item: record) { view Col { Node(item: item) } }\nview root Panel { }",
    );
    assert!(!report.valid);
    assert!(
        report.diagnostics[0].message.contains("recursive"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn mutual_recursion_is_rejected_too() {
    let report = check_ui_l0_named(
        "mutual",
        "component A(x: record) { view Col { B(x: x) } }\n\
         component B(x: record) { view Col { A(x: x) } }\n\
         view root Panel { }",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("recursive")),
        "{:#?}",
        report.diagnostics
    );
}

// ────────────────────────────────────────────────────────────────────── bounds ──

#[test]
fn diagnostics_are_bounded() {
    let mut source = String::from("view root Panel {\n");
    for _ in 0..500 {
        source.push_str("  Blockchain()\n");
    }
    source.push('}');
    let report = check_ui_l0_named("many", &source);
    assert!(!report.valid);
    assert!(report.diagnostics.len() <= splash_core::ui_l0::MAX_UI_L0_DIAGNOSTICS);
    assert!(report.diagnostics_truncated);
}

#[test]
fn deeply_nested_source_terminates() {
    let depth = 4_000;
    let mut source = String::from("view root ");
    for _ in 0..depth {
        source.push_str("Panel { ");
    }
    for _ in 0..depth {
        source.push('}');
    }
    // The contract is that it returns, bounded, rather than overflowing.
    let report = check_ui_l0_named("deep", &source);
    assert!(!report.valid);
}

#[test]
fn a_truncated_card_terminates() {
    for source in [
        "view root Panel {",
        "component A(",
        "state x {",
        "view root Row(align:",
        "for d in",
    ] {
        let _ = check_ui_l0_named("partial", source);
    }
}

// ─────────────────────────────────────────────────────────── the negative fixture ──

/// The shipping nav card. It is the reason the level model exists: 127
/// conditionals that L0 could express, and a `fn tick()` plus 65 imperative
/// widget commands that it cannot. Level is the MAXIMUM any construct requires,
/// so this must classify L2 rather than stopping at the first arithmetic it hits.
const NAV: &str = include_str!("fixtures/trip_planner.splash");

#[test]
fn the_nav_card_is_l2() {
    let report = check_ui_l0_named("trip-planner", NAV);
    assert!(!report.valid);
    assert_eq!(
        report.level,
        Level::L2,
        "expected L2, got {:?}: {:#?}",
        report.level,
        report.diagnostics.first()
    );
}

// ─────────────────────────────────────────────────────────────────── transitions ──

#[test]
fn a_transition_must_name_a_declared_state() {
    let report = check_ui_l0_named("nostate", "event go { nowhere: clear }\nview root Panel");
    assert!(!report.valid);
    assert!(
        report.diagnostics[0]
            .message
            .contains("not a state declared"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn toggle_requires_a_bool() {
    let report = check_ui_l0_named(
        "toggle",
        "state u { shape: enum[c, f], initial: .c }\nevent t { u: toggle }\nview root Panel",
    );
    assert!(!report.valid);
    assert!(
        report.diagnostics[0].message.contains("needs a bool"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn cycle_members_must_be_declared_in_the_shape() {
    let report = check_ui_l0_named(
        "cycle",
        "state u { shape: enum[c, f], initial: .c }\nevent t { u: cycle(.c, .kelvin) }\nview root Panel",
    );
    assert!(!report.valid);
    assert!(
        report.diagnostics[0].message.contains("not a member"),
        "{:#?}",
        report.diagnostics
    );
}

/// Enum members are tokens. A bare name is a path and cannot stand in for one.
#[test]
fn cycle_rejects_bare_names() {
    let report = check_ui_l0_named(
        "cyclebare",
        "state u { shape: enum[c, f], initial: .c }\nevent t { u: cycle(c, f) }\nview root Panel",
    );
    assert!(!report.valid);
    assert!(
        report.diagnostics[0].message.contains("leading dot"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_untotal_transition_form_is_rejected() {
    let report = check_ui_l0_named(
        "untotal",
        "state n { shape: number, initial: 0 }\nevent t { n: increment }\nview root Panel",
    );
    assert!(!report.valid);
    assert!(
        report.diagnostics[0]
            .message
            .contains("total transition form"),
        "{:#?}",
        report.diagnostics
    );
}
