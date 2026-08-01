//! Level 0 of the UI profile — parser and validator for generated card source.
//!
//! See [`docs/ui-profile-l0.md`](../../../docs/ui-profile-l0.md) for the normative
//! grammar and [`docs/ui-l0-constructors.toml`](../../../docs/ui-l0-constructors.toml)
//! for the argument contract. The catalog in [`catalog`] is the implementation's
//! copy of that TOML; a test asserts the two agree.
//!
//! ## What this establishes, and what it does not
//!
//! Two properties are decided here: **membership in the L0 grammar**, and **absence
//! of any authority-bearing operation**. The second is structural rather than
//! enforced — the grammar has no syntax for a module access or a call, so there is
//! nothing to reject; the checks for `mod.` and `ui.<id>.<method>` exist to classify
//! such source as a higher level with a useful diagnostic, not to hold a boundary.
//!
//! Termination is **not** established. It needs the five conditions in §6 of the
//! profile, of which this checks two: the component graph is acyclic, and iteration
//! is over a bound path rather than an expression. Collection length, node count and
//! nesting depth are the runtime's to bound.
//!
//! Everything is effect-free and bounded: no evaluation, no imports, no host access.

use crate::{SyntaxDiagnostic, DEFAULT_MAX_SOURCE_BYTES, DEFAULT_MAX_SYNTAX_NESTING};

/// The narrowest profile a source belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    /// Constructors, bindings, keyed loops, guarded branches, components with
    /// declared local state. No expression form, no reachable capability.
    L0,
    /// Adds pure expressions. Not accepted by this checker.
    L1,
    /// Full Splash, including imperative widget commands. Not accepted here.
    L2,
}

/// Result of checking source against the L0 profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiL0Report {
    pub valid: bool,
    /// The narrowest level the source belongs to. `valid` is true only for
    /// [`Level::L0`].
    pub level: Level,
    pub diagnostics: Vec<SyntaxDiagnostic>,
    pub diagnostics_truncated: bool,
    /// The header a card declares — profile §2. `None` when it has none.
    pub header: Option<CardHeader>,
    /// Profile §7: every component the card depends on, with a digest of its
    /// definition, sorted by name.
    ///
    /// This is what a host pins. `level` is a judgment over the transitive
    /// component closure, so it is only as durable as the definitions it was
    /// computed from — replace one and an approved L0 card can silently become
    /// L2. Storing this alongside the approved level makes that detectable:
    /// a digest that moved means the level must be re-derived before the card
    /// is accepted, rather than inherited from the record it no longer matches.
    pub closure: Vec<(String, u64)>,
}

/// What a card's header declares — ledger name, version, level and profile.
///
/// §7 says "level is declared in the header and checked at append", and nothing
/// read it: every `#` line lexed as a comment, so a card could declare
/// `# level: L2` and be accepted and realized as L0. A declared level that is
/// never compared to the derived one is not a check, it is a comment.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CardHeader {
    pub ledger: Option<String>,
    pub version: Option<String>,
    pub level: Option<String>,
    pub profile: Option<String>,
    pub model: Option<String>,
    /// Fields declared more than once with different values. Last-wins made
    /// `# level: L2` followed by `# level: L0` accepted, so a card could
    /// declare whatever it needed and then declare the truth after it.
    pub contradictions: Vec<String>,
}

/// Read the header from the leading `#` lines, which the lexer discards.
pub fn parse_header(source: &str) -> Option<CardHeader> {
    let mut header = CardHeader::default();
    let mut saw_any = false;
    for line in source.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('#') else {
            if line.is_empty() {
                continue;
            }
            break; // The header is the leading comment block only.
        };
        let rest = rest.trim();
        if let Some(decl) = rest.strip_prefix("ledger ") {
            saw_any = true;
            let (name, version) = match decl.split_once('@') {
                Some((n, v)) => (n.trim().to_string(), Some(v.trim().to_string())),
                None => (decl.trim().to_string(), None),
            };
            // A card has ONE identity. Last-wins let a record declare two.
            match (&header.ledger, &header.version) {
                (Some(first), v) if *first != name || *v != version => {
                    header.contradictions.push(format!(
                        "the header declares the ledger twice, as {first:?} and {name:?}"
                    ));
                }
                _ => {
                    header.ledger = Some(name);
                    header.version = version;
                }
            }
        } else if let Some((key, value)) = rest.split_once(':') {
            let value = value.trim().to_string();
            match key.trim() {
                "level" => {
                    saw_any = true;
                    match &header.level {
                        Some(first) if *first != value => {
                            header.contradictions.push(format!(
                                "the header declares level twice, as {first:?} and {value:?}"
                            ));
                        }
                        _ => header.level = Some(value),
                    }
                }
                "profile" => {
                    saw_any = true;
                    match &header.profile {
                        Some(first) if *first != value => {
                            header.contradictions.push(format!(
                                "the header declares profile twice, as {first:?} and {value:?}"
                            ));
                        }
                        _ => header.profile = Some(value),
                    }
                }
                "model" => {
                    // Presence counts: a header of only `model:` lines used to
                    // return None, taking any contradiction in it along too.
                    saw_any = true;
                    match &header.model {
                        Some(first) if *first != value => {
                            header.contradictions.push(format!(
                                "the header declares the model twice, as {first:?} and {value:?}"
                            ));
                        }
                        _ => header.model = Some(value),
                    }
                }
                _ => {}
            }
        }
    }
    saw_any.then_some(header)
}

/// Diagnostics retained before truncation, so a pathological source cannot grow
/// the report without bound.
pub const MAX_UI_L0_DIAGNOSTICS: usize = 64;

/// Maximum declarations retained from one card.
pub const MAX_UI_L0_DECLARATIONS: usize = 1_024;

/// Check a card written on the **Splash surface** against the L0 profile.
///
/// Projection alone is not checking. `project_declarations` recognises shapes;
/// this runs the same `validate()` the bespoke surface runs, so §5.2's prop
/// rules, §4's no-facts rule, §6's termination conditions and every other check
/// apply identically. Without it a prop's declared shape is *retained and never
/// consulted* — the mirror of the "specified but not retained" defect this
/// surface exists to avoid repeating.
///
/// The carrier is checked separately by `check_vm_compatibility`: that says the
/// VM could run it, this says L0 admits it, and neither subsumes the other.
pub fn check_ui_l0_splash(source: &str) -> UiL0Report {
    let projection = splash_surface::project_declarations(source);
    let mut sink = Diagnostics::default();
    for d in &projection.diagnostics {
        sink.push(d.line, d.column, d.message.clone());
    }
    let card = projection.into_card();
    validate(&card, &mut sink);
    let mut report = sink.into_report(Level::L0);
    report.closure = component_closure(&card);
    report
}

/// Parse the BESPOKE surface into the same report shape the Splash surface
/// produces, so the two can be compared without either knowing about the other.
///
/// Exists for the equality gate: two surfaces are a second *surface* only while
/// they project the same tree, and a second *language* the moment they stop.
pub fn project_bespoke(source: &str) -> splash_surface::ProjectionReport {
    let mut sink = Diagnostics::default();
    let card = match lex(source, &mut sink) {
        Some(tokens) => Parser::new(&tokens, &mut sink).parse_card(),
        None => Card::default(),
    };
    splash_surface::ProjectionReport::from_card(card, sink.items)
}

/// Check source against the L0 profile.
pub fn check_ui_l0(source: &str) -> UiL0Report {
    check_ui_l0_named("card.l0", source)
}

/// Check source against the L0 profile, naming it for diagnostics.
pub fn check_ui_l0_named(_name: &str, source: &str) -> UiL0Report {
    let mut sink = Diagnostics::default();
    let header = parse_header(source);

    if source.len() > DEFAULT_MAX_SOURCE_BYTES {
        sink.push(
            1,
            1,
            format!(
                "source is {} bytes, over the {DEFAULT_MAX_SOURCE_BYTES}-byte limit",
                source.len()
            ),
        );
        return sink.into_report(Level::L0);
    }

    let tokens = match lex(source, &mut sink) {
        Some(tokens) => tokens,
        None => return sink.into_report(Level::L0),
    };

    // A construct outside the grammar tells us the level before parsing can
    // finish, and a level diagnostic is far more useful than "unexpected token".
    if let Some((level, diag)) = classify_beyond_l0(&tokens) {
        sink.push(diag.line, diag.column, diag.message);
        let mut report = sink.into_report(level);
        report.header = header.clone();
        // Compare here too: this is the path a card that UNDER-declares its
        // level takes, and it was the one path that skipped the comparison.
        check_header(&header, level, &mut report);
        return report;
    }

    let mut parser = Parser::new(&tokens, &mut sink);
    let card = parser.parse_card();
    if sink.any() {
        return sink.into_report(Level::L0);
    }

    validate(&card, &mut sink);
    let mut report = sink.into_report(Level::L0);
    report.closure = component_closure(&card);
    report.header = header.clone();
    check_header(&header, report.level, &mut report);
    report
}

/// §7: a declared level must match the derived one, and escalation is never
/// silent. Reading the header without comparing it would leave the declaration
/// decorative — which is what it was.
fn check_header(header: &Option<CardHeader>, derived: Level, report: &mut UiL0Report) {
    let Some(header) = header else { return };

    for message in &header.contradictions {
        report.valid = false;
        report.diagnostics.push(SyntaxDiagnostic {
            line: 1,
            column: 1,
            message: message.clone(),
        });
    }

    if let Some(declared) = &header.level {
        let matches = match declared.trim() {
            "L0" => derived == Level::L0,
            "L1" => derived == Level::L1,
            "L2" => derived == Level::L2,
            _ => false,
        };
        if !matches {
            report.valid = false;
            report.diagnostics.push(SyntaxDiagnostic {
                line: 1,
                column: 1,
                message: format!(
                    "the header declares level {declared}, but the card requires {derived:?} \
                     — a record needing a wider grammar is rejected until the level is \
                     explicitly raised (profile §7)"
                ),
            });
        }
    }

    // The profile names which grammar the card was written against. A card
    // claiming a profile this checker does not implement must not be silently
    // accepted as though it had claimed this one.
    if let Some(profile) = &header.profile {
        if profile.trim() != "ui/l0" {
            report.valid = false;
            report.diagnostics.push(SyntaxDiagnostic {
                line: 1,
                column: 1,
                message: format!(
                    "the header declares profile {profile:?}, but this checker implements \
                     \"ui/l0\""
                ),
            });
        }
    }
}

/// A digest per component definition, sorted by name — profile §7.
///
/// The digest covers everything that can move a card's level or change what it
/// renders: params, state, events and the whole view body. It deliberately does
/// NOT cover the component's position in the file, so reordering declarations is
/// not a version change.
fn component_closure(card: &Card) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = card
        .components
        .iter()
        .map(|c| (c.name.clone(), definition_digest(c)))
        .collect();
    out.sort();
    out
}

fn definition_digest(component: &Component) -> u64 {
    fn element(e: &Element, into: &mut String) {
        into.push_str(&e.name);
        into.push('(');
        for b in &e.binders {
            into.push_str(b);
            into.push(',');
        }
        for a in &e.args {
            into.push_str(&a.name);
            into.push(':');
            into.push_str(&format!("{:?}", a.value));
            into.push(',');
        }
        if let Some(cmp) = &e.cmp {
            into.push_str(cmp);
        }
        if let Some(rhs) = &e.rhs {
            into.push_str(&format!("{rhs:?}"));
        }
        into.push('|');
        for child in &e.children {
            element(child, into);
        }
        into.push(')');
    }

    let mut acc = String::new();
    acc.push_str(&component.name);
    for p in &component.params {
        acc.push_str(&format!("{}:{:?}:{:?}", p.name, p.shape, p.default));
        acc.push(',');
    }
    for st in &component.states {
        acc.push_str(&format!(
            "{}:{:?}:{:?}:{:?}:{}",
            st.path, st.shape, st.initial, st.initial_path, st.keep
        ));
    }
    for ev in &component.events {
        acc.push_str(&ev.name);
        for t in &ev.transitions {
            acc.push_str(&format!("{}={:?}{:?}", t.target, t.form, t.tokens));
        }
    }
    element(&component.body, &mut acc);

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in acc.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

// ─────────────────────────────────────────────────────────────────── diagnostics ──

#[derive(Default)]
struct Diagnostics {
    items: Vec<SyntaxDiagnostic>,
    truncated: bool,
}

impl Diagnostics {
    fn push(&mut self, line: usize, column: usize, message: String) {
        if self.items.len() >= MAX_UI_L0_DIAGNOSTICS {
            self.truncated = true;
            return;
        }
        self.items.push(SyntaxDiagnostic {
            line,
            column,
            message,
        });
    }

    fn at(&mut self, t: &Token, message: String) {
        self.push(t.line, t.column, message);
    }

    fn any(&self) -> bool {
        !self.items.is_empty()
    }

    fn into_report(self, level: Level) -> UiL0Report {
        UiL0Report {
            header: None,
            closure: Vec::new(),
            valid: self.items.is_empty() && level == Level::L0,
            level,
            diagnostics: self.items,
            diagnostics_truncated: self.truncated,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────── lexer ──

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Ident,
    /// A dotted token: `.fill`. Tokens are lexically marked so a bare identifier
    /// is always a path — see profile §2.2.
    Token,
    Str,
    Num,
    Punct,
    Cmp,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    text: String,
    line: usize,
    column: usize,
}

impl Token {
    fn is(&self, kind: Kind, text: &str) -> bool {
        self.kind == kind && self.text == text
    }
    fn is_kw(&self, kw: &str) -> bool {
        self.is(Kind::Ident, kw)
    }
    fn is_punct(&self, p: &str) -> bool {
        self.is(Kind::Punct, p)
    }
}

fn lex(source: &str, sink: &mut Diagnostics) -> Option<Vec<Token>> {
    let bytes: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1usize, 1usize);

    while i < bytes.len() {
        let c = bytes[i];

        if c == '\n' {
            line += 1;
            col = 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        // Comments: `#` to end of line. The profile's cards use `#`; `//` is
        // rejected so a Splash-syntax card cannot be mistaken for an L0 one.
        if c == '#' || (c == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/') {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
            continue;
        }

        let (start_line, start_col) = (line, col);

        // String literal.
        if c == '"' {
            let mut text = String::new();
            i += 1;
            col += 1;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == '"' {
                    i += 1;
                    col += 1;
                    closed = true;
                    break;
                }
                if bytes[i] == '\n' {
                    break;
                }
                text.push(bytes[i]);
                i += 1;
                col += 1;
            }
            if !closed {
                sink.push(start_line, start_col, "unterminated string".into());
                return None;
            }
            out.push(Token {
                kind: Kind::Str,
                text,
                line: start_line,
                column: start_col,
            });
            continue;
        }

        // Dotted token, but only when a name follows — `.` between path segments
        // is punctuation and is handled below.
        if c == '.' && i + 1 < bytes.len() && is_ident_start(bytes[i + 1]) {
            // A `.` directly after an identifier, token, or `]` is a path
            // separator, not a token marker.
            let after_value = out
                .last()
                .map(|t: &Token| {
                    matches!(t.kind, Kind::Ident | Kind::Token | Kind::Num) || t.is_punct("]")
                })
                .unwrap_or(false);
            if !after_value {
                let mut text = String::from(".");
                i += 1;
                col += 1;
                while i < bytes.len() && is_ident_continue(bytes[i]) {
                    text.push(bytes[i]);
                    i += 1;
                    col += 1;
                }
                out.push(Token {
                    kind: Kind::Token,
                    text,
                    line: start_line,
                    column: start_col,
                });
                continue;
            }
        }

        // Comparators.
        if matches!(c, '=' | '!' | '<' | '>') {
            let two = i + 1 < bytes.len() && bytes[i + 1] == '=';
            let text: String = if two {
                let s: String = bytes[i..i + 2].iter().collect();
                i += 2;
                col += 2;
                s
            } else {
                i += 1;
                col += 1;
                c.to_string()
            };
            // A lone `=` or `!` is punctuation; only the two-character forms are
            // comparators. Without this `!x` lexes as Cmp and the level
            // classifier's punctuation check never fires.
            if !two {
                out.push(Token {
                    kind: Kind::Punct,
                    text,
                    line: start_line,
                    column: start_col,
                });
            } else {
                out.push(Token {
                    kind: Kind::Cmp,
                    text,
                    line: start_line,
                    column: start_col,
                });
            }
            continue;
        }

        // Number.
        if c.is_ascii_digit() {
            let mut text = String::new();
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                // Stop at a `.` that starts a name — `7.foo` is a path, not a float.
                if bytes[i] == '.' && (i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_digit()) {
                    break;
                }
                text.push(bytes[i]);
                i += 1;
                col += 1;
            }
            out.push(Token {
                kind: Kind::Num,
                text,
                line: start_line,
                column: start_col,
            });
            continue;
        }

        // Identifier.
        if is_ident_start(c) {
            let mut text = String::new();
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                text.push(bytes[i]);
                i += 1;
                col += 1;
            }
            out.push(Token {
                kind: Kind::Ident,
                text,
                line: start_line,
                column: start_col,
            });
            continue;
        }

        // Punctuation, including operators the grammar does not admit. Those are
        // kept as tokens so `classify_beyond_l0` can report them as L1 rather
        // than as a lex failure.
        if "{}()[]:,.$+-*/%?|&".contains(c) {
            i += 1;
            col += 1;
            out.push(Token {
                kind: Kind::Punct,
                text: c.to_string(),
                line: start_line,
                column: start_col,
            });
            continue;
        }

        sink.push(start_line, start_col, format!("unexpected character {c:?}"));
        return None;
    }

    Some(out)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

// ───────────────────────────────────────────────────────────── level classifier ──

/// Report the level implied by any construct outside L0, before parsing.
///
/// Level is a property of the whole card: the maximum any record requires
/// (profile §7). Component-closure resolution happens in [`validate`]; this is
/// the lexical half.
fn classify_beyond_l0(tokens: &[Token]) -> Option<(Level, SyntaxDiagnostic)> {
    let mut worst: Option<(Level, SyntaxDiagnostic)> = None;
    let mut keep = |found: Option<(Level, SyntaxDiagnostic)>| {
        if let Some((level, diag)) = found {
            let better = match &worst {
                Some((Level::L2, _)) => false,
                Some((Level::L1, _)) => level == Level::L2,
                _ => true,
            };
            if better {
                worst = Some((level, diag));
            }
        }
    };
    for (i, t) in tokens.iter().enumerate() {
        let bad = |level: Level, msg: &str| {
            Some((
                level,
                SyntaxDiagnostic {
                    line: t.line,
                    column: t.column,
                    message: msg.into(),
                },
            ))
        };

        if t.is_kw("fn") {
            keep(bad(
                Level::L2,
                "`fn` is not in L0: a function reintroduces unbounded work",
            ));
        }
        if t.is_kw("while") {
            keep(bad(
                Level::L2,
                "`while` is not in L0: iteration must be over a declared collection",
            ));
        }
        if t.is_kw("let") || t.is_kw("var") {
            keep(bad(
                Level::L2,
                "`let` is not in L0: state is declared, not bound imperatively",
            ));
        }
        if t.is_kw("mod") && tokens.get(i + 1).is_some_and(|n| n.is_punct(".")) {
            keep(bad(
                Level::L2,
                "module access is not in L0: the host surface is empty",
            ));
        }
        // `ui.<id>.<method>` — an imperative widget command.
        if t.is_kw("ui")
            && tokens.get(i + 1).is_some_and(|n| n.is_punct("."))
            && tokens.get(i + 3).is_some_and(|n| n.is_punct("."))
        {
            keep(bad(
                Level::L2,
                "imperative widget commands are L2: L0 declares a tree",
            ));
        }
        if t.kind == Kind::Punct && matches!(t.text.as_str(), "+" | "*" | "/" | "%" | "?") {
            keep(bad(
                Level::L1,
                "arithmetic and conditional expressions are not in L0: an expression can hide a fabricated fact",
            ));
        }
        // A copy class is a hyphenated NAME, not a subtraction: `class:
        // user-copy` lexes as `user`, `-`, `copy`, and reading that as
        // arithmetic made the two spellings §4 depends on unusable — the
        // provenance rule could not be written in a card at all.
        let is_class_name = tokens
            .get(i.wrapping_sub(3))
            .is_some_and(|k| k.text == "class")
            && tokens.get(i.wrapping_sub(1)).is_some_and(|p| {
                p.kind == Kind::Ident && matches!(p.text.as_str(), "user" | "model")
            });

        // `-` is only rejected between values; a negative literal is fine.
        if t.is_punct("-")
            && !is_class_name
            && tokens
                .get(i.wrapping_sub(1))
                .is_some_and(|p| matches!(p.kind, Kind::Ident | Kind::Num | Kind::Token))
        {
            keep(bad(Level::L1, "arithmetic is not in L0"));
        }
        if t.is_punct("!") {
            keep(bad(
                Level::L1,
                "negation is computation: use the `toggle` transition form",
            ));
        }
    }
    worst
}

// ────────────────────────────────────────────────────────────────────────── ast ──

#[derive(Debug, Default)]
struct Card {
    sources: Vec<SourceDecl>,
    states: Vec<StateDecl>,
    events: Vec<EventDecl>,
    /// `copy` declarations: name -> locale -> text. Authored in the ledger, so
    /// they resolve from the CARD rather than the injected data. Carrying them
    /// in the data blob duplicated what the card already said, and the two
    /// silently drifted.
    copies: Vec<CopyDecl>,
    components: Vec<Component>,
    views: Vec<View>,
}

/// A declared capability dependency: the name it binds, the helper that answers
/// it, and the arguments. Parsed rather than skipped — a card that declares what
/// data it needs is useless if nothing can read the declaration.
#[derive(Clone, Debug)]
struct SourceDecl {
    name: String,
    helper: String,
    args: Vec<(String, SourceArg)>,
    line: usize,
}

/// One argument to a capability query. None of these is computed.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceArg {
    /// A path into another source, card state, or a declared value.
    Path(String),
    Text(String),
    Number(f64),
    /// `fields: [temp, hi, lo]` — which fields the card needs back.
    List(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Shape {
    Text,
    Number,
    Bool,
    Enum(Vec<String>),
    /// A structured value a source returns — §5.2, a component prop shape.
    Record,
    /// An iterable a `for` walks.
    Collection,
    /// An event passed in, so a component can report outward (§5.4).
    Event,
    /// A shape the declaration did not name.
    ///
    /// `Text` and `Number` used to land here too, which made them
    /// indistinguishable: retyping a state between them changed neither the
    /// schema id nor the closure digest, so live values survived a change that
    /// should have reset them.
    Other,
}

/// Authored text, with the provenance that decides whether it may be rendered.
///
/// The provenance was parsed and thrown away — and worse, `parse_locale_map`
/// only kept entries whose value was a STRING, so `class: vocabulary` (an
/// identifier) was dropped before anything could read it. §4's "a literal in a
/// data position must be vocabulary or source-derived" was therefore not merely
/// unenforced but undecidable: nothing recorded which kind a string was.
#[derive(Clone, Debug)]
struct CopyDecl {
    name: String,
    provenance: Provenance,
    locales: Vec<(String, String)>,
}

/// Who wrote a piece of text, and therefore whether it can be trusted on screen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Provenance {
    /// Fixed interface language — labels, units, headings. Translatable, and
    /// carries no claim about the world.
    #[default]
    Vocabulary,
    /// Text the user wrote. Theirs to be wrong about.
    UserCopy,
    /// Text the MODEL wrote. May not appear in a rendering position: this is
    /// the class §4 exists to keep off the screen, because a model-authored
    /// string is indistinguishable from a fact it invented.
    ModelCopy,
}

/// A component's declared input — §5.2.
///
/// The shape was parsed and dropped, which cost three separate defects: an
/// unknown argument at a call site could not be told from a known one, an event
/// prop could not be told from a value, and every param had to be treated as a
/// possible event name because none could be ruled out.
#[derive(Clone, Debug)]
struct Param {
    name: String,
    shape: Shape,
    /// `title: text = "Safe"`. Dropped, an omitted argument created no binding
    /// at all and lookup fell through to top-level injected data — so an
    /// unfilled prop could capture whatever the host happened to supply under
    /// the same name.
    default: Option<crate::JsonValue>,
}

#[derive(Debug)]
struct StateDecl {
    path: String,
    shape: Shape,
    /// The declared `initial`. Parsed rather than discarded: without it a guard
    /// on a freshly-mounted state compares against null and takes the wrong
    /// branch on first render.
    initial: Option<crate::JsonValue>,
    /// A path-valued `initial`, e.g. `initial: env.locale.temp_unit`. Held
    /// separately because it resolves against injected data at realization,
    /// not at parse. Dropping it made weather.card fall back to the enum's
    /// first member, so a device set to Fahrenheit still rendered Celsius.
    initial_path: Option<String>,
    /// `keep: true` — profile §5.8. A schema change resets local state by
    /// default; this opts one cell out, and even then only while its own shape
    /// is unchanged. Preservation is never inferred from a shape that merely
    /// looks compatible.
    keep: bool,
}

/// An event and the transition batch it applies. Profile §3: an event names
/// several transitions, applied atomically.
#[derive(Debug)]
struct EventDecl {
    name: String,
    transitions: Vec<Transition>,
}

#[derive(Debug)]
struct Transition {
    target: String,
    form: Form,
    /// `cycle` members, which must be declared in the target's enum shape.
    tokens: Vec<String>,
    line: usize,
    column: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Form {
    Set(SetSource),
    Toggle,
    Cycle,
    Clear,
    /// Anything else — an expression, which L0 has no form for.
    NotTotal(String),
}

/// What `set(…)` assigns. Each is a value the runtime already holds; none is
/// computed, so the transition stays total.
#[derive(Clone, Debug, PartialEq)]
enum SetSource {
    /// `$value` — the payload from the element that raised the event.
    Payload,
    /// A `.token`, e.g. an enum member.
    Token(String),
    /// A string literal.
    Text(String),
    /// A bound path, resolved against the data the runtime injected.
    Path(String),
    /// A numeric literal. Without this `set(1)` parsed as `Payload` and did
    /// nothing at all when no payload was present.
    Num(f64),
    /// A boolean literal.
    Bool(bool),
}

#[derive(Debug)]
struct Component {
    name: String,
    params: Vec<Param>,
    states: Vec<StateDecl>,
    events: Vec<EventDecl>,
    body: Element,
    line: usize,
    /// Components this one instantiates, for the acyclicity check.
    instantiates: Vec<String>,
}

#[derive(Debug)]
struct View {
    name: String,
    body: Element,
}

#[derive(Clone, Debug, Default)]
struct Element {
    /// Constructor name, a referenced view/component name, or empty for a group.
    name: String,
    args: Vec<Arg>,
    children: Vec<Element>,
    /// Bindings a `for` introduces: the item and the optional index.
    binders: Vec<String>,
    line: usize,
    column: usize,
    is_reference: bool,
    /// `when` guard: comparator and right-hand operand, absent for a bare bool.
    cmp: Option<String>,
    rhs: Option<Operand>,
    /// `for` loop: the per-item key expression. Identity never comes from position.
    key_path: Option<String>,
}

#[derive(Clone, Debug)]
struct Arg {
    name: String,
    value: Operand,
    line: usize,
    column: usize,
}

#[derive(Clone, Debug)]
enum Operand {
    Path(String),
    Token(String),
    Str(String),
    Num(f64),
    /// A comparison in argument position: `active: range == .d1`.
    ///
    /// Previously this kept only the left path and discarded the comparator and
    /// right operand, so `active: range == .d1` realized as the VALUE of
    /// `range` rather than as a boolean — live in stock.card, and invisible
    /// because the device golden recorded the wrong rendering as correct.
    Predicate {
        path: String,
        cmp: String,
        rhs: Box<Operand>,
    },
}

// ─────────────────────────────────────────────────────────────────────── parser ──

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    sink: &'a mut Diagnostics,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token], sink: &'a mut Diagnostics) -> Self {
        Self {
            tokens,
            at: 0,
            sink,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        if t.is_some() {
            self.at += 1;
        }
        t
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.peek().is_some_and(|t| t.is_punct(p)) {
            self.at += 1;
            return true;
        }
        false
    }

    fn expect_punct(&mut self, p: &str) -> bool {
        if self.eat_punct(p) {
            return true;
        }
        match self.peek().cloned() {
            Some(t) => self
                .sink
                .at(&t, format!("expected {p:?}, found {:?}", t.text)),
            None => self
                .sink
                .push(0, 0, format!("expected {p:?}, found end of source")),
        }
        false
    }

    fn ident(&mut self) -> Option<String> {
        match self.peek().cloned() {
            Some(t) if t.kind == Kind::Ident => {
                self.at += 1;
                Some(t.text)
            }
            Some(t) => {
                self.sink
                    .at(&t, format!("expected a name, found {:?}", t.text));
                None
            }
            None => {
                self.sink
                    .push(0, 0, "expected a name, found end of source".into());
                None
            }
        }
    }

    fn parse_card(&mut self) -> Card {
        let mut card = Card::default();
        let mut guard = 0usize;

        while self.peek().is_some() {
            guard += 1;
            if guard > MAX_UI_L0_DECLARATIONS {
                self.sink.push(0, 0, "too many declarations".into());
                break;
            }
            let Some(t) = self.peek().cloned() else { break };
            let before = self.at;

            match t.text.as_str() {
                "source" => {
                    self.at += 1;
                    let line = t.line;
                    if let Some(name) = self.dotted_name() {
                        let (helper, args) = self.parse_source_query();
                        card.sources.push(SourceDecl {
                            name,
                            helper,
                            args,
                            line,
                        });
                    }
                }
                "state" => {
                    self.at += 1;
                    if let Some(d) = self.parse_state() {
                        card.states.push(d);
                    }
                }
                "event" => {
                    self.at += 1;
                    if let Some(e) = self.parse_event() {
                        card.events.push(e);
                    }
                }
                "copy" => {
                    self.at += 1;
                    if let Some(n) = self.ident() {
                        let (provenance, locales) = self.parse_locale_map();
                        // Absent `class` is NOT vocabulary. Defaulting to
                        // trusted meant the rule only bit when a card
                        // volunteered the label, which a model with something to
                        // hide would simply not do.
                        let provenance = match provenance {
                            Some(p) => p,
                            None => {
                                self.sink.at(
                                    &t,
                                    format!(
                                        "copy {n:?} declares no `class`; L0 needs one of \
                                         `vocabulary`, `user-copy` or `model-copy` \
                                         (profile §4)"
                                    ),
                                );
                                Provenance::ModelCopy
                            }
                        };
                        card.copies.push(CopyDecl {
                            name: n,
                            provenance,
                            locales,
                        });
                    }
                }
                "component" => {
                    self.at += 1;
                    if let Some(c) = self.parse_component() {
                        card.components.push(c);
                    }
                }
                "view" => {
                    self.at += 1;
                    let Some(name) = self.ident() else { break };
                    let body = self.parse_element();
                    card.views.push(View { name, body });
                }
                _ => {
                    self.sink.at(
                        &t,
                        format!(
                            "expected a declaration (source, state, event, copy, component, view), found {:?}",
                            t.text
                        ),
                    );
                    self.at += 1;
                }
            }

            // Never loop without consuming; a malformed card must terminate.
            if self.at == before {
                self.at += 1;
            }
        }
        card
    }

    /// `a.b.c`, `a.0.c`, `a[b]` — amendments #3 and #4. A constant index and an
    /// index by a bound path are both total lookups, so neither is an expression.
    fn dotted_name(&mut self) -> Option<String> {
        let mut name = self.ident()?;
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > 32 {
                break;
            }
            // `.$state` — the source lifecycle suffix (§5.9). It is the one
            // segment that starts with a sigil, which is what keeps it from
            // colliding with a field a source might actually return.
            let next_is_status = self.peek().is_some_and(|t| t.is_punct("."))
                && self
                    .tokens
                    .get(self.at + 1)
                    .is_some_and(|t| t.is_punct("$"))
                && self
                    .tokens
                    .get(self.at + 2)
                    .is_some_and(|t| t.kind == Kind::Ident && t.text == "state");
            if next_is_status {
                self.at += 3;
                name.push_str(".$state");
                continue;
            }

            let next_is_segment = self.peek().is_some_and(|t| t.is_punct("."))
                && self
                    .tokens
                    .get(self.at + 1)
                    .is_some_and(|t| matches!(t.kind, Kind::Ident | Kind::Num));
            if next_is_segment {
                self.at += 1;
                let Some(seg) = self.next() else { break };
                name.push('.');
                name.push_str(&seg.text);
                continue;
            }
            if self.peek().is_some_and(|t| t.is_punct("[")) {
                if self.depth > DEFAULT_MAX_SYNTAX_NESTING {
                    if let Some(t) = self.peek().cloned() {
                        self.sink.at(&t, "nesting is too deep".into());
                    }
                    return Some(name);
                }
                self.at += 1;
                self.depth += 1;
                // The index is itself a path and must be RETAINED, not just
                // recognised: dropping it turned `stories[selected].title` into
                // `stories.title`, which resolved to nothing and rendered an
                // em dash. Held as a `[…]` segment so lookup can resolve the
                // inner path and use its value as the key.
                let index = self.dotted_name();
                self.depth -= 1;
                if let Some(index) = index {
                    name.push_str(".[");
                    name.push_str(&index);
                    name.push(']');
                }
                self.expect_punct("]");
                continue;
            }
            break;
        }
        Some(name)
    }

    /// A source body is a capability query the runtime executes. It is not
    /// evaluated here and its shape is the runtime's contract, so it is skipped
    /// to the end of its balanced parentheses.
    /// `sys.weather(lat: place.lat, fields: [temp, hi])` — the helper and its
    /// arguments. The body is a query the RUNTIME executes; the view can only
    /// bind to its result, which is what keeps the host surface empty.
    fn parse_source_query(&mut self) -> (String, Vec<(String, SourceArg)>) {
        let mut helper = String::new();
        while let Some(t) = self.peek().cloned() {
            if t.kind == Kind::Ident || t.is_punct(".") {
                helper.push_str(&t.text);
                self.at += 1;
            } else {
                break;
            }
        }

        let mut args = Vec::new();
        if !self.eat_punct("(") {
            return (helper, args);
        }
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > 64 || self.peek().is_none() {
                break;
            }
            if self.eat_punct(")") {
                break;
            }
            let Some(name) = self.ident() else {
                self.at += 1;
                continue;
            };
            if !self.eat_punct(":") {
                break;
            }
            let value = match self.peek().cloned() {
                Some(t) if t.is_punct("[") => {
                    self.at += 1;
                    let mut items = Vec::new();
                    while let Some(i) = self.peek().cloned() {
                        if i.is_punct("]") {
                            break;
                        }
                        if matches!(i.kind, Kind::Ident | Kind::Str | Kind::Num) {
                            items.push(i.text.clone());
                        }
                        self.at += 1;
                    }
                    self.expect_punct("]");
                    SourceArg::List(items)
                }
                Some(t) if t.kind == Kind::Str => {
                    self.at += 1;
                    SourceArg::Text(t.text)
                }
                Some(t) if t.kind == Kind::Num => {
                    self.at += 1;
                    SourceArg::Number(t.text.parse().unwrap_or(0.0))
                }
                Some(t) if t.kind == Kind::Token => {
                    self.at += 1;
                    SourceArg::Text(t.text.trim_start_matches('.').to_string())
                }
                Some(t) if t.kind == Kind::Ident => {
                    SourceArg::Path(self.dotted_name().unwrap_or(t.text))
                }
                _ => {
                    self.at += 1;
                    continue;
                }
            };
            args.push((name, value));
            if !self.eat_punct(",") {
                self.expect_punct(")");
                break;
            }
        }
        (helper, args)
    }

    fn skip_braced(&mut self) {
        if self.peek().is_some_and(|t| t.is_punct("{")) {
            self.skip_balanced("{", "}");
        }
    }

    fn skip_balanced(&mut self, open: &str, close: &str) {
        let mut depth = 0usize;
        while let Some(t) = self.peek().cloned() {
            if t.is_punct(open) {
                depth += 1;
            } else if t.is_punct(close) {
                depth -= 1;
                self.at += 1;
                if depth == 0 {
                    return;
                }
                continue;
            }
            self.at += 1;
        }
    }

    fn parse_state(&mut self) -> Option<StateDecl> {
        let path = self.dotted_name()?;
        let mut shape = Shape::Other;
        let mut initial = None;
        let mut initial_path = None;
        let mut keep = false;
        // `{ shape: enum[a, b], initial: .a, keep: true }`.
        //
        // Read the value AFTER each key rather than scanning the body for shape
        // words. Scanning meant a member of an enum could retype the state it
        // belonged to: `shape: enum[a, text]` parsed as the enum and then the
        // member `text` overwrote it, so the field accepted any string.
        if self.peek().is_some_and(|t| t.is_punct("{")) {
            let mut scan = self.at + 1;
            let mut saw_shape = false;
            while let Some(t) = self.tokens.get(scan) {
                if t.is_punct("}") {
                    break;
                }
                let is_key = t.kind == Kind::Ident
                    && self.tokens.get(scan + 1).is_some_and(|n| n.is_punct(":"));
                if !is_key {
                    scan += 1;
                    continue;
                }
                let key = t.text.clone();
                let value = self.tokens.get(scan + 2);
                match key.as_str() {
                    "shape" => {
                        saw_shape = true;
                        shape = match value.map(|v| v.text.as_str()) {
                            Some("text") => Shape::Text,
                            Some("number") => Shape::Number,
                            Some("bool") => Shape::Bool,
                            Some("record") => Shape::Record,
                            Some("collection") => Shape::Collection,
                            Some("event") => Shape::Event,
                            Some("enum") => {
                                let mut members = Vec::new();
                                let mut at = scan + 4; // past `enum` and `[`
                                while let Some(m) = self.tokens.get(at) {
                                    if m.is_punct("]") {
                                        break;
                                    }
                                    if m.kind == Kind::Ident {
                                        members.push(m.text.clone());
                                    }
                                    at += 1;
                                }
                                Shape::Enum(members)
                            }
                            Some(other) => {
                                // A misspelling used to become `Other`, which
                                // the `set` checks treat as "accepts anything" —
                                // so a typo silently disabled them.
                                let other = other.to_string();
                                if let Some(v) = value {
                                    self.sink.at(
                                        v,
                                        format!(
                                            "{other:?} is not an L0 shape; L0 has text, \
                                             number, bool, enum[…], record, collection \
                                             and event"
                                        ),
                                    );
                                }
                                Shape::Other
                            }
                            None => Shape::Other,
                        };
                    }
                    "initial" => {
                        if let Some(v) = value {
                            initial = match v.kind {
                                Kind::Str => Some(crate::JsonValue::String(v.text.clone())),
                                Kind::Token => Some(crate::JsonValue::String(
                                    v.text.trim_start_matches('.').to_string(),
                                )),
                                Kind::Num => v.text.parse::<f64>().ok().map(crate::JsonValue::from),
                                Kind::Ident if v.text == "true" || v.text == "false" => {
                                    Some(crate::JsonValue::Bool(v.text == "true"))
                                }
                                Kind::Ident => {
                                    let mut path = v.text.clone();
                                    let mut at = scan + 3;
                                    while self.tokens.get(at).is_some_and(|t| t.is_punct("."))
                                        && self
                                            .tokens
                                            .get(at + 1)
                                            .is_some_and(|t| t.kind == Kind::Ident)
                                    {
                                        path.push('.');
                                        path.push_str(&self.tokens[at + 1].text);
                                        at += 2;
                                    }
                                    initial_path = Some(path);
                                    None
                                }
                                _ => None,
                            };
                        }
                    }
                    "keep" => {
                        keep = value.is_some_and(|v| v.kind == Kind::Ident && v.text == "true");
                    }
                    _ => {}
                }
                scan += 1;
            }

            if !saw_shape {
                if let Some(t) = self.peek().cloned() {
                    self.sink
                        .at(&t, format!("state {path:?} declares no `shape`"));
                }
            }
            self.skip_braced();
        }

        Some(StateDecl {
            path,
            shape,
            initial,
            initial_path,
            keep,
        })
    }

    /// `{ class: vocabulary, en: "…", zh: "…" }` — the text per locale.
    fn parse_locale_map(&mut self) -> (Option<Provenance>, Vec<(String, String)>) {
        let mut out = Vec::new();
        let mut provenance = None;
        if !self.eat_punct("{") {
            return (provenance, out);
        }
        let mut guard = 0usize;
        while let Some(t) = self.peek().cloned() {
            guard += 1;
            if guard > 64 || t.is_punct("}") {
                break;
            }
            if t.kind == Kind::Ident
                && self
                    .tokens
                    .get(self.at + 1)
                    .is_some_and(|n| n.is_punct(":"))
            {
                if let Some(v) = self.tokens.get(self.at + 2) {
                    // `class:` names a provenance and its value is an
                    // IDENTIFIER, which the string-only filter below discarded.
                    if t.text == "class" {
                        // `-` lexes as punctuation, so `user-copy` arrives as
                        // the identifier `user`. Match the head rather than
                        // reassembling: the three classes differ there.
                        provenance = Some(match v.text.as_str() {
                            "user" => Provenance::UserCopy,
                            "model" => Provenance::ModelCopy,
                            "vocabulary" => Provenance::Vocabulary,
                            other => {
                                self.sink.at(
                                    v,
                                    format!(
                                        "{other:?} is not a copy class; L0 has \
                                         `vocabulary`, `user-copy` and `model-copy`"
                                    ),
                                );
                                Provenance::Vocabulary
                            }
                        });
                    } else if v.kind == Kind::Str {
                        out.push((t.text.clone(), v.text.clone()));
                    }
                }
            }
            self.at += 1;
        }
        self.expect_punct("}");
        (provenance, out)
    }

    /// `event name { path: total-form, … }`
    fn parse_event(&mut self) -> Option<EventDecl> {
        let name = self.ident()?;
        let mut transitions = Vec::new();

        if !self.eat_punct("{") {
            return Some(EventDecl { name, transitions });
        }
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > 64 || self.peek().is_none() {
                break;
            }
            if self.eat_punct("}") {
                break;
            }
            let Some(head) = self.peek().cloned() else {
                break;
            };
            let Some(target) = self.dotted_name() else {
                self.at += 1;
                continue;
            };
            if !self.expect_punct(":") {
                break;
            }

            let (form, tokens) = self.parse_total_form();
            transitions.push(Transition {
                target,
                form,
                tokens,
                line: head.line,
                column: head.column,
            });

            if !self.eat_punct(",") {
                self.expect_punct("}");
                break;
            }
        }
        Some(EventDecl { name, transitions })
    }

    /// One of `set(v)`, `toggle`, `cycle(.a, .b)`, `clear` — profile §3. Anything
    /// else is recorded as not-total so validation can name it.
    fn parse_total_form(&mut self) -> (Form, Vec<String>) {
        let Some(t) = self.peek().cloned() else {
            return (Form::NotTotal(String::new()), Vec::new());
        };
        match t.text.as_str() {
            "toggle" => {
                self.at += 1;
                (Form::Toggle, Vec::new())
            }
            "clear" => {
                self.at += 1;
                (Form::Clear, Vec::new())
            }
            "set" => {
                self.at += 1;
                let mut source = SetSource::Payload;
                if self.peek().is_some_and(|t| t.is_punct("(")) {
                    self.at += 1;
                    if let Some(inner) = self.peek().cloned() {
                        match inner.kind {
                            Kind::Token => {
                                source = SetSource::Token(
                                    inner.text.trim_start_matches('.').to_string(),
                                );
                                self.at += 1;
                            }
                            Kind::Str => {
                                source = SetSource::Text(inner.text.clone());
                                self.at += 1;
                            }
                            Kind::Punct if inner.text == "$" => {
                                // `$value` — the payload. The name is checked:
                                // `$anything` used to be accepted as a synonym,
                                // so a typo silently became the payload.
                                self.at += 1;
                                match self.ident() {
                                    Some(name) if name == "value" => {}
                                    Some(other) => {
                                        self.sink.at(
                                            &inner,
                                            format!(
                                                "`${other}` is not a payload; L0 names it `$value`"
                                            ),
                                        );
                                    }
                                    None => {}
                                }
                            }
                            Kind::Num => {
                                source = SetSource::Num(inner.text.parse().unwrap_or(0.0));
                                self.at += 1;
                            }
                            Kind::Ident if inner.text == "true" || inner.text == "false" => {
                                source = SetSource::Bool(inner.text == "true");
                                self.at += 1;
                            }
                            Kind::Ident => {
                                source = self
                                    .dotted_name()
                                    .map(SetSource::Path)
                                    .unwrap_or(SetSource::Payload);
                            }
                            _ => {
                                self.at += 1;
                            }
                        }
                    }
                    self.expect_punct(")");
                }
                (Form::Set(source), Vec::new())
            }
            "cycle" => {
                self.at += 1;
                let mut members = Vec::new();
                if self.peek().is_some_and(|t| t.is_punct("(")) {
                    self.at += 1;
                    let mut guard = 0usize;
                    while let Some(m) = self.peek().cloned() {
                        guard += 1;
                        if guard > 64 || m.is_punct(")") {
                            break;
                        }
                        if m.kind == Kind::Token {
                            members.push(m.text.trim_start_matches('.').to_string());
                        } else if m.kind == Kind::Ident {
                            // A bare name here is a path, and a path is not a token.
                            members.push(format!("!{}", m.text));
                        }
                        self.at += 1;
                    }
                    self.expect_punct(")");
                }
                (Form::Cycle, members)
            }
            other => {
                self.at += 1;
                (Form::NotTotal(other.to_string()), Vec::new())
            }
        }
    }

    fn parse_component(&mut self) -> Option<Component> {
        let line = self.peek().map(|t| t.line).unwrap_or(0);
        let name = self.ident()?;
        let mut params = Vec::new();

        if self.eat_punct("(") {
            let mut guard = 0usize;
            while self.peek().is_some_and(|t| !t.is_punct(")")) {
                guard += 1;
                if guard > 64 {
                    break;
                }
                if let Some(t) = self.peek().cloned() {
                    if t.kind == Kind::Ident
                        && self
                            .tokens
                            .get(self.at + 1)
                            .is_some_and(|n| n.is_punct(":"))
                    {
                        // The declared shape follows the colon. `enum[a, b]`
                        // carries its members, which is what lets a token
                        // argument be checked against the prop it fills.
                        let shape = match self.tokens.get(self.at + 2) {
                            Some(k) if k.text == "text" => Shape::Text,
                            Some(k) if k.text == "number" => Shape::Number,
                            Some(k) if k.text == "bool" => Shape::Bool,
                            Some(k) if k.text == "record" => Shape::Record,
                            Some(k) if k.text == "collection" => Shape::Collection,
                            Some(k) if k.text == "event" => Shape::Event,
                            Some(k) if k.text == "enum" => {
                                let mut members = Vec::new();
                                let mut at = self.at + 4; // past `enum` and `[`
                                while let Some(m) = self.tokens.get(at) {
                                    if m.is_punct("]") {
                                        break;
                                    }
                                    if m.kind == Kind::Ident {
                                        members.push(m.text.clone());
                                    }
                                    at += 1;
                                }
                                Shape::Enum(members)
                            }
                            _ => Shape::Other,
                        };
                        // `= <literal>` after the shape.
                        let mut default = None;
                        let mut at = self.at + 3;
                        if matches!(shape, Shape::Enum(_)) {
                            // Skip `[…]`, BALANCED. Seeking the first `]` put
                            // the cursor wherever a bracket happened to be.
                            let mut depth = 0usize;
                            while let Some(t) = self.tokens.get(at) {
                                if t.is_punct("[") {
                                    depth += 1;
                                } else if t.is_punct("]") {
                                    depth -= 1;
                                    if depth == 0 {
                                        at += 1;
                                        break;
                                    }
                                }
                                at += 1;
                            }
                        }
                        if self.tokens.get(at).is_some_and(|t| t.is_punct("=")) {
                            if let Some(v) = self.tokens.get(at + 1) {
                                default = match v.kind {
                                    Kind::Str => Some(crate::JsonValue::String(v.text.clone())),
                                    Kind::Token => Some(crate::JsonValue::String(
                                        v.text.trim_start_matches('.').to_string(),
                                    )),
                                    Kind::Num => {
                                        v.text.parse::<f64>().ok().map(crate::JsonValue::from)
                                    }
                                    Kind::Ident if v.text == "true" || v.text == "false" => {
                                        Some(crate::JsonValue::Bool(v.text == "true"))
                                    }
                                    // A bare enum member: `unit: enum[c, f] = c`,
                                    // which is how §5.2 spells it and which
                                    // produced no default at all.
                                    Kind::Ident if matches!(shape, Shape::Enum(_)) => {
                                        Some(crate::JsonValue::String(v.text.clone()))
                                    }
                                    _ => None,
                                };
                            }
                        }
                        params.push(Param {
                            name: t.text.clone(),
                            shape,
                            default,
                        });
                    }
                }
                self.at += 1;
            }
            self.expect_punct(")");
        }

        self.expect_punct("{");
        let mut states = Vec::new();
        let mut events = Vec::new();
        let mut body = Element::default();

        let mut guard = 0usize;
        while let Some(t) = self.peek().cloned() {
            guard += 1;
            if guard > MAX_UI_L0_DECLARATIONS || t.is_punct("}") {
                break;
            }
            match t.text.as_str() {
                "state" => {
                    self.at += 1;
                    if let Some(d) = self.parse_state() {
                        states.push(d);
                    }
                }
                "event" => {
                    self.at += 1;
                    if let Some(e) = self.parse_event() {
                        events.push(e);
                    }
                }
                "view" => {
                    self.at += 1;
                    body = self.parse_element();
                }
                _ => {
                    self.sink.at(
                        &t,
                        format!(
                            "expected state, event or view in a component, found {:?}",
                            t.text
                        ),
                    );
                    self.at += 1;
                }
            }
        }
        self.expect_punct("}");

        let mut instantiates = Vec::new();
        collect_constructors(&body, &mut instantiates);
        Some(Component {
            name,
            params,
            states,
            events,
            body,
            line,
            instantiates,
        })
    }

    fn parse_element(&mut self) -> Element {
        self.depth += 1;
        let element = self.parse_element_inner();
        self.depth -= 1;
        element
    }

    fn parse_element_inner(&mut self) -> Element {
        if self.depth > DEFAULT_MAX_SYNTAX_NESTING {
            if let Some(t) = self.peek().cloned() {
                self.sink.at(&t, "nesting is too deep".into());
            }
            return Element::default();
        }

        let Some(t) = self.peek().cloned() else {
            self.sink
                .push(0, 0, "expected an element, found end of source".into());
            return Element::default();
        };

        // `for item[, index] in path key path { … }`
        if t.is_kw("for") {
            self.at += 1;
            let mut binders = Vec::new();
            if let Some(b) = self.ident() {
                binders.push(b);
            }
            if self.eat_punct(",") {
                if let Some(b) = self.ident() {
                    binders.push(b);
                }
            }
            if !self.peek().is_some_and(|t| t.is_kw("in")) {
                if let Some(t) = self.peek().cloned() {
                    self.sink
                        .at(&t, "expected `in` after the loop binder".into());
                }
            } else {
                self.at += 1;
            }
            let collection = self.dotted_name().unwrap_or_default();
            let key_path = if self.peek().is_some_and(|t| t.is_kw("key")) {
                self.at += 1;
                self.dotted_name()
            } else {
                if let Some(t) = self.peek().cloned() {
                    self.sink.at(
                        &t,
                        "a loop must declare `key`: identity may not come from position".into(),
                    );
                }
                None
            };
            let mut element = Element {
                name: "for".into(),
                binders,
                line: t.line,
                column: t.column,
                key_path,
                ..Default::default()
            };
            element.args.push(Arg {
                name: "in".into(),
                value: Operand::Path(collection),
                line: t.line,
                column: t.column,
            });
            element.children = self.parse_block();
            return element;
        }

        // `when predicate { … }`
        if t.is_kw("when") {
            self.at += 1;
            let left = self.dotted_name().unwrap_or_default();
            // A comparison, or a bare path when the state is bool-shaped —
            // `when expanded { … }`. Neither form computes.
            let mut cmp = None;
            let mut rhs = None;
            if let Some(op) = self.peek().cloned() {
                if op.kind == Kind::Cmp {
                    self.at += 1;
                    cmp = Some(op.text.clone());
                    rhs = Some(self.parse_operand());
                }
            }
            let mut element = Element {
                name: "when".into(),
                line: t.line,
                column: t.column,
                cmp,
                rhs,
                ..Default::default()
            };
            element.args.push(Arg {
                name: "predicate".into(),
                value: Operand::Path(left),
                line: t.line,
                column: t.column,
            });
            element.children = self.parse_block();
            return element;
        }

        // `into name { … }` — a call site directing children at a named slot.
        if t.is_kw("into") {
            self.at += 1;
            let mut element = Element {
                name: "into".into(),
                line: t.line,
                column: t.column,
                ..Default::default()
            };
            if let Some(slot_name) = self.ident() {
                element.binders.push(slot_name);
            }
            element.children = self.parse_block();
            return element;
        }

        // `slot` or `slot name` — §5.5. The anonymous form is the common case;
        // a name lets a component place two groups of children.
        if t.is_kw("slot") {
            self.at += 1;
            let mut element = Element {
                name: "slot".into(),
                line: t.line,
                column: t.column,
                ..Default::default()
            };
            if self.peek().is_some_and(|n| n.kind == Kind::Ident) {
                if let Some(slot_name) = self.ident() {
                    element.binders.push(slot_name);
                }
            }
            return element;
        }

        let Some(name_tok) = self.next() else {
            return Element::default();
        };
        if name_tok.kind != Kind::Ident {
            self.sink.at(
                &name_tok,
                format!("expected an element, found {:?}", name_tok.text),
            );
            return Element::default();
        }

        // A lowercase name with no arguments references another view record.
        let is_constructor = name_tok
            .text
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase());

        let mut element = Element {
            name: name_tok.text.clone(),
            line: name_tok.line,
            column: name_tok.column,
            is_reference: !is_constructor,
            ..Default::default()
        };

        if self.peek().is_some_and(|t| t.is_punct("(")) {
            self.at += 1;
            element.args = self.parse_args();
        }
        if self.peek().is_some_and(|t| t.is_punct("{")) {
            element.children = self.parse_block();
        }
        element
    }

    fn parse_block(&mut self) -> Vec<Element> {
        let mut out = Vec::new();
        if !self.expect_punct("{") {
            return out;
        }
        let mut guard = 0usize;
        while let Some(t) = self.peek().cloned() {
            guard += 1;
            if guard > MAX_UI_L0_DECLARATIONS {
                self.sink.at(&t, "too many elements in one block".into());
                break;
            }
            if t.is_punct("}") {
                break;
            }
            let before = self.at;
            out.push(self.parse_element());
            if self.at == before {
                self.at += 1;
            }
        }
        self.expect_punct("}");
        out
    }

    fn parse_args(&mut self) -> Vec<Arg> {
        let mut out = Vec::new();
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > 64 {
                break;
            }
            if self.peek().is_none() || self.peek().is_some_and(|t| t.is_punct(")")) {
                break;
            }
            let Some(name_tok) = self.peek().cloned() else {
                break;
            };
            let Some(name) = self.ident() else {
                self.at += 1;
                continue;
            };
            if !self.expect_punct(":") {
                break;
            }
            let value = self.parse_operand();
            out.push(Arg {
                name,
                value,
                line: name_tok.line,
                column: name_tok.column,
            });
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct(")");
        out
    }

    fn parse_operand(&mut self) -> Operand {
        let Some(t) = self.peek().cloned() else {
            return Operand::Str(String::new());
        };
        match t.kind {
            Kind::Token => {
                self.at += 1;
                Operand::Token(t.text)
            }
            Kind::Str => {
                self.at += 1;
                Operand::Str(t.text.clone())
            }
            Kind::Num => {
                self.at += 1;
                Operand::Num(t.text.parse().unwrap_or(0.0))
            }
            Kind::Ident => {
                let path = self.dotted_name().unwrap_or_default();
                // A comparison in argument position is a predicate operand.
                if let Some(op) = self.peek().cloned() {
                    if op.kind == Kind::Cmp {
                        if self.depth > DEFAULT_MAX_SYNTAX_NESTING {
                            self.sink.at(&op, "nesting is too deep".into());
                            return Operand::Path(path);
                        }
                        self.at += 1;
                        self.depth += 1;
                        let rhs = self.parse_operand();
                        self.depth -= 1;
                        return Operand::Predicate {
                            path,
                            cmp: op.text,
                            rhs: Box::new(rhs),
                        };
                    }
                }
                Operand::Path(path)
            }
            _ => {
                self.at += 1;
                Operand::Str(String::new())
            }
        }
    }
}

/// Split a call site's children by the slot each targets.
///
/// `into header { … }` groups the elements that follow into the named slot;
/// anything outside a group goes to the anonymous one. Keeping the default
/// unnamed means a component with one slot needs no ceremony.
/// The slot names a component declares. An anonymous `slot` is `""`.
/// Resolve an operand in a comparison's right-hand position.
/// Follow a dotted path into injected data. Used for a path-valued `initial`,
/// which resolves against what the host supplied rather than at parse time.
/// A literal operand's value.
fn literal_of(operand: &Operand) -> crate::JsonValue {
    match operand {
        Operand::Str(t) => crate::JsonValue::String(t.clone()),
        Operand::Num(n) => crate::JsonValue::from(*n),
        Operand::Token(t) => crate::JsonValue::String(t.trim_start_matches('.').to_string()),
        Operand::Path(p) | Operand::Predicate { path: p, .. } => {
            crate::JsonValue::String(p.clone())
        }
    }
}

fn data_path(data: &crate::JsonValue, path: &str) -> Option<crate::JsonValue> {
    let mut current = data.clone();
    for segment in path.split('.') {
        current = match segment.parse::<usize>() {
            Ok(i) => current.get(i)?.clone(),
            Err(_) => current.get(segment)?.clone(),
        };
    }
    Some(current)
}

fn scope_value(scope: &ValueScope, operand: &Operand) -> Option<crate::JsonValue> {
    match operand {
        Operand::Path(p) => scope.lookup(p),
        Operand::Token(t) => Some(crate::JsonValue::String(
            t.trim_start_matches('.').to_string(),
        )),
        Operand::Str(s) => Some(crate::JsonValue::String(s.clone())),
        Operand::Num(n) => Some(crate::JsonValue::from(*n)),
        // A nested comparison is not in the grammar.
        Operand::Predicate { .. } => None,
    }
}

/// Compare two resolved values. Shared by guards (`when a == b`) and by
/// predicate arguments (`active: range == .d1`), so the two cannot drift.
fn compare(left: Option<crate::JsonValue>, cmp: &str, right: Option<crate::JsonValue>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        // An operand that did not resolve cannot be compared. `!=` used to
        // return true here, so a guard against an UNDECLARED name took its
        // branch; an unresolvable comparison is now simply false.
        return false;
    };
    match cmp {
        "==" => left == right,
        "!=" => left != right,
        _ => match (left.as_f64(), right.as_f64()) {
            (Some(a), Some(b)) => match cmp {
                "<" => a < b,
                "<=" => a <= b,
                ">" => a > b,
                ">=" => a >= b,
                _ => false,
            },
            _ => false,
        },
    }
}

fn slot_names(component: &Component) -> Vec<String> {
    fn walk(elements: &[Element], out: &mut Vec<String>) {
        for e in elements {
            if e.name == "slot" {
                out.push(e.binders.first().cloned().unwrap_or_default());
            }
            walk(&e.children, out);
        }
    }
    let mut out = Vec::new();
    walk(std::slice::from_ref(&component.body), &mut out);
    out
}

/// Path segments for a list of siblings, stable against insertion.
///
/// Profile §5.1 identifies an instance by its *instantiation site*, not its
/// position, "so that identity survives a ledger edit that inserts an element
/// above". A raw child index does not: inserting a `Rule()` renumbers every
/// sibling after it, and each one loses its local state — which is React's rule,
/// and precisely the rule §5.1 says L0 does not use.
///
/// Numbering within each element NAME fixes that. Two sibling `Row`s are still
/// distinct (`Row#0`, `Row#1`), so keys stay unique for hit-testing, but a
/// `Rule()` inserted above a `Toggly` leaves `Toggly#0` untouched.
fn sibling_segments(children: &[Element]) -> Vec<(String, &Element)> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    children
        .iter()
        .map(|child| {
            let n = seen.entry(child.name.as_str()).or_insert(0);
            let segment = format!("{}#{}", child.name, *n);
            *n += 1;
            (segment, child)
        })
        .collect()
}

fn split_slots(children: &[Element]) -> Vec<(String, Vec<Element>)> {
    let mut groups: Vec<(String, Vec<Element>)> = vec![(String::new(), Vec::new())];
    for child in children {
        if child.name == "into" {
            let name = child.binders.first().cloned().unwrap_or_default();
            groups.push((name, child.children.clone()));
        } else {
            groups[0].1.push(child.clone());
        }
    }
    groups
}

fn collect_constructors(e: &Element, out: &mut Vec<String>) {
    if !e.name.is_empty() && e.name != "for" && e.name != "when" && e.name != "slot" {
        out.push(e.name.clone());
    }
    for c in &e.children {
        collect_constructors(c, out);
    }
}

// ─────────────────────────────────────────────────────────────────────── catalog ──

/// The argument contract. Mirrors `docs/ui-l0-constructors.toml`, which is the
/// human-readable spec; a test asserts the two agree.
pub mod catalog {
    /// What an argument admits.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ArgKind {
        /// One of a closed token set, written with a leading dot.
        Token(&'static [&'static str]),
        /// A token from the set, or a bound path.
        TokenOrPath(&'static [&'static str]),
        /// A bound path only.
        Path,
        /// A literal or a path.
        Any,
        /// A declared event name.
        Event,
        /// A number literal or a path; also admits a width token. Layout, not
        /// data — `gap: 6` and `cols: 2` are structure the card chooses.
        Number,
        /// A bound path that renders a FACT. Distinct from `Number` because
        /// `gap: 6` is layout the card is entitled to choose and
        /// `value: 41.2` is a measurement it is not (§4). Without the
        /// distinction the no-facts rule cannot be stated in the catalog at
        /// all, so both were `Number` and neither was checked.
        Data,
        /// Text literal or path.
        Text,
        /// A predicate or a bool path.
        Bool,
    }

    pub const UNIT: &[&str] = &[
        "c", "f", "pct", "speed", "pressure", "index", "distance", "money",
    ];
    pub const FORMAT: &[&str] = &[
        "money",
        "signed_money",
        "signed_pct",
        "compact",
        "ratio",
        "time",
        "date",
    ];
    pub const WIDTH: &[&str] = &["fill", "fit", "day", "rank", "temp"];
    pub const ALIGN: &[&str] = &["start", "center", "end", "baseline"];
    pub const PAD: &[&str] = &["page", "tight", "none"];
    pub const ICON_SIZE: &[&str] = &["hero", "row", "tile"];

    pub type Args = &'static [(&'static str, ArgKind)];

    use ArgKind::*;

    /// Every constructor L0 admits, with the arguments each accepts.
    pub const CONSTRUCTORS: &[(&str, Args)] = &[
        ("Surface", &[("pad", Token(PAD))]),
        ("Photo", &[("src", Path), ("pad", Token(PAD))]),
        ("Panel", &[]),
        ("Card", &[("on_tap", Event), ("value", Any)]),
        ("Col", &[("align", Token(ALIGN)), ("gap", Number)]),
        (
            "Row",
            &[
                ("align", Token(ALIGN)),
                ("gap", Number),
                ("on_tap", Event),
                ("value", Any),
            ],
        ),
        ("Grid", &[("cols", Number)]),
        ("Rule", &[]),
        (
            "TextHero",
            &[
                ("text", Text),
                ("value", Data),
                ("unit", TokenOrPath(UNIT)),
                ("format", Token(FORMAT)),
                ("on_tap", Event),
            ],
        ),
        (
            "TextTitle",
            &[("text", Text), ("width", TokenOrPath(WIDTH))],
        ),
        ("TextBody", &[("text", Text), ("width", TokenOrPath(WIDTH))]),
        (
            "TextStat",
            &[("value", Data), ("format", Token(FORMAT)), ("tint", Path)],
        ),
        ("TextRow", &[("text", Text), ("width", TokenOrPath(WIDTH))]),
        (
            "TextCaption",
            &[
                ("text", Text),
                ("value", Data),
                ("glyph", Text),
                ("suffix", Text),
                ("unit", TokenOrPath(UNIT)),
                ("width", TokenOrPath(WIDTH)),
            ],
        ),
        (
            "TextValue",
            &[
                ("value", Data),
                ("unit", TokenOrPath(UNIT)),
                ("format", Token(FORMAT)),
                ("tint", Path),
            ],
        ),
        (
            "Tile",
            &[
                ("label", Text),
                ("value", Data),
                ("unit", TokenOrPath(UNIT)),
                ("format", Token(FORMAT)),
            ],
        ),
        (
            "Chip",
            &[
                ("text", Text),
                ("on_tap", Event),
                ("value", Any),
                ("active", Bool),
            ],
        ),
        ("WeatherIcon", &[("cond", Path), ("size", Token(ICON_SIZE))]),
        (
            "TempBar",
            &[("lo", Path), ("hi", Path), ("min", Path), ("max", Path)],
        ),
        ("SunArc", &[("rise", Path), ("set", Path), ("now", Path)]),
        ("MoonPhase", &[("phase", Path), ("illum", Path)]),
        (
            "AqiContour",
            &[("field", Path), ("index", Path), ("band", Path)],
        ),
        (
            "StockPlot",
            &[
                ("points", Path),
                ("min", Path),
                ("max", Path),
                ("tint", Path),
            ],
        ),
    ];

    pub fn lookup(name: &str) -> Option<Args> {
        CONSTRUCTORS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, a)| *a)
    }

    /// The CLOSED set of helpers a `source` may name, with the arguments each
    /// accepts. Mirrors `[sources.*]` in `docs/ui-l0-constructors.toml`.
    ///
    /// Without this the parser accepted any dotted name and `source_plan` handed
    /// it to the host as the function to resolve, so
    /// `source leak sys.shell(command: "cat /etc/passwd")` produced a clean,
    /// well-formed request. Confinement then rested on the host keeping its own
    /// allowlist — the policy boundary §1 claims L0 removes. A card must not be
    /// able to NAME a capability it was not granted.
    pub const SOURCES: &[(&str, &[&str])] = &[
        ("sys.geocode", &["name"]),
        (
            "sys.weather",
            &["lat", "lon", "days", "fields", "aggregate"],
        ),
        ("sys.daylight", &["lat", "lon"]),
        ("sys.airquality", &["lat", "lon"]),
        ("sys.moonphase", &["lat", "lon"]),
        ("sys.photo", &["query"]),
        ("sys.locale", &[]),
        ("sys.news", &["count", "offset", "fields"]),
        ("sys.news_item", &["id", "fields"]),
        ("sys.movers", &["count", "fields"]),
        ("sys.quote", &["ticker", "fields"]),
        (
            "sys.series",
            &["ticker", "range", "points", "fields", "aggregate"],
        ),
    ];

    pub fn source(name: &str) -> Option<&'static [&'static str]> {
        SOURCES.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
    }
}

// ──────────────────────────────────────────────────────────────────── validation ──

fn validate(card: &Card, sink: &mut Diagnostics) {
    // A card with no `view root` has nothing to render. Realization says so,
    // but the CHECKER accepted it — and the checker is what a host asks before
    // storing a record, so an unusable card could be appended and only fail
    // when someone tried to look at it.
    if !card.views.iter().any(|v| v.name == "root") {
        sink.push(
            1,
            1,
            "a card needs a `view root`: it is the entry point a host realizes".into(),
        );
    }

    // Components must not form a cycle — profile §6, condition 1. Without this
    // an L0 realization need not terminate.
    check_component_acyclicity(card, sink);
    validate_transitions(card, sink);
    validate_sources(card, sink);

    // Which props of which components end up rendering a fact (§4).
    let data_positions = data_position_params(card);

    let view_names: Vec<&str> = card.views.iter().map(|v| v.name.as_str()).collect();
    let component_names: Vec<&str> = card.components.iter().map(|c| c.name.as_str()).collect();

    for view in &card.views {
        let mut scope = Scope::card(card);
        walk(
            &data_positions,
            &view.body,
            &mut scope,
            &view_names,
            &component_names,
            card,
            sink,
        );
    }
    for component in &card.components {
        let mut scope = Scope::component(card, component);
        walk(
            &data_positions,
            &component.body,
            &mut scope,
            &view_names,
            &component_names,
            card,
            sink,
        );
    }
}

/// Profile §3: every transition names a declared state and uses a total form.
///
/// A component's events target its OWN state. Card state is written by card-scope
/// events, which a component reaches only by receiving one as a prop (§5.4) — so
/// the transition still runs in the scope that declared it.
fn validate_transitions(card: &Card, sink: &mut Diagnostics) {
    check_state_initials(&card.states, sink, 1);
    for component in &card.components {
        check_state_initials(&component.states, sink, component.line);
        for param in &component.params {
            let Some(default) = &param.default else {
                continue;
            };
            if !value_fits_shape(&param.shape, default) {
                sink.push(
                    component.line,
                    1,
                    format!(
                        "{}.{} declares {} but a default of {default}",
                        component.name,
                        param.name,
                        shape_name(&param.shape)
                    ),
                );
            }
        }
    }

    // A path-valued `initial` is a rendering position by proxy: whatever it
    // names ends up on screen. Checking only direct `copy.x` references let
    // model-copy through by way of a state.
    let card_scope = Scope::card(card);
    for state in &card.states {
        if let Some(path) = &state.initial_path {
            check_path(path, &card_scope, 1, 1, sink);
        }
    }
    for component in &card.components {
        let scope = Scope::component(card, component);
        for state in &component.states {
            if let Some(path) = &state.initial_path {
                check_path(path, &scope, component.line, 1, sink);
            }
        }
    }

    // `set(path)` READS. Its operand must therefore be a readable name, which
    // an event is not: `set(some_event)` parses, resolves to nothing and writes
    // garbage into the cell.
    let mut card_readable: Vec<String> = card.sources.iter().map(|s| root_of(&s.name)).collect();
    card_readable.extend(card.states.iter().map(|s| root_of(&s.path)));
    card_readable.push("copy".into());
    let card_events: Vec<String> = card.events.iter().map(|e| e.name.clone()).collect();

    check_event_batch(
        &card.events,
        &card.states,
        &card_readable,
        &card_events,
        &card.copies,
        sink,
    );
    for component in &card.components {
        let mut readable: Vec<String> = card.sources.iter().map(|s| root_of(&s.name)).collect();
        readable.push("copy".into());
        // An EVENT prop is not readable — `set(out)` where `out` is an event
        // was accepted, because a param's shape was unknown so none could be
        // excluded. Now only value-shaped props are readable, and event-shaped
        // ones join the event names below.
        readable.extend(
            component
                .params
                .iter()
                .filter(|p| !matches!(p.shape, Shape::Event))
                .map(|p| p.name.clone()),
        );
        readable.extend(component.states.iter().map(|s| root_of(&s.path)));
        let mut events: Vec<String> = component.events.iter().map(|e| e.name.clone()).collect();
        events.extend(
            component
                .params
                .iter()
                .filter(|p| matches!(p.shape, Shape::Event))
                .map(|p| p.name.clone()),
        );
        check_event_batch(
            &component.events,
            &component.states,
            &readable,
            &events,
            &card.copies,
            sink,
        );
    }
}

/// A shape's canonical spelling — what a card writes. Distinct from
/// `shape_name`, which is prose for a diagnostic: a test comparing shapes should
/// not be coupled to how an error message reads.
fn shape_spelling(shape: &Shape) -> &'static str {
    match shape {
        Shape::Text => "text",
        Shape::Number => "number",
        Shape::Bool => "bool",
        Shape::Enum(_) => "enum",
        Shape::Record => "record",
        Shape::Collection => "collection",
        Shape::Event => "event",
        Shape::Other => "?",
    }
}

/// A shape's name, for diagnostics.
fn shape_name(shape: &Shape) -> &'static str {
    match shape {
        Shape::Text => "text",
        Shape::Number => "a number",
        Shape::Bool => "bool",
        Shape::Enum(_) => "an enum",
        Shape::Record => "a record",
        Shape::Collection => "a collection",
        Shape::Event => "an event",
        Shape::Other => "of an unnamed shape",
    }
}

/// Profile §2: a declared `initial` must fit the shape it is declared with.
/// Neither was checked against the other, so `shape: number, initial: "oops"`
/// mounted a string into a numeric cell before any event ran.
/// Whether a literal agrees with the shape it was declared alongside. Shared by
/// state initials and parameter defaults so the two cannot drift.
fn value_fits_shape(shape: &Shape, value: &crate::JsonValue) -> bool {
    match (shape, value) {
        (Shape::Text, crate::JsonValue::String(_)) => true,
        (Shape::Number, v) => v.is_number(),
        (Shape::Bool, crate::JsonValue::Bool(_)) => true,
        (Shape::Enum(members), crate::JsonValue::String(s)) => members.iter().any(|m| m == s),
        // A shape that names no literal form, or one we could not classify.
        (Shape::Record | Shape::Collection | Shape::Event | Shape::Other, _) => true,
        _ => false,
    }
}

fn check_state_initials(states: &[StateDecl], sink: &mut Diagnostics, line: usize) {
    for state in states {
        let Some(initial) = &state.initial else {
            continue;
        };
        let fits = value_fits_shape(&state.shape, initial);
        if !fits {
            sink.push(
                line,
                1,
                format!(
                    "state {:?} declares {} but an initial of {initial}",
                    state.path,
                    shape_name(&state.shape)
                ),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_event_batch(
    events: &[EventDecl],
    states: &[StateDecl],
    readable: &[String],
    event_names: &[String],
    copies: &[CopyDecl],
    sink: &mut Diagnostics,
) {
    for event in events {
        for transition in &event.transitions {
            let Some(state) = states.iter().find(|s| s.path == transition.target) else {
                sink.push(
                    transition.line,
                    transition.column,
                    format!(
                        "{:?} is not a state declared in this scope",
                        transition.target
                    ),
                );
                continue;
            };

            match &transition.form {
                Form::NotTotal(text) => sink.push(
                    transition.line,
                    transition.column,
                    format!(
                        "{text:?} is not a total transition form; L0 admits set(…), toggle, cycle(…) and clear"
                    ),
                ),
                Form::Toggle if state.shape != Shape::Bool => sink.push(
                    transition.line,
                    transition.column,
                    format!(
                        "`toggle` needs a bool state, but {:?} is not bool",
                        transition.target
                    ),
                ),
                Form::Set(SetSource::Num(_)) if !matches!(state.shape, Shape::Number | Shape::Other) => {
                    sink.push(
                        transition.line,
                        transition.column,
                        format!(
                            "`set` writes a number, but {:?} is {}",
                            transition.target,
                            shape_name(&state.shape)
                        ),
                    );
                }
                Form::Set(SetSource::Bool(_)) if !matches!(state.shape, Shape::Bool | Shape::Other) => {
                    sink.push(
                        transition.line,
                        transition.column,
                        format!(
                            "`set` writes a bool, but {:?} is {}",
                            transition.target,
                            shape_name(&state.shape)
                        ),
                    );
                }
                Form::Set(SetSource::Token(t)) => {
                    if !matches!(state.shape, Shape::Enum(_) | Shape::Other) {
                        sink.push(
                            transition.line,
                            transition.column,
                            format!(
                                "`set` writes the token .{t}, but {:?} is {} — only an enum \
                                 has members",
                                transition.target,
                                shape_name(&state.shape)
                            ),
                        );
                    }
                    if let Shape::Enum(members) = &state.shape {
                        if !members.iter().any(|m| m == t) {
                            sink.push(
                                transition.line,
                                transition.column,
                                format!(
                                    ".{t} is not a member of {:?}'s enum; it accepts {}",
                                    transition.target,
                                    members
                                        .iter()
                                        .map(|m| format!(".{m}"))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            );
                        }
                    }
                }
                Form::Set(SetSource::Text(_))
                    if !matches!(state.shape, Shape::Text | Shape::Other) =>
                {
                    sink.push(
                        transition.line,
                        transition.column,
                        format!(
                            "`set` writes text, but {:?} is {}",
                            transition.target,
                            shape_name(&state.shape)
                        ),
                    );
                }
                Form::Set(SetSource::Path(path)) => {
                    // §4 travels with the value: writing model-authored text
                    // into state renders it just as surely as naming it in a
                    // view, one step later.
                    if let Some(name) = path.strip_prefix("copy.") {
                        match copies.iter().find(|c| c.name == name) {
                            Some(decl) if decl.provenance == Provenance::ModelCopy => {
                                sink.push(
                                    transition.line,
                                    transition.column,
                                    format!(
                                        "copy.{name} is `model-copy` and may not be written \
                                         into state; only `vocabulary` and `user-copy` may \
                                         (profile §4)"
                                    ),
                                );
                            }
                            None => sink.push(
                                transition.line,
                                transition.column,
                                format!("copy.{name} is not declared"),
                            ),
                            _ => {}
                        }
                    }
                    let root = root_of(path);
                    if event_names.contains(&root) {
                        sink.push(
                            transition.line,
                            transition.column,
                            format!(
                                "{root:?} is an event, not a value; `set` reads a name and \
                                 cannot trigger one (profile §6.4)"
                            ),
                        );
                    } else if !readable.contains(&root) {
                        sink.push(
                            transition.line,
                            transition.column,
                            format!("{root:?} is not a readable name"),
                        );
                    }
                }
                Form::Set(_) => {}
                Form::Cycle => match &state.shape {
                    Shape::Enum(members) => {
                        for member in &transition.tokens {
                            if let Some(bare) = member.strip_prefix('!') {
                                sink.push(
                                    transition.line,
                                    transition.column,
                                    format!(
                                        "`cycle` takes tokens; {bare:?} is a path — enum members carry a leading dot"
                                    ),
                                );
                            } else if !members.contains(member) {
                                sink.push(
                                    transition.line,
                                    transition.column,
                                    format!(
                                        ".{member} is not a member of {:?}'s enum",
                                        transition.target
                                    ),
                                );
                            }
                        }
                        // Advancing uses the ENUM's declaration order (§3), so a
                        // `cycle` listing a different one would run an order the
                        // card does not say. Rejected rather than reordered:
                        // what is written must be what happens.
                        if !transition.tokens.is_empty()
                            && transition.tokens.iter().all(|t| members.contains(t))
                            && transition.tokens != *members
                        {
                            sink.push(
                                transition.line,
                                transition.column,
                                format!(
                                    "`cycle` advances in declaration order ({}), but lists {} — \
                                     reorder the enum or the cycle so they agree (profile §3)",
                                    members.join(", "),
                                    transition.tokens.join(", ")
                                ),
                            );
                        }
                    }
                    _ => sink.push(
                        transition.line,
                        transition.column,
                        format!("`cycle` needs an enum state, but {:?} is not", transition.target),
                    ),
                },
                _ => {}
            }
        }
    }
}

/// Profile §4: a `source` may only name a catalogued capability.
///
/// This is the difference between "the host refuses to run it" and "the card
/// cannot ask". `source leak sys.shell(command: "cat /etc/passwd")` previously
/// parsed clean and reached the host as a well-formed request; confinement then
/// depended on the host keeping an allowlist that the profile claims to make
/// unnecessary.
fn validate_sources(card: &Card, sink: &mut Diagnostics) {
    // A source may read card state, other sources and copy — the same names a
    // view may, minus anything a component introduces.
    let scope = &Scope::card(card);
    for source in &card.sources {
        let Some(accepted) = catalog::source(&source.helper) else {
            let mut known: Vec<&str> = catalog::SOURCES.iter().map(|(n, _)| *n).collect();
            known.sort();
            sink.push(
                source.line,
                1,
                format!(
                    "{:?} is not a source capability L0 admits; a card may only name a \
                     catalogued helper (profile §4). Known: {}",
                    source.helper,
                    known.join(", ")
                ),
            );
            continue;
        };
        for (name, arg) in &source.args {
            // The VALUE is a binding too. Checking only the argument's name let
            // a source carry any path to the host — the helper was constrained
            // and its arguments were not.
            if let SourceArg::Path(path) = arg {
                // A source addresses card state with an explicit `state.`
                // prefix — `sys.geocode(name: state.city)` — which the view
                // scope does not carry, since a view names its state directly.
                match path.strip_prefix("state.") {
                    Some(rest) => {
                        let root = root_of(rest);
                        if !card.states.iter().any(|st| root_of(&st.path) == root) {
                            sink.push(source.line, 1, format!("{root:?} is not a declared state"));
                        }
                    }
                    None => check_path(path, scope, source.line, 1, sink),
                }
            }
            if !accepted.contains(&name.as_str()) {
                sink.push(
                    source.line,
                    1,
                    format!(
                        "{:?} does not accept an argument named {name:?}; it accepts: {}",
                        source.helper,
                        if accepted.is_empty() {
                            "(none)".to_string()
                        } else {
                            accepted.join(", ")
                        }
                    ),
                );
            }
        }
    }
}

/// Profile §6, condition 1: the declaration graph must be acyclic.
///
/// This walks COMPONENTS AND VIEWS as one graph. Walking components alone
/// missed two shapes that both recurse forever at realization:
///
///     view a b        view b a          (two views referencing each other)
///     component A { view v }  view v { A() }   (a component via a view)
///
/// Both passed the old check and recursed until the node cap tripped, which is
/// a resource bound catching a structural error — and only because the caller
/// happened to set one.
fn check_component_acyclicity(card: &Card, sink: &mut Diagnostics) {
    // name -> (what it references, line)
    let mut edges: Vec<(String, Vec<String>, usize)> = Vec::new();
    for component in &card.components {
        let mut refs = component.instantiates.clone();
        let mut views = Vec::new();
        collect_references(&component.body, &mut views);
        refs.extend(views);
        edges.push((component.name.clone(), refs, component.line));
    }
    for view in &card.views {
        let mut refs = Vec::new();
        collect_constructors(&view.body, &mut refs);
        let mut views = Vec::new();
        collect_references(&view.body, &mut views);
        refs.extend(views);
        edges.push((view.name.clone(), refs, 1));
    }

    // Keep only edges that name a NODE of this graph. `collect_constructors`
    // yields every catalog constructor, so an unfiltered edge list is
    // proportional to the card's size rather than to its component graph — and
    // the traversal budget could then be exhausted by padding before the walk
    // reached a real cycle. Filtering makes the budget a backstop rather than
    // the thing that decides the answer.
    let node_names: Vec<String> = edges.iter().map(|(n, _, _)| n.clone()).collect();
    for (_, refs, _) in edges.iter_mut() {
        refs.retain(|r| node_names.contains(r));
        refs.dedup();
    }

    let referenced = |name: &str| -> Option<&Vec<String>> {
        edges.iter().find(|(n, _, _)| n == name).map(|(_, r, _)| r)
    };

    for (name, _, line) in &edges {
        let mut seen: Vec<&str> = vec![name.as_str()];
        let mut queue: Vec<&str> = referenced(name)
            .map(|r| r.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        let mut steps = 0usize;
        while let Some(next) = queue.pop() {
            steps += 1;
            if steps > MAX_UI_L0_DECLARATIONS {
                break;
            }
            if next == name.as_str() {
                sink.push(
                    *line,
                    1,
                    format!(
                        "{name:?} is recursive; L0 requires an acyclic declaration graph \
                         (profile §6.1)"
                    ),
                );
                break;
            }
            if seen.contains(&next) {
                continue;
            }
            seen.push(next);
            if let Some(more) = referenced(next) {
                queue.extend(more.iter().map(|s| s.as_str()));
            }
        }
    }
}

/// Names a body SPLICES — `view` references, as distinct from component calls.
fn collect_references(element: &Element, out: &mut Vec<String>) {
    if element.is_reference {
        out.push(element.name.clone());
    }
    for child in &element.children {
        collect_references(child, out);
    }
}

/// Names visible to one record. Profile §5.3: a component sees its props, its own
/// state and events, and card-scope `source` and `copy` — but never card `state`.
struct Scope {
    roots: Vec<String>,
    /// Roots that name a source. Only these carry a `$state` lifecycle.
    source_roots: Vec<String>,
    /// Declared copy, so a rendering position can refuse model-authored text.
    copies: Vec<CopyDecl>,
    events: Vec<String>,
    /// Card state, tracked separately so reading it from a component is a
    /// specific diagnostic rather than "unknown name".
    forbidden_state: Vec<String>,
}

impl Scope {
    fn card(card: &Card) -> Self {
        let mut roots: Vec<String> = Vec::new();
        // The FULL declared name. Storing the root meant `source env.locale`
        // authorised `env.theme.secret` — an undeclared dependency the card
        // then renders, which §4 says is a defect by construction.
        roots.extend(card.sources.iter().map(|s| s.name.clone()));
        roots.extend(card.states.iter().map(|s| root_of(&s.path)));
        roots.push("copy".into());
        let events = card.events.iter().map(|e| e.name.clone()).collect();
        Self {
            roots,
            // The FULL declared name, not its root. `source env.locale` answers to
            // `env.locale.$state`; comparing against the root alone rejected that
            // while accepting `env.$state`, which names no source and has no
            // lifecycle to report.
            source_roots: card.sources.iter().map(|s| s.name.clone()).collect(),
            copies: card.copies.clone(),
            events,
            forbidden_state: Vec::new(),
        }
    }

    fn component(card: &Card, component: &Component) -> Self {
        let mut roots: Vec<String> = Vec::new();
        roots.extend(card.sources.iter().map(|s| s.name.clone()));
        roots.push("copy".into());
        roots.extend(component.params.iter().map(|p| p.name.clone()));
        roots.extend(component.states.iter().map(|s| root_of(&s.path)));
        let mut events: Vec<String> = component.events.iter().map(|e| e.name.clone()).collect();
        // Events arrive as props too — §5.4, outputs are event props. Only the
        // ones DECLARED as events: every param used to be added here, so a
        // `text` prop could be handed to `on_tap` and a runtime string that
        // happened to match an event name would route it.
        events.extend(
            component
                .params
                .iter()
                .filter(|p| matches!(p.shape, Shape::Event))
                .map(|p| p.name.clone()),
        );
        Self {
            roots,
            source_roots: card.sources.iter().map(|s| s.name.clone()).collect(),
            copies: card.copies.clone(),
            events,
            forbidden_state: card.states.iter().map(|s| root_of(&s.path)).collect(),
        }
    }

    /// Whether `path` is covered by a declared name.
    ///
    /// A declared name matches its own segments only: `env.locale` answers
    /// `env.locale.lang` and does NOT answer `env.theme.secret`. Comparing
    /// roots made every sibling of a dotted source readable.
    fn knows(&self, path: &str) -> bool {
        self.roots.iter().any(|r| {
            path == r
                || path
                    .strip_prefix(r.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
        })
    }
}

fn root_of<S: AsRef<str>>(path: S) -> String {
    path.as_ref()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}

#[allow(clippy::too_many_arguments)]
fn walk(
    data_positions: &BTreeMap<String, Vec<String>>,
    element: &Element,
    scope: &mut Scope,
    views: &[&str],
    components: &[&str],
    card: &Card,
    sink: &mut Diagnostics,
) {
    match element.name.as_str() {
        "" | "slot" | "into" => {}
        "for" => {
            // The key expression names the loop's own binder. Its first segment
            // was discarded unconditionally, so `key typo.id` behaved exactly
            // like `key item.id` — and §5.1's claim that changing a key moves
            // identity was false, because the name was never read.
            if let (Some(binder), Some(key)) = (element.binders.first(), &element.key_path) {
                let root = root_of(key);
                if root != *binder {
                    sink.push(
                        element.line,
                        element.column,
                        format!(
                            "the key names {root:?}, but this loop binds {binder:?} — a key \
                             must be a path into the item being iterated (profile §5.1)"
                        ),
                    );
                }
            }
            for binder in &element.binders {
                scope.roots.push(binder.clone());
            }
            if let Some(Arg {
                value: Operand::Path(p),
                line,
                column,
                ..
            }) = element.args.first()
            {
                check_path(p, scope, *line, *column, sink);
            }
        }
        "when" => {
            if let Some(Arg {
                value: Operand::Path(p),
                line,
                column,
                ..
            }) = element.args.first()
            {
                check_path(p, scope, *line, *column, sink);
                // The right operand is a binding too. Unchecked, an undeclared
                // name on the right resolved to nothing and the comparison
                // decided the branch on that absence.
                if let Some(Operand::Path(r)) = element.rhs.as_ref() {
                    check_path(r, scope, *line, *column, sink);
                }
            }
        }
        _ if element.is_reference => {
            if !views.contains(&element.name.as_str()) {
                sink.push(
                    element.line,
                    element.column,
                    format!("{:?} is not a declared view", element.name),
                );
            }
        }
        name => {
            let known = catalog::lookup(name);
            let is_component = components.contains(&name);
            if is_component {
                if let Some(component) = card.components.iter().find(|c| c.name == name) {
                    let offered = slot_names(component);
                    for child in &element.children {
                        if child.name != "into" {
                            continue;
                        }
                        let wanted = child.binders.first().cloned().unwrap_or_default();
                        if !offered.contains(&wanted) {
                            let list = if offered.iter().all(|s| s.is_empty()) {
                                "it has only an anonymous slot".to_string()
                            } else {
                                format!(
                                    "it offers: {}",
                                    offered
                                        .iter()
                                        .filter(|s| !s.is_empty())
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            };
                            sink.push(
                                child.line,
                                child.column,
                                format!(
                                    "{name:?} has no slot named {wanted:?}, so these children \
                                     would not render — {list} (profile §5.5)"
                                ),
                            );
                        }
                    }
                }
            }
            if known.is_none() && !is_component {
                sink.push(
                    element.line,
                    element.column,
                    format!("{name:?} is not an L0 constructor or a declared component"),
                );
            }
            for arg in &element.args {
                check_arg(
                    data_positions,
                    name,
                    arg,
                    known,
                    is_component,
                    scope,
                    card,
                    sink,
                );
            }
        }
    }

    for child in &element.children {
        walk(data_positions, child, scope, views, components, card, sink);
    }

    if element.name == "for" {
        for _ in &element.binders {
            scope.roots.pop();
        }
    }
}

/// Eight parameters because a card-level check needs card-level context:
/// the catalog entry, the scope, the card, and the data-position map are all
/// consulted per argument. Splitting them into a struct would move the noise
/// rather than remove it.
#[allow(clippy::too_many_arguments)]
fn check_arg(
    data_positions: &BTreeMap<String, Vec<String>>,
    ctor: &str,
    arg: &Arg,
    known: Option<catalog::Args>,
    is_component: bool,
    scope: &Scope,
    card: &Card,
    sink: &mut Diagnostics,
) {
    use catalog::ArgKind::*;

    // A component's own props are declared by the card, not the catalog.
    let kind = match known {
        Some(args) => match args.iter().find(|(n, _)| *n == arg.name) {
            Some((_, k)) => Some(*k),
            None => {
                sink.push(
                    arg.line,
                    arg.column,
                    format!("{ctor} has no argument {:?}", arg.name),
                );
                return;
            }
        },
        None if is_component => {
            // A component's props are declared by the CARD, not the catalog —
            // but they ARE declared, so an argument the component does not
            // accept is a typo, not a pass-through. It used to be accepted and
            // then silently dropped at realization, rendering an em dash.
            if let Some(component) = card.components.iter().find(|c| c.name == ctor) {
                match component.params.iter().find(|p| p.name == arg.name) {
                    Some(param) => {
                        check_prop(ctor, arg, param, scope, sink);
                        // §4 again, one level out: a literal handed to a prop
                        // that lands in a data position is the same authored
                        // fact, just laundered.
                        if matches!(arg.value, Operand::Str(_) | Operand::Num(_))
                            && data_positions
                                .get(ctor)
                                .is_some_and(|ps| ps.contains(&arg.name))
                        {
                            sink.push(
                                arg.line,
                                arg.column,
                                format!(
                                    "{ctor}.{} reaches a data position and must be bound; \
                                     a literal passed here is a model-authored fact \
                                     (profile §4)",
                                    arg.name
                                ),
                            );
                        }
                    }
                    None => {
                        let mut names: Vec<&str> =
                            component.params.iter().map(|p| p.name.as_str()).collect();
                        names.sort();
                        sink.push(
                            arg.line,
                            arg.column,
                            format!(
                                "{ctor} has no prop {:?}; it declares: {}",
                                arg.name,
                                if names.is_empty() {
                                    "(none)".to_string()
                                } else {
                                    names.join(", ")
                                }
                            ),
                        );
                    }
                }
            }
            None
        }
        None => return,
    };

    match (&arg.value, kind) {
        (Operand::Token(tok), Some(Token(set) | TokenOrPath(set))) => {
            let bare = tok.trim_start_matches('.');
            if !set.contains(&bare) {
                sink.push(
                    arg.line,
                    arg.column,
                    format!(
                        "{tok:?} is not a legal token for {ctor}.{}; expected one of {}",
                        arg.name,
                        set.iter()
                            .map(|t| format!(".{t}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
        }
        (Operand::Str(lit), Some(Path)) => {
            sink.push(
                arg.line,
                arg.column,
                format!(
                    "{ctor}.{} takes a bound path, not the literal {lit:?} — a literal in a \
                     data position is a model-authored fact (profile §4)",
                    arg.name
                ),
            );
        }
        (Operand::Num(lit), Some(Path)) => {
            sink.push(
                arg.line,
                arg.column,
                format!(
                    "{ctor}.{} takes a bound path, not the literal {lit} — a literal in a \
                     data position is a model-authored fact (profile §4)",
                    arg.name
                ),
            );
        }
        // A data position renders a fact and must be bound, not authored.
        (Operand::Token(t), Some(Data)) => {
            sink.push(
                arg.line,
                arg.column,
                format!(
                    "{ctor}.{} renders a value and must be bound; {t:?} is a token \
                     (profile §4)",
                    arg.name
                ),
            );
        }
        (Operand::Str(_) | Operand::Num(_), Some(Data)) => {
            let shown = match &arg.value {
                Operand::Num(n) => format!("{n}"),
                Operand::Str(t) => format!("{t:?}"),
                other => format!("{other:?}"),
            };
            sink.push(
                arg.line,
                arg.column,
                format!(
                    "{ctor}.{} renders a value and must be bound; {shown} is a literal the \
                     model authored (profile §4). Layout numbers like `gap:` may be literal; \
                     a measurement may not",
                    arg.name
                ),
            );
        }
        // A number position rendering a fact must be bound, not authored.
        // `TextHero(value: "34 mph")` is the case §4 exists to make
        // unrepresentable: units concatenated onto a value the model invented.
        (Operand::Str(lit), Some(Number)) => {
            sink.push(
                arg.line,
                arg.column,
                format!(
                    "{ctor}.{} renders a value and must be bound; {lit:?} is a literal, and \
                     text in a number position is a fact the model authored (profile §4). \
                     Units belong in `unit:`, formatting in `format:`",
                    arg.name
                ),
            );
        }
        (Operand::Token(tok), Some(Path)) => {
            sink.push(
                arg.line,
                arg.column,
                format!(
                    "{ctor}.{} takes a bound path, not the token {tok:?}",
                    arg.name
                ),
            );
        }
        (Operand::Path(path), Some(Token(_))) => {
            // Enum members are tokens; a bare name here is a path and cannot
            // stand in for one.
            sink.push(
                arg.line,
                arg.column,
                format!(
                    "{ctor}.{} takes a token; {path:?} is a path — tokens carry a leading dot",
                    arg.name
                ),
            );
        }
        (Operand::Path(path), Some(Event)) if !scope.events.iter().any(|e| e == path) => {
            sink.push(
                arg.line,
                arg.column,
                format!("{path:?} is not a declared event"),
            );
        }
        (Operand::Path(_), Some(Event)) => {}
        // A component prop has no catalogued kind, and may legitimately receive an
        // event name — profile §5.4, outputs are event props. Accept either.
        (Operand::Path(path), None) if scope.events.iter().any(|e| e == path) => {}
        (Operand::Path(path), _) => check_path(path, scope, arg.line, arg.column, sink),
        (Operand::Predicate { path, rhs, .. }, _) => {
            check_path(path, scope, arg.line, arg.column, sink);
            // The right operand is a binding too. Leaving it unchecked is how
            // `when selected != absent` took its branch with `absent` undeclared.
            if let Operand::Path(r) = rhs.as_ref() {
                check_path(r, scope, arg.line, arg.column, sink);
            }
        }
        _ => {}
    }

    let _ = card;
}

/// Check an argument against the prop it fills — §5.2.
///
/// Only the cases a card can actually get wrong and that the shape decides:
/// an event where a value belongs, a value where an event belongs, and a token
/// outside a declared enum. Anything else is left alone, because a prop shape
/// is a contract about the DATA and most positions legitimately accept a path
/// whose type is not knowable until the host answers.
/// Which of a component's params end up in a DATA position — an argument the
/// catalog declares as `Data` or `Path`, i.e. one that renders a fact.
///
/// Without this, §4 is enforced only at the literal's immediate call site, and
/// laundering it through one prop defeats it entirely:
///
///     component Bar(n: number) { view TempBar(lo: n, hi: n, min: n, max: n) }
///     view root Bar(n: 41.2)
///
/// Computed to a fixpoint so passing the value through a chain of components is
/// no better a hiding place than passing it directly.
fn data_position_params(card: &Card) -> BTreeMap<String, Vec<String>> {
    let mut tainted: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // Bound by the number of PARAM HOPS a value can take, not by the component
    // count. A single component whose params chain a -> b -> c -> data needs
    // three rounds, which `N + 1` with N = 1 does not allow — and the loop must
    // still terminate if the graph turns out to be cyclic, since this runs
    // before acyclicity is proven.
    let rounds = card.components.len()
        + card
            .components
            .iter()
            .map(|c| c.params.len())
            .sum::<usize>()
        + 2;
    for _ in 0..rounds {
        let mut changed = false;
        for component in &card.components {
            let names: Vec<&str> = component.params.iter().map(|p| p.name.as_str()).collect();
            let mut found: Vec<String> = Vec::new();

            fn walk(
                element: &Element,
                names: &[&str],
                tainted: &BTreeMap<String, Vec<String>>,
                out: &mut Vec<String>,
            ) {
                let known = catalog::lookup(&element.name);
                for arg in &element.args {
                    let Operand::Path(path) = &arg.value else {
                        continue;
                    };
                    let root = root_of(path);
                    if !names.iter().any(|n| *n == root) {
                        continue;
                    }
                    // Directly in a data position...
                    let direct = known
                        .and_then(|a| a.iter().find(|(n, _)| *n == arg.name))
                        .is_some_and(|(_, k)| {
                            matches!(k, catalog::ArgKind::Data | catalog::ArgKind::Path)
                        });
                    // ...or handed to another component that puts it in one.
                    let onward = tainted
                        .get(&element.name)
                        .is_some_and(|ps| ps.contains(&arg.name));
                    if (direct || onward) && !out.contains(&root) {
                        out.push(root);
                    }
                }
                for child in &element.children {
                    walk(child, names, tainted, out);
                }
            }

            walk(&component.body, &names, &tainted, &mut found);
            let entry = tainted.entry(component.name.clone()).or_default();
            for name in found {
                if !entry.contains(&name) {
                    entry.push(name);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    tainted
}

fn check_prop(ctor: &str, arg: &Arg, param: &Param, scope: &Scope, sink: &mut Diagnostics) {
    let name = &param.name;
    match (&param.shape, &arg.value) {
        // An event prop must receive an event, and nothing else may.
        (Shape::Event, Operand::Path(p)) if !scope.events.iter().any(|e| e == p) => {
            sink.push(
                arg.line,
                arg.column,
                format!("{ctor}.{name} takes an event; {p:?} is not a declared event"),
            );
        }
        (Shape::Event, Operand::Str(_) | Operand::Num(_) | Operand::Token(_)) => {
            sink.push(
                arg.line,
                arg.column,
                format!("{ctor}.{name} takes an event, not a literal"),
            );
        }
        (shape, Operand::Path(p))
            if !matches!(shape, Shape::Event) && scope.events.iter().any(|e| e == p) =>
        {
            sink.push(
                arg.line,
                arg.column,
                format!(
                    "{ctor}.{name} takes a value, but {p:?} is an event — declare the prop \
                     as `{name}: event` to pass one (profile §5.4)"
                ),
            );
        }
        (Shape::Enum(members), Operand::Token(t)) => {
            let bare = t.trim_start_matches('.');
            if !members.iter().any(|m| m == bare) {
                sink.push(
                    arg.line,
                    arg.column,
                    format!(
                        "{t:?} is not a member of {ctor}.{name}; it accepts {}",
                        members
                            .iter()
                            .map(|m| format!(".{m}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
        }
        _ => {}
    }
}

fn check_path(path: &str, scope: &Scope, line: usize, column: usize, sink: &mut Diagnostics) {
    if path.is_empty() {
        return;
    }

    // `copy.x` where `x` is model-authored. §4's whole point: a model-written
    // string on screen is indistinguishable from a fact the model invented, so
    // the class exists to be refused rather than to be recorded.
    if let Some(name) = path.strip_prefix("copy.") {
        if let Some(decl) = scope.copies.iter().find(|c| c.name == name) {
            if decl.provenance == Provenance::ModelCopy {
                sink.push(
                    line,
                    column,
                    format!(
                        "copy.{name} is `model-copy` and may not be rendered; only \
                         `vocabulary` and `user-copy` may (profile §4)"
                    ),
                );
            }
        } else {
            sink.push(line, column, format!("copy.{name} is not declared"));
        }
        return;
    }

    // A `[inner]` segment holds a path of its own. Checking it here is what
    // makes `stories[typo].title` a diagnostic rather than a silent em dash.
    for segment in path.split('.') {
        if let Some(inner) = segment.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            check_path(inner, scope, line, column, sink);
        }
    }
    let root = root_of(path);

    // `<source>.$state` is the fetch lifecycle (§5.9). It is meaningful only for
    // something that is fetched — a local state has no pending or failed.
    if let Some(owner) = path.strip_suffix(".$state") {
        if scope.source_roots.iter().any(|r| r == owner) {
            return;
        }
        sink.push(
            line,
            column,
            format!("`$state` is only available on a source; {owner:?} is not one (profile §5.9)"),
        );
        return;
    }
    if path.contains("$state") {
        sink.push(
            line,
            column,
            "`$state` must be the last segment of a path (profile §5.9)".to_string(),
        );
        return;
    }

    // The whole path, so a declared name can be matched by its own segments.
    if scope.knows(path) {
        return;
    }
    if scope.forbidden_state.contains(&root) {
        sink.push(
            line,
            column,
            format!(
                "a component may not read card state {root:?}; pass it as a prop (profile §5.3)"
            ),
        );
        return;
    }
    sink.push(line, column, format!("{root:?} is not a declared name"));
}

// ─────────────────────────────────────────────────────────────────── realization ──

/// A realized node. Renderer-neutral: a backend maps `kind` to its own widget.
#[derive(Clone, Debug, PartialEq)]
pub struct UiNode {
    pub kind: String,
    /// Instance identity — the declaration path plus every enclosing loop key.
    /// Never derived from position, so an edit that inserts a sibling above does
    /// not rebind this node's state (profile §5.1).
    pub key: String,
    pub args: Vec<(String, NodeValue)>,
    pub children: Vec<UiNode>,
}

/// A resolved argument value.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeValue {
    Text(String),
    Number(f64),
    Bool(bool),
    /// A `.token`, carried through with its meaning intact for the backend.
    Token(String),
    /// A declared event name for the runtime to bind at dispatch — never a
    /// string constructed by the card (profile §3.1).
    Event(String),
    /// A binding that resolved to nothing. Surfaced rather than defaulted: a
    /// silently empty field is how a card renders an em dash and looks fine.
    Missing,
    /// A source's lifecycle, readable as `now.$state` — §5.9. A card can show a
    /// spinner or an error rather than an em dash, and the distinction between
    /// "not yet" and "failed" is the runtime's to report, not the card's to
    /// guess from an absent value.
    Status(SourceStatus),
}

/// Where a source is in its lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceStatus {
    /// Never resolved.
    Pending,
    /// Resolved, and current.
    Ready,
    /// Resolved, but the value is past its freshness budget.
    Stale,
    /// The fetch failed. The card may say so; it may not say why.
    Failed,
}

impl SourceStatus {
    pub fn as_token(self) -> &'static str {
        match self {
            SourceStatus::Pending => "pending",
            SourceStatus::Ready => "ready",
            SourceStatus::Stale => "stale",
            SourceStatus::Failed => "failed",
        }
    }
}

/// Traversal bounds. L0 has no expressions, so there is nothing to evaluate and
/// no instruction budget to set — these bound the walk instead.
#[derive(Clone, Copy, Debug)]
pub struct RealizeLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_collection: usize,
}

impl Default for RealizeLimits {
    fn default() -> Self {
        Self {
            max_nodes: 8_192,
            max_depth: 64,
            max_collection: 512,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RealizeReport {
    pub root: Option<UiNode>,
    pub diagnostics: Vec<SyntaxDiagnostic>,
    pub nodes: usize,
    /// A bound was reached; the tree is partial.
    pub truncated: bool,
    /// Every component instance this realization produced, plus the card cell.
    /// Pass to [`InstanceStore::prune`] to forget instances that went away —
    /// without it the store grows for the life of the app, and a key that
    /// reappears inherits a stale value (§5.7, unmount).
    pub live_keys: Vec<String>,
}

/// Realize a card against resolved data.
///
/// `data` carries what the runtime fetched and holds: source results, state
/// values and copy, keyed by declaration name. **No capability is reachable from
/// here** — not because one is blocked, but because realization is a tree walk
/// and there is no evaluator to reach anything with.
pub fn realize(source: &str, data: &crate::JsonValue, limits: RealizeLimits) -> RealizeReport {
    realize_inner(source, data, None, limits)
}

fn realize_inner(
    source: &str,
    data: &crate::JsonValue,
    store: Option<&InstanceStore>,
    limits: RealizeLimits,
) -> RealizeReport {
    let mut sink = Diagnostics::default();

    let check = check_ui_l0_named("card.l0", source);
    if !check.valid {
        return RealizeReport {
            root: None,
            diagnostics: check.diagnostics,
            nodes: 0,
            truncated: false,
            live_keys: Vec::new(),
        };
    }

    let tokens = match lex(source, &mut sink) {
        Some(t) => t,
        None => {
            return RealizeReport {
                root: None,
                diagnostics: sink.items,
                nodes: 0,
                truncated: false,
                live_keys: Vec::new(),
            }
        }
    };
    let card = Parser::new(&tokens, &mut sink).parse_card();

    let Some(root) = card.views.iter().find(|v| v.name == "root") else {
        sink.push(1, 1, "a card needs a `view root`".into());
        return RealizeReport {
            root: None,
            diagnostics: sink.items,
            nodes: 0,
            truncated: false,
            live_keys: Vec::new(),
        };
    };

    let mut ctx = Realizer {
        depth: 0,
        card: &card,
        limits,
        nodes: 0,
        truncated: false,
        sink: &mut sink,
        store,
        live: vec![CARD_STATE_KEY.to_string()],
        slots: Vec::new(),
    };
    // Card-scope state resolves from the store first, then the injected data,
    // then its declared initial. Without this, an event that wrote card state
    // would apply and be invisible at the next realization — and a guard on a
    // state with no value at all would take the wrong branch on first render.
    let mut frames: Vec<(String, crate::JsonValue)> = Vec::new();
    for state in &card.states {
        let value = store
            .and_then(|s| s.get(CARD_STATE_KEY, &state.path))
            .cloned()
            .or_else(|| data.get(&state.path).cloned())
            .or_else(|| state.initial.clone())
            .or_else(|| state.initial_path.as_ref().and_then(|p| data_path(data, p)))
            .unwrap_or_else(|| initial_for(&state.shape));
        frames.push((state.path.clone(), value));
    }
    let mut scope = ValueScope {
        frames,
        data,
        copies: &card.copies,
    };
    let mut out = Vec::new();
    ctx.element(&root.body, &mut scope, "root", &mut out);

    let nodes = ctx.nodes;
    let truncated = ctx.truncated;
    let live_keys = std::mem::take(&mut ctx.live);
    RealizeReport {
        root: out.into_iter().next(),
        diagnostics: sink.items,
        nodes,
        truncated,
        live_keys,
    }
}

/// Bindings introduced by loops and component props, innermost last.
struct ValueScope<'a> {
    frames: Vec<(String, crate::JsonValue)>,
    data: &'a crate::JsonValue,
    copies: &'a [CopyDecl],
}

impl ValueScope<'_> {
    fn lookup(&self, path: &str) -> Option<crate::JsonValue> {
        let mut segments = path.split('.');
        let root = segments.next()?;

        // `<source>.$state` is the lifecycle the runtime reported. It is data
        // like any other, so a guard can branch on it without an expression.
        if let Some(source) = path.strip_suffix(".$state") {
            // A dotted source name navigates the data, so `env.locale` looks
            // under {env: {locale: …}} rather than for a key spelled
            // "env.locale" — which is nowhere.
            let resolved = data_path(self.data, source);
            let status = self
                .data
                .get("$status")
                .and_then(|m| m.get(source))
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| {
                    // No report means: resolved if there is a value, pending if
                    // not. A host that tracks status explicitly overrides this.
                    if resolved.is_some() {
                        "ready"
                    } else {
                        "pending"
                    }
                });
            return Some(crate::JsonValue::String(status.to_string()));
        }

        // `copy.<name>` is authored in the ledger, not fetched. Resolve it from
        // the card's declarations, choosing the language the runtime reported
        // and falling back to English — a card that ships zh text should not
        // render an em dash because the device is in en.
        if root == "copy" {
            let name = segments.next()?;
            let entry = self.copies.iter().find(|c| c.name == name)?;
            let lang = self
                .data
                .get("env")
                .and_then(|e| e.get("locale"))
                .and_then(|l| l.get("lang"))
                .and_then(|v| v.as_str())
                .unwrap_or("en");
            let text = entry
                .locales
                .iter()
                .find(|(k, _)| k == lang)
                .or_else(|| entry.locales.iter().find(|(k, _)| k == "en"))
                .or_else(|| entry.locales.first())?;
            return Some(crate::JsonValue::String(text.1.clone()));
        }

        let mut current = match self.frames.iter().rev().find(|(n, _)| n == root) {
            Some((_, v)) => v.clone(),
            None => self.data.get(root)?.clone(),
        };
        for segment in segments {
            // `[inner]` — resolve the inner path, then index by its value.
            if let Some(inner) = segment.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                let key = self.lookup(inner)?;
                current = match &key {
                    crate::JsonValue::String(k) => current.get(k.as_str())?.clone(),
                    v => match v.as_u64() {
                        Some(i) => current.get(i as usize)?.clone(),
                        None => return None,
                    },
                };
                continue;
            }
            current = match segment.parse::<usize>() {
                Ok(index) => current.get(index)?.clone(),
                Err(_) => current.get(segment)?.clone(),
            };
        }
        Some(current)
    }
}

struct Realizer<'a> {
    card: &'a Card,
    limits: RealizeLimits,
    nodes: usize,
    /// Actual recursion depth. Previously approximated by the instance key's
    /// LENGTH, which bounded neither reliably.
    depth: usize,
    truncated: bool,
    sink: &'a mut Diagnostics,
    /// Where component-local values live. Absent means every instance sees its
    /// declared initial.
    store: Option<&'a InstanceStore>,
    /// Instance keys produced, for pruning.
    live: Vec<String>,
    /// Children passed at a component call site, realized where `slot` appears.
    /// A stack, so a component instantiated inside another's slot still sees its
    /// own children.
    slots: Vec<Vec<(String, Vec<Element>)>>,
}

impl Realizer<'_> {
    fn element(
        &mut self,
        element: &Element,
        scope: &mut ValueScope,
        key: &str,
        out: &mut Vec<UiNode>,
    ) {
        if self.nodes >= self.limits.max_nodes || self.depth >= self.limits.max_depth {
            self.truncated = true;
            return;
        }
        self.depth += 1;
        self.element_inner(element, scope, key, out);
        self.depth -= 1;
    }

    fn element_inner(
        &mut self,
        element: &Element,
        scope: &mut ValueScope,
        key: &str,
        out: &mut Vec<UiNode>,
    ) {
        match element.name.as_str() {
            "" => {}
            "slot" => {
                // Children are realized in the CALLER's scope, not the
                // component's — a child expression sees the call site's
                // bindings (§5.5). Taking the frame off the stack while
                // realizing prevents a component's own slot from re-entering.
                if let Some(groups) = self.slots.pop() {
                    // An unnamed `slot` takes the default group; `slot header`
                    // takes the group a call site directed with `into header`.
                    let wanted = element.binders.first().cloned().unwrap_or_default();
                    if let Some((_, children)) = groups.iter().find(|(n, _)| *n == wanted) {
                        for (segment, child) in sibling_segments(children) {
                            self.element(child, scope, &format!("{key}/s:{segment}"), out);
                        }
                    }
                    self.slots.push(groups);
                }
            }
            "for" => self.iteration(element, scope, key, out),
            "when" => {
                if self.guard_holds(element, scope) {
                    for (segment, child) in sibling_segments(&element.children) {
                        self.element(child, scope, &format!("{key}/w:{segment}"), out);
                    }
                }
            }
            _ if element.is_reference => {
                let card = self.card;
                match card.views.iter().find(|v| v.name == element.name) {
                    Some(view) => {
                        // Keyed by the referenced view's own name, so its nodes
                        // keep identity wherever it is spliced.
                        self.element(&view.body, scope, &format!("{key}/{}", element.name), out);
                    }
                    None => self.sink.push(
                        element.line,
                        element.column,
                        format!("{:?} is not a declared view", element.name),
                    ),
                }
            }
            name => {
                let card = self.card;
                if let Some(component) = card.components.iter().find(|c| c.name == name) {
                    self.component(component, element, scope, key, out);
                } else {
                    self.constructor(element, scope, key, out);
                }
            }
        }
    }

    fn constructor(
        &mut self,
        element: &Element,
        scope: &mut ValueScope,
        key: &str,
        out: &mut Vec<UiNode>,
    ) {
        self.nodes += 1;
        let mut node = UiNode {
            kind: element.name.clone(),
            key: key.to_string(),
            args: Vec::new(),
            children: Vec::new(),
        };
        for arg in &element.args {
            let value = self.value(&element.name, &arg.name, &arg.value, scope);
            node.args.push((arg.name.clone(), value));
        }
        for (segment, child) in sibling_segments(&element.children) {
            self.element(
                child,
                scope,
                &format!("{key}/{segment}"),
                &mut node.children,
            );
        }
        out.push(node);
    }

    fn component(
        &mut self,
        component: &Component,
        call: &Element,
        scope: &mut ValueScope,
        key: &str,
        out: &mut Vec<UiNode>,
    ) {
        // Props bind by name; a prop the caller omits is simply absent, and any
        // path inside the component resolves against this frame first.
        // Bind by DECLARED SHAPE. The shape was retained at parse and then
        // discarded here, which cost two bugs: an event prop was looked up as
        // data, so a state whose name collided with an event hijacked the
        // routing; and an enum token bound with its leading dot, so every
        // comparison against it inside the component was false.
        let mut bound = 0usize;
        for param in &component.params {
            let supplied = call.args.iter().find(|a| a.name == param.name);
            let value = match (supplied, &param.shape) {
                // An event names itself. It has no value in data, and looking
                // one up is how the wrong event came to fire.
                (Some(arg), Shape::Event) => match &arg.value {
                    Operand::Path(p) => crate::JsonValue::String(p.clone()),
                    other => literal_of(other),
                },
                // A token is a bare member; the dot is lexical marking, not
                // part of the value.
                (Some(arg), _) => match &arg.value {
                    Operand::Token(t) => {
                        crate::JsonValue::String(t.trim_start_matches('.').to_string())
                    }
                    Operand::Path(p) | Operand::Predicate { path: p, .. } => scope
                        .lookup(p)
                        .unwrap_or_else(|| crate::JsonValue::String(p.clone())),
                    other => literal_of(other),
                },
                // Omitted: the declared default, or nothing. Binding a frame
                // either way is what stops lookup falling through to injected
                // data and letting the host capture an unfilled prop.
                (None, _) => param.default.clone().unwrap_or(crate::JsonValue::Null),
            };
            scope.frames.push((param.name.clone(), value));
            bound += 1;
        }
        let instance_key = format!("{key}/{}", component.name);
        self.live.push(instance_key.clone());
        self.slots.push(split_slots(&call.children));

        // Local state resolves PER INSTANCE: from the store when it holds a
        // value for this key, otherwise from the declared initial. Two instances
        // of one component therefore never share a cell — the property the
        // component model exists to provide.
        let schema = schema_id(component);
        for state in &component.states {
            // A live cell is readable when the schema still matches, or — under
            // §5.8's opt-in — when this field asked to survive and its own shape
            // is what it was written under. The shape check is what keeps
            // `keep` from becoming the guess the reset exists to prevent.
            let live = self
                .store
                .filter(|s| {
                    s.schemas.get(&component.name).is_none_or(|k| {
                        *k == schema || state.keep && s.shape_unchanged(component, state)
                    })
                })
                .and_then(|s| s.get(&instance_key, &state.path))
                .cloned();
            let from_path = state.initial_path.as_ref().and_then(|p| scope.lookup(p));
            scope.frames.push((
                state.path.clone(),
                live.or_else(|| state.initial.clone())
                    .or(from_path)
                    .unwrap_or_else(|| initial_for(&state.shape)),
            ));
            bound += 1;
        }

        self.element(&component.body, scope, &instance_key, out);
        self.slots.pop();
        for _ in 0..bound {
            scope.frames.pop();
        }
    }

    fn iteration(
        &mut self,
        element: &Element,
        scope: &mut ValueScope,
        key: &str,
        out: &mut Vec<UiNode>,
    ) {
        let Some(Arg {
            value: Operand::Path(path),
            ..
        }) = element.args.first()
        else {
            return;
        };
        let Some(collection) = scope.lookup(path) else {
            // A source that has not resolved yet renders nothing rather than an
            // empty row; pending is the runtime's to surface, not ours to invent.
            return;
        };
        let Some(items) = collection.as_array() else {
            self.sink.push(
                element.line,
                element.column,
                format!("{path:?} is not a collection"),
            );
            return;
        };

        // §5.7: duplicate keys are an error at realization. Untracked, two items
        // with the same key produced the same instance key — one state cell
        // shared by two rows, so tapping either toggled both.
        let mut seen_keys: Vec<String> = Vec::new();

        for (index, item) in items.iter().enumerate() {
            if index >= self.limits.max_collection {
                self.truncated = true;
                break;
            }
            // The key expression is evaluated per item, and identity is the
            // enclosing key plus it — never the index.
            let item_key = element
                .key_path
                .as_deref()
                .and_then(|k| {
                    let mut segments = k.split('.');
                    segments.next()?;
                    let mut current = item.clone();
                    for segment in segments {
                        current = current.get(segment)?.clone();
                    }
                    Some(json_to_key(&current))
                })
                .unwrap_or_else(|| index.to_string());

            // Report the collision, then disambiguate so the two instances do
            // not share a cell. Rendering both with distinct identity is safer
            // than dropping one: a missing row is harder to notice than a
            // diagnostic.
            let item_key = if seen_keys.contains(&item_key) {
                self.sink.push(
                    element.line,
                    element.column,
                    format!(
                        "duplicate key {item_key:?} in this loop; keys must be unique or two \
                         instances share one state cell (profile §5.7)"
                    ),
                );
                format!("{item_key}#{index}")
            } else {
                seen_keys.push(item_key.clone());
                item_key
            };

            let mut bound = 0usize;
            if let Some(binder) = element.binders.first() {
                scope.frames.push((binder.clone(), item.clone()));
                bound += 1;
            }
            if let Some(index_binder) = element.binders.get(1) {
                scope
                    .frames
                    .push((index_binder.clone(), crate::JsonValue::from(index + 1)));
                bound += 1;
            }

            for (segment, child) in sibling_segments(&element.children) {
                self.element(child, scope, &format!("{key}[{item_key}]/{segment}"), out);
            }

            for _ in 0..bound {
                scope.frames.pop();
            }
        }
    }

    fn guard_holds(&mut self, element: &Element, scope: &mut ValueScope) -> bool {
        let Some(Arg {
            value: Operand::Path(path),
            ..
        }) = element.args.first()
        else {
            return false;
        };
        let left = scope.lookup(path);

        let Some(cmp) = element.cmp.as_deref() else {
            // Bare boolean guard — amendment #10.
            return matches!(left, Some(crate::JsonValue::Bool(true)));
        };
        let right = element.rhs.as_ref().and_then(|r| scope_value(scope, r));
        compare(left, cmp, right)
    }

    fn value(
        &mut self,
        ctor: &str,
        arg: &str,
        operand: &Operand,
        scope: &mut ValueScope,
    ) -> NodeValue {
        // An event argument carries a declared NAME through to the runtime,
        // which binds it at dispatch. Nothing about it is looked up in data, and
        // nothing about it is constructed by the card.
        let is_event = catalog::lookup(ctor)
            .and_then(|args| args.iter().find(|(n, _)| *n == arg))
            .is_some_and(|(_, k)| matches!(k, catalog::ArgKind::Event));
        if is_event {
            if let Operand::Path(p) = operand {
                // Through a component prop the name arrives bound; directly it
                // is the path itself.
                return match scope.lookup(p) {
                    Some(crate::JsonValue::String(name)) => NodeValue::Event(name),
                    _ => NodeValue::Event(p.clone()),
                };
            }
        }
        match operand {
            Operand::Token(t) => NodeValue::Token(t.trim_start_matches('.').to_string()),
            Operand::Str(s) => NodeValue::Text(s.clone()),
            Operand::Num(n) => NodeValue::Number(*n),
            Operand::Predicate { path, cmp, rhs } => {
                NodeValue::Bool(compare(scope.lookup(path), cmp, scope_value(scope, rhs)))
            }
            Operand::Path(p) => match scope.lookup(p) {
                Some(crate::JsonValue::String(s)) => NodeValue::Text(s),
                Some(crate::JsonValue::Bool(b)) => NodeValue::Bool(b),
                Some(v) => match v.as_f64() {
                    Some(n) => NodeValue::Number(n),
                    None => NodeValue::Missing,
                },
                None => NodeValue::Missing,
            },
        }
    }
}

fn json_to_key(value: &crate::JsonValue) -> String {
    match value {
        crate::JsonValue::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// ──────────────────────────────────────────────────────── makepad text lowering ──

/// Lower a realized tree to the Splash DSL a Makepad host renders.
///
/// **This is one backend's lowering and belongs in Splash-Makepad**, not here. It
/// lives beside the node model only until that crate is wired up; nothing in
/// [`realize`] depends on it, and a different backend consumes [`UiNode`]
/// directly rather than going through text.
///
/// The output is fully concrete: realization already substituted the runtime's
/// data, so the DSL carries values rather than `sys.*` calls. That is the ledger
/// discipline holding — the card declares questions, and only the *realized
/// output* contains answers.
pub mod makepad {
    use super::{NodeValue, UiNode};
    use std::fmt::Write as _;

    // Matching octos-one's `plan/common.rs`, so a lowered card sits beside the
    // existing ones rather than looking like a different app.
    const BASE: &str = "#0a0e14";
    const SCRIM: &str = "#00000066";
    const PANEL: &str = "#ffffff12";
    /// A selected chip. Brighter than PANEL so "which one is active" is legible
    /// at a glance rather than a subtle tint.
    const ACTIVE: &str = "#ffffff38";
    const HAIRLINE: &str = "#ffffff1a";
    const TEXT: &str = "#ffffff";
    const SOFT: &str = "#ffffffe6";
    const DIM: &str = "#ffffff99";
    const UP: &str = "#32d74b";
    const DOWN: &str = "#ff453a";
    const PAGE_PAD: &str = "Inset{left: 20 top: 54 right: 20 bottom: 24}";

    pub fn lower(root: &UiNode) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "// name: l0-card");
        let _ = writeln!(out, "// REALIZED from an L0 ledger — do not edit.");
        element(root, 0, &mut out);
        out
    }

    /// The tap binding for a node that declares `on_tap`.
    ///
    /// `UiNode` carried the event name and the instance key all along and
    /// lowering emitted neither, so every Row, Card, Chip and TextHero rendered
    /// inert: the stock range controls could not reach `dispatch` at all. The
    /// key is what lets a tap name WHICH instance was hit, which is the whole
    /// basis of §5.1 identity.
    fn tap_binding(node: &UiNode) -> String {
        let Some(NodeValue::Event(event)) = arg(node, "on_tap") else {
            return String::new();
        };
        let mut out = format!(" l0_event: {event:?} l0_key: {:?}", node.key);
        // `value:` is the payload the event carries — `$value` at the receiving
        // transition. Without it a tap says "this happened" but not "to what".
        match arg(node, "value") {
            Some(NodeValue::Text(t)) => out.push_str(&format!(" l0_value: {t:?}")),
            Some(NodeValue::Number(n)) => out.push_str(&format!(" l0_value: {:?}", trim_num(*n))),
            Some(NodeValue::Token(t)) => out.push_str(&format!(" l0_value: {t:?}")),
            _ => {}
        }
        out
    }

    fn arg<'a>(node: &'a UiNode, name: &str) -> Option<&'a NodeValue> {
        node.args.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// A value in text position. `Missing` becomes an em dash rather than an
    /// empty string, so an unresolved binding is visible on screen instead of
    /// silently rendering a blank field.
    fn text_of(value: Option<&NodeValue>) -> String {
        match value {
            Some(NodeValue::Text(s)) => format!("{s:?}"),
            Some(NodeValue::Number(n)) => format!("{:?}", trim_num(*n)),
            Some(NodeValue::Bool(b)) => format!("{b:?}"),
            Some(NodeValue::Token(t)) => format!("{t:?}"),
            Some(NodeValue::Event(_)) | None => "\"\"".into(),
            Some(NodeValue::Missing) => "\"—\"".into(),
            Some(NodeValue::Status(st)) => format!("{:?}", st.as_token()),
        }
    }

    fn num_of(value: Option<&NodeValue>) -> f64 {
        match value {
            Some(NodeValue::Number(n)) => *n,
            Some(NodeValue::Text(s)) => s.parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub(super) fn trim_num(n: f64) -> String {
        if (n - n.round()).abs() < f64::EPSILON {
            format!("{}", n as i64)
        } else {
            format!("{n:.1}")
        }
    }

    /// Apply a declared `format`. This is the runtime's job, not the card's: a
    /// card that wrote "$" or "41.2M" itself would be authoring a fact, and
    /// currency, grouping and compaction are all locale-dependent besides.
    fn formatted(kind: &str, n: f64) -> String {
        let sign = if n > 0.0 { "+" } else { "" };
        match kind {
            "money" => format!("${:.2}", n),
            "signed_money" => format!("{sign}${:.2}", n),
            "signed_pct" => format!("{sign}{:.1}%", n),
            "ratio" => format!("{:.1}", n),
            "compact" => {
                let a = n.abs();
                let (v, suffix) = if a >= 1e12 {
                    (n / 1e12, "T")
                } else if a >= 1e9 {
                    (n / 1e9, "B")
                } else if a >= 1e6 {
                    (n / 1e6, "M")
                } else if a >= 1e3 {
                    (n / 1e3, "K")
                } else {
                    (n, "")
                };
                if suffix.is_empty() {
                    trim_num(v)
                } else {
                    format!("{:.1}{suffix}", v)
                }
            }
            // A float hour — 18.9 is 18:54, not "18.9".
            "time" => {
                let h = n.floor().max(0.0) as i64;
                let m = ((n - n.floor()) * 60.0).round() as i64;
                format!("{h}:{m:02}")
            }
            _ => trim_num(n),
        }
    }

    /// A value plus its unit suffix, which is a presentation concern the runtime
    /// owns — the card never wrote "°" next to a number.
    fn valued(node: &UiNode) -> String {
        let value = arg(node, "value");
        let unit = match arg(node, "unit") {
            Some(NodeValue::Token(t)) if t == "c" || t == "f" => "°",
            Some(NodeValue::Text(t)) if t == "c" || t == "f" => "°",
            Some(NodeValue::Token(t)) if t == "pct" => "%",
            _ => "",
        };
        let glyph = match arg(node, "glyph") {
            Some(NodeValue::Text(g)) => g.clone(),
            _ => String::new(),
        };
        // `suffix` is a declared unit word — "412 pts", not "412". It was in the
        // catalog and ignored here, so every score and comment count on the news
        // card rendered as a bare number.
        let suffix = match arg(node, "suffix") {
            Some(NodeValue::Text(t)) => format!(" {t}"),
            _ => String::new(),
        };
        let format_kind = match arg(node, "format") {
            Some(NodeValue::Token(t)) => Some(t.clone()),
            _ => None,
        };
        match value {
            Some(NodeValue::Missing) => "\"—\"".into(),
            Some(v) => {
                let body = match (v, format_kind.as_deref()) {
                    (NodeValue::Number(n), Some(kind)) => formatted(kind, *n),
                    (NodeValue::Number(n), None) => trim_num(*n),
                    (NodeValue::Text(s), _) => s.clone(),
                    _ => String::new(),
                };
                format!("{:?}", format!("{glyph}{body}{unit}{suffix}"))
            }
            None => text_of(arg(node, "text")),
        }
    }

    fn pad(depth: usize) -> String {
        "  ".repeat(depth.min(32))
    }

    fn children(node: &UiNode, depth: usize, out: &mut String) {
        for child in &node.children {
            element(child, depth + 1, out);
        }
    }

    fn element(node: &UiNode, depth: usize, out: &mut String) {
        let p = pad(depth);
        match node.kind.as_str() {
            "Surface" => {
                let _ = writeln!(
                    out,
                    "{p}SolidView{{ width: Fill height: Fit flow: Down new_batch: true \
                     draw_bg.color: {BASE} padding: {PAGE_PAD}"
                );
                children(node, depth, out);
                let _ = writeln!(out, "{p}}}");
            }
            "Photo" => {
                // Overlay: photo, scrim, then the column. A `Fit` overlay takes
                // its tallest child, so the image needs a fixed height or the
                // card ends in a band of bare base colour.
                let src = match arg(node, "src") {
                    Some(NodeValue::Text(s)) => s.clone(),
                    _ => String::new(),
                };
                let _ = writeln!(
                    out,
                    "{p}SolidView{{ width: Fill height: Fit flow: Overlay new_batch: true \
                     draw_bg.color: {BASE}"
                );
                if !src.is_empty() {
                    let _ = writeln!(
                        out,
                        "{p}  Image{{ src: http_resource({src:?}) fit: ImageFit.CropToFill \
                         width: Fill height: 2000 }}"
                    );
                }
                let _ = writeln!(
                    out,
                    "{p}  SolidView{{ width: Fill height: Fill draw_bg.color: {SCRIM} }}"
                );
                let _ = writeln!(
                    out,
                    "{p}  View{{ width: Fill height: Fit flow: Down padding: {PAGE_PAD}"
                );
                children(node, depth + 1, out);
                let _ = writeln!(out, "{p}  }}");
                let _ = writeln!(out, "{p}}}");
            }
            "Col" => {
                let align =
                    matches!(arg(node, "align"), Some(NodeValue::Token(t)) if t == "center");
                let a = if align { " align: Align{x: 0.5}" } else { "" };
                let _ = writeln!(out, "{p}View{{ width: Fill height: Fit flow: Down{a}");
                children(node, depth, out);
                let _ = writeln!(out, "{p}}}");
            }
            "Row" => {
                let _ = writeln!(
                    out,
                    "{p}View{{ width: Fill height: Fit flow: Right align: Align{{y: 0.5}} spacing: 6{}",
                    tap_binding(node)
                );
                children(node, depth, out);
                let _ = writeln!(out, "{p}}}");
            }
            "Panel" | "Card" => {
                let _ = writeln!(
                    out,
                    "{p}RoundedView{{ width: Fill height: Fit flow: Down new_batch: true \
                     draw_bg.color: {PANEL} draw_bg.border_radius: 14.0 \
                     margin: Inset{{top: 16}} padding: Inset{{left: 14 right: 14 top: 12 bottom: 12}}{}",
                    tap_binding(node)
                );
                children(node, depth, out);
                let _ = writeln!(out, "{p}}}");
            }
            "Grid" => {
                // Two columns, emitted as rows of two so the existing layout
                // engine needs no grid primitive.
                let _ = writeln!(
                    out,
                    "{p}View{{ width: Fill height: Fit flow: Down spacing: 8"
                );
                for pair in node.children.chunks(2) {
                    let _ = writeln!(
                        out,
                        "{p}  View{{ width: Fill height: Fit flow: Right spacing: 8"
                    );
                    for cell in pair {
                        element(cell, depth + 2, out);
                    }
                    let _ = writeln!(out, "{p}  }}");
                }
                let _ = writeln!(out, "{p}}}");
            }
            "Rule" => {
                let _ = writeln!(
                    out,
                    "{p}SolidView{{ width: Fill height: 1 draw_bg.color: {HAIRLINE} }}"
                );
            }
            "Tile" => {
                let _ = writeln!(
                    out,
                    "{p}RoundedView{{ width: Fill height: Fit flow: Down new_batch: true \
                     draw_bg.color: {PANEL} draw_bg.border_radius: 18.0 \
                     padding: Inset{{left: 14 right: 14 top: 12 bottom: 12}} spacing: 4"
                );
                let _ = writeln!(
                    out,
                    "{p}  TextCaption{{ text: {} draw_text.color: {DIM} }}",
                    text_of(arg(node, "label"))
                );
                let _ = writeln!(
                    out,
                    "{p}  TextStat{{ text: {} draw_text.color: {TEXT} }}",
                    valued(node)
                );
                let _ = writeln!(out, "{p}}}");
            }
            "TempBar" => {
                let _ = writeln!(
                    out,
                    "{p}TempBar{{ width: Fill height: 8 margin: Inset{{left: 10 right: 10}} \
                     draw_bg.tlo: {} draw_bg.thi: {} draw_bg.wmin: {} draw_bg.wmax: {} }}",
                    trim_num(num_of(arg(node, "lo"))),
                    trim_num(num_of(arg(node, "hi"))),
                    trim_num(num_of(arg(node, "min"))),
                    trim_num(num_of(arg(node, "max"))),
                );
            }
            "WeatherIcon" => {
                let size = matches!(arg(node, "size"), Some(NodeValue::Token(t)) if t == "hero");
                let (w, h) = if size { (64, 64) } else { (34, 26) };
                let _ = writeln!(
                    out,
                    "{p}WeatherIcon{{ width: {w} height: {h} draw_bg.cond: {} }}",
                    trim_num(num_of(arg(node, "cond")))
                );
            }
            "SunArc" => {
                let _ = writeln!(
                    out,
                    "{p}SunArc{{ width: Fill height: 90 draw_bg.rise: {} draw_bg.set: {} draw_bg.now: {} }}",
                    trim_num(num_of(arg(node, "rise"))),
                    trim_num(num_of(arg(node, "set"))),
                    trim_num(num_of(arg(node, "now"))),
                );
            }
            "MoonPhase" => {
                let _ = writeln!(
                    out,
                    "{p}MoonPhase{{ width: 64 height: 64 draw_bg.phase: {} }}",
                    trim_num(num_of(arg(node, "phase")))
                );
            }
            "AqiContour" => {
                let _ = writeln!(
                    out,
                    "{p}AqiContour{{ width: Fill height: 190 draw_bg.have: 1.0 draw_bg.idx: {} }}",
                    trim_num(num_of(arg(node, "index")))
                );
            }
            "StockPlot" => {
                let _ = writeln!(out, "{p}StockPlot{{ width: Fill height: 180 }}");
            }
            "Chip" => {
                // `active` selects the fill. Dropping it made every range chip
                // render identically, so a card could not show which was
                // selected — and the golden recorded that as correct.
                let on = matches!(arg(node, "active"), Some(NodeValue::Bool(true)));
                let (fill, ink) = if on { (ACTIVE, TEXT) } else { (PANEL, SOFT) };
                let _ = writeln!(
                    out,
                    "{p}RoundedView{{ width: Fit height: Fit draw_bg.color: {fill} \
                     draw_bg.border_radius: 12.0 padding: Inset{{left: 10 right: 10 top: 5 bottom: 5}}\
                     {}",
                    tap_binding(node)
                );
                let _ = writeln!(
                    out,
                    "{p}  TextCaption{{ text: {} draw_text.color: {ink} }}",
                    text_of(arg(node, "text"))
                );
                let _ = writeln!(out, "{p}}}");
            }
            role if role.starts_with("Text") => {
                // `tint` colours by SIGN — the card says "tint by this number",
                // and which colours mean up and down is the runtime's to decide.
                let colour = match arg(node, "tint") {
                    Some(NodeValue::Number(n)) if *n > 0.0 => UP,
                    Some(NodeValue::Number(n)) if *n < 0.0 => DOWN,
                    _ => match role {
                        "TextCaption" => DIM,
                        "TextRow" => SOFT,
                        _ => TEXT,
                    },
                };
                // Only constrain the width when the card asked. Forcing
                // `width: Fill` made a long hero title wrap one character per
                // line — "Top Movers" became "Top / Mover / s" on device.
                let width = match arg(node, "width") {
                    Some(NodeValue::Token(t)) if t == "fill" => " width: Fill",
                    Some(NodeValue::Token(t)) if t == "fit" => " width: Fit",
                    Some(NodeValue::Token(_)) => " width: Fit",
                    _ => "",
                };
                let body = if arg(node, "value").is_some() || arg(node, "glyph").is_some() {
                    valued(node)
                } else {
                    text_of(arg(node, "text"))
                };
                // A hero is sized for the ONE dominant value. "18°" fits at 62;
                // "$184.20" clipped off the right edge on device. Scaling to fit
                // is the runtime's call — the card says "this is the hero", not
                // "this is 62 points".
                let size = if role == "TextHero" {
                    let glyphs = body.trim_matches('"').chars().count();
                    let pt = match glyphs {
                        0..=4 => 62,
                        5..=6 => 50,
                        7..=8 => 40,
                        9..=12 => 32,
                        _ => 24,
                    };
                    format!(" draw_text.text_style.font_size: {pt}")
                } else {
                    String::new()
                };
                let _ = writeln!(
                    out,
                    "{p}{role}{{{width} text: {body} draw_text.color: {colour}{size}{} }}",
                    tap_binding(node)
                );
            }
            other => {
                // An unmapped constructor is shown, not skipped. A card that
                // silently drops a section looks correct and is not.
                let _ = writeln!(
                    out,
                    "{p}TextCaption{{ text: {:?} draw_text.color: #ffb4b4 }}",
                    format!("⚠ no makepad lowering for {other}")
                );
            }
        }
    }
}

// ────────────────────────────────────────────────────────────── instance state ──

use std::collections::BTreeMap;

/// The key card-scope state lives under. Card state has no instance, so it needs
/// a fixed cell rather than one derived from a position in the tree.
pub const CARD_STATE_KEY: &str = "@card";

/// Per-instance local state — the runtime's half of the immutable/mutable split.
///
/// The ledger declares a component's state *shape*; the values live here, keyed
/// by instance identity (§5.1). Seven `ForecastRow` instances therefore hold
/// seven independent cells, none of which appear in the card.
///
/// Nothing in the store is authored by the model. A card cannot name an
/// instance, cannot address another instance's cell, and cannot write anything
/// except through a declared transition on its own state.
#[derive(Clone, Debug, Default)]
pub struct InstanceStore {
    /// `instance-key` → field → value.
    cells: BTreeMap<String, BTreeMap<String, crate::JsonValue>>,
    /// Component name → the schema id its live cells were written under.
    schemas: BTreeMap<String, u64>,
    /// Component name → state path → the shape that cell was written under.
    /// A `keep` cell is preserved across a schema change only when this still
    /// matches, so opting in never means "trust whatever was there".
    shapes: BTreeMap<String, BTreeMap<String, String>>,
}

impl InstanceStore {
    pub fn get(&self, key: &str, field: &str) -> Option<&crate::JsonValue> {
        self.cells.get(key)?.get(field)
    }

    fn set(&mut self, key: &str, field: &str, value: crate::JsonValue) {
        self.cells
            .entry(key.to_string())
            .or_default()
            .insert(field.to_string(), value);
    }

    /// Whether `state` has the shape the store's live cells were written under.
    fn shape_unchanged(&self, component: &Component, state: &StateDecl) -> bool {
        self.shapes
            .get(&component.name)
            .and_then(|m| m.get(&state.path))
            .is_some_and(|old| *old == format!("{:?}", state.shape))
    }

    /// Drop every cell belonging to instances of `component`, except the fields
    /// listed in `keep` — §5.8's opt-in migration.
    fn reset_component(&mut self, component: &str, keep: &[String]) {
        let marker = format!("/{component}");
        self.cells.retain(|key, fields| {
            if !key.contains(&marker) {
                return true;
            }
            fields.retain(|field, _| keep.iter().any(|k| k == field));
            !fields.is_empty()
        });
    }

    /// Profile §5.8: a component whose state declarations changed gets a new
    /// schema id, and its live cells are dropped rather than migrated. Guessing
    /// that an old value still fits is how a rolled-back ledger feeds version-A
    /// state to version-B code.
    fn reconcile_schema(&mut self, component: &Component, schema: u64) {
        let name = component.name.as_str();
        let shapes: BTreeMap<String, String> = component
            .states
            .iter()
            .map(|s| (s.path.clone(), format!("{:?}", s.shape)))
            .collect();

        match self.schemas.get(name) {
            Some(&known) if known == schema => {}
            Some(_) => {
                // A cell survives only if it asked to AND its own shape is
                // unchanged. Opting in is a claim about one field, not a
                // blanket assertion that the old values still fit.
                let previous = self.shapes.get(name);
                let keep: Vec<String> = component
                    .states
                    .iter()
                    .filter(|s| s.keep)
                    .filter(|s| {
                        previous
                            .and_then(|p| p.get(&s.path))
                            .is_some_and(|old| *old == format!("{:?}", s.shape))
                    })
                    .map(|s| s.path.clone())
                    .collect();
                self.reset_component(name, &keep);
                self.schemas.insert(name.to_string(), schema);
                self.shapes.insert(name.to_string(), shapes);
            }
            None => {
                self.schemas.insert(name.to_string(), schema);
                self.shapes.insert(name.to_string(), shapes);
            }
        }
    }

    /// Forget instances that no longer appear — the unmount half of §5.7.
    pub fn prune(&mut self, live: &[String]) {
        self.cells.retain(|key, _| live.iter().any(|k| k == key));
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

/// A component's state schema: its declarations, order-independent. Changing a
/// shape, name or initial changes this; changing the view does not.
/// A tree as `Kind(child, child)` — structure without spans, so two surfaces can
/// be compared on the thing that matters rather than byte-for-byte. Codex made
/// exactly this point about "byte-identical Card": spans differ whenever the
/// formatting does, so they cannot be part of an equality test between surfaces.
fn shape_of(e: &Element) -> String {
    let kids: Vec<String> = e.children.iter().map(shape_of).collect();
    let head = if e.name.is_empty() {
        "_"
    } else {
        e.name.as_str()
    };
    if kids.is_empty() {
        head.to_string()
    } else {
        format!("{head}({})", kids.join(", "))
    }
}

fn schema_id(component: &Component) -> u64 {
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    let mut fields: Vec<String> = component
        .states
        .iter()
        .map(|s| {
            format!(
                "{}:{:?}:{:?}:{:?}",
                s.path, s.shape, s.initial, s.initial_path
            )
        })
        .collect();
    fields.sort();
    for byte in fields.join(",").bytes() {
        acc ^= byte as u64;
        acc = acc.wrapping_mul(0x100_0000_01b3);
    }
    acc
}

fn initial_for(shape: &Shape) -> crate::JsonValue {
    match shape {
        Shape::Bool => crate::JsonValue::Bool(false),
        Shape::Enum(members) => members
            .first()
            .map(|m| crate::JsonValue::String(m.clone()))
            .unwrap_or(crate::JsonValue::Null),
        Shape::Text => crate::JsonValue::String(String::new()),
        Shape::Number => crate::JsonValue::from(0.0),
        Shape::Record | Shape::Collection | Shape::Event | Shape::Other => crate::JsonValue::Null,
    }
}

/// Realize against a store, so component-local state resolves per instance.
///
/// [`realize`] is this with an empty store: a card with no local state behaves
/// identically either way.
pub fn realize_with_state(
    source: &str,
    data: &crate::JsonValue,
    store: &InstanceStore,
    limits: RealizeLimits,
) -> RealizeReport {
    realize_inner(source, data, Some(store), limits)
}

/// Apply an event's transitions to one instance's cells.
///
/// Returns whether anything applied. The event name is resolved against the
/// component's *declarations* — a card cannot construct one, because it has no
/// expression form to build a name with.
pub fn dispatch(source: &str, store: &mut InstanceStore, instance_key: &str, event: &str) -> bool {
    dispatch_with(source, store, instance_key, event, None)
}

/// Apply an event's transitions, supplying the payload the raising element
/// carried in its `value:` argument.
///
/// The payload is data the runtime already holds — a bound path or a literal
/// resolved at realization — never anything the card computed.
pub fn dispatch_with(
    source: &str,
    store: &mut InstanceStore,
    instance_key: &str,
    event: &str,
    payload: Option<&crate::JsonValue>,
) -> bool {
    dispatch_with_data(
        source,
        store,
        instance_key,
        event,
        payload,
        &crate::JsonValue::Null,
    )
}

/// `dispatch_with`, plus the data a `set(path)` reads from.
///
/// Separated rather than folded in because most events do not read: `toggle`,
/// `cycle`, `clear` and `set(.token)` need nothing. Only a transition that
/// names a path needs the host's data, and passing Null simply makes that one
/// form a no-op instead of writing something wrong.
pub fn dispatch_with_data(
    source: &str,
    store: &mut InstanceStore,
    instance_key: &str,
    event: &str,
    payload: Option<&crate::JsonValue>,
    data: &crate::JsonValue,
) -> bool {
    let mut sink = Diagnostics::default();
    let Some(tokens) = lex(source, &mut sink) else {
        return false;
    };
    let card = Parser::new(&tokens, &mut sink).parse_card();

    // A backend dispatches with the key of the NODE that was tapped, which sits
    // inside the component rather than at its boundary — `…/Rowy/0/1`. The cell
    // belongs to the instance, so the key is truncated to the component segment.
    // Where components nest, the innermost one declaring this event wins.
    let mut owner: Option<(&Component, usize)> = None;
    for component in &card.components {
        if !component.events.iter().any(|e| e.name == event) {
            continue;
        }
        let marker = format!("/{}", component.name);
        if let Some(at) = instance_key.rfind(&marker) {
            let end = at + marker.len();
            // Only a whole segment matches: `/Row` must not match `/Rowy`.
            let whole = instance_key[end..]
                .chars()
                .next()
                .is_none_or(|c| c == '/' || c == '[');
            if whole && owner.as_ref().is_none_or(|(_, best)| end > *best) {
                owner = Some((component, end));
            }
        }
    }
    // An event declared on the CARD rather than a component targets card state,
    // which lives under a fixed key. A component event wins when both declare the
    // name, since the innermost scope owns the cell.
    let (declared, states, instance_key, schema_owner) = match owner {
        Some((component, boundary)) => match component.events.iter().find(|e| e.name == event) {
            Some(d) => (
                d,
                &component.states,
                &instance_key[..boundary],
                Some((component, schema_id(component))),
            ),
            None => return false,
        },
        None => match card.events.iter().find(|e| e.name == event) {
            Some(d) => (d, &card.states, CARD_STATE_KEY, None),
            None => return false,
        },
    };

    if let Some((component, schema)) = schema_owner {
        store.reconcile_schema(component, schema);
    }

    // §3: a batch is applied ATOMICALLY. Writing straight to the store as we go
    // left the earlier writes standing when a later one could not be computed —
    // a half-applied event, which is the thing atomicity exists to prevent.
    // Stage every write first, commit only if the whole batch resolves.
    let mut staged: Vec<(String, crate::JsonValue)> = Vec::new();

    for transition in &declared.transitions {
        let Some(state) = states.iter().find(|s| s.path == transition.target) else {
            return false;
        };
        // The declared initial, path-valued or not. `clear` and a first `cycle`
        // both fell back to the SHAPE default here, so a card whose initial
        // comes from the host started in the wrong state.
        let declared_initial = state
            .initial
            .clone()
            .or_else(|| state.initial_path.as_ref().and_then(|p| data_path(data, p)));

        let current = staged
            .iter()
            .rev()
            .find(|(t, _)| *t == transition.target)
            .map(|(_, v)| v.clone())
            .or_else(|| store.get(instance_key, &transition.target).cloned())
            .or_else(|| declared_initial.clone())
            .unwrap_or_else(|| initial_for(&state.shape));

        let next = match (&transition.form, &state.shape) {
            (Form::Toggle, Shape::Bool) => {
                crate::JsonValue::Bool(!current.as_bool().unwrap_or(false))
            }
            (Form::Clear, shape) => declared_initial
                .clone()
                .unwrap_or_else(|| initial_for(shape)),
            (Form::Cycle, Shape::Enum(members)) if !members.is_empty() => {
                let at = current
                    .as_str()
                    .and_then(|s| members.iter().position(|m| m == s))
                    .unwrap_or(0);
                crate::JsonValue::String(members[(at + 1) % members.len()].clone())
            }
            (Form::Set(source), _) => match source {
                SetSource::Payload => match payload {
                    // The one value that arrives from OUTSIDE, so it is the one
                    // that has to be checked at runtime: every other set form is
                    // decided against the declared shape at check time.
                    Some(v) if value_fits_shape(&state.shape, v) => v.clone(),
                    Some(_) => return false,
                    // No payload is a no-op rather than a silent clear: falling
                    // back to the initial would look like a deliberate reset.
                    None => return false,
                },
                // A path READS. Treating it as the payload meant
                // `n: set(config.answer)` wrote whatever the tap carried — or
                // nothing at all when it carried nothing.
                SetSource::Path(p) => match data_path(data, p) {
                    Some(v) if value_fits_shape(&state.shape, &v) => v,
                    // Unresolvable or ill-shaped: the batch cannot complete, and
                    // §3 says a batch is all or nothing.
                    _ => return false,
                },
                SetSource::Token(t) | SetSource::Text(t) => crate::JsonValue::String(t.clone()),
                SetSource::Num(n) => crate::JsonValue::from(*n),
                SetSource::Bool(b) => crate::JsonValue::Bool(*b),
            },
            _ => return false,
        };
        staged.push((transition.target.clone(), next));
    }

    // Commit.
    let applied = !staged.is_empty();
    for (target, value) in staged {
        store.set(instance_key, &target, value);
    }
    applied
}

// ───────────────────────────────────────────────────────────────── source plan ──

/// One capability query for the host to answer, with the name its result binds.
#[derive(Clone, Debug)]
pub struct SourceRequest {
    /// The name the card binds the result to — `now`, `week`, `env.locale`.
    pub name: String,
    /// The helper that answers it — `sys.weather`. Resolving this to an actual
    /// function is the HOST's job; splash-core never calls anything.
    pub helper: String,
    pub args: Vec<(String, SourceArg)>,
    /// Names of other sources this one reads. The host must resolve those first.
    pub depends_on: Vec<String>,
}

/// What a card needs fetched, in an order that satisfies its dependencies.
#[derive(Clone, Debug, Default)]
pub struct SourcePlan {
    pub requests: Vec<SourceRequest>,
    pub diagnostics: Vec<SyntaxDiagnostic>,
}

/// Read a card's `source` declarations and order them for resolution.
///
/// Sources form a DAG: `source now sys.weather(lat: place.lat, …)` cannot be
/// fetched before `place`. The order is topological, so a host can walk the list
/// and always have what the next request refers to.
///
/// This returns *what to fetch*, never fetches it. The card names a helper; only
/// the host knows what answers it. That separation is what lets realization run
/// against an empty host surface.
pub fn source_plan(source: &str) -> SourcePlan {
    let mut sink = Diagnostics::default();
    let Some(tokens) = lex(source, &mut sink) else {
        return SourcePlan {
            requests: Vec::new(),
            diagnostics: sink.items,
        };
    };
    let card = Parser::new(&tokens, &mut sink).parse_card();

    // The plan is what a host acts on, so the capability check has to happen
    // HERE too and not only in `check_ui_l0`. A caller that skips checking must
    // still not be handed `sys.shell` as a function to resolve.
    validate_sources(&card, &mut sink);

    let declared: Vec<&str> = card.sources.iter().map(|s| s.name.as_str()).collect();

    let mut pending: Vec<SourceRequest> = card
        .sources
        .iter()
        .filter(|decl| catalog::source(&decl.helper).is_some())
        .map(|decl| {
            let mut depends_on: Vec<String> = decl
                .args
                .iter()
                .filter_map(|(_, arg)| match arg {
                    SourceArg::Path(p) => {
                        // Match a declared name by its own segments, longest
                        // first: `env.locale.lang` depends on `env.locale`.
                        // Reducing to the root compared `env` against the full
                        // name and recorded no edge at all, so the dependent
                        // was planned before the thing it reads.
                        let mut hits: Vec<&&str> = declared
                            .iter()
                            .filter(|d| {
                                *p == ***d
                                    || p.strip_prefix(**d).is_some_and(|r| r.starts_with('.'))
                            })
                            .collect();
                        hits.sort_by_key(|d| std::cmp::Reverse(d.len()));
                        hits.first().map(|d| (**d).to_string())
                    }
                    _ => None,
                })
                .collect();
            depends_on.sort();
            depends_on.dedup();
            SourceRequest {
                name: decl.name.clone(),
                helper: decl.helper.clone(),
                args: decl.args.clone(),
                depends_on,
            }
        })
        .collect();

    // Topological order. A cycle is reported rather than resolved: two sources
    // that read each other cannot both go first, and picking one silently would
    // fetch against a value that does not exist yet.
    let mut ordered: Vec<SourceRequest> = Vec::new();
    let mut done: Vec<String> = Vec::new();
    while !pending.is_empty() {
        let ready: Vec<usize> = pending
            .iter()
            .enumerate()
            .filter(|(_, r)| r.depends_on.iter().all(|d| done.iter().any(|n| n == d)))
            .map(|(i, _)| i)
            .collect();

        if ready.is_empty() {
            for stuck in &pending {
                sink.push(
                    card.sources
                        .iter()
                        .find(|d| d.name == stuck.name)
                        .map(|d| d.line)
                        .unwrap_or(0),
                    1,
                    format!(
                        "source {:?} is in a dependency cycle with {:?}",
                        stuck.name,
                        stuck.depends_on.join(", ")
                    ),
                );
            }
            break;
        }

        for i in ready.into_iter().rev() {
            let request = pending.remove(i);
            done.push(request.name.clone());
            ordered.push(request);
        }
    }

    SourcePlan {
        requests: ordered,
        diagnostics: sink.items,
    }
}

// ─────────────────────────────────────────────────────────────── reconciliation ──

/// What one record reads, so a state write can be confined to its dependents.
#[derive(Clone, Debug)]
pub struct RecordDeps {
    /// A `view` or `component` name.
    pub record: String,
    /// Roots it reads, transitively through referenced views and instantiated
    /// components — `now`, `units`, `week`.
    pub reads: Vec<String>,
}

/// What each record reads.
///
/// L0 needs no dynamic read-tracking: it has no expression form, so every
/// binding is structurally visible and the dependency set is computable without
/// evaluating anything. That is the difference between this and the signal
/// graphs a dynamic language requires — there is no branch whose reads depend on
/// a value.
///
/// The set is transitive and deliberately **over**-approximate at the edges: a
/// record inherits everything the views it splices and the components it
/// instantiates read. Over-approximating re-realizes too much; under-
/// approximating shows stale data, and only one of those is a correctness bug.
pub fn record_dependencies(source: &str) -> Vec<RecordDeps> {
    let mut sink = Diagnostics::default();
    let Some(tokens) = lex(source, &mut sink) else {
        return Vec::new();
    };
    let card = Parser::new(&tokens, &mut sink).parse_card();

    // Direct reads, plus the records each one pulls in.
    let mut direct: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for view in &card.views {
        let (reads, pulls) = scan_element(&view.body);
        direct.push((view.name.clone(), reads, pulls));
    }
    for component in &card.components {
        let (mut reads, pulls) = scan_element(&component.body);
        // A prop is supplied by the caller, so it is not a dependency of its own.
        reads.retain(|r| !component.params.iter().any(|p| p.name == *r));
        // Local state is a dependency: writing it must re-realize this instance.
        for state in &component.states {
            reads.push(root_of(&state.path));
        }
        direct.push((component.name.clone(), reads, pulls));
    }

    // Close over references. Bounded by record count, so a reference cycle
    // terminates rather than spinning.
    let mut out: Vec<RecordDeps> = Vec::new();
    for (name, _, _) in &direct {
        let mut reads: Vec<String> = Vec::new();
        let mut queue = vec![name.clone()];
        let mut seen: Vec<String> = Vec::new();
        let mut steps = 0usize;
        while let Some(current) = queue.pop() {
            steps += 1;
            if steps > MAX_UI_L0_DECLARATIONS || seen.contains(&current) {
                continue;
            }
            seen.push(current.clone());
            if let Some((_, r, pulls)) = direct.iter().find(|(n, _, _)| *n == current) {
                reads.extend(r.iter().cloned());
                queue.extend(pulls.iter().cloned());
            }
        }
        reads.sort();
        reads.dedup();
        out.push(RecordDeps {
            record: name.clone(),
            reads,
        });
    }
    out
}

/// The smallest set of records a backend must re-realize.
///
/// [`dirty_records`] is transitive, so `root` appears for almost any write — it
/// splices every other record, so it reads everything they read. Patching root
/// would rebuild the tree, which is the behaviour reconciliation exists to
/// avoid. A record is a **patch point** only when its OWN bindings read a
/// changed path; an ancestor that merely contains it is handled by patching the
/// descendant.
pub fn patch_points(source: &str, changed: &[&str]) -> Vec<String> {
    let mut sink = Diagnostics::default();
    let Some(tokens) = lex(source, &mut sink) else {
        return Vec::new();
    };
    let card = Parser::new(&tokens, &mut sink).parse_card();
    let roots: Vec<String> = changed.iter().map(root_of).collect();

    // A source reading changed state must be re-fetched, and anything binding
    // its result is then stale.
    let mut invalidated: Vec<String> = roots.clone();
    let mut steps = 0usize;
    loop {
        steps += 1;
        if steps > MAX_UI_L0_DECLARATIONS {
            break;
        }
        let before = invalidated.len();
        for decl in &card.sources {
            let reads_changed = decl.args.iter().any(|(_, arg)| match arg {
                SourceArg::Path(p) => {
                    // `state.city` names the state directly; `place.lat` names
                    // another source.
                    let mut parts = p.split('.');
                    let head = parts.next().unwrap_or_default();
                    let target = if head == "state" {
                        parts.next().unwrap_or_default()
                    } else {
                        head
                    };
                    invalidated.iter().any(|i| i == target)
                }
                _ => false,
            });
            if reads_changed && !invalidated.contains(&decl.name) {
                invalidated.push(decl.name.clone());
            }
        }
        if invalidated.len() == before {
            break;
        }
    }

    let mut out = Vec::new();
    for view in &card.views {
        let (reads, _) = scan_element(&view.body);
        if reads.iter().any(|r| invalidated.iter().any(|i| i == r)) {
            out.push(view.name.clone());
        }
    }
    for component in &card.components {
        let (mut reads, _) = scan_element(&component.body);
        reads.retain(|r| !component.params.iter().any(|p| p.name == *r));
        for state in &component.states {
            reads.push(root_of(&state.path));
        }
        if reads.iter().any(|r| invalidated.iter().any(|i| i == r)) {
            out.push(component.name.clone());
        }
    }
    out
}

/// Which records must re-realize when these paths change.
///
/// The measurable claim behind "one event, one state change, one reconciliation
/// pass": toggling a unit should touch the records that read it, not the tree.
pub fn dirty_records(source: &str, changed: &[&str]) -> Vec<String> {
    let roots: Vec<String> = changed.iter().map(root_of).collect();
    record_dependencies(source)
        .into_iter()
        .filter(|d| d.reads.iter().any(|r| roots.iter().any(|c| c == r)))
        .map(|d| d.record)
        .collect()
}

/// Paths read directly by an element tree, and the records it pulls in.
fn scan_element(element: &Element) -> (Vec<String>, Vec<String>) {
    let mut reads = Vec::new();
    let mut pulls = Vec::new();
    collect_reads(element, &mut reads, &mut pulls);
    reads.sort();
    reads.dedup();
    pulls.sort();
    pulls.dedup();
    (reads, pulls)
}

fn collect_reads(element: &Element, reads: &mut Vec<String>, pulls: &mut Vec<String>) {
    // A loop binder shadows: `for d in week` makes `d` local, not a dependency.
    let binders = &element.binders;

    for arg in &element.args {
        match &arg.value {
            Operand::Path(p) | Operand::Predicate { path: p, .. } => {
                let root = root_of(p);
                if !binders.contains(&root) {
                    reads.push(root);
                }
            }
            _ => {}
        }
    }
    if let Some(Operand::Path(p)) = element.rhs.as_ref() {
        reads.push(root_of(p));
    }

    if element.is_reference || !element.name.is_empty() {
        // A referenced view or an instantiated component contributes its own
        // reads; a plain constructor is not a record and contributes nothing.
        let n = &element.name;
        if !n.is_empty() && n != "for" && n != "when" && n != "slot" {
            pulls.push(n.clone());
        }
    }

    for child in &element.children {
        let mut child_reads = Vec::new();
        collect_reads(child, &mut child_reads, pulls);
        // Binders introduced here shadow inside this subtree only.
        for r in child_reads {
            if !binders.contains(&r) {
                reads.push(r);
            }
        }
    }
}

// ─────────────────────────────────────────────────── the Splash-surface projector ──

/// Recognise an L0 card written in **Splash's own grammar** and project it into
/// the same `Card` the bespoke surface produces.
///
/// Why a second surface at all: L0 currently has its own lexer and parser and
/// imports one type from the rest of the crate, so "a Splash profile" describes
/// a philosophy rather than an implementation. A card written as Splash is one
/// language across the whole stack — card, theme kit, DSL, VM — and `splash-lsp`
/// already exists for it.
///
/// **This is a positive recogniser, not a denylist.** Every shape below is
/// admitted by name; anything else is refused. That is the difference between a
/// surface that happens to reject bad input today and one that rejects it by
/// construction, and it is the condition under which a general-purpose carrier
/// grammar can host a restricted language at all.
///
/// The carrier is verified separately: `check_ui_l0_splash` runs
/// `check_vm_compatibility` first, so a card must be real Splash *and* project
/// cleanly. Neither check subsumes the other — the first says the VM could run
/// it, the second says L0 will admit it.
pub mod splash_surface {
    use super::{
        collect_constructors, lex, Arg, Component, CopyDecl, Diagnostics, Element, EventDecl,
        Operand, Param, Provenance, Shape, SourceArg, SourceDecl, StateDecl, Token, View,
    };

    /// What a `let` binding was recognised as.
    enum Recognised {
        Source(SourceDecl),
        State(StateDecl),
        Event(EventDecl),
        Copy(CopyDecl),
    }

    enum Declared {
        AsView(View),
        AsComponent(Component),
    }

    /// Project declarations out of Splash-shaped source.
    ///
    /// Returns the declarations it recognised and diagnostics for every `let`
    /// it could not. A `let` bound to anything outside the four forms is an
    /// error rather than an ignored statement: silently skipping it is how a
    /// card declares a source the runtime never fetches.
    pub fn project_declarations(source: &str) -> ProjectionReport {
        let mut sink = Diagnostics::default();
        let Some(tokens) = lex(source, &mut sink) else {
            return ProjectionReport {
                sources: Vec::new(),
                states: Vec::new(),
                events: Vec::new(),
                copies: Vec::new(),
                views: Vec::new(),
                components: Vec::new(),
                diagnostics: sink.items,
            };
        };

        let mut p = Projector {
            t: &tokens,
            at: 0,
            sink: &mut sink,
        };
        let mut sources = Vec::new();
        let mut states = Vec::new();
        let mut events = Vec::new();
        let mut copies = Vec::new();

        let mut views = Vec::new();
        let mut components = Vec::new();

        while p.at < p.t.len() {
            if p.eat_ident("let") {
                match p.let_binding() {
                    Some(Recognised::Source(d)) => sources.push(d),
                    Some(Recognised::State(d)) => states.push(d),
                    Some(Recognised::Event(d)) => events.push(d),
                    Some(Recognised::Copy(d)) => copies.push(d),
                    None => {}
                }
                continue;
            }
            if p.eat_ident("fn") {
                // Capitalised is a component, lowercase a view — the same
                // convention the bespoke surface already uses (`component
                // StoryRow`, `view stream`), so a card reads the same either way.
                match p.function() {
                    Some(Declared::AsView(v)) => views.push(v),
                    Some(Declared::AsComponent(c)) => components.push(c),
                    None => {}
                }
                continue;
            }
            p.at += 1;
        }

        ProjectionReport {
            sources,
            states,
            events,
            copies,
            views,
            components,
            diagnostics: sink.items,
        }
    }

    /// What the projector recognised. Names are exposed for tests and tooling;
    /// the declarations themselves stay crate-private, like the bespoke
    /// surface's, so the two cannot drift into different public shapes.
    pub struct ProjectionReport {
        pub(super) sources: Vec<SourceDecl>,
        pub(super) states: Vec<StateDecl>,
        pub(super) events: Vec<EventDecl>,
        pub(super) copies: Vec<CopyDecl>,
        pub(super) views: Vec<View>,
        pub(super) components: Vec<Component>,
        pub diagnostics: Vec<super::SyntaxDiagnostic>,
    }

    impl ProjectionReport {
        pub(super) fn from_card(
            card: super::Card,
            diagnostics: Vec<super::SyntaxDiagnostic>,
        ) -> Self {
            Self {
                sources: card.sources,
                states: card.states,
                events: card.events,
                copies: card.copies,
                views: card.views,
                components: card.components,
                diagnostics,
            }
        }

        /// The projected declarations as a `Card`, so the shared validator can
        /// run over them.
        pub(super) fn into_card(self) -> super::Card {
            super::Card {
                sources: self.sources,
                states: self.states,
                events: self.events,
                copies: self.copies,
                views: self.views,
                components: self.components,
            }
        }

        pub fn is_clean(&self) -> bool {
            self.diagnostics.is_empty()
        }
        pub fn source_names(&self) -> Vec<String> {
            self.sources.iter().map(|d| d.name.clone()).collect()
        }
        pub fn source_helpers(&self) -> Vec<String> {
            self.sources.iter().map(|d| d.helper.clone()).collect()
        }
        pub fn state_names(&self) -> Vec<String> {
            self.states.iter().map(|d| d.path.clone()).collect()
        }
        pub fn event_names(&self) -> Vec<String> {
            self.events.iter().map(|d| d.name.clone()).collect()
        }
        pub fn copy_names(&self) -> Vec<String> {
            self.copies.iter().map(|d| d.name.clone()).collect()
        }
        pub fn view_names(&self) -> Vec<String> {
            self.views.iter().map(|v| v.name.clone()).collect()
        }
        pub fn component_names(&self) -> Vec<String> {
            self.components.iter().map(|c| c.name.clone()).collect()
        }
        /// The projected tree of a view, as `Kind(child, child)`, for tests
        /// that must compare structure rather than eyeball it.
        pub fn view_shape(&self, name: &str) -> Option<String> {
            self.views
                .iter()
                .find(|v| v.name == name)
                .map(|v| super::shape_of(&v.body))
        }
        /// A component's props with their declared shapes, in order.
        pub fn prop_shapes(&self, name: &str) -> Vec<(String, String)> {
            self.components
                .iter()
                .find(|c| c.name == name)
                .map(|c| {
                    c.params
                        .iter()
                        .map(|p| (p.name.clone(), super::shape_spelling(&p.shape).to_string()))
                        .collect()
                })
                .unwrap_or_default()
        }
        pub fn component_shape(&self, name: &str) -> Option<String> {
            self.components
                .iter()
                .find(|c| c.name == name)
                .map(|c| super::shape_of(&c.body))
        }
    }

    struct Projector<'a> {
        t: &'a [Token],
        at: usize,
        sink: &'a mut Diagnostics,
    }

    impl Projector<'_> {
        fn eat_ident(&mut self, want: &str) -> bool {
            let hit = self
                .t
                .get(self.at)
                .is_some_and(|t| t.kind == super::Kind::Ident && t.text == want);
            if hit {
                self.at += 1;
            }
            hit
        }

        fn eat_punct(&mut self, want: &str) -> bool {
            let hit = self.t.get(self.at).is_some_and(|t| t.is_punct(want));
            if hit {
                self.at += 1;
            }
            hit
        }

        fn ident(&mut self) -> Option<String> {
            let t = self.t.get(self.at)?;
            (t.kind == super::Kind::Ident).then(|| {
                self.at += 1;
                t.text.clone()
            })
        }

        fn string(&mut self) -> Option<String> {
            let t = self.t.get(self.at)?;
            (t.kind == super::Kind::Str).then(|| {
                self.at += 1;
                t.text.clone()
            })
        }

        fn line(&self) -> usize {
            self.t.get(self.at).map(|t| t.line).unwrap_or(1)
        }

        fn reject(&mut self, message: String) {
            let line = self.line();
            let column = self.t.get(self.at).map(|t| t.column).unwrap_or(1);
            self.sink.push(line, column, message);
        }

        /// `let <name> = <recognised-form>(…)`.
        ///
        /// The name and `=` are consumed before dispatch, so an unrecognised
        /// form reports what it saw rather than "unexpected token".
        fn let_binding(&mut self) -> Option<Recognised> {
            let line = self.line();
            let name = self.ident()?;
            if !self.eat_punct("=") {
                return None;
            }
            let form = match self.ident() {
                Some(f) => f,
                None => {
                    self.reject(format!(
                        "`let {name}` must bind one of source(…), state(…), event(…) or \
                         copy(…) — L0 admits no other binding (profile §2)"
                    ));
                    return None;
                }
            };
            if !self.eat_punct("(") {
                self.reject(format!("expected `(` after {form:?}"));
                return None;
            }
            match form.as_str() {
                "source" => self.source(name, line).map(Recognised::Source),
                "state" => self.state(name, line).map(Recognised::State),
                "event" => self.event(name).map(Recognised::Event),
                "copy" => self.copy(name).map(Recognised::Copy),
                other => {
                    self.reject(format!(
                        "{other:?} is not an L0 declaration; L0 admits source, state, event \
                         and copy, and nothing else"
                    ));
                    None
                }
            }
        }

        /// `source("sys.news", {count: 1, fields: ["id", "title"]})`
        fn source(&mut self, name: String, line: usize) -> Option<SourceDecl> {
            let Some(helper) = self.string() else {
                self.reject(
                    "a source names its capability as a string: source(\"sys.news\", {…})".into(),
                );
                return None;
            };
            let mut args = Vec::new();
            if self.eat_punct(",") && self.eat_punct("{") {
                while !self.eat_punct("}") {
                    if self.at >= self.t.len() {
                        break;
                    }
                    let Some(key) = self.ident() else {
                        self.at += 1;
                        continue;
                    };
                    if !self.eat_punct(":") {
                        continue;
                    }
                    if let Some(value) = self.source_arg() {
                        args.push((key, value));
                    }
                    let _ = self.eat_punct(",");
                }
            }
            let _ = self.eat_punct(")");
            Some(SourceDecl {
                name,
                helper,
                args,
                line,
            })
        }

        fn source_arg(&mut self) -> Option<SourceArg> {
            let t = self.t.get(self.at)?.clone();
            match t.kind {
                super::Kind::Str => {
                    self.at += 1;
                    Some(SourceArg::Text(t.text))
                }
                super::Kind::Num => {
                    self.at += 1;
                    Some(SourceArg::Number(t.text.parse().unwrap_or(0.0)))
                }
                super::Kind::Ident => {
                    // A dotted path: `st.selected`, `place.lat`.
                    let mut path = t.text.clone();
                    self.at += 1;
                    while self.t.get(self.at).is_some_and(|n| n.is_punct("."))
                        && self
                            .t
                            .get(self.at + 1)
                            .is_some_and(|n| n.kind == super::Kind::Ident)
                    {
                        path.push('.');
                        path.push_str(&self.t[self.at + 1].text);
                        self.at += 2;
                    }
                    Some(SourceArg::Path(path))
                }
                _ if t.is_punct("[") => {
                    self.at += 1;
                    let mut fields = Vec::new();
                    while !self.eat_punct("]") {
                        if self.at >= self.t.len() {
                            break;
                        }
                        if let Some(f) = self.string().or_else(|| self.ident()) {
                            fields.push(f);
                        } else {
                            self.at += 1;
                        }
                        let _ = self.eat_punct(",");
                    }
                    Some(SourceArg::List(fields))
                }
                _ => {
                    self.reject(
                        "a source argument is a string, number, path or list of fields".into(),
                    );
                    self.at += 1;
                    None
                }
            }
        }

        /// `state("text", "")` / `state("enum", ["c", "f"], "c")`
        fn state(&mut self, path: String, _line: usize) -> Option<StateDecl> {
            let Some(shape_name) = self.string() else {
                self.reject("a state names its shape as a string: state(\"text\", \"\")".into());
                return None;
            };
            let mut members = Vec::new();
            if shape_name == "enum" {
                let _ = self.eat_punct(",");
                if self.eat_punct("[") {
                    while !self.eat_punct("]") {
                        if self.at >= self.t.len() {
                            break;
                        }
                        if let Some(m) = self.string().or_else(|| self.ident()) {
                            members.push(m);
                        } else {
                            self.at += 1;
                        }
                        let _ = self.eat_punct(",");
                    }
                }
            }
            let shape = match shape_name.as_str() {
                "text" => Shape::Text,
                "number" => Shape::Number,
                "bool" => Shape::Bool,
                "record" => Shape::Record,
                "collection" => Shape::Collection,
                "event" => Shape::Event,
                "enum" => Shape::Enum(members),
                other => {
                    self.reject(format!(
                        "{other:?} is not an L0 shape; L0 has text, number, bool, enum, \
                         record, collection and event"
                    ));
                    Shape::Other
                }
            };

            let _ = self.eat_punct(",");
            let initial = self.t.get(self.at).cloned().and_then(|t| match t.kind {
                super::Kind::Str => {
                    self.at += 1;
                    Some(crate::JsonValue::String(t.text))
                }
                super::Kind::Num => {
                    self.at += 1;
                    t.text.parse::<f64>().ok().map(crate::JsonValue::from)
                }
                super::Kind::Ident if t.text == "true" || t.text == "false" => {
                    self.at += 1;
                    Some(crate::JsonValue::Bool(t.text == "true"))
                }
                _ => None,
            });
            let _ = self.eat_punct(")");
            Some(StateDecl {
                path,
                shape,
                initial,
                initial_path: None,
                keep: false,
            })
        }

        /// `event({selected: set_payload()})` — a batch of total forms.
        fn event(&mut self, name: String) -> Option<EventDecl> {
            let mut transitions = Vec::new();
            if self.eat_punct("{") {
                while !self.eat_punct("}") {
                    if self.at >= self.t.len() {
                        break;
                    }
                    let line = self.line();
                    let column = self.t.get(self.at).map(|t| t.column).unwrap_or(1);
                    let Some(target) = self.ident() else {
                        self.at += 1;
                        continue;
                    };
                    if !self.eat_punct(":") {
                        continue;
                    }
                    let Some(form_name) = self.ident() else {
                        self.reject("a transition names a total form".into());
                        continue;
                    };
                    let _ = self.eat_punct("(");
                    let (form, tokens) = match form_name.as_str() {
                        "toggle" => (super::Form::Toggle, Vec::new()),
                        "clear" => (super::Form::Clear, Vec::new()),
                        "set_payload" => (super::Form::Set(super::SetSource::Payload), Vec::new()),
                        "set" => {
                            let src = match self.t.get(self.at).cloned() {
                                Some(t) if t.kind == super::Kind::Str => {
                                    self.at += 1;
                                    super::SetSource::Text(t.text)
                                }
                                Some(t) if t.kind == super::Kind::Num => {
                                    self.at += 1;
                                    super::SetSource::Num(t.text.parse().unwrap_or(0.0))
                                }
                                Some(t) if t.kind == super::Kind::Ident => {
                                    self.at += 1;
                                    super::SetSource::Path(t.text)
                                }
                                _ => super::SetSource::Payload,
                            };
                            (super::Form::Set(src), Vec::new())
                        }
                        "cycle" => {
                            let mut members = Vec::new();
                            while !self.t.get(self.at).is_some_and(|t| t.is_punct(")")) {
                                if self.at >= self.t.len() {
                                    break;
                                }
                                if let Some(m) = self.string().or_else(|| self.ident()) {
                                    members.push(m);
                                } else {
                                    self.at += 1;
                                }
                                let _ = self.eat_punct(",");
                            }
                            (super::Form::Cycle, members)
                        }
                        other => {
                            self.reject(format!(
                                "{other:?} is not a total form; L0 admits set, set_payload, \
                                 toggle, cycle and clear (profile §3)"
                            ));
                            (super::Form::NotTotal(other.to_string()), Vec::new())
                        }
                    };
                    let _ = self.eat_punct(")");
                    let _ = self.eat_punct(",");
                    transitions.push(super::Transition {
                        target,
                        form,
                        tokens,
                        line,
                        column,
                    });
                }
            }
            let _ = self.eat_punct(")");
            Some(EventDecl { name, transitions })
        }

        /// `fn name() { return <element> }` — a view, or a component when the
        /// name is capitalised and it takes parameters.
        fn function(&mut self) -> Option<Declared> {
            let line = self.line();
            let name = self.ident()?;
            let mut params = Vec::new();
            if self.eat_punct("(") {
                while !self.eat_punct(")") {
                    if self.at >= self.t.len() {
                        break;
                    }
                    if let Some(p) = self.ident() {
                        // A Splash `fn` parameter carries no type, so the shape
                        // is unknown here. The bespoke surface declares one, and
                        // §5.2's checks depend on it — see the note in
                        // `projection` about what this surface cannot yet say.
                        params.push(Param {
                            name: p,
                            shape: Shape::Other,
                            default: None,
                        });
                    } else {
                        self.at += 1;
                    }
                    let _ = self.eat_punct(",");
                }
            }
            if !self.eat_punct("{") {
                self.reject(format!("`fn {name}` needs a body"));
                return None;
            }
            // `props({...})` is the ONE statement admitted before the return.
            // §5.2's checks — event-vs-value, enum membership, defaults, the
            // readable/event scope split — all consult a prop's declared shape,
            // and a Splash `fn` parameter carries none. Declaring it beats
            // inferring it from a name: `online_count` starts with `on`.
            let declared = if self.peek_ident("props") {
                self.at += 1;
                Some(self.props())
            } else {
                None
            };

            if !self.eat_ident("return") {
                self.reject(format!(
                    "`fn {name}` must be a single `return <element>`, optionally preceded by \
                     `props(…)`; L0 admits no other statement in a view (profile §2)"
                ));
                return None;
            }
            let body = self.element()?;
            let _ = self.eat_punct("}");

            // Reconcile. The bespoke surface cannot have this check, because
            // there the parameter list IS the declaration; here they are
            // separate, so redundancy that drifts must become redundancy that
            // is checked.
            if !params.is_empty() {
                match declared {
                    None => self.reject(format!(
                        "`fn {name}` takes parameters but declares no `props(…)`; a prop with \
                         no shape cannot be checked against profile §5.2"
                    )),
                    Some(shapes) => {
                        for p in &params {
                            if !shapes.iter().any(|(n, _)| *n == p.name) {
                                self.reject(format!(
                                    "`{name}` declares no shape for parameter {:?}",
                                    p.name
                                ));
                            }
                        }
                        for (n, _) in &shapes {
                            if !params.iter().any(|p| p.name == *n) {
                                self.reject(format!(
                                    "`{name}` declares a shape for {n:?}, which is not one of \
                                     its parameters"
                                ));
                            }
                        }
                        for p in &mut params {
                            if let Some((_, shape)) = shapes.iter().find(|(n, _)| *n == p.name) {
                                p.shape = shape.clone();
                            }
                        }
                    }
                }
            }

            let capitalised = name.chars().next().is_some_and(|c| c.is_uppercase());
            if capitalised {
                let mut instantiates = Vec::new();
                collect_constructors(&body, &mut instantiates);
                Some(Declared::AsComponent(Component {
                    name,
                    params,
                    states: Vec::new(),
                    events: Vec::new(),
                    body,
                    line,
                    instantiates,
                }))
            } else {
                Some(Declared::AsView(View { name, body }))
            }
        }

        fn peek_ident(&self, want: &str) -> bool {
            self.t
                .get(self.at)
                .is_some_and(|t| t.kind == super::Kind::Ident && t.text == want)
        }

        /// `props({story: "record", on_open: "event"})`
        fn props(&mut self) -> Vec<(String, Shape)> {
            let mut out = Vec::new();
            if !self.eat_punct("(") || !self.eat_punct("{") {
                self.reject("props takes an object: props({name: \"shape\", …})".into());
                return out;
            }
            while !self.eat_punct("}") {
                if self.at >= self.t.len() {
                    break;
                }
                let Some(name) = self.ident() else {
                    self.at += 1;
                    continue;
                };
                if !self.eat_punct(":") {
                    continue;
                }
                let shape = match self.string().as_deref() {
                    Some("text") => Shape::Text,
                    Some("number") => Shape::Number,
                    Some("bool") => Shape::Bool,
                    Some("record") => Shape::Record,
                    Some("collection") => Shape::Collection,
                    Some("event") => Shape::Event,
                    Some(other) => {
                        self.reject(format!(
                            "{other:?} is not an L0 shape; L0 has text, number, bool, enum, \
                             record, collection and event"
                        ));
                        Shape::Other
                    }
                    None => {
                        self.reject(format!("prop {name:?} declares no shape"));
                        Shape::Other
                    }
                };
                out.push((name, shape));
                let _ = self.eat_punct(",");
            }
            let _ = self.eat_punct(")");
            out
        }

        /// An element: `Ctor({args}, [children])`, with `{args}` and
        /// `[children]` each optional, plus the three structural forms.
        fn element(&mut self) -> Option<Element> {
            let line = self.line();
            let column = self.t.get(self.at).map(|t| t.column).unwrap_or(1);
            let name = self.ident()?;

            // Structural forms first — they are recognised by name, so a card
            // cannot reach them by accident and cannot spell them another way.
            match name.as_str() {
                "each" => return self.each(line, column),
                "when_is" | "when_not" => return self.guard(&name, line, column),
                _ => {}
            }

            if !self.eat_punct("(") {
                // A bare name is a reference to another view.
                return Some(Element {
                    name,
                    line,
                    column,
                    is_reference: true,
                    ..Default::default()
                });
            }

            let mut args = Vec::new();
            let mut children = Vec::new();

            // `{...}` arguments, then `[...]` children, in that order.
            if self.eat_punct("{") {
                while !self.eat_punct("}") {
                    if self.at >= self.t.len() {
                        break;
                    }
                    let aline = self.line();
                    let acol = self.t.get(self.at).map(|t| t.column).unwrap_or(1);
                    let Some(key) = self.ident() else {
                        self.at += 1;
                        continue;
                    };
                    if !self.eat_punct(":") {
                        continue;
                    }
                    if let Some(value) = self.operand() {
                        args.push(Arg {
                            name: key,
                            value,
                            line: aline,
                            column: acol,
                        });
                    }
                    let _ = self.eat_punct(",");
                }
                let _ = self.eat_punct(",");
            }
            if self.eat_punct("[") {
                while !self.eat_punct("]") {
                    if self.at >= self.t.len() {
                        break;
                    }
                    match self.element() {
                        Some(child) => children.push(child),
                        None => self.at += 1,
                    }
                    let _ = self.eat_punct(",");
                }
            }
            let _ = self.eat_punct(")");

            // A capitalised call with no catalogue entry is a component
            // instantiation; the validator decides which, exactly as it does on
            // the bespoke surface.
            Some(Element {
                name,
                args,
                children,
                line,
                column,
                ..Default::default()
            })
        }

        /// `each(feed, "id", StoryRow)` — collection, key field, body component.
        fn each(&mut self, line: usize, column: usize) -> Option<Element> {
            if !self.eat_punct("(") {
                return None;
            }
            let path = self.dotted()?;
            let _ = self.eat_punct(",");
            let Some(key_field) = self.string() else {
                self.reject("each names its key field as a string: each(xs, \"id\", Row)".into());
                return None;
            };
            let _ = self.eat_punct(",");
            let body = self.element()?;
            let _ = self.eat_punct(")");

            // The binder is implicit and named for the body it feeds, so a key
            // path can be reconstructed the way the bespoke surface writes it.
            let binder = "it".to_string();
            Some(Element {
                name: "for".into(),
                args: vec![Arg {
                    name: "collection".into(),
                    value: Operand::Path(path),
                    line,
                    column,
                }],
                children: vec![body],
                binders: vec![binder.clone(), "i".into()],
                key_path: Some(format!("{binder}.{key_field}")),
                line,
                column,
                ..Default::default()
            })
        }

        /// `when_is(path, "value", child)` / `when_not(...)`
        fn guard(&mut self, form: &str, line: usize, column: usize) -> Option<Element> {
            if !self.eat_punct("(") {
                return None;
            }
            let path = self.dotted()?;
            let _ = self.eat_punct(",");
            let rhs = self.operand();
            let _ = self.eat_punct(",");
            let child = self.element()?;
            let _ = self.eat_punct(")");
            Some(Element {
                name: "when".into(),
                args: vec![Arg {
                    name: "predicate".into(),
                    value: Operand::Path(path),
                    line,
                    column,
                }],
                cmp: Some(if form == "when_is" { "==" } else { "!=" }.into()),
                rhs,
                children: vec![child],
                line,
                column,
                ..Default::default()
            })
        }

        fn dotted(&mut self) -> Option<String> {
            let mut path = self.ident()?;
            while self.t.get(self.at).is_some_and(|n| n.is_punct("."))
                && self
                    .t
                    .get(self.at + 1)
                    .is_some_and(|n| n.kind == super::Kind::Ident)
            {
                path.push('.');
                path.push_str(&self.t[self.at + 1].text);
                self.at += 2;
            }
            Some(path)
        }

        /// An argument value. A string beginning with `.` is a TOKEN, keeping
        /// §2.2's lexical marking on a surface whose carrier has no token form —
        /// the identity codex flagged as lossy if both spell as plain strings.
        fn operand(&mut self) -> Option<Operand> {
            let t = self.t.get(self.at)?.clone();
            match t.kind {
                super::Kind::Str => {
                    self.at += 1;
                    Some(match t.text.strip_prefix('.') {
                        Some(bare) => Operand::Token(bare.to_string()),
                        None => Operand::Str(t.text),
                    })
                }
                super::Kind::Num => {
                    self.at += 1;
                    Some(Operand::Num(t.text.parse().unwrap_or(0.0)))
                }
                super::Kind::Ident => self.dotted().map(Operand::Path),
                _ => None,
            }
        }

        /// `copy("vocabulary", {en: "pts", zh: "分"})`
        fn copy(&mut self, name: String) -> Option<CopyDecl> {
            let provenance = match self.string().as_deref() {
                Some("vocabulary") => Provenance::Vocabulary,
                Some("user-copy") => Provenance::UserCopy,
                Some("model-copy") => Provenance::ModelCopy,
                Some(other) => {
                    self.reject(format!(
                        "{other:?} is not a copy class; L0 has vocabulary, user-copy and \
                         model-copy (profile §4)"
                    ));
                    Provenance::ModelCopy
                }
                None => {
                    // Absent provenance is NOT vocabulary — §4 fails closed.
                    self.reject(format!(
                        "copy {name:?} declares no class; L0 needs one of vocabulary, \
                         user-copy or model-copy (profile §4)"
                    ));
                    Provenance::ModelCopy
                }
            };
            let mut locales = Vec::new();
            if self.eat_punct(",") && self.eat_punct("{") {
                while !self.eat_punct("}") {
                    if self.at >= self.t.len() {
                        break;
                    }
                    let Some(lang) = self.ident() else {
                        self.at += 1;
                        continue;
                    };
                    if !self.eat_punct(":") {
                        continue;
                    }
                    if let Some(text) = self.string() {
                        locales.push((lang, text));
                    }
                    let _ = self.eat_punct(",");
                }
            }
            let _ = self.eat_punct(")");
            Some(CopyDecl {
                name,
                provenance,
                locales,
            })
        }
    }
}

// ─────────────────────────────────────────────────────────── DSL lowering (§1.1) ──

/// Lower a realized card to **Splash DSL**, the data form `splash-render`
/// evaluates into a `UiNode` and every backend renders from.
///
/// This is what §1.1 requires and what `makepad::lower` does not do. That one
/// emits makepad's widget dialect with ten hardcoded colours and a font-size
/// ramp, which reaches one backend of three and puts a theme inside the crate
/// whose job is deciding whether a card is safe.
///
/// The split §1.1 settles:
///
/// - **16 roles** become an existing node kind carrying their data. The seven
///   text roles collapse onto `text` with a `role:` attribute, because a hero
///   and a caption differ in *presentation* and that is the theme's to decide.
/// - **6 roles** keep their own kind. They are fragment shaders parameterised by
///   data and cannot be composed from boxes and text, so each backend implements
///   them — the category `splash-render` already has for `Map`.
/// - **`Photo`** is an image.
///
/// No colour, size, weight or radius is emitted. If one appears here, §1.1 has
/// been broken and `lowering_emits_splash_dsl_not_a_backend_dialect` will say so.
pub fn lower_dsl(root: &UiNode) -> String {
    let mut out = String::new();
    out.push_str("// REALIZED from an L0 ledger — do not edit.\n");
    emit(root, 0, &mut out);
    out.push('\n');
    out
}

/// The node kind a role lowers to, and the role name it carries when several
/// roles share a kind.
fn kind_of(role: &str) -> (&'static str, Option<&'static str>) {
    match role {
        // ── containers and structure ────────────────────────────────────────
        "Surface" => ("column", Some("surface")),
        "Panel" => ("card", Some("panel")),
        "Card" => ("card", Some("elevated")),
        "Col" => ("column", None),
        "Row" => ("row", None),
        "Grid" => ("grid", None),
        "Rule" => ("divider", None),
        "Tile" => ("listitem", None),
        "Chip" => ("chip", None),
        "Photo" => ("image", None),

        // ── text roles: one kind, seven roles ───────────────────────────────
        "TextHero" => ("text", Some("hero")),
        "TextTitle" => ("text", Some("title")),
        "TextBody" => ("text", Some("body")),
        "TextStat" => ("text", Some("stat")),
        "TextRow" => ("text", Some("row")),
        "TextCaption" => ("text", Some("caption")),
        "TextValue" => ("text", Some("value")),

        // ── data visualisations: backend widgets (§1.1) ─────────────────────
        "WeatherIcon" => ("weathericon", None),
        "TempBar" => ("tempbar", None),
        "SunArc" => ("sunarc", None),
        "MoonPhase" => ("moonphase", None),
        "AqiContour" => ("aqicontour", None),
        "StockPlot" => ("stockplot", None),

        // An unmapped role is emitted as itself. A backend that does not know it
        // should fail visibly; silently dropping a section is how a card looks
        // finished and is not.
        other => (Box::leak(other.to_lowercase().into_boxed_str()), None),
    }
}

fn emit(node: &UiNode, depth: usize, out: &mut String) {
    use std::fmt::Write as _;
    let pad = "  ".repeat(depth);
    let (kind, role) = kind_of(&node.kind);

    let _ = write!(out, "{pad}{{t: {kind:?}");
    if let Some(role) = role {
        let _ = write!(out, ", role: {role:?}");
    }

    for (name, value) in &node.args {
        // `on_tap` and its payload are interaction, not presentation, and the
        // key is what lets a tap name WHICH instance was hit.
        let _ = match value {
            NodeValue::Text(s) => write!(out, ", {name}: {s:?}"),
            NodeValue::Number(n) => write!(out, ", {name}: {}", makepad::trim_num(*n)),
            NodeValue::Bool(b) => write!(out, ", {name}: {b}"),
            NodeValue::Token(t) => write!(out, ", {name}: {t:?}"),
            NodeValue::Event(e) => write!(out, ", {name}: {e:?}"),
            NodeValue::Status(s) => write!(out, ", {name}: {:?}", s.as_token()),
            // An unresolved binding is emitted as such rather than as an em
            // dash: the theme decides how absence looks, and a dash baked in
            // here would be presentation.
            NodeValue::Missing => write!(out, ", {name}: null"),
        };
    }
    if !node.key.is_empty() {
        let _ = write!(out, ", key: {:?}", node.key);
    }

    if node.children.is_empty() {
        out.push_str("}\n");
        return;
    }
    out.push_str(", c: [\n");
    for child in &node.children {
        emit(child, depth + 1, out);
    }
    let _ = writeln!(out, "{pad}]}}");
}
