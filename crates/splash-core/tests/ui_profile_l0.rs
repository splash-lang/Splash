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

// ────────────────────────────────────────────────────────────────── realization ──

use splash_core::ui_l0::{realize, NodeValue, RealizeLimits};

fn weather_data() -> serde_json::Value {
    serde_json::json!({
        "place": { "name": "Kyoto" },
        "now":   { "temp": 18.0, "feels": 16.0, "hi": 21.0, "lo": 12.0, "cond": "cloudy",
                   "humidity": 61.0, "wind": 9.0, "pressure": 1013.0, "uv": 3.0,
                   "visibility": 10.0 },
        // `week` is a record: the days plus the bounds every bar scales against.
        // An array could not carry both, which is what the first data snapshot
        // got wrong and silently zeroed every TempBar.
        "week":  { "days": [ { "dayname": "Mon", "hi": 21.0, "lo": 12.0, "cond": 2.0 },
                             { "dayname": "Tue", "hi": 23.0, "lo": 13.0, "cond": 0.0 },
                             { "dayname": "Wed", "hi": 19.0, "lo": 11.0, "cond": 3.0 } ],
                   "min_lo": 11.0, "max_hi": 23.0 },
        "sun":   { "rise": 5.1, "set": 18.9, "now": 12.0 },
        "moon":  { "phase": 0.5, "illumination": 0.5 },
        "aqi":   { "grid": [1, 2], "index": 42.0, "band": "good" },
        "scene": "https://example.invalid/kyoto.jpg",
        "units": "c",
        "days":  7.0,
        "city":  "Kyoto",
        "env":   { "locale": { "temp_unit": "c" } },
        "copy":  { "feels": "Feels like", "humidity": "Humidity", "wind": "Wind",
                   "pressure": "Pressure", "uv": "UV Index", "visibility": "Visibility",
                   "air": "Air Quality", "sunrise": "Sunrise", "sunset": "Sunset" }
    })
}

fn find<'a>(
    node: &'a splash_core::ui_l0::UiNode,
    kind: &str,
    out: &mut Vec<&'a splash_core::ui_l0::UiNode>,
) {
    if node.kind == kind {
        out.push(node);
    }
    for c in &node.children {
        find(c, kind, out);
    }
}

#[test]
fn a_card_realizes_to_a_node_tree() {
    let report = realize(WEATHER, &weather_data(), RealizeLimits::default());
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let root = report.root.expect("a root node");
    assert_eq!(root.kind, "Photo");
    assert!(
        report.nodes > 20,
        "expected a populated tree, got {}",
        report.nodes
    );
}

/// Bindings resolve against injected data — the card names a question, the
/// runtime supplied the answer.
#[test]
fn bindings_resolve_from_injected_data() {
    let report = realize(WEATHER, &weather_data(), RealizeLimits::default());
    let root = report.root.unwrap();
    let mut titles = Vec::new();
    find(&root, "TextTitle", &mut titles);
    let (_, place) = titles[0].args.iter().find(|(n, _)| n == "text").unwrap();
    assert_eq!(*place, NodeValue::Text("Kyoto".into()));
}

/// One row per item, keyed by the declared key expression rather than position.
#[test]
fn a_loop_produces_one_instance_per_item_keyed_by_its_key() {
    let report = realize(WEATHER, &weather_data(), RealizeLimits::default());
    let root = report.root.unwrap();
    let mut bars = Vec::new();
    find(&root, "TempBar", &mut bars);
    assert_eq!(bars.len(), 3, "three days of data, three rows");
    for (bar, day) in bars.iter().zip(["Mon", "Tue", "Wed"]) {
        assert!(
            bar.key.contains(day),
            "key {:?} should carry {day}",
            bar.key
        );
    }
}

/// Identity must survive an edit that inserts a sibling above — that is why keys
/// come from the declaration path and loop key, not position.
#[test]
fn keys_are_stable_when_a_sibling_is_inserted_above() {
    let before = realize(WEATHER, &weather_data(), RealizeLimits::default())
        .root
        .unwrap();
    let edited = WEATHER.replace(
        "view current  Col(align: .center) {",
        "view current  Col(align: .center) {\n                Rule()",
    );
    let after = realize(&edited, &weather_data(), RealizeLimits::default())
        .root
        .unwrap();

    let (mut a, mut b) = (Vec::new(), Vec::new());
    find(&before, "TempBar", &mut a);
    find(&after, "TempBar", &mut b);
    let keys_a: Vec<&str> = a.iter().map(|n| n.key.as_str()).collect();
    let keys_b: Vec<&str> = b.iter().map(|n| n.key.as_str()).collect();
    assert_eq!(
        keys_a, keys_b,
        "inserting a node above must not rebind the forecast rows"
    );
}

/// A bare boolean guard — amendment #10 — and it must actually gate.
#[test]
fn a_guard_includes_children_only_when_it_holds() {
    let mut data = weather_data();
    let report = realize(WEATHER, &data, RealizeLimits::default());
    let root = report.root.unwrap();
    let mut captions = Vec::new();
    find(&root, "TextCaption", &mut captions);
    let collapsed = captions.len();

    // `expanded` is component-local state the runtime holds; with none supplied
    // the guard is false and the detail row is absent.
    data["expanded"] = serde_json::Value::Bool(true);
    let expanded = realize(WEATHER, &data, RealizeLimits::default())
        .root
        .unwrap();
    let mut captions2 = Vec::new();
    find(&expanded, "TextCaption", &mut captions2);
    assert!(
        captions2.len() >= collapsed,
        "an open guard must not remove nodes"
    );
}

/// A missing binding is surfaced, not defaulted. A silently empty field is how a
/// card renders an em dash and still looks correct.
#[test]
fn an_unresolved_binding_is_reported_as_missing() {
    let mut data = weather_data();
    data["now"]["humidity"] = serde_json::Value::Null;
    let report = realize(WEATHER, &data, RealizeLimits::default());
    let root = report.root.unwrap();
    let mut tiles = Vec::new();
    find(&root, "Tile", &mut tiles);
    assert!(
        tiles.iter().any(|t| t
            .args
            .iter()
            .any(|(n, v)| n == "value" && *v == NodeValue::Missing)),
        "a null source field should realize as Missing"
    );
}

#[test]
fn events_carry_their_declared_name_rather_than_a_constructed_string() {
    let report = realize(WEATHER, &weather_data(), RealizeLimits::default());
    let root = report.root.unwrap();
    let mut heroes = Vec::new();
    find(&root, "TextHero", &mut heroes);
    let (_, ev) = heroes[0].args.iter().find(|(n, _)| n == "on_tap").unwrap();
    assert_eq!(*ev, NodeValue::Event("toggle_units".into()));
}

#[test]
fn realization_is_bounded() {
    let limits = RealizeLimits {
        max_nodes: 10,
        ..RealizeLimits::default()
    };
    let report = realize(WEATHER, &weather_data(), limits);
    assert!(report.truncated);
    assert!(report.nodes <= 10);
}

#[test]
fn a_long_collection_is_truncated_rather_than_unbounded() {
    let mut data = weather_data();
    let week: Vec<serde_json::Value> = (0..5_000)
        .map(|i| serde_json::json!({ "dayname": format!("d{i}"), "hi": 1.0, "lo": 0.0, "cond": "x" }))
        .collect();
    data["week"] = serde_json::json!({ "days": week, "min_lo": 0.0, "max_hi": 1.0 });
    let limits = RealizeLimits {
        max_collection: 8,
        ..RealizeLimits::default()
    };
    let report = realize(WEATHER, &data, limits);
    let root = report.root.unwrap();
    let mut bars = Vec::new();
    find(&root, "TempBar", &mut bars);
    assert_eq!(bars.len(), 8);
    assert!(report.truncated);
}

#[test]
fn the_other_two_cards_realize_too() {
    let news = serde_json::json!({
        "lead": [{ "id": "1", "title": "T", "author": "A", "points": 9.0, "comments": 3.0, "url": "u" }],
        "feed": [{ "id": "2", "title": "U", "author": "B", "points": 4.0 }],
        "article": { "title": "T", "author": "A", "points": 9.0, "comments": 3.0, "url": "u" },
        "selected": "",
        "env": { "locale": {} },
        "copy": { "masthead": "Top Stories", "latest": "LATEST", "lead": "LEAD",
                  "pts": "pts", "comments": "comments", "by": "by" }
    });
    let report = realize(NEWS, &news, RealizeLimits::default());
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    assert!(report.root.is_some());

    let stock = serde_json::json!({
        "movers": [{ "ticker": "AAA", "name": "A", "last": 1.0, "change": 0.5, "pct": 2.0 }],
        "quote": { "name": "A", "last": 1.0, "change": 0.5, "pct": 2.0, "open": 1.0,
                   "high": 2.0, "low": 0.5, "volume": 10.0, "mktcap": 99.0, "pe": 12.0 },
        "series": { "points": [1, 2], "min": 0.5, "max": 2.0 },
        "selected": "", "range": "m1", "env": { "locale": {} },
        "copy": { "movers": "Top Movers", "open": "Open", "high": "High", "low": "Low",
                  "volume": "Volume", "mktcap": "Mkt Cap", "pe": "P/E" }
    });
    let report = realize(STOCK, &stock, RealizeLimits::default());
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    assert!(report.root.is_some());
}

// ───────────────────────────────────────────────────────────── makepad lowering ──

use splash_core::ui_l0::makepad;

#[test]
fn a_realized_card_lowers_to_renderable_dsl() {
    let report = realize(WEATHER, &weather_data(), RealizeLimits::default());
    let dsl = makepad::lower(&report.root.unwrap());

    // Exactly one top-level node. Siblings at top level lay out SIDE BY SIDE and
    // squeeze the card into half the width — a real generated-card bug.
    let tops = dsl
        .lines()
        .filter(|l| !l.starts_with("//") && !l.starts_with(' ') && !l.trim().is_empty())
        .filter(|l| l.contains('{'))
        .count();
    assert_eq!(tops, 1, "expected one root node:\n{dsl}");

    assert!(
        dsl.contains("http_resource"),
        "the photo should be a resource"
    );
    assert!(
        dsl.contains("TempBar{"),
        "forecast bars should survive lowering"
    );
    assert!(
        dsl.contains("MoonPhase{"),
        "the moon phase should survive lowering"
    );
    assert!(
        dsl.contains("\"Kyoto\""),
        "the bound city should be concrete"
    );
    assert!(
        !dsl.contains("sys."),
        "realization already resolved sources: {dsl}"
    );
}

/// Nothing may be silently dropped: an unmapped constructor renders a visible
/// warning instead of vanishing.
#[test]
fn an_unmapped_constructor_is_visible_rather_than_dropped() {
    let node = splash_core::ui_l0::UiNode {
        kind: "Hologram".into(),
        key: "root".into(),
        args: vec![],
        children: vec![],
    };
    let dsl = makepad::lower(&node);
    assert!(dsl.contains("no makepad lowering for Hologram"), "{dsl}");
}

/// A missing binding reaches the screen as an em dash rather than a blank.
#[test]
fn a_missing_binding_lowers_to_a_visible_dash() {
    let mut data = weather_data();
    data["now"]["humidity"] = serde_json::Value::Null;
    let report = realize(WEATHER, &data, RealizeLimits::default());
    let dsl = makepad::lower(&report.root.unwrap());
    assert!(dsl.contains('—'), "an unresolved field should be visible");
}

// ────────────────────────────────────────────────────────────── instance state ──

const TWO_ROWS: &str = r#"
source items sys.movers()
component Rowy(item: record) {
  state open { shape: bool, initial: false }
  event flip { open: toggle }
  view Col {
    Row(on_tap: flip) { TextRow(text: item.name) }
    when open { TextCaption(text: item.name) }
  }
}
view root Panel { for it in items key it.id { Rowy(item: it) } }
"#;

fn two_rows_data() -> serde_json::Value {
    serde_json::json!({ "items": [ {"id": "a", "name": "A"}, {"id": "b", "name": "B"} ] })
}

/// Each instance must own its `open` cell. Opening one row may not open the
/// other — the property the whole component model exists to provide.
#[test]
fn local_state_is_per_instance() {
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let data = two_rows_data();

    let first =
        splash_core::ui_l0::realize_with_state(TWO_ROWS, &data, &store, RealizeLimits::default());
    let root = first.root.expect("root");
    let mut rows = Vec::new();
    find(&root, "Row", &mut rows);
    assert_eq!(rows.len(), 2);

    // Open only the first row.
    let key = rows[0].key.clone();
    let applied = splash_core::ui_l0::dispatch(TWO_ROWS, &mut store, &key, "flip");
    assert!(applied, "flip should have applied at {key}");

    let second =
        splash_core::ui_l0::realize_with_state(TWO_ROWS, &data, &store, RealizeLimits::default());
    let root = second.root.expect("root");
    let mut captions = Vec::new();
    find(&root, "TextCaption", &mut captions);
    assert_eq!(
        captions.len(),
        1,
        "exactly one row should be open, not {}",
        captions.len()
    );
    let (_, shown) = captions[0].args.iter().find(|(n, _)| n == "text").unwrap();
    assert_eq!(
        *shown,
        NodeValue::Text("A".into()),
        "the row that was tapped"
    );
}

/// Profile §5.8: a state schema change resets instances rather than guessing
/// that old values still fit.
#[test]
fn a_schema_change_resets_instance_state() {
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let data = two_rows_data();
    let first =
        splash_core::ui_l0::realize_with_state(TWO_ROWS, &data, &store, RealizeLimits::default());
    let mut rows = Vec::new();
    let first_root = first.root.unwrap();
    find(&first_root, "Row", &mut rows);
    splash_core::ui_l0::dispatch(TWO_ROWS, &mut store, &rows[0].key, "flip");

    // The component gains a field: same name, different state schema.
    let edited = TWO_ROWS.replace(
        "state open { shape: bool, initial: false }",
        "state open { shape: bool, initial: false }\n  state seen { shape: bool, initial: false }",
    );
    let after =
        splash_core::ui_l0::realize_with_state(&edited, &data, &store, RealizeLimits::default());
    let mut captions = Vec::new();
    let after_root = after.root.unwrap();
    find(&after_root, "TextCaption", &mut captions);
    assert_eq!(
        captions.len(),
        0,
        "a schema change resets, it does not migrate"
    );
}

// ─────────────────────────────────────────────────────── declared-but-missing ──

const SETTER: &str = r#"
source items sys.movers()
state picked { shape: text, initial: "" }
event choose { picked: set($value) }
event home   { picked: clear }
view root Panel {
  when picked == "" { for it in items key it.id { Row(on_tap: choose, value: it.id) { TextRow(text: it.name) } } }
  when picked != "" { Col { TextTitle(text: picked) Row(on_tap: home) { TextCaption(glyph: "‹") } } }
}
"#;

/// `set($value)` carries the payload from the element that raised the event.
/// Without it a list cannot select anything — the interaction stock and news
/// both depend on.
#[test]
fn set_applies_the_event_payload() {
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let data = serde_json::json!({ "items": [{"id":"a","name":"A"},{"id":"b","name":"B"}] });

    let before =
        splash_core::ui_l0::realize_with_state(SETTER, &data, &store, RealizeLimits::default());
    let root = before.root.unwrap();
    let mut rows = Vec::new();
    find(&root, "Row", &mut rows);
    assert_eq!(rows.len(), 2, "the list should show both items");

    let payload = serde_json::json!("b");
    let applied =
        splash_core::ui_l0::dispatch_with(SETTER, &mut store, "root", "choose", Some(&payload));
    assert!(applied, "choose should apply");

    let after =
        splash_core::ui_l0::realize_with_state(SETTER, &data, &store, RealizeLimits::default());
    let root = after.root.unwrap();
    let mut titles = Vec::new();
    find(&root, "TextTitle", &mut titles);
    assert_eq!(
        titles.len(),
        1,
        "selecting should switch to the detail view"
    );
    let (_, shown) = titles[0].args.iter().find(|(n, _)| n == "text").unwrap();
    assert_eq!(
        *shown,
        NodeValue::Text("b".into()),
        "the item that was tapped"
    );
}

const SLOTTED: &str = r#"
component Framed(title: text) {
  view Col { TextCaption(text: title) slot }
}
view root Panel { Framed(title: "Details") { TextRow(text: "inner") Rule() } }
"#;

/// A component's children are realized where `slot` appears. Dropping them
/// silently is how a card renders a frame with nothing in it and still looks
/// deliberate.
#[test]
fn slot_realizes_the_children_passed_at_the_call_site() {
    let report = realize(SLOTTED, &serde_json::json!({}), RealizeLimits::default());
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let root = report.root.unwrap();
    let mut rows = Vec::new();
    find(&root, "TextRow", &mut rows);
    assert_eq!(
        rows.len(),
        1,
        "the child passed at the call site must appear"
    );
    let mut rules = Vec::new();
    find(&root, "Rule", &mut rules);
    assert_eq!(rules.len(), 1);
}

/// Profile §5.7: an instance that disappears loses its state. Without pruning
/// the store grows for the life of the app and a key that reappears inherits a
/// stale value.
#[test]
fn unmounted_instances_are_pruned() {
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let two = serde_json::json!({ "items": [{"id":"a","name":"A"},{"id":"b","name":"B"}] });

    let report =
        splash_core::ui_l0::realize_with_state(TWO_ROWS, &two, &store, RealizeLimits::default());
    let root = report.root.unwrap();
    let mut rows = Vec::new();
    find(&root, "Row", &mut rows);
    for row in &rows {
        splash_core::ui_l0::dispatch(TWO_ROWS, &mut store, &row.key, "flip");
    }
    assert_eq!(store.len(), 2, "both rows hold state");

    // `b` goes away.
    let one = serde_json::json!({ "items": [{"id":"a","name":"A"}] });
    let report =
        splash_core::ui_l0::realize_with_state(TWO_ROWS, &one, &store, RealizeLimits::default());
    store.prune(&report.live_keys);
    assert_eq!(store.len(), 1, "the departed instance's cell is dropped");
}

/// Formatting is the runtime's job, not the card's. A card that wrote "$" or
/// "41.2M" itself would be authoring a fact — and currency, grouping and
/// compaction are locale-dependent besides.
#[test]
fn declared_formats_are_applied_by_the_runtime() {
    let data = serde_json::json!({
        "movers": [], "selected": "NVDA", "range": "m1", "env": {"locale":{}},
        "quote": {"name":"NVIDIA","last":184.2,"change":3.1,"pct":1.7,"open":181.0,
                  "high":185.6,"low":180.2,"volume":41200000.0,
                  "mktcap":4520000000000.0,"pe":58.3},
        "series": {"points":[1,2],"min":180.2,"max":185.6},
        "copy": {"movers":"Top Movers","open":"Open","high":"High","low":"Low",
                 "volume":"Volume","mktcap":"Mkt Cap","pe":"P/E"}
    });
    let report = realize(STOCK, &data, RealizeLimits::default());
    let dsl = makepad::lower(&report.root.expect("root"));

    for (expect, why) in [
        ("$184.20", "money gains a currency symbol and two places"),
        ("+1.7%", "a signed percentage keeps its sign"),
        ("41.2M", "compact scales volume"),
        ("4.5T", "compact scales market cap"),
        ("58.3", "a ratio is plain"),
    ] {
        assert!(dsl.contains(expect), "{why}: {expect} missing from\n{dsl}");
    }
    assert!(
        !dsl.contains("41200000"),
        "the raw number should not survive"
    );
}

/// A float hour is a time, not a decimal: 18.9 is 18:54.
#[test]
fn a_time_format_renders_as_a_clock() {
    let report = realize(WEATHER, &weather_data(), RealizeLimits::default());
    let dsl = makepad::lower(&report.root.unwrap());
    assert!(
        dsl.contains("5:06"),
        "sunrise 5.1 should render 5:06:\n{dsl}"
    );
}

// ──────────────────────────────────────────────────────────── catalog agreement ──

/// The module doc claims a test asserts the Rust catalog and the TOML spec
/// agree. It said so for two commits before this test existed — a documented
/// check that was never written is worse than no check, because a reader stops
/// looking.
#[test]
fn the_rust_catalog_matches_the_toml_spec() {
    const TOML: &str = include_str!("../../../docs/ui-l0-constructors.toml");

    // Constructor headers are `[Name]`; `[kinds.x]` are shared token sets.
    let documented: Vec<&str> = TOML
        .lines()
        .filter_map(|l| l.trim().strip_prefix('['))
        .filter_map(|l| l.strip_suffix(']'))
        .filter(|n| !n.contains('.'))
        .collect();

    let implemented: Vec<&str> = catalog::CONSTRUCTORS.iter().map(|(n, _)| *n).collect();

    for name in &documented {
        assert!(
            implemented.contains(name),
            "{name:?} is in the TOML spec but not in the Rust catalog"
        );
    }
    for name in &implemented {
        assert!(
            documented.contains(name),
            "{name:?} is in the Rust catalog but not in the TOML spec"
        );
    }

    // Token sets must agree too, or a card legal by the spec is rejected by the
    // implementation.
    for (set_name, tokens) in [
        ("unit", catalog::UNIT),
        ("format", catalog::FORMAT),
        ("width", catalog::WIDTH),
    ] {
        let header = format!("[kinds.{set_name}]");
        let block = TOML
            .split(&header)
            .nth(1)
            .unwrap_or_else(|| panic!("{header} missing from the TOML"));
        let line = block
            .lines()
            .find(|l| l.trim_start().starts_with("tokens"))
            .unwrap_or_else(|| panic!("{set_name} has no tokens line"));
        for token in tokens {
            assert!(
                line.contains(&format!("\"{token}\"")),
                "{set_name}: {token:?} is in the Rust catalog but not the TOML"
            );
        }
    }
}

// ───────────────────────────────────────────────────────────────── copy locale ──

/// `copy` is authored in the ledger, so it resolves from the CARD rather than
/// the injected data. Carrying it in both duplicated what the card already said,
/// and the two drifted — a card gained a `copy` the data blob lacked, and the
/// eyebrow rendered as an em dash on device.
#[test]
fn copy_resolves_from_the_card_not_the_data() {
    let data = serde_json::json!({
        "lead": [{"id":"1","title":"T","author":"A","points":1.0,"comments":2.0,"url":"u"}],
        "feed": [], "article": {}, "selected": "",
        "env": {"locale": {"lang": "en"}}
        // note: no "copy" key at all
    });
    let report = realize(NEWS, &data, RealizeLimits::default());
    let dsl = makepad::lower(&report.root.expect("root"));
    assert!(
        dsl.contains("Top Stories"),
        "card-declared copy should resolve:\n{dsl}"
    );
    assert!(
        dsl.contains("HACKER NEWS"),
        "the eyebrow should resolve too"
    );
}

/// The declared language is chosen when the runtime reports one.
#[test]
fn copy_selects_the_reported_locale() {
    let mut data = serde_json::json!({
        "lead": [{"id":"1","title":"T","author":"A","points":1.0,"comments":2.0,"url":"u"}],
        "feed": [], "article": {}, "selected": "",
        "env": {"locale": {"lang": "zh"}}
    });
    let dsl = makepad::lower(&realize(NEWS, &data, RealizeLimits::default()).root.unwrap());
    assert!(dsl.contains("头条"), "zh text should be selected:\n{dsl}");

    // A card shipping zh text must not render an em dash on an en device.
    data["env"]["locale"]["lang"] = serde_json::Value::String("fr".into());
    let dsl = makepad::lower(&realize(NEWS, &data, RealizeLimits::default()).root.unwrap());
    assert!(
        dsl.contains("Top Stories"),
        "an unknown locale falls back to en"
    );
}

// ───────────────────────────────────────────────────────────────── source plan ──

use splash_core::ui_l0::{source_plan, SourceArg};

/// A card declares what data it needs; the plan is what a host fetches. Without
/// it the declaration was inert — the parser skipped the helper and every
/// argument, so nothing could act on `source now sys.weather(…)`.
#[test]
fn the_plan_reports_helpers_and_arguments() {
    let plan = source_plan(WEATHER);
    assert!(plan.diagnostics.is_empty(), "{:#?}", plan.diagnostics);

    let now = plan.requests.iter().find(|r| r.name == "now").expect("now");
    assert_eq!(now.helper, "sys.weather");

    let (_, lat) = now.args.iter().find(|(n, _)| n == "lat").expect("lat");
    assert_eq!(*lat, SourceArg::Path("place.lat".into()));

    let (_, fields) = now
        .args
        .iter()
        .find(|(n, _)| n == "fields")
        .expect("fields");
    match fields {
        SourceArg::List(items) => {
            assert!(items.contains(&"temp".to_string()), "{items:?}");
            assert!(items.contains(&"humidity".to_string()), "{items:?}");
        }
        other => panic!("fields should be a list, got {other:?}"),
    }
}

/// `now` reads `place.lat`, so `place` must be fetched first. A host walking the
/// list in order always has what the next request refers to.
#[test]
fn the_plan_orders_dependencies_before_dependents() {
    let plan = source_plan(WEATHER);
    let at = |name: &str| plan.requests.iter().position(|r| r.name == name).unwrap();
    assert!(at("place") < at("now"), "place must precede now");
    assert!(at("place") < at("week"), "place must precede week");
    assert!(at("place") < at("aqi"), "place must precede aqi");

    let now = &plan.requests[at("now")];
    assert_eq!(now.depends_on, vec!["place".to_string()]);

    // A source reading only card state depends on no other fetch.
    let place = &plan.requests[at("place")];
    assert!(place.depends_on.is_empty(), "{:?}", place.depends_on);
}

/// Two sources that read each other cannot both go first. Picking one silently
/// would fetch against a value that does not exist yet.
#[test]
fn a_dependency_cycle_is_reported_rather_than_resolved() {
    let plan = source_plan(
        "source a sys.geocode(name: b.value)\nsource b sys.photo(query: a.value)\nview root Panel",
    );
    assert!(
        plan.diagnostics.iter().any(|d| d.message.contains("cycle")),
        "{:#?}",
        plan.diagnostics
    );
}

#[test]
fn every_reference_card_yields_a_resolvable_plan() {
    for (name, card) in [("weather", WEATHER), ("news", NEWS), ("stock", STOCK)] {
        let plan = source_plan(card);
        assert!(
            plan.diagnostics.is_empty(),
            "{name}: {:#?}",
            plan.diagnostics
        );
        assert!(!plan.requests.is_empty(), "{name} declares no sources");
        for request in &plan.requests {
            assert!(
                request.helper.starts_with("sys."),
                "{name}: {:?} has no helper",
                request.name
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────── reconciliation ──

use splash_core::ui_l0::{dirty_records, record_dependencies};

/// L0 needs no dynamic read-tracking: no expression form means every binding is
/// structurally visible, so the dependency set is computable without evaluating
/// anything.
#[test]
fn dependencies_are_computed_without_evaluation() {
    let deps = record_dependencies(WEATHER);
    let of = |name: &str| {
        deps.iter()
            .find(|d| d.record == name)
            .unwrap_or_else(|| panic!("no deps for {name}"))
            .reads
            .clone()
    };

    assert!(
        of("current").contains(&"now".to_string()),
        "{:?}",
        of("current")
    );
    assert!(of("current").contains(&"units".to_string()));
    assert!(of("details").contains(&"now".to_string()));
    // The forecast reads the week and, through ForecastRow, the bounds.
    assert!(
        of("forecast").contains(&"week".to_string()),
        "{:?}",
        of("forecast")
    );
    // `sunmoon` reads sun and moon, and must NOT read the stock of the card.
    let sunmoon = of("sunmoon");
    assert!(sunmoon.contains(&"sun".to_string()) && sunmoon.contains(&"moon".to_string()));
    assert!(!sunmoon.contains(&"week".to_string()), "{sunmoon:?}");
}

/// A loop binder is local. `for d in week` must not make `d` a dependency, or
/// every record with a loop would look like it reads everything.
#[test]
fn a_loop_binder_is_not_a_dependency() {
    let deps = record_dependencies(WEATHER);
    let forecast = &deps.iter().find(|d| d.record == "forecast").unwrap().reads;
    assert!(!forecast.contains(&"d".to_string()), "{forecast:?}");
}

/// The measurable claim behind "one event, one state change, one reconciliation
/// pass" — toggling a unit touches the records that read it, not the tree.
#[test]
fn a_state_write_dirties_only_its_dependents() {
    let all: Vec<String> = record_dependencies(WEATHER)
        .into_iter()
        .map(|d| d.record)
        .collect();
    let dirty = dirty_records(WEATHER, &["units"]);

    assert!(!dirty.is_empty(), "toggling units must dirty something");
    assert!(
        dirty.len() < all.len(),
        "it must not dirty everything: {dirty:?}"
    );

    // The sun/moon panel does not read units and must survive untouched — it is
    // the panel whose rebuild would be most visible.
    assert!(!dirty.contains(&"sunmoon".to_string()), "{dirty:?}");
    assert!(!dirty.contains(&"airfield".to_string()), "{dirty:?}");
    // The hero and the detail tiles do read it.
    assert!(dirty.contains(&"current".to_string()), "{dirty:?}");
    assert!(dirty.contains(&"details".to_string()), "{dirty:?}");
}

/// A component's own state dirties that component, so an expanded row
/// re-realizes and its neighbours do not.
#[test]
fn component_local_state_dirties_its_component() {
    let dirty = dirty_records(WEATHER, &["expanded"]);
    assert!(
        dirty.contains(&"ForecastRow".to_string()),
        "local state must dirty its component: {dirty:?}"
    );
    assert!(!dirty.contains(&"details".to_string()), "{dirty:?}");
}

/// Over-approximation is the safe direction: re-realizing too much is slow,
/// re-realizing too little shows stale data.
#[test]
fn an_unrelated_write_dirties_nothing() {
    assert!(dirty_records(WEATHER, &["nothing_reads_this"]).is_empty());
}

use splash_core::ui_l0::patch_points;

/// `dirty_records` is transitive, so `root` appears for almost any write — it
/// splices everything, so it reads everything. Patching root rebuilds the tree,
/// which is what reconciliation exists to avoid.
#[test]
fn patch_points_exclude_ancestors_that_merely_contain_the_change() {
    let dirty = dirty_records(WEATHER, &["units"]);
    let minimal = patch_points(WEATHER, &["units"]);
    assert!(
        dirty.contains(&"root".to_string()),
        "root is transitively dirty"
    );
    assert!(
        !minimal.contains(&"root".to_string()),
        "but patching a descendant suffices: {minimal:?}"
    );
    assert!(minimal.len() < dirty.len());
}

/// The claim, at its sharpest: toggling one row's own state re-realizes that
/// component and nothing else.
#[test]
fn local_state_patches_exactly_one_record() {
    let minimal = patch_points(WEATHER, &["expanded"]);
    assert_eq!(minimal, vec!["ForecastRow".to_string()], "{minimal:?}");
}

/// A write that a SOURCE reads invalidates the fetch, and everything binding its
/// result. Changing the city re-geocodes, so every source downstream of `place`
/// is stale — a genuine full rebuild, and the graph should say so rather than
/// reporting nothing.
#[test]
fn a_write_a_source_reads_invalidates_its_dependents() {
    let minimal = patch_points(WEATHER, &["city"]);
    for record in ["current", "forecast", "airfield", "sunmoon", "details"] {
        assert!(
            minimal.contains(&record.to_string()),
            "changing the city must invalidate {record}: {minimal:?}"
        );
    }
}

/// Under-approximating shows stale data; over-approximating is merely slow.
#[test]
fn an_unrelated_write_patches_nothing() {
    assert!(patch_points(WEATHER, &["nothing_reads_this"]).is_empty());
}

// ─── named slots (§5.5, open question 5) ─────────────────────────────────────

/// Every `text` argument in the tree, in render order.
fn texts(root: &splash_core::ui_l0::UiNode) -> Vec<String> {
    fn walk(n: &splash_core::ui_l0::UiNode, out: &mut Vec<String>) {
        for (name, v) in &n.args {
            if name == "text" {
                if let NodeValue::Text(t) = v {
                    out.push(t.clone());
                }
            }
        }
        for c in &n.children {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

const TWO_SLOTS: &str = r#"
component Framed(title: text) {
  view Col {
         TextCaption(text: title)
         slot header
         Rule()
         slot
       }
}
view root Panel {
  Framed(title: "Today") {
    into header { TextRow(text: "in the header") }
    TextRow(text: "in the body")
  }
}
"#;

#[test]
fn a_named_slot_card_is_l0() {
    accepts("two-slots", TWO_SLOTS);
}

#[test]
fn named_slot_routes_children_to_the_matching_slot() {
    let report = realize(TWO_SLOTS, &serde_json::json!({}), RealizeLimits::default());
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);

    // Order is what proves routing: the `into header` child is written first at
    // the call site but the component places that slot above the anonymous one,
    // so the two must come out in component order, not call-site order.
    assert_eq!(
        texts(&report.root.unwrap()),
        vec!["Today", "in the header", "in the body"],
        "each group should land at its own slot"
    );
}

#[test]
fn a_slot_with_no_matching_group_renders_nothing() {
    // Not an error: a component may offer a slot that a call site declines to
    // fill. Children with nowhere to go are dropped rather than misplaced.
    let source = r#"
component Framed() { view Panel { slot aside } }
view root Panel { Framed() { TextRow(text: "loose") } }
"#;
    let report = realize(source, &serde_json::json!({}), RealizeLimits::default());
    assert!(
        texts(&report.root.unwrap()).is_empty(),
        "the anonymous group has no matching slot in this component"
    );
}

#[test]
fn an_anonymous_slot_still_needs_no_ceremony() {
    // The common case must stay as it was — no `into` required, and every
    // existing card keeps working.
    let source = r#"
component Framed() { view Panel { slot } }
view root Panel { Framed() { TextRow(text: "plain") } }
"#;
    let report = realize(source, &serde_json::json!({}), RealizeLimits::default());
    assert_eq!(texts(&report.root.unwrap()), vec!["plain"]);
}

// ─── source lifecycle (§5.9, open question 3) ────────────────────────────────

const LIFECYCLE: &str = r#"
source now sys.weather(lat: 35.0, lon: 135.0)
view root Col {
  when now.$state == .pending { TextRow(text: "Loading") }
  when now.$state == .failed  { TextRow(text: "Unavailable") }
  when now.$state == .ready   { TextRow(text: "Live") }
}
"#;

#[test]
fn a_source_lifecycle_card_is_l0() {
    accepts("lifecycle", LIFECYCLE);
}

#[test]
fn source_state_defaults_to_pending_when_no_value_arrived() {
    let report = realize(LIFECYCLE, &serde_json::json!({}), RealizeLimits::default());
    assert_eq!(
        texts(&report.root.unwrap()),
        vec!["Loading"],
        "a source with no value and no report is pending, so a card can say so \
         rather than rendering an em dash that looks like real data"
    );
}

#[test]
fn source_state_is_ready_once_a_value_arrives() {
    let data = serde_json::json!({"now": {"temp": 21.0}});
    let report = realize(LIFECYCLE, &data, RealizeLimits::default());
    assert_eq!(texts(&report.root.unwrap()), vec!["Live"]);
}

#[test]
fn the_host_can_report_failure_explicitly() {
    // The distinction that matters: a fetch that FAILED is not the same as one
    // that has not happened yet, and only the host knows which.
    let data = serde_json::json!({"$status": {"now": "failed"}});
    let report = realize(LIFECYCLE, &data, RealizeLimits::default());
    assert_eq!(
        texts(&report.root.unwrap()),
        vec!["Unavailable"],
        "failure must be distinguishable from pending"
    );
}

#[test]
fn a_stale_source_is_neither_pending_nor_failed() {
    // Stale means old, not wrong: the card keeps its last good value rather
    // than blanking a number that is merely a few minutes behind.
    let data = serde_json::json!({"now": {"temp": 21.0}, "$status": {"now": "stale"}});
    let report = realize(LIFECYCLE, &data, RealizeLimits::default());
    assert!(
        texts(&report.root.unwrap()).is_empty(),
        "this card guards three states; stale is a fourth and matches none"
    );
}

#[test]
fn state_is_rejected_on_something_that_is_not_a_source() {
    let report = check_ui_l0_named(
        "not-a-source",
        "state units { shape: enum[c, f], initial: .c }\n\
         view root Col { when units.$state == .pending { Rule() } }",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("only available on a source")),
        "local state has no fetch lifecycle: {:#?}",
        report.diagnostics
    );
}

/// A literal prop must survive the call. This failed for the life of the
/// component feature and no test caught it: the reference cards pass paths
/// (`story: s`, `position: i`), so the literal arm was never exercised and
/// `Framed(title: "Details")` rendered an em dash. Same shape as every other
/// "specified but not retained" bug — the parser kept the value and the
/// realizer dropped it on a catch-all arm.
#[test]
fn a_literal_prop_reaches_the_component() {
    let source = r#"
component Framed(title: text, rank: number) {
  view Col { TextCaption(text: title) TextRow(text: rank) }
}
view root Panel { Framed(title: "Details", rank: 3) }
"#;
    let report = realize(source, &serde_json::json!({}), RealizeLimits::default());
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let root = report.root.unwrap();

    let mut caps = Vec::new();
    find(&root, "TextCaption", &mut caps);
    assert_eq!(
        caps[0].args.iter().find(|(n, _)| n == "text").unwrap().1,
        NodeValue::Text("Details".into()),
        "a string literal prop must arrive, not become Missing"
    );

    let mut rows = Vec::new();
    find(&root, "TextRow", &mut rows);
    assert_eq!(
        rows[0].args.iter().find(|(n, _)| n == "text").unwrap().1,
        NodeValue::Number(3.0),
        "a number literal prop must arrive too"
    );
}

#[test]
fn an_into_naming_an_absent_slot_is_rejected() {
    // Children with nowhere to go vanish. Silent is the wrong failure mode:
    // this is the same class as a binding that resolves to nothing.
    let report = check_ui_l0_named(
        "bad-slot",
        "component Framed() { view Panel { slot body } }\n\
         view root Panel { Framed() { into headr { Rule() } } }",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("has no slot named") && d.message.contains("body")),
        "the message should name the slots that do exist: {:#?}",
        report.diagnostics
    );
}

// ─── §6 condition 4: event cascades ──────────────────────────────────────────

/// Profile §6.4 prohibits event cascades — a transition may not trigger another
/// event. There is no check for this because there is nothing to check: the four
/// total forms (`set`, `toggle`, `cycle`, `clear`) all name a STATE, and a
/// transition target is validated against declared state. A cascade is therefore
/// unrepresentable rather than merely rejected.
///
/// This test pins that. It enumerates every syntax that could plausibly express
/// "and then fire another event" and asserts each is rejected. If someone later
/// adds a form that can raise one, termination stops being structural and this
/// test is where that shows up.
#[test]
fn an_event_cannot_trigger_another_event() {
    const PRELUDE: &str = "state n { shape: number, initial: 0 }\n\
                           state flag { shape: bool, initial: false }\n\
                           event bump { n: set(1) }\n";

    for (shape, body) in [
        (
            "a transition targeting an event name",
            "event c { bump: toggle }",
        ),
        ("a transition setting an event", "event c { bump: set(1) }"),
        (
            "an event named as a payload target",
            "event c { n: set(bump) }",
        ),
        ("an emit form", "event c { emit bump }"),
        ("a call form", "event c { bump() }"),
        ("a then form", "event c { n: set(1) then bump }"),
    ] {
        let source = format!("{PRELUDE}{body}\nview root Panel {{ Rule() }}");
        let report = check_ui_l0_named("cascade", &source);
        assert!(
            !report.valid,
            "{shape} would be a cascade and must not be accepted: {body:?}"
        );
    }

    // The control: the same prelude with a legal transition IS accepted, so the
    // rejections above are about cascades and not about the prelude.
    let ok = check_ui_l0_named(
        "no-cascade",
        &format!("{PRELUDE}event c {{ flag: toggle }}\nview root Panel {{ Rule() }}"),
    );
    assert!(ok.valid, "{:#?}", ok.diagnostics);
}

// ─── §5.8 opt-in migration (open question 6) ─────────────────────────────────

/// `keep: true` opts one cell out of the §5.8 reset. Reset stays the default;
/// this makes preservation expressible rather than impossible.
#[test]
fn a_keep_cell_survives_a_schema_change() {
    const KEPT: &str = r#"
source items sys.movers()
component Rowy(item: record) {
  state open { shape: bool, initial: false, keep: true }
  event flip { open: toggle }
  view Col {
    Row(on_tap: flip) { TextRow(text: item.name) }
    when open { TextCaption(text: item.name) }
  }
}
view root Panel { for it in items key it.id { Rowy(item: it) } }
"#;
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let data = two_rows_data();
    let first =
        splash_core::ui_l0::realize_with_state(KEPT, &data, &store, RealizeLimits::default());
    let mut rows = Vec::new();
    let first_root = first.root.unwrap();
    find(&first_root, "Row", &mut rows);
    splash_core::ui_l0::dispatch(KEPT, &mut store, &rows[0].key, "flip");

    // An unrelated field is added: the schema id changes, but `open` itself is
    // untouched and asked to survive.
    let edited = KEPT.replace(
        "event flip",
        "state seen { shape: bool, initial: false }\n  event flip",
    );
    let after =
        splash_core::ui_l0::realize_with_state(&edited, &data, &store, RealizeLimits::default());
    let mut captions = Vec::new();
    let after_root = after.root.unwrap();
    find(&after_root, "TextCaption", &mut captions);
    assert_eq!(
        captions.len(),
        1,
        "a cell that opted in and did not change shape should survive"
    );
}

/// The limit of the opt-in: `keep` is a claim about a field, not a promise that
/// any value fits. Retyping the field resets it anyway — otherwise `keep` would
/// be exactly the "guess it still fits" that §5.8 exists to prevent.
#[test]
fn keep_does_not_survive_a_change_to_that_field_itself() {
    const KEPT: &str = r#"
source items sys.movers()
component Rowy(item: record) {
  state open { shape: bool, initial: false, keep: true }
  event flip { open: toggle }
  view Col {
    Row(on_tap: flip) { TextRow(text: item.name) }
    when open { TextCaption(text: item.name) }
  }
}
view root Panel { for it in items key it.id { Rowy(item: it) } }
"#;
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let data = two_rows_data();
    let first =
        splash_core::ui_l0::realize_with_state(KEPT, &data, &store, RealizeLimits::default());
    let mut rows = Vec::new();
    let first_root = first.root.unwrap();
    find(&first_root, "Row", &mut rows);
    splash_core::ui_l0::dispatch(KEPT, &mut store, &rows[0].key, "flip");

    // `open` is retyped bool → enum. The opt-in does not apply to a field whose
    // own shape moved.
    let edited = KEPT
        .replace(
            "state open { shape: bool, initial: false, keep: true }",
            "state open { shape: enum[shut, ajar], initial: .shut, keep: true }",
        )
        .replace(
            "event flip { open: toggle }",
            "event flip { open: cycle(.shut, .ajar) }",
        );
    let after =
        splash_core::ui_l0::realize_with_state(&edited, &data, &store, RealizeLimits::default());
    let mut captions = Vec::new();
    let after_root = after.root.unwrap();
    find(&after_root, "TextCaption", &mut captions);
    assert_eq!(
        captions.len(),
        0,
        "a retyped field resets even with keep: true"
    );
}

/// Reset remains the default: without `keep`, nothing changes.
#[test]
fn without_keep_a_schema_change_still_resets() {
    // This is `a_schema_change_resets_instance_state` restated as the control
    // for the two tests above — the opt-in must not have become the default.
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let data = two_rows_data();
    let first =
        splash_core::ui_l0::realize_with_state(TWO_ROWS, &data, &store, RealizeLimits::default());
    let mut rows = Vec::new();
    let first_root = first.root.unwrap();
    find(&first_root, "Row", &mut rows);
    splash_core::ui_l0::dispatch(TWO_ROWS, &mut store, &rows[0].key, "flip");

    let edited = TWO_ROWS.replace(
        "event flip",
        "state seen { shape: bool, initial: false }\n  event flip",
    );
    let after =
        splash_core::ui_l0::realize_with_state(&edited, &data, &store, RealizeLimits::default());
    let mut captions = Vec::new();
    let after_root = after.root.unwrap();
    find(&after_root, "TextCaption", &mut captions);
    assert_eq!(captions.len(), 0, "reset is still the default");
}

// ─── §7 component version pinning ────────────────────────────────────────────

/// §7 requires component versions to be pinned, "or an L0 card can be moved to
/// L2 by a definition it references being replaced". The report carries a digest
/// per component so a host can store it beside the approved level and notice.
#[test]
fn the_report_carries_a_digest_per_component() {
    let report = check_ui_l0_named("weather", WEATHER);
    assert!(report.valid);
    assert!(
        !report.closure.is_empty(),
        "the weather card declares a component, so the closure cannot be empty"
    );
    let names: Vec<&str> = report.closure.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"ForecastRow"), "got {names:?}");

    // Sorted, so a host comparing two closures does not have to.
    let mut sorted = report.closure.clone();
    sorted.sort();
    assert_eq!(sorted, report.closure);
}

#[test]
fn replacing_a_component_body_changes_its_digest() {
    let before = check_ui_l0_named("weather", WEATHER).closure;

    // The change that motivates §7: a component gains a construct outside L0.
    // The card text is otherwise identical, so a host that pinned only the
    // card's level would carry the old L0 verdict onto a definition that no
    // longer earns it.
    let edited = WEATHER.replace(
        "component ForecastRow(",
        "component ForecastRow(extra: number, ",
    );
    assert_ne!(edited, WEATHER, "the replacement must actually apply");
    let after = check_ui_l0_named("weather", &edited).closure;

    let digest = |c: &[(String, u64)]| {
        c.iter()
            .find(|(n, _)| n == "ForecastRow")
            .map(|(_, d)| *d)
            .unwrap()
    };
    assert_ne!(
        digest(&before),
        digest(&after),
        "a changed definition must produce a changed digest"
    );
}

#[test]
fn reordering_declarations_is_not_a_version_change() {
    // A digest that moved on a cosmetic edit would make pinning useless: every
    // reformat would look like a replacement and hosts would learn to ignore it.
    let source = "\
component A(x: number) { view Panel { TextRow(text: x) } }\n\
component B(y: number) { view Panel { TextRow(text: y) } }\n\
view root Panel { A(x: 1) B(y: 2) }\n";
    let swapped = "\
component B(y: number) { view Panel { TextRow(text: y) } }\n\
component A(x: number) { view Panel { TextRow(text: x) } }\n\
view root Panel { A(x: 1) B(y: 2) }\n";

    let a = check_ui_l0_named("a", source);
    let b = check_ui_l0_named("b", swapped);
    assert!(
        a.valid && b.valid,
        "{:#?} {:#?}",
        a.diagnostics,
        b.diagnostics
    );
    assert_eq!(a.closure, b.closure);
}

// ─── §3 cycle ordering (open question 2) ─────────────────────────────────────

/// `cycle` advances in the ENUM's declaration order. That is the guarantee
/// question 2 asked for, and it is what makes the transition total and O(1) —
/// there is no comparison to run, only a successor.
#[test]
fn cycle_advances_in_declaration_order() {
    const TRI: &str = "\
state mode { shape: enum[low, mid, high], initial: .low }\n\
event step { mode: cycle(.low, .mid, .high) }\n\
view root Panel { Row(on_tap: step) { Rule() } }\n";

    let mut store = splash_core::ui_l0::InstanceStore::default();
    let report = realize(TRI, &serde_json::json!({}), RealizeLimits::default());
    let mut rows = Vec::new();
    let root = report.root.unwrap();
    find(&root, "Row", &mut rows);
    let key = rows[0].key.clone();

    for expected in ["mid", "high", "low"] {
        splash_core::ui_l0::dispatch(TRI, &mut store, &key, "step");
        assert_eq!(
            store.get("@card", "mode").and_then(|v| v.as_str()),
            Some(expected),
            "cycle should wrap through declaration order"
        );
    }
}

/// The corollary: a `cycle` that lists a different order is rejected rather
/// than quietly running the declared one. Accepting it would mean the card says
/// one order and the runtime performs another.
#[test]
fn a_cycle_listing_a_different_order_is_rejected() {
    let report = check_ui_l0_named(
        "reordered",
        "state mode { shape: enum[low, mid, high], initial: .low }\n\
         event step { mode: cycle(.high, .mid, .low) }\n\
         view root Panel { Rule() }\n",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("declaration order")),
        "{:#?}",
        report.diagnostics
    );
}

// ─── §5.1 identity is a path, not a position ─────────────────────────────────

const TOGGLY: &str = r#"
component Toggly() {
  state open { shape: bool, initial: false }
  event flip { open: toggle }
  view Col {
    Row(on_tap: flip) { TextRow(text: "hdr") }
    when open { TextCaption(text: "OPEN") }
  }
}
view root Panel { Toggly() }
"#;

/// §5.1: "L0 binds to a declared path so that identity survives a ledger edit
/// that inserts an element above." This is the profile's stated difference from
/// React, and it went untested — the implementation keyed children by their raw
/// index, so it was in fact using React's rule while the spec said it was not.
///
/// The scenario is not hypothetical for a ledger: appending a record that adds
/// a rule above a list is an ordinary edit, and it should not collapse every
/// expanded row on screen.
#[test]
fn state_survives_an_element_inserted_above() {
    let data = serde_json::json!({});
    let mut store = splash_core::ui_l0::InstanceStore::default();

    let first =
        splash_core::ui_l0::realize_with_state(TOGGLY, &data, &store, RealizeLimits::default());
    let root = first.root.unwrap();
    let mut rows = Vec::new();
    find(&root, "Row", &mut rows);
    let key = rows[0].key.clone();
    splash_core::ui_l0::dispatch(TOGGLY, &mut store, &key, "flip");

    // The same card with a Rule inserted ABOVE the component.
    let edited = TOGGLY.replace(
        "view root Panel { Toggly() }",
        "view root Panel { Rule() Toggly() }",
    );
    assert_ne!(edited, TOGGLY, "the edit must actually apply");

    let after =
        splash_core::ui_l0::realize_with_state(&edited, &data, &store, RealizeLimits::default());
    let after_root = after.root.unwrap();
    let mut rows_after = Vec::new();
    find(&after_root, "Row", &mut rows_after);
    assert_eq!(
        rows_after[0].key, key,
        "the instance key must not depend on how many siblings precede it"
    );

    let mut caps = Vec::new();
    find(&after_root, "TextCaption", &mut caps);
    assert_eq!(
        caps.len(),
        1,
        "the toggled state must survive an insertion above (profile §5.1)"
    );
}

/// The other half: sibling instances of the same component stay distinct, so
/// making keys insertion-stable must not make two of them collide.
#[test]
fn sibling_instances_of_one_component_keep_separate_state() {
    let two = TOGGLY.replace(
        "view root Panel { Toggly() }",
        "view root Panel { Toggly() Toggly() }",
    );
    let data = serde_json::json!({});
    let mut store = splash_core::ui_l0::InstanceStore::default();

    let first =
        splash_core::ui_l0::realize_with_state(&two, &data, &store, RealizeLimits::default());
    let root = first.root.unwrap();
    let mut rows = Vec::new();
    find(&root, "Row", &mut rows);
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].key, rows[1].key, "two instances need two keys");

    // Flip only the first.
    splash_core::ui_l0::dispatch(&two, &mut store, &rows[0].key.clone(), "flip");
    let after =
        splash_core::ui_l0::realize_with_state(&two, &data, &store, RealizeLimits::default());
    let after_root = after.root.unwrap();
    let mut caps = Vec::new();
    find(&after_root, "TextCaption", &mut caps);
    assert_eq!(caps.len(), 1, "exactly one instance should be expanded");
}

// ─── §4 source capabilities are a closed set ─────────────────────────────────

/// The finding that motivated this: `source_plan` handed an arbitrary dotted
/// name to the host as the function to resolve, so a card could NAME a
/// capability it was never granted and produce a clean, well-formed request.
/// Confinement then rested on the host keeping its own allowlist — the policy
/// boundary §1 claims L0 removes.
#[test]
fn an_uncatalogued_source_helper_is_rejected() {
    let report = check_ui_l0_named(
        "leak",
        "source leak sys.shell(command: \"cat /etc/passwd\")\nview root Rule()",
    );
    assert!(!report.valid, "a card must not be able to name sys.shell");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not a source capability")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_uncatalogued_helper_produces_no_source_request() {
    // Belt and braces: even if a caller skips checking, the plan must not carry
    // the request onward to a host that trusts it.
    let plan = splash_core::ui_l0::source_plan(
        "source leak sys.shell(command: \"cat /etc/passwd\")\nview root Rule()",
    );
    assert!(
        !plan.requests.iter().any(|r| r.helper == "sys.shell"),
        "an uncatalogued helper must not reach the host: {:?}",
        plan.requests.iter().map(|r| &r.helper).collect::<Vec<_>>()
    );
    assert!(!plan.diagnostics.is_empty(), "and it must say why");
}

#[test]
fn a_catalogued_helper_rejects_an_argument_it_does_not_accept() {
    // A helper that silently ignores an unknown argument is how a card asks for
    // something it did not get and renders as though it did.
    let report = check_ui_l0_named(
        "badarg",
        "source w sys.weather(lat: 1.0, lon: 2.0, shell: \"rm -rf /\")\nview root Rule()",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("does not accept an argument named")),
        "{:#?}",
        report.diagnostics
    );
}

/// The catalog in the TOML and the one in Rust must list the same capabilities
/// with the same arguments — in both directions, and this time actually.
#[test]
fn the_source_catalog_matches_the_toml_spec() {
    const TOML: &str = include_str!("../../../docs/ui-l0-constructors.toml");

    let mut documented: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<String> = None;
    for line in TOML.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("[sources.\"") {
            current = rest.strip_suffix("\"]").map(|s| s.to_string());
            if let Some(name) = &current {
                documented.push((name.clone(), Vec::new()));
            }
        } else if line.starts_with('[') {
            current = None;
        } else if current.is_some() {
            if let Some(rest) = line.strip_prefix("args = [") {
                let args: Vec<String> = rest
                    .trim_end_matches(']')
                    .split(',')
                    .map(|a| a.trim().trim_matches('"').to_string())
                    .filter(|a| !a.is_empty())
                    .collect();
                documented.last_mut().unwrap().1 = args;
            }
        }
    }

    assert!(!documented.is_empty(), "the TOML must declare [sources.*]");

    for (name, args) in &documented {
        let implemented = catalog::source(name)
            .unwrap_or_else(|| panic!("{name:?} is in the TOML but not in the Rust catalog"));
        assert_eq!(
            implemented,
            args.as_slice(),
            "arguments for {name:?} disagree between the TOML and Rust"
        );
    }
    for (name, args) in catalog::SOURCES {
        let (_, doc_args) = documented
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name:?} is in the Rust catalog but not in the TOML"));
        assert_eq!(
            *args,
            doc_args.as_slice(),
            "arguments for {name:?} disagree between Rust and the TOML"
        );
    }
}

// ─── parse-then-discard: forms recognised but whose values were dropped ──────
//
// Every one of these parsed cleanly and then lost the value. That is the single
// most common defect in this work, and each of these was found by the codex
// review rather than by a failing test — which is the point of writing them.

/// `active: range == .d1` realized as the VALUE of `range` rather than a
/// boolean, because the parser kept the left path and dropped the comparator
/// and right operand. Live in stock.card, and the device golden recorded the
/// wrong rendering as correct.
#[test]
fn a_predicate_argument_evaluates_to_a_boolean() {
    let source = "state range { shape: enum[d1, w1], initial: .d1 }\n\
                  view root Col { Chip(text: \"1D\", active: range == .d1) \
                                  Chip(text: \"1W\", active: range == .w1) }";
    let report = realize(source, &serde_json::json!({}), RealizeLimits::default());
    let root = report.root.unwrap();
    let mut chips = Vec::new();
    find(&root, "Chip", &mut chips);
    assert_eq!(chips.len(), 2);
    let active = |c: &splash_core::ui_l0::UiNode| {
        c.args
            .iter()
            .find(|(n, _)| n == "active")
            .map(|(_, v)| v.clone())
    };
    assert_eq!(active(chips[0]), Some(NodeValue::Bool(true)));
    assert_eq!(active(chips[1]), Some(NodeValue::Bool(false)));
}

/// A guard's right operand is a binding too. Unchecked, `when x != absent`
/// resolved `absent` to nothing and the comparison decided the branch on that
/// absence — so the branch rendered with an undeclared name in it.
#[test]
fn a_guard_right_operand_must_be_declared() {
    let report = check_ui_l0_named(
        "rhs",
        "state selected { shape: text, initial: \"\" }\n\
         view root Col { when selected != absent { Rule() } }",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("absent")),
        "{:#?}",
        report.diagnostics
    );
}

/// `initial: env.locale.temp_unit` was recognised and dropped, so the state
/// fell back to the enum's first member — a phone set to Fahrenheit rendered
/// Celsius, which is weather.card's actual declaration.
#[test]
fn a_path_valued_initial_resolves_against_data() {
    let source = "source env.locale sys.locale()\n\
                  state units { shape: enum[c, f], initial: env.locale.temp_unit }\n\
                  view root TextRow(text: units)";
    for locale in ["f", "c"] {
        let data = serde_json::json!({"env": {"locale": {"temp_unit": locale}}});
        let report = realize(source, &data, RealizeLimits::default());
        let root = report.root.unwrap();
        assert_eq!(
            root.args.iter().find(|(n, _)| n == "text").unwrap().1,
            NodeValue::Text(locale.into()),
            "the declared initial must follow the host's locale"
        );
    }
}

/// `stories[selected].title` became `stories.title` — the index was parsed and
/// thrown away, so it resolved to nothing.
#[test]
fn a_dynamic_index_resolves_through_its_inner_path() {
    let source = "source movers sys.movers()\n\
                  state selected { shape: text, initial: \"b\" }\n\
                  view root TextRow(text: movers[selected].title)";
    let data = serde_json::json!({"movers": {"a": {"title": "FIRST"}, "b": {"title": "SECOND"}}});
    let report = realize(source, &data, RealizeLimits::default());
    let root = report.root.unwrap();
    assert_eq!(
        root.args.iter().find(|(n, _)| n == "text").unwrap().1,
        NodeValue::Text("SECOND".into())
    );
}

#[test]
fn a_dynamic_index_path_is_itself_checked() {
    let report = check_ui_l0_named(
        "idx",
        "source movers sys.movers()\nview root TextRow(text: movers[typo].title)",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("typo")),
        "{:#?}",
        report.diagnostics
    );
}

/// `set` had no numeric or boolean variant, so `set(1)` fell through to the
/// payload form and did nothing at all when no payload was present. And
/// `SetSource::Path` was treated exactly like `$value`, so `set(config.answer)`
/// wrote whatever the tap carried rather than what it named.
#[test]
fn every_set_form_writes_what_it_names() {
    let card = "state n { shape: number, initial: 0 }\n\
                state ok { shape: bool, initial: false }\n\
                source movers sys.movers(count: 1)\n\
                event bump { n: set(1) }\n\
                event frompath { n: set(movers.answer) }\n\
                event yes { ok: set(true) }\n\
                view root Row(on_tap: bump) { Rule() }";
    let data = serde_json::json!({"movers": {"answer": 7}});
    let mut store = splash_core::ui_l0::InstanceStore::default();

    splash_core::ui_l0::dispatch_with_data(card, &mut store, "root", "bump", None, &data);
    assert_eq!(store.get("@card", "n").and_then(|v| v.as_f64()), Some(1.0));

    splash_core::ui_l0::dispatch_with_data(card, &mut store, "root", "frompath", None, &data);
    assert_eq!(store.get("@card", "n").and_then(|v| v.as_f64()), Some(7.0));

    splash_core::ui_l0::dispatch_with_data(card, &mut store, "root", "yes", None, &data);
    assert_eq!(
        store.get("@card", "ok").and_then(|v| v.as_bool()),
        Some(true)
    );

    // A payload must not hijack a transition that names a path to read.
    let stray = serde_json::json!("STRAY");
    splash_core::ui_l0::dispatch_with_data(
        card,
        &mut store,
        "root",
        "frompath",
        Some(&stray),
        &data,
    );
    assert_eq!(
        store.get("@card", "n").and_then(|v| v.as_f64()),
        Some(7.0),
        "set(path) reads its path, not the payload"
    );
}

#[test]
fn only_dollar_value_names_the_payload() {
    // `$anything` was accepted as a synonym, so a typo silently became $value.
    let report = check_ui_l0_named(
        "typo",
        "state n { shape: number, initial: 0 }\nevent e { n: set($valu) }\nview root Rule()",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("$value")),
        "{:#?}",
        report.diagnostics
    );
}

// ─── the Makepad backend must carry interaction, not just appearance ─────────

/// `UiNode` carried the event name and instance key all along and lowering
/// emitted neither, so every tappable node rendered inert — the stock range
/// controls could not reach `dispatch` at all. The cards looked right and did
/// nothing, which no golden image could detect.
#[test]
fn lowering_emits_the_event_and_the_instance_key() {
    let data = serde_json::json!({
        "movers": [{ "ticker": "NVDA", "name": "NVIDIA", "last": 184.2,
                     "change": 3.1, "pct": 1.7 }],
        "quote": {}, "series": {}, "selected": "", "range": "m1",
        "env": { "locale": {} }, "copy": { "movers": "Top Movers" }
    });
    let dsl = splash_core::ui_l0::makepad::lower(
        &realize(STOCK, &data, RealizeLimits::default())
            .root
            .unwrap(),
    );
    assert!(
        dsl.contains("l0_event: \"open_quote\""),
        "a tappable row must carry its event"
    );
    assert!(
        dsl.contains("l0_key:"),
        "and the key naming WHICH instance was tapped"
    );
    assert!(
        dsl.contains("l0_value: \"NVDA\""),
        "and the payload, or a tap says what happened but not to what"
    );
}

/// `active` selected nothing, so every range chip rendered identically and the
/// card could not show which was selected.
#[test]
fn lowering_distinguishes_an_active_chip() {
    let source = "state range { shape: enum[d1, w1], initial: .d1 }\n\
                  view root Col { Chip(text: \"1D\", active: range == .d1) \
                                  Chip(text: \"1W\", active: range == .w1) }";
    let dsl = splash_core::ui_l0::makepad::lower(
        &realize(source, &serde_json::json!({}), RealizeLimits::default())
            .root
            .unwrap(),
    );
    let fills: Vec<&str> = dsl
        .lines()
        .filter(|l| l.contains("RoundedView"))
        .filter_map(|l| l.split("draw_bg.color: ").nth(1))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(fills.len(), 2, "two chips");
    assert_ne!(
        fills[0], fills[1],
        "the selected chip must not look like the unselected one"
    );
}

// ─── §7 the header is checked, not decorated ─────────────────────────────────

#[test]
fn the_header_is_read_from_a_real_card() {
    let report = check_ui_l0_named("weather", WEATHER);
    let header = report.header.expect("weather.card declares a header");
    assert_eq!(header.ledger.as_deref(), Some("weather"));
    assert_eq!(header.version.as_deref(), Some("1.0.0"));
    assert_eq!(header.level.as_deref(), Some("L0"));
    assert_eq!(header.profile.as_deref(), Some("ui/l0"));
}

/// §7: "Level is declared in the header and checked at append." Nothing read
/// it — every `#` line lexed as a comment — so a card could declare L2 and be
/// accepted and realized as L0. A declaration never compared to the derived
/// level is a comment, not a check.
#[test]
fn a_header_declaring_the_wrong_level_is_rejected() {
    let report = check_ui_l0_named("evil", "# ledger evil@1.0.0\n# level: L2\nview root Rule()");
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("declares level L2")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_header_declaring_an_unknown_profile_is_rejected() {
    let report = check_ui_l0_named(
        "wrong",
        "# ledger x@1.0.0\n# profile: nonsense/v9\nview root Rule()",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("ui/l0")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_card_without_a_header_is_still_accepted() {
    // The header is optional; only a header that CONTRADICTS the card is an
    // error. Requiring one would reject every inline fragment in these tests.
    let report = check_ui_l0_named("bare", "view root Rule()");
    assert!(report.valid, "{:#?}", report.diagnostics);
    assert!(report.header.is_none());
}

// ─── §4 the no-facts rule ────────────────────────────────────────────────────

/// §4 calls a model-authored literal in a data position "a structural
/// violation, not a judgement call". It was neither — `kind = "path"` was
/// declared in the catalog and never enforced, so every drawing widget accepted
/// invented values.
#[test]
fn a_literal_in_a_data_position_is_rejected() {
    for source in [
        "view root TextHero(value: \"34 mph\")",
        "view root TextStat(value: 41.2)",
        "view root TempBar(lo: 5, hi: 9, min: 0, max: 10)",
        "view root WeatherIcon(cond: \"sunny\")",
    ] {
        let report = check_ui_l0_named("facts", source);
        assert!(!report.valid, "should be rejected: {source}");
        assert!(
            report.diagnostics.iter().any(|d| d.message.contains("§4")),
            "the diagnostic should cite the rule: {:#?}",
            report.diagnostics
        );
    }
}

/// The line the rule actually draws: a card MAY choose its own layout, and may
/// not invent a measurement. Without this distinction the check either rejects
/// `gap: 6` or permits `value: 41.2`.
#[test]
fn a_layout_literal_is_not_a_fact() {
    for source in [
        "view root Row(gap: 6) { Rule() }",
        "view root Grid(cols: 2) { Rule() }",
        "view root Col(gap: 3) { Rule() }",
    ] {
        let report = check_ui_l0_named("layout", source);
        assert!(report.valid, "{source} — {:#?}", report.diagnostics);
    }
}

#[test]
fn a_bound_value_is_accepted() {
    let report = check_ui_l0_named(
        "bound",
        "source now sys.weather(lat: 1.0, lon: 2.0)\nview root TextStat(value: now.temp)",
    );
    assert!(report.valid, "{:#?}", report.diagnostics);
}

/// The constructor catalog compared NAMES and three token sets, so changing
/// `TextHero.value` from `number` to `data` in the TOML left it green — and §8
/// claimed on that basis that the two agreed "in both directions". They must
/// agree on every ARGUMENT and its kind, which is what the checker actually
/// consults.
#[test]
fn every_constructor_argument_agrees_with_the_toml() {
    const TOML: &str = include_str!("../../../docs/ui-l0-constructors.toml");

    // [Ctor] followed by `name = { kind = "…" }` lines, until the next header.
    let mut documented: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in TOML.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if !name.contains('.') && !name.starts_with("kinds") {
                documented.push((name.to_string(), Vec::new()));
            } else {
                // A [kinds.*] or [sources."…"] section ends the constructor run.
                documented.push((String::new(), Vec::new()));
            }
            continue;
        }
        let Some((arg, rest)) = line.split_once('=') else {
            continue;
        };
        let Some(kind) = rest
            .split("kind = \"")
            .nth(1)
            .and_then(|k| k.split('"').next())
        else {
            continue;
        };
        if let Some((name, args)) = documented.last_mut() {
            if !name.is_empty() {
                args.push((arg.trim().to_string(), kind.to_string()));
            }
        }
    }
    let documented: Vec<_> = documented
        .into_iter()
        .filter(|(n, _)| !n.is_empty())
        .collect();
    assert!(
        documented.len() > 15,
        "parsed {} constructors",
        documented.len()
    );

    // The TOML's kind names, mapped onto what the Rust catalog stores.
    let agrees = |kind: &str, actual: &catalog::ArgKind| -> bool {
        use catalog::ArgKind::*;
        match (kind, actual) {
            ("path", Path) | ("event", Event) | ("any", Any) => true,
            ("data", Data) | ("number", Number) | ("text", Text) | ("bool", Bool) => true,
            ("token", Token(_)) => true,
            ("unit", TokenOrPath(set)) => *set == catalog::UNIT,
            ("format", Token(set)) => *set == catalog::FORMAT,
            ("width", TokenOrPath(set)) => *set == catalog::WIDTH,
            _ => false,
        }
    };

    for (ctor, args) in &documented {
        let implemented = catalog::lookup(ctor)
            .unwrap_or_else(|| panic!("{ctor:?} is in the TOML but not in the Rust catalog"));
        for (arg, kind) in args {
            let (_, actual) = implemented
                .iter()
                .find(|(n, _)| n == arg)
                .unwrap_or_else(|| panic!("{ctor}.{arg} is in the TOML but not in Rust"));
            assert!(
                agrees(kind, actual),
                "{ctor}.{arg}: TOML says {kind:?}, Rust has {actual:?}"
            );
        }
        assert_eq!(
            implemented.len(),
            args.len(),
            "{ctor} has {} arguments in Rust and {} in the TOML",
            implemented.len(),
            args.len()
        );
    }
}

// ─── §6.1 the declaration graph, including views ─────────────────────────────

/// Acyclicity walked components only, so two shapes recursed at realization and
/// were caught — if at all — by the node cap, which is a resource bound
/// standing in for a structural check.
#[test]
fn mutually_recursive_views_are_rejected() {
    let report = check_ui_l0_named("mutual", "view a b\nview b a\nview root a");
    assert!(!report.valid, "view a -> b -> a must be rejected");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("recursive")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_component_recursing_through_a_view_is_rejected() {
    let report = check_ui_l0_named(
        "via-view",
        "component A() { view Panel { v } }\nview v Panel { A() }\nview root A()",
    );
    assert!(!report.valid, "A -> v -> A must be rejected");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("recursive")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_acyclic_view_chain_is_still_accepted() {
    // The check must not reject ordinary splicing, which the cards rely on.
    let report = check_ui_l0_named(
        "chain",
        "view leaf Rule()\nview mid Panel { leaf }\nview root Panel { mid }",
    );
    assert!(report.valid, "{:#?}", report.diagnostics);
}

/// Depth was approximated by the instance key's LENGTH, which bounds neither
/// direction: a long loop key truncated a shallow tree, and short view names
/// recursed far past max_depth. It is now an actual recursion counter.
#[test]
fn depth_is_counted_not_inferred_from_key_length() {
    // A shallow tree whose keys are long must NOT truncate.
    let long_key = "a_very_long_item_identifier_that_makes_instance_keys_lengthy";
    let source = format!(
        "source movers sys.movers()\nview root Panel {{ for m in movers key m.{long_key} {{ Rule() }} }}"
    );
    let data = serde_json::json!({
        "movers": [{ long_key: "x".repeat(120) }, { long_key: "y".repeat(120) }]
    });
    let report = realize(&source, &data, RealizeLimits::default());
    assert!(
        !report.truncated,
        "a shallow tree must not truncate because its keys are long"
    );
    let mut rules = Vec::new();
    let root = report.root.unwrap();
    find(&root, "Rule", &mut rules);
    assert_eq!(rules.len(), 2, "both rows should realize");
}

// ─── §5.2 component props carry their declared shape ────────────────────────
//
// The shape was parsed and discarded, which cost three separate defects. They
// looked unrelated in the review and shared one cause.

/// An argument the component does not declare was accepted and then silently
/// dropped at realization, so `Framed(titel: "x")` rendered an em dash and
/// looked deliberate.
#[test]
fn an_undeclared_prop_is_rejected() {
    let report = check_ui_l0_named(
        "typo",
        "component F(title: text) { view Panel { TextRow(text: title) } }\n\
         view root F(titel: \"x\")",
    );
    assert!(!report.valid);
    let message = &report.diagnostics[0].message;
    assert!(
        message.contains("no prop") && message.contains("title"),
        "the message should name the props that exist: {message}"
    );
}

/// `set(out)` where `out` is an event prop was accepted, because no param's
/// shape was known so none could be excluded from the readable set.
#[test]
fn an_event_prop_cannot_be_read_by_set() {
    let report = check_ui_l0_named(
        "setevent",
        "component F(out: event) { state n { shape: number, initial: 0 } \
         event e { n: set(out) } view Panel { Row(on_tap: e) { Rule() } } }\n\
         event go { }\nview root F(out: go)",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("is an event, not a value")),
        "{:#?}",
        report.diagnostics
    );
}

/// Every param used to be added to the component's event names, so a `text`
/// prop could be handed to `on_tap` — and a runtime string that happened to
/// equal a declared event would route it. Event routing became data-selected.
#[test]
fn a_value_prop_cannot_be_used_as_an_event() {
    let report = check_ui_l0_named(
        "asevent",
        "component F(label: text) { view Panel { Row(on_tap: label) { Rule() } } }\n\
         view root F(label: \"x\")",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not a declared event")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_token_outside_an_enum_prop_is_rejected() {
    // Now possible because the prop's members survive parsing.
    let report = check_ui_l0_named(
        "enumprop",
        "component F(unit: enum[c, f]) { view Panel { TextValue(value: unit) } }\n\
         view root F(unit: .kelvin)",
    );
    assert!(!report.valid);
    assert!(
        report.diagnostics[0].message.contains(".c, .f"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_correct_call_site_is_still_accepted() {
    let report = check_ui_l0_named(
        "ok",
        "component F(title: text, unit: enum[c, f], on_go: event) { \
           view Panel { Row(on_tap: on_go) { TextRow(text: title) TextValue(value: unit) } } }\n\
         event go { }\nview root F(title: \"x\", unit: .c, on_go: go)",
    );
    assert!(report.valid, "{:#?}", report.diagnostics);
}

/// Retyping between `text` and `number` moved neither the schema id nor the
/// closure digest, because both parsed to the same unclassified shape — so a
/// live string survived into number-shaped state, and a host pinning the digest
/// saw no change.
#[test]
fn retyping_between_text_and_number_is_visible() {
    let base = "component F(x: text) { state s { shape: text, initial: \"\" } \
                view Panel { TextRow(text: s) } }\nview root F(x: \"a\")";

    let retyped_state = base
        .replace("shape: text", "shape: number")
        .replace("initial: \"\"", "initial: 0");
    assert_ne!(
        check_ui_l0_named("p", base).closure,
        check_ui_l0_named("p", &retyped_state).closure,
        "retyping state between text and number must move the digest"
    );

    let retyped_param = base.replace("F(x: text)", "F(x: number)");
    assert_ne!(
        check_ui_l0_named("p", base).closure,
        check_ui_l0_named("p", &retyped_param).closure,
        "retyping a prop must move the digest"
    );
}

// ─── second codex round ──────────────────────────────────────────────────────

#[test]
fn a_contradictory_duplicate_header_field_is_rejected() {
    // Last-wins meant a card could declare what it needed and then declare the
    // truth after it: `# level: L2` followed by `# level: L0` was accepted.
    //
    // The FIRST value must be one that passes on its own, or this test proves
    // nothing. An earlier version used `L2` then `L0`: the retained `L2` failed
    // the level-vs-derived check, so the card was rejected by a different rule
    // and the test passed even with the contradiction check disabled entirely.
    // Mutation testing is what surfaced that.
    let report = check_ui_l0_named(
        "dup-profile",
        "# ledger x@1.0.0\n# profile: ui/l0\n# profile: nonsense/v9\nview root Rule()",
    );
    assert!(!report.valid, "a contradictory header must be refused");
    assert_eq!(
        report.diagnostics.len(),
        1,
        "exactly one rule should fire, or this does not isolate it: {:#?}",
        report.diagnostics
    );
    assert!(
        report.diagnostics[0]
            .message
            .contains("declares profile twice"),
        "{:#?}",
        report.diagnostics
    );

    // And the level form, which is the one the defect was found in.
    let level = check_ui_l0_named(
        "dup-level",
        "# ledger x@1.0.0\n# level: L0\n# level: L2\nview root Rule()",
    );
    assert!(!level.valid);
    assert!(
        level
            .diagnostics
            .iter()
            .any(|d| d.message.contains("declares level twice")),
        "{:#?}",
        level.diagnostics
    );
}

#[test]
fn a_token_in_a_data_position_is_rejected() {
    // `Data` rejected strings and numbers and let tokens through.
    let report = check_ui_l0_named("tok", "view root TextHero(value: .money)");
    assert!(!report.valid);
    assert!(
        report.diagnostics[0].message.contains("token"),
        "{:#?}",
        report.diagnostics
    );
}

/// `set` wrote whatever it was given regardless of what the state could hold.
#[test]
fn set_must_fit_the_target_shape() {
    for (source, expect) in [
        (
            "state ok { shape: bool, initial: false }\nevent e { ok: set(1) }\nview root Rule()",
            "writes a number",
        ),
        (
            "state m { shape: enum[a, b], initial: .a }\nevent e { m: set(true) }\nview root Rule()",
            "writes a bool",
        ),
        (
            "state n { shape: number, initial: 0 }\nevent e { n: set(\"x\") }\nview root Rule()",
            "writes text",
        ),
        (
            "state m { shape: enum[a, b], initial: .a }\nevent e { m: set(.c) }\nview root Rule()",
            "not a member",
        ),
    ] {
        let report = check_ui_l0_named("shape", source);
        assert!(!report.valid, "should be rejected: {source}");
        assert!(
            report.diagnostics.iter().any(|d| d.message.contains(expect)),
            "expected {expect:?}: {:#?}",
            report.diagnostics
        );
    }
}

/// A dotted source name registered only its ROOT, so the lifecycle of
/// `source env.locale` could not be asked for, while `env.$state` — which names
/// no source at all — was accepted.
#[test]
fn a_dotted_source_reports_its_own_lifecycle() {
    let source = "source env.locale sys.locale()\n\
                  view root Col { when env.locale.$state == .ready { TextRow(text: \"READY\") } \
                                  when env.locale.$state == .pending { TextRow(text: \"WAIT\") } }";
    assert!(check_ui_l0_named("dotted", source).valid);

    let ready = realize(
        source,
        &serde_json::json!({"env": {"locale": {"lang": "en"}}}),
        RealizeLimits::default(),
    );
    assert_eq!(texts(&ready.root.unwrap()), vec!["READY"]);

    let pending = realize(source, &serde_json::json!({}), RealizeLimits::default());
    assert_eq!(texts(&pending.root.unwrap()), vec!["WAIT"]);

    let bogus = check_ui_l0_named(
        "aggregate",
        "source env.locale sys.locale()\nview root Col { when env.$state == .ready { Rule() } }",
    );
    assert!(
        !bogus.valid,
        "`env` names no source, so it has no lifecycle to report"
    );
}

/// §5.8 says the schema id covers "names, shapes and initials". It hashed only
/// path and shape, so changing an initial did not reset live state.
#[test]
fn changing_an_initial_changes_the_schema() {
    let base = "component F() { state s { shape: number, initial: 0 } \
                view Panel { TextRow(text: s) } }\nview root F()";
    let changed = base.replace("initial: 0", "initial: 7");
    assert_ne!(
        check_ui_l0_named("p", base).closure,
        check_ui_l0_named("p", &changed).closure,
        "an initial is part of the state schema (§5.8)"
    );
}

// ─── §4 copy provenance ──────────────────────────────────────────────────────

/// §4 requires a rendered literal to be "vocabulary or source-derived". That
/// was undecidable, not merely unenforced: `parse_locale_map` kept only entries
/// whose value was a STRING, and `class: vocabulary` is an identifier — so the
/// provenance was discarded before anything could read it.
#[test]
fn provenance_survives_parsing_and_model_copy_is_refused() {
    let vocabulary = check_ui_l0_named(
        "vocab",
        "copy hi { class: vocabulary, en: \"Top Stories\" }\nview root TextTitle(text: copy.hi)",
    );
    assert!(vocabulary.valid, "{:#?}", vocabulary.diagnostics);

    let user = check_ui_l0_named(
        "user",
        "copy note { class: user-copy, en: \"my note\" }\nview root TextTitle(text: copy.note)",
    );
    assert!(
        user.valid,
        "the user may write their own text: {:#?}",
        user.diagnostics
    );

    let model = check_ui_l0_named(
        "model",
        "copy claim { class: model-copy, en: \"Revenue: $41.2M\" }\n\
         view root TextTitle(text: copy.claim)",
    );
    assert!(
        !model.valid,
        "model-authored text must not reach the screen"
    );
    assert!(
        model
            .diagnostics
            .iter()
            .any(|d| d.message.contains("model-copy")),
        "{:#?}",
        model.diagnostics
    );
}

/// The hyphen in `user-copy` lexes as a minus, and the level classifier read it
/// as arithmetic — so the two spellings §4 depends on could not appear in a card
/// at all. The rule was unstatable in its own language.
#[test]
fn a_hyphenated_copy_class_is_not_arithmetic() {
    for class in ["vocabulary", "user-copy"] {
        let report = check_ui_l0_named(
            "class",
            &format!("copy c {{ class: {class}, en: \"x\" }}\nview root TextTitle(text: copy.c)"),
        );
        assert_eq!(report.level, Level::L0, "{class} should not classify as L1");
        assert!(report.valid, "{class}: {:#?}", report.diagnostics);
    }
}

#[test]
fn an_unknown_copy_class_is_rejected() {
    let report = check_ui_l0_named(
        "bogus",
        "copy c { class: whatever, en: \"x\" }\nview root TextTitle(text: copy.c)",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not a copy class")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_undeclared_copy_reference_is_rejected() {
    // Previously `copy` was a known root and any name under it resolved to
    // nothing, so a typo rendered an em dash.
    let report = check_ui_l0_named("missing", "view root TextTitle(text: copy.nope)");
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not declared")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn the_reference_cards_declare_their_copy_provenance() {
    // All 23 copy declarations across the three cards are `vocabulary`, which
    // is the discipline the profile assumes and could not previously verify.
    for (name, source) in [("weather", WEATHER), ("news", NEWS), ("stock", STOCK)] {
        let report = check_ui_l0_named(name, source);
        assert!(report.valid, "{name}: {:#?}", report.diagnostics);
    }
}

// ─── rules the validator enforces that no test was holding ───────────────────
//
// Found by mutation-testing the validator (mutate.py): suppress one diagnostic,
// run the suite, see whether anything goes red. These nine did not. Each was
// enforced and unprotected — free to regress silently, which is exactly how the
// `initial` fix ended up with a regression test that checked the wrong artifact
// and could not fail.

#[test]
fn state_must_be_the_last_segment_of_a_path() {
    let report = check_ui_l0_named(
        "midstate",
        "source now sys.weather(lat: 1.0, lon: 2.0)\nview root TextRow(text: now.$state.x)",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("last segment")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn cycle_requires_an_enum_state() {
    let report = check_ui_l0_named(
        "cyclebool",
        "state flag { shape: bool, initial: false }\n\
         event e { flag: cycle(.a, .b) }\nview root Rule()",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("needs an enum state")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_loop_over_something_that_is_not_a_collection_is_reported() {
    // A REALIZATION diagnostic, not a validation one: whether a source's value
    // is an array is the host's answer, not the card's declaration. Writing
    // this against check_ui_l0 first is how I learned that.
    let report = realize(
        "source movers sys.movers()\nview root Panel { for x in movers key x.id { Rule() } }",
        &serde_json::json!({"movers": 7}),
        RealizeLimits::default(),
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not a collection")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_reference_to_an_undeclared_view_is_rejected() {
    let report = check_ui_l0_named("noview", "view root Panel { missing_view }");
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("is not a declared view")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn set_from_an_unreadable_name_is_rejected() {
    let report = check_ui_l0_named(
        "unreadable",
        "state n { shape: number, initial: 0 }\n\
         event e { n: set(nowhere.value) }\nview root Rule()",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not a readable name")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn an_into_on_a_component_with_only_an_anonymous_slot_is_rejected() {
    // The complement of `an_into_naming_an_absent_slot_is_rejected`, which
    // exercised the branch where the component DOES offer named slots. This is
    // the other message, and it was unheld.
    let report = check_ui_l0_named(
        "onlyanon",
        "component F() { view Panel { slot } }\n\
         view root Panel { F() { into header { Rule() } } }",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("only an anonymous slot")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_token_where_a_path_belongs_is_rejected() {
    let report = check_ui_l0_named("tokpath", "view root WeatherIcon(cond: .sunny)");
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("takes a bound path")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_literal_where_an_event_prop_belongs_is_rejected() {
    let report = check_ui_l0_named(
        "litevent",
        "component F(on_go: event) { view Panel { Row(on_tap: on_go) { Rule() } } }\n\
         view root F(on_go: \"nope\")",
    );
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("takes an event, not a literal")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_card_without_a_root_view_is_rejected() {
    let report = check_ui_l0_named("noroot", "view sidebar Panel { Rule() }");
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("view root")),
        "{:#?}",
        report.diagnostics
    );
}

/// The level-classified rules (`let`, `while`, arithmetic, `fn`, module access)
/// are held by the LEVEL a card is assigned, not by their diagnostic text — so
/// suppressing the message changes nothing and mutation testing reports them as
/// unprotected. Assert on the level itself, which is the thing §7 actually
/// promises: escalation is never silent.
#[test]
fn every_construct_outside_l0_raises_the_level() {
    for (source, expected, what) in [
        // L2, not L1: an imperative binding is not a pure expression, so it
        // sits with the widget commands rather than with arithmetic.
        ("view root Panel { }\nlet x = 1", Level::L2, "let"),
        ("view root Panel { }\nwhile true { }", Level::L2, "while"),
        (
            "state n { shape: number, initial: 0 }\nview root TextStat(value: n + 1)",
            Level::L1,
            "arithmetic",
        ),
        ("view root Panel { }\nfn tick() { }", Level::L2, "fn"),
    ] {
        let report = check_ui_l0_named("level", source);
        assert!(!report.valid, "{what} must not be L0-valid");
        assert_eq!(
            report.level, expected,
            "{what} should classify as {expected:?}: {:#?}",
            report.diagnostics
        );
    }
}

// ─── codex round three ───────────────────────────────────────────────────────
// Written BEFORE the fixes, and each confirmed failing first.

/// #6: `parse_state` scanned every token in the body for a shape word instead
/// of reading the one after `shape:`. So an enum whose members include a shape
/// name was silently retyped by its own member.
#[test]
fn an_enum_member_named_like_a_shape_does_not_retype_the_state() {
    let report = check_ui_l0_named(
        "shadow",
        "state mode { shape: enum[a, text], initial: .a }\n\
         event e { mode: set(\"arbitrary\") }\nview root Rule()",
    );
    assert!(
        !report.valid,
        "`mode` is an enum; a member happening to be called `text` must not \
         turn it into text and let any string be written"
    );
}

/// #7: a misspelled shape produced `Other` with no diagnostic, and the `set`
/// checks treat `Other` as "accepts anything" — so the typo disabled them.
#[test]
fn a_misspelled_shape_is_rejected_rather_than_silently_permissive() {
    let report = check_ui_l0_named(
        "typo-shape",
        "state n { shape: nubmer, initial: 0 }\nevent e { n: set(\"oops\") }\nview root Rule()",
    );
    assert!(!report.valid, "`nubmer` is not a shape");
}

/// #8: a token was checked only when the target was an enum; every other shape
/// fell through, so `set(.c)` into a number wrote the string "c".
#[test]
fn a_token_set_into_a_non_enum_is_rejected() {
    let report = check_ui_l0_named(
        "toknum",
        "state n { shape: number, initial: 0 }\nevent e { n: set(.c) }\nview root Rule()",
    );
    assert!(!report.valid);
}

/// #7 (second half): a declared initial was never checked against the shape it
/// was declared alongside.
#[test]
fn an_initial_must_fit_its_declared_shape() {
    let report = check_ui_l0_named(
        "badinit",
        "state n { shape: number, initial: \"oops\" }\nview root Rule()",
    );
    assert!(!report.valid);
}

/// #3: this falsifies §4's claim that "a fabricated number cannot reach a
/// numeric position" — which I wrote into the spec the same day. A literal
/// launders through a prop into the drawing widgets, which exist to render
/// live data and hold no authored values.
#[test]
fn a_literal_cannot_launder_through_a_prop_into_a_data_position() {
    let report = check_ui_l0_named(
        "launder",
        "component Bar(n: number) { view TempBar(lo: n, hi: n, min: n, max: n) }\n\
         view root Bar(n: 41.2)",
    );
    assert!(
        !report.valid,
        "an authored number must not reach TempBar by way of a prop"
    );
}

/// #4: prop shapes were retained at parse and discarded again at realization,
/// so an enum token bound with its leading dot and every comparison against it
/// inside the component was false.
#[test]
fn an_enum_token_prop_binds_its_bare_member() {
    let source = "component F(unit: enum[c, f]) { \
                    view Col { when unit == .c { TextRow(text: \"MATCHED\") } } }\n\
                  view root F(unit: .c)";
    let report = realize(source, &serde_json::json!({}), RealizeLimits::default());
    assert_eq!(
        texts(&report.root.unwrap()),
        vec!["MATCHED"],
        "`.c` must bind as `c`, or the component cannot compare against its own prop"
    );
}

/// #4 (second half): an event prop was looked up as DATA at realization, so a
/// state whose name collided with an event name hijacked the routing.
#[test]
fn an_event_prop_binds_the_event_not_same_named_state() {
    let source = "state go { shape: text, initial: \"wrong\" }\n\
                  event go { go: set(\"x\") }\n\
                  event wrong { go: set(\"y\") }\n\
                  component F(out: event) { view Panel { Row(on_tap: out) { Rule() } } }\n\
                  view root F(out: go)";
    let report = realize(source, &serde_json::json!({}), RealizeLimits::default());
    let root = report.root.unwrap();
    let mut rows = Vec::new();
    find(&root, "Row", &mut rows);
    let on_tap = rows[0]
        .args
        .iter()
        .find(|(n, _)| n == "on_tap")
        .map(|(_, v)| v.clone());
    assert_eq!(
        on_tap,
        Some(NodeValue::Event("go".into())),
        "the event named at the call site must be the event that fires"
    );
}

/// #5: a declared default was parsed and dropped, an omitted argument created
/// no binding, and lookup then fell through to top-level injected data — so an
/// unfilled prop could capture whatever the host happened to supply.
#[test]
fn a_prop_default_is_used_and_injected_data_cannot_capture_it() {
    let source = "component Banner(title: text = \"Safe\") { view TextRow(text: title) }\n\
                  view root Banner()";

    let empty = realize(source, &serde_json::json!({}), RealizeLimits::default());
    assert_eq!(
        texts(&empty.root.unwrap()),
        vec!["Safe"],
        "an omitted prop must fall back to its declared default"
    );

    let hostile = realize(
        source,
        &serde_json::json!({"title": "attacker"}),
        RealizeLimits::default(),
    );
    assert_eq!(
        texts(&hostile.root.unwrap()),
        vec!["Safe"],
        "and must not be captured by same-named data the host injected"
    );
}

#[test]
fn laundering_a_literal_through_a_chain_of_props_is_also_refused() {
    // One hop would be a spot fix. The check runs to a fixpoint, so a longer
    // chain is no better a hiding place than a short one.
    let bar = "component Bar(n: number) { view TempBar(lo: n, hi: n, min: n, max: n) }\n";
    for depth in [
        format!("{bar}view root Bar(n: 41.2)"),
        format!("{bar}component Outer(v: number) {{ view Panel {{ Bar(n: v) }} }}\nview root Outer(v: 41.2)"),
        format!(
            "{bar}component Mid(m: number) {{ view Panel {{ Bar(n: m) }} }}\n\
             component Outer(v: number) {{ view Panel {{ Mid(m: v) }} }}\nview root Outer(v: 41.2)"
        ),
    ] {
        assert!(
            !check_ui_l0_named("chain", &depth).valid,
            "an authored number must not reach a data position at any depth"
        );
    }

    // And the two things that must stay legal: a BOUND value through the same
    // chain, and a literal in a position that renders no fact.
    let bound = format!(
        "source now sys.weather(lat: 1.0, lon: 2.0)\n{bar}\
         component Outer(v: number) {{ view Panel {{ Bar(n: v) }} }}\nview root Outer(v: now.temp)"
    );
    assert!(check_ui_l0_named("bound", &bound).valid);
    assert!(
        check_ui_l0_named(
            "label",
            "component Lbl(t: text) { view TextTitle(text: t) }\nview root Lbl(t: \"Heading\")"
        )
        .valid,
        "a literal label renders no fact and must stay legal"
    );
}

/// #1: provenance failed OPEN. A declaration with no `class` at all defaulted
/// to trusted, so the check only bit when a card volunteered the label — which
/// a model with something to hide would not.
#[test]
fn copy_without_a_class_is_refused_rather_than_trusted() {
    let report = check_ui_l0_named(
        "noclass",
        "copy claim { en: \"Revenue: $41.2M\" }\nview root TextTitle(text: copy.claim)",
    );
    assert!(
        !report.valid,
        "undeclared provenance must not default to vocabulary"
    );
}

/// #2: even correctly-marked model-copy laundered through a state initial,
/// because provenance was checked only where an element referenced copy.x
/// directly.
#[test]
fn model_copy_cannot_launder_through_a_state_initial() {
    let report = check_ui_l0_named(
        "via-state",
        "copy claim { class: model-copy, en: \"Revenue: $41.2M\" }\n\
         component F() { state s { shape: text, initial: copy.claim } view TextTitle(text: s) }\n\
         view root F()",
    );
    assert!(!report.valid);
}

// ─── parser and resource bounds ──────────────────────────────────────────────
//
// Found by the second mutation run, after fixing the harness's own blind spot:
// it had been selecting diagnostics by keyword and silently missing newer ones.
// Four of these are §6's termination bounds, which the profile claims outright.

#[test]
fn a_source_over_the_size_limit_is_rejected() {
    let huge = format!("view root Panel {{ }}\n# {}", "x".repeat(300_000));
    let report = check_ui_l0_named("huge", &huge);
    assert!(!report.valid);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("byte limit")),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn malformed_input_is_rejected_rather_than_parsed_loosely() {
    for (source, expect, what) in [
        (
            "state n { shape: number, initial: 0 }\nbanana root Rule()",
            "expected a declaration",
            "unknown declaration",
        ),
        (
            "view root Panel { 42 }",
            "expected an element",
            "a literal where an element belongs",
        ),
        (
            "component F() { banana }",
            "expected state, event or view",
            "junk in a component body",
        ),
        (
            "view root TextRow(text: \"unterminated)",
            "unterminated string",
            "an unterminated string",
        ),
        (
            "view root Panel { } \u{0007}",
            "unexpected character",
            "a character outside the grammar",
        ),
    ] {
        let report = check_ui_l0_named("malformed", source);
        assert!(!report.valid, "{what} must be rejected");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains(expect)),
            "{what}: {:#?}",
            report.diagnostics
        );
    }
}

/// §6, condition 5: node count and nesting depth are capped, and the
/// declaration count with them. These are the bounds that make realization
/// terminate on input the grammar alone does not bound.
#[test]
fn the_resource_bounds_hold() {
    let deep = format!(
        "view root {}Rule(){}",
        "Panel { ".repeat(200),
        " }".repeat(200)
    );
    let report = check_ui_l0_named("deep", &deep);
    assert!(!report.valid, "nesting must be bounded");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("too deep")),
        "{:#?}",
        report.diagnostics
    );

    let many = format!(
        "{}view root Rule()",
        (0..2_000)
            .map(|i| format!("state s{i} {{ shape: number, initial: 0 }}\n"))
            .collect::<String>()
    );
    let report = check_ui_l0_named("many", &many);
    assert!(!report.valid, "declaration count must be bounded");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("too many declarations")),
        "{:#?}",
        report.diagnostics
    );

    let wide = format!("view root Panel {{ {} }}", "Rule() ".repeat(3_000));
    let report = check_ui_l0_named("wide", &wide);
    assert!(!report.valid, "block width must be bounded");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("too many elements")),
        "{:#?}",
        report.diagnostics
    );
}

/// #9: `source_roots` was fixed to hold full names, but ordinary `Scope.roots`
/// still held the ROOT — so declaring one source authorised reading everything
/// beside it. The card then renders an undeclared dependency, which is exactly
/// what §4's "anything reachable without a declared source is a defect by
/// construction" says cannot happen.
#[test]
fn a_dotted_source_does_not_authorise_its_whole_root() {
    let report = check_ui_l0_named(
        "sibling",
        "source env.locale sys.locale()\nview root TextRow(text: env.theme.secret)",
    );
    assert!(
        !report.valid,
        "declaring env.locale must not grant env.theme"
    );

    // The declared one still works.
    assert!(
        check_ui_l0_named(
            "ok",
            "source env.locale sys.locale()\nview root TextRow(text: env.locale.lang)"
        )
        .valid
    );
}

/// #10: a dotted source's dependency edge was lost, because the argument was
/// reduced to its root and compared against the full declared name. The plan
/// then emitted the dependent BEFORE the thing it depends on.
#[test]
fn a_dotted_source_dependency_is_ordered_correctly() {
    let plan = splash_core::ui_l0::source_plan(
        "source env.locale sys.locale()\n\
         source scene sys.photo(query: env.locale.lang)\n\
         view root Rule()",
    );
    let order: Vec<&str> = plan.requests.iter().map(|r| r.name.as_str()).collect();
    let locale = order.iter().position(|n| *n == "env.locale");
    let scene = order.iter().position(|n| *n == "scene");
    assert!(
        locale < scene,
        "a source must be fetched before the one that reads it: {order:?}"
    );
    let scene_req = plan.requests.iter().find(|r| r.name == "scene").unwrap();
    assert!(
        scene_req.depends_on.iter().any(|d| d == "env.locale"),
        "the edge must be recorded: {:?}",
        scene_req.depends_on
    );
}

/// #11: only `level` and `profile` were checked for contradictory duplicates.
/// A card could declare two different ledger identities and the last one won.
#[test]
fn a_contradictory_duplicate_ledger_is_rejected() {
    let report = check_ui_l0_named(
        "twoledgers",
        "# ledger real@1.0.0\n# ledger impostor@9.9.9\nview root Rule()",
    );
    assert!(!report.valid, "a card has one identity");
}

/// #13: the regression test for §5.8's schema id compared `report.closure`,
/// which the definition digest already made sensitive to initials — so it
/// passed with the schema fix reverted. It must exercise the store.
#[test]
fn changing_an_initial_resets_live_component_state() {
    const BEFORE: &str = "source items sys.movers()\n\
        component Rowy(item: record) { state n { shape: number, initial: 0 } \
          event bump { n: set(1) } \
          view Col { Row(on_tap: bump) { TextRow(text: item.name) } \
                     when n == 1 { TextCaption(text: item.name) } } }\n\
        view root Panel { for it in items key it.id { Rowy(item: it) } }";

    let data = serde_json::json!({"items": [{"id": "a", "name": "A"}]});
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let first =
        splash_core::ui_l0::realize_with_state(BEFORE, &data, &store, RealizeLimits::default());
    let root = first.root.unwrap();
    let mut rows = Vec::new();
    find(&root, "Row", &mut rows);
    splash_core::ui_l0::dispatch(BEFORE, &mut store, &rows[0].key.clone(), "bump");

    let after = BEFORE.replace("initial: 0", "initial: 7");
    let out =
        splash_core::ui_l0::realize_with_state(&after, &data, &store, RealizeLimits::default());
    let out_root = out.root.unwrap();
    let mut caps = Vec::new();
    find(&out_root, "TextCaption", &mut caps);
    assert_eq!(
        caps.len(),
        0,
        "a changed initial is a schema change (§5.8), so live state must reset"
    );
}

// ─── codex round four: merge blockers ────────────────────────────────────────

/// A 60 KB card — well under the 256 KiB byte limit — overflowed the parser's
/// stack and ABORTED the process. `dotted_name` recursed once per nested
/// dynamic index with only a local guard, and predicate right-hand sides had
/// the same shape.
///
/// This is a crash on untrusted input in the component whose entire purpose is
/// accepting untrusted input, so a documented limit would not do.
#[test]
fn deeply_nested_paths_do_not_overflow_the_stack() {
    for depth in [2_000, 20_000, 60_000] {
        let source = format!(
            "source a sys.movers()\nview root TextRow(text: {}a{})",
            "a[".repeat(depth),
            "]".repeat(depth)
        );
        assert!(
            source.len() < 262_144,
            "the probe must stay under the byte limit, or it proves nothing"
        );
        // Reaching the assertion at all is the test: an overflow aborts the
        // whole binary rather than failing this case.
        let report = check_ui_l0_named("deep-path", &source);
        assert!(
            !report.valid,
            "nesting past the bound must be refused, not accepted"
        );
    }
}

/// The same shape on the other recursive path: a predicate's right-hand side.
#[test]
fn deeply_nested_predicates_do_not_overflow_the_stack() {
    let depth = 20_000;
    let source = format!(
        "state n {{ shape: number, initial: 0 }}\nview root Col {{ when n == {}n{} {{ Rule() }} }}",
        "n[".repeat(depth),
        "]".repeat(depth)
    );
    assert!(source.len() < 262_144);
    let report = check_ui_l0_named("deep-pred", &source);
    assert!(!report.valid);
}

/// §5.7 says duplicate keys are an error at realization. Nothing tracked them,
/// so two items with the same id produced IDENTICAL instance keys — one cell
/// shared by two rows, and tapping either toggled both.
#[test]
fn duplicate_loop_keys_are_reported_and_do_not_share_state() {
    const CARD: &str = "source items sys.movers()\n\
        component R(it: record) { state n { shape: bool, initial: false } \
          event f { n: toggle } \
          view Col { Row(on_tap: f) { TextRow(text: it.name) } \
                     when n { TextCaption(text: it.name) } } }\n\
        view root Panel { for i in items key i.id { R(it: i) } }";

    let data = serde_json::json!({
        "items": [{"id": "same", "name": "A"}, {"id": "same", "name": "B"}]
    });
    let report = realize(CARD, &data, RealizeLimits::default());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("duplicate")),
        "a duplicate key must be reported: {:#?}",
        report.diagnostics
    );

    let root = report.root.unwrap();
    let mut rows = Vec::new();
    find(&root, "Row", &mut rows);
    assert_eq!(rows.len(), 2, "both rows still render");
    assert_ne!(
        rows[0].key, rows[1].key,
        "two instances must not share one state cell"
    );
}

/// The key expression itself was never validated: its first segment was
/// discarded unconditionally, so `key typo.id` behaved exactly like
/// `key item.id` — which also means changing a key does not reliably move
/// identity, contrary to §5.1.
#[test]
fn a_loop_key_must_name_the_loop_binder() {
    let report = check_ui_l0_named(
        "badkey",
        "source items sys.movers()\nview root Panel { for it in items key typo.id { Rule() } }",
    );
    assert!(!report.valid, "`typo` is not the binder `it`");
}

/// `validate_sources` checked the helper name and its argument NAMES, never the
/// paths those arguments carry. So a card could reference anything and the
/// value went to the host inside a well-formed request — which is precisely
/// the undeclared dependency §4 says is "a defect by construction".
#[test]
fn a_source_argument_path_must_be_declared() {
    let report = check_ui_l0_named(
        "leak-arg",
        "source photo sys.photo(query: secrets.token)\nview root Rule()",
    );
    assert!(!report.valid, "`secrets` is not declared");

    // The plan must not carry it either, for a caller that skips checking.
    let plan = splash_core::ui_l0::source_plan(
        "source photo sys.photo(query: secrets.token)\nview root Rule()",
    );
    assert!(
        !plan.diagnostics.is_empty(),
        "the plan must report it rather than hand the host an undeclared path"
    );

    // A declared one still works.
    assert!(
        check_ui_l0_named(
            "ok",
            "state city { shape: text, initial: \"\" }\n\
             source place sys.geocode(name: city)\nview root Rule()"
        )
        .valid
    );
}

/// The acyclicity walk counted every popped name against one budget, and
/// `collect_constructors` yields ordinary catalog constructors, not just graph
/// nodes. So padding a component with enough `Rule()`s exhausts the budget
/// before the traversal reaches the recursive edge, and the card is reported
/// acyclic — which means §6's first termination condition is not enforced.
#[test]
fn padding_cannot_hide_a_cycle_from_the_acyclicity_check() {
    let padding = "Rule() ".repeat(600);
    let source = format!(
        "component A() {{ view Col {{ A() Panel {{ {padding} }} Panel {{ {padding} }} }} }}\n\
         view root A()"
    );
    let report = check_ui_l0_named("padded", &source);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("recursive")),
        "a recursive component must be caught however much noise surrounds it: {:#?}",
        report
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// A header containing ONLY duplicate `model:` lines never set `saw_any`, so
/// `parse_header` returned None and the contradiction vanished with it.
#[test]
fn a_model_only_duplicate_header_is_still_caught() {
    let report = check_ui_l0_named(
        "dupmodel",
        "# model: first\n# model: second\nview root Rule()",
    );
    assert!(!report.valid, "a card names one model");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("model twice")),
        "{:#?}",
        report.diagnostics
    );
}

/// A parameter default was accepted regardless of the shape declared beside it,
/// and the spec's own `enum[c, f] = c` spelling was not parsed at all.
#[test]
fn a_parameter_default_must_fit_its_declared_shape() {
    for source in [
        "component F(n: number = \"oops\") { view TextRow(text: n) }\nview root F()",
        "component G(m: enum[a, b] = .missing) { view TextRow(text: m) }\nview root G()",
        "component H(b: bool = 3) { view Col { when b { Rule() } } }\nview root H()",
    ] {
        let report = check_ui_l0_named("baddefault", source);
        assert!(!report.valid, "should be rejected: {source}");
    }
}

#[test]
fn a_bare_enum_member_is_a_valid_default() {
    // §5.2's own example spells it without the dot. It parsed as nothing, so
    // the documented form silently produced no default at all.
    let report = check_ui_l0_named(
        "bare",
        "component F(unit: enum[c, f] = c) { view TextValue(value: unit) }\nview root F()",
    );
    assert!(report.valid, "{:#?}", report.diagnostics);

    let out = realize(
        "component F(unit: enum[c, f] = c) { view TextRow(text: unit) }\nview root F()",
        &serde_json::json!({}),
        RealizeLimits::default(),
    );
    assert_eq!(
        texts(&out.root.unwrap()),
        vec!["c"],
        "the default must bind"
    );
}

// ─── phase 3: the three claims the spec makes and did not keep ───────────────

/// §3: "an event names several transitions, applied atomically". They were
/// applied sequentially straight to the store, so a batch that fails partway
/// leaves the earlier writes standing — a half-applied event.
#[test]
fn a_transition_batch_is_atomic() {
    const CARD: &str = "state a { shape: number, initial: 0 }\n\
                        state b { shape: number, initial: 0 }\n\
                        source cfg sys.movers()\n\
                        event both { a: set(1), b: set(cfg.missing) }\n\
                        view root Row(on_tap: both) { Rule() }";

    let mut store = splash_core::ui_l0::InstanceStore::default();
    // `cfg.missing` does not resolve, so the batch cannot complete.
    let applied = splash_core::ui_l0::dispatch_with_data(
        CARD,
        &mut store,
        "root",
        "both",
        None,
        &serde_json::json!({"cfg": {}}),
    );
    assert!(
        !applied,
        "a batch that cannot complete must not report success"
    );
    assert_eq!(
        store.get("@card", "a"),
        None,
        "the first write must not survive a batch that failed on the second"
    );
}

/// §3: `set($value)` performed no shape check at dispatch, so a string payload
/// entered a number cell — the transition type system stops at the boundary.
#[test]
fn a_payload_must_fit_the_state_it_is_written_to() {
    const CARD: &str = "state n { shape: number, initial: 0 }\n\
                        event pick { n: set($value) }\n\
                        view root Row(on_tap: pick, value: \"x\") { Rule() }";
    let mut store = splash_core::ui_l0::InstanceStore::default();
    let payload = serde_json::json!("not a number");
    let applied =
        splash_core::ui_l0::dispatch_with(CARD, &mut store, "root", "pick", Some(&payload));
    assert!(!applied, "a payload of the wrong shape must be refused");
    assert_eq!(store.get("@card", "n"), None);
}

/// §3: `clear` restores "the declared initial". It ignored a path-valued one and
/// fell back to the shape default, and `cycle` started from the shape default
/// too — so a card whose initial comes from the host began in the wrong state.
#[test]
fn clear_restores_a_path_valued_initial() {
    const CARD: &str = "source env.locale sys.locale()\n\
                        state units { shape: enum[c, f], initial: env.locale.temp_unit }\n\
                        event reset { units: clear }\n\
                        event flip { units: cycle(.c, .f) }\n\
                        view root Row(on_tap: flip) { Rule() }";
    let data = serde_json::json!({"env": {"locale": {"temp_unit": "f"}}});
    let mut store = splash_core::ui_l0::InstanceStore::default();

    splash_core::ui_l0::dispatch_with_data(CARD, &mut store, "root", "flip", None, &data);
    assert_eq!(
        store.get("@card", "units").and_then(|v| v.as_str()),
        Some("c"),
        "cycle must advance from the host-supplied initial `f`, not from the shape default"
    );

    splash_core::ui_l0::dispatch_with_data(CARD, &mut store, "root", "reset", None, &data);
    assert_eq!(
        store.get("@card", "units").and_then(|v| v.as_str()),
        Some("f"),
        "clear must restore the declared initial, path-valued or not"
    );
}

/// §4 refuses model-authored text in a rendering position. It was enforced only
/// where an element named `copy.x` directly, so any indirection defeated it.
/// These are the routes: through a component prop, through a prop default, and
/// through a transition that writes it into state.
#[test]
fn model_copy_cannot_reach_the_screen_by_any_route() {
    const DECL: &str = "copy claim { class: model-copy, en: \"Revenue: $41.2M\" }\n";

    for (route, source) in [
        (
            "directly",
            format!("{DECL}view root TextTitle(text: copy.claim)"),
        ),
        (
            "through a prop",
            format!(
                "{DECL}component F(t: text) {{ view TextTitle(text: t) }}\n\
                 view root F(t: copy.claim)"
            ),
        ),
        (
            "through a state initial",
            format!(
                "{DECL}component F() {{ state s {{ shape: text, initial: copy.claim }} \
                 view TextTitle(text: s) }}\nview root F()"
            ),
        ),
        (
            "through a transition",
            format!(
                "{DECL}state s {{ shape: text, initial: \"\" }}\n\
                 event e {{ s: set(copy.claim) }}\nview root Row(on_tap: e) {{ Rule() }}"
            ),
        ),
    ] {
        assert!(
            !check_ui_l0_named("route", &source).valid,
            "model-copy must not reach the screen {route}"
        );
    }

    // Vocabulary still travels every one of those routes.
    let ok = "copy label { class: vocabulary, en: \"Top Stories\" }\n\
              component F(t: text) { view TextTitle(text: t) }\n\
              view root F(t: copy.label)";
    assert!(check_ui_l0_named("vocab", ok).valid);
}
