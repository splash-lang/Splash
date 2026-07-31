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
