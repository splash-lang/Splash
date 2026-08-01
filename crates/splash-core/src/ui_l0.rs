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
            header.ledger = Some(name);
            header.version = version;
        } else if let Some((key, value)) = rest.split_once(':') {
            let value = value.trim().to_string();
            match key.trim() {
                "level" => {
                    saw_any = true;
                    header.level = Some(value);
                }
                "profile" => {
                    saw_any = true;
                    header.profile = Some(value);
                }
                "model" => header.model = Some(value),
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
        return sink.into_report(level);
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
        acc.push_str(p);
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
        // `-` is only rejected between values; a negative literal is fine.
        if t.is_punct("-")
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
    copies: Vec<(String, Vec<(String, String)>)>,
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
    Bool,
    Enum(Vec<String>),
    Other,
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
    params: Vec<String>,
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
                        card.copies.push((n, self.parse_locale_map()));
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
                self.at += 1;
                // The index is itself a path and must be RETAINED, not just
                // recognised: dropping it turned `stories[selected].title` into
                // `stories.title`, which resolved to nothing and rendered an
                // em dash. Held as a `[…]` segment so lookup can resolve the
                // inner path and use its value as the key.
                if let Some(index) = self.dotted_name() {
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
        // `{ shape: enum[a, b], initial: .a }` — capture the shape, skip the rest.
        let start = self.at;
        if self.peek().is_some_and(|t| t.is_punct("{")) {
            let mut scan = self.at;
            while let Some(t) = self.tokens.get(scan) {
                if t.is_punct("}") {
                    break;
                }
                if t.is_kw("bool") {
                    shape = Shape::Bool;
                }
                // `initial: <literal>` — a token, string or number.
                if t.is_kw("initial") && self.tokens.get(scan + 1).is_some_and(|n| n.is_punct(":"))
                {
                    if let Some(v) = self.tokens.get(scan + 2) {
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
                                // A path. Collect its dotted segments from the
                                // token stream; it resolves at realization.
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
                if t.is_kw("keep") && self.tokens.get(scan + 1).is_some_and(|n| n.is_punct(":")) {
                    keep = self
                        .tokens
                        .get(scan + 2)
                        .is_some_and(|v| v.kind == Kind::Ident && v.text == "true");
                }
                if t.is_kw("enum") {
                    let mut members = Vec::new();
                    let mut j = scan + 1;
                    if self.tokens.get(j).is_some_and(|t| t.is_punct("[")) {
                        j += 1;
                        while let Some(m) = self.tokens.get(j) {
                            if m.is_punct("]") {
                                break;
                            }
                            if m.kind == Kind::Ident {
                                members.push(m.text.clone());
                            }
                            j += 1;
                        }
                    }
                    shape = Shape::Enum(members);
                }
                scan += 1;
            }
            self.at = start;
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
    fn parse_locale_map(&mut self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if !self.eat_punct("{") {
            return out;
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
                    if v.kind == Kind::Str {
                        out.push((t.text.clone(), v.text.clone()));
                    }
                }
            }
            self.at += 1;
        }
        self.expect_punct("}");
        out
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
                        params.push(t.text.clone());
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
                        self.at += 1;
                        let rhs = self.parse_operand();
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
    // Components must not form a cycle — profile §6, condition 1. Without this
    // an L0 realization need not terminate.
    check_component_acyclicity(card, sink);
    validate_transitions(card, sink);
    validate_sources(card, sink);

    let view_names: Vec<&str> = card.views.iter().map(|v| v.name.as_str()).collect();
    let component_names: Vec<&str> = card.components.iter().map(|c| c.name.as_str()).collect();

    for view in &card.views {
        let mut scope = Scope::card(card);
        walk(
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
        sink,
    );
    for component in &card.components {
        let mut readable: Vec<String> = card.sources.iter().map(|s| root_of(&s.name)).collect();
        readable.push("copy".into());
        readable.extend(component.params.iter().cloned());
        readable.extend(component.states.iter().map(|s| root_of(&s.path)));
        let events: Vec<String> = component.events.iter().map(|e| e.name.clone()).collect();
        check_event_batch(
            &component.events,
            &component.states,
            &readable,
            &events,
            sink,
        );
    }
}

fn check_event_batch(
    events: &[EventDecl],
    states: &[StateDecl],
    readable: &[String],
    event_names: &[String],
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
                Form::Set(SetSource::Path(path)) => {
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
        for (name, _) in &source.args {
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
    events: Vec<String>,
    /// Card state, tracked separately so reading it from a component is a
    /// specific diagnostic rather than "unknown name".
    forbidden_state: Vec<String>,
}

impl Scope {
    fn card(card: &Card) -> Self {
        let mut roots: Vec<String> = Vec::new();
        roots.extend(card.sources.iter().map(|s| root_of(&s.name)));
        roots.extend(card.states.iter().map(|s| root_of(&s.path)));
        roots.push("copy".into());
        let events = card.events.iter().map(|e| e.name.clone()).collect();
        Self {
            roots,
            source_roots: card.sources.iter().map(|s| root_of(&s.name)).collect(),
            events,
            forbidden_state: Vec::new(),
        }
    }

    fn component(card: &Card, component: &Component) -> Self {
        let mut roots: Vec<String> = Vec::new();
        roots.extend(card.sources.iter().map(|s| root_of(&s.name)));
        roots.push("copy".into());
        roots.extend(component.params.iter().cloned());
        roots.extend(component.states.iter().map(|s| root_of(&s.path)));
        let mut events: Vec<String> = component.events.iter().map(|e| e.name.clone()).collect();
        // Events arrive as props too — §5.4, outputs are event props.
        events.extend(component.params.iter().cloned());
        Self {
            roots,
            source_roots: card.sources.iter().map(|s| root_of(&s.name)).collect(),
            events,
            forbidden_state: card.states.iter().map(|s| root_of(&s.path)).collect(),
        }
    }

    fn knows(&self, root: &str) -> bool {
        self.roots.iter().any(|r| r == root)
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
                check_arg(name, arg, known, is_component, scope, card, sink);
            }
        }
    }

    for child in &element.children {
        walk(child, scope, views, components, card, sink);
    }

    if element.name == "for" {
        for _ in &element.binders {
            scope.roots.pop();
        }
    }
}

fn check_arg(
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
        None if is_component => None,
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

fn check_path(path: &str, scope: &Scope, line: usize, column: usize, sink: &mut Diagnostics) {
    if path.is_empty() {
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

    if scope.knows(&root) {
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
    copies: &'a [(String, Vec<(String, String)>)],
}

impl ValueScope<'_> {
    fn lookup(&self, path: &str) -> Option<crate::JsonValue> {
        let mut segments = path.split('.');
        let root = segments.next()?;

        // `<source>.$state` is the lifecycle the runtime reported. It is data
        // like any other, so a guard can branch on it without an expression.
        if let Some(source) = path.strip_suffix(".$state") {
            let status = self
                .data
                .get("$status")
                .and_then(|m| m.get(source))
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| {
                    // No report means: resolved if there is a value, pending if
                    // not. A host that tracks status explicitly overrides this.
                    if self.data.get(source).is_some() {
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
            let entry = self.copies.iter().find(|(n, _)| n == name)?;
            let lang = self
                .data
                .get("env")
                .and_then(|e| e.get("locale"))
                .and_then(|l| l.get("lang"))
                .and_then(|v| v.as_str())
                .unwrap_or("en");
            let text = entry
                .1
                .iter()
                .find(|(k, _)| k == lang)
                .or_else(|| entry.1.iter().find(|(k, _)| k == "en"))
                .or_else(|| entry.1.iter().find(|(k, _)| k != "class"))?;
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
        let mut bound = 0usize;
        for arg in &call.args {
            if component.params.contains(&arg.name) {
                let value = match &arg.value {
                    Operand::Path(p) => scope
                        .lookup(p)
                        // An event prop has no value in data; pass the name.
                        .unwrap_or_else(|| crate::JsonValue::String(p.clone())),
                    Operand::Token(t) => crate::JsonValue::String(t.clone()),
                    // A literal prop is a value like any other. Letting these
                    // fall to Null is how `Framed(title: "Details")` rendered an
                    // em dash: the parser kept the string and the realizer threw
                    // it away.
                    Operand::Str(t) => crate::JsonValue::String(t.clone()),
                    Operand::Num(n) => crate::JsonValue::from(*n),
                    Operand::Predicate { path: p, .. } => scope
                        .lookup(p)
                        .unwrap_or_else(|| crate::JsonValue::String(p.clone())),
                };
                scope.frames.push((arg.name.clone(), value));
                bound += 1;
            }
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

    fn trim_num(n: f64) -> String {
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
fn schema_id(component: &Component) -> u64 {
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    let mut fields: Vec<String> = component
        .states
        .iter()
        .map(|s| format!("{}:{:?}", s.path, s.shape))
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
        Shape::Other => crate::JsonValue::Null,
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

    let mut applied = false;
    for transition in &declared.transitions {
        let Some(state) = states.iter().find(|s| s.path == transition.target) else {
            continue;
        };
        let current = store
            .get(instance_key, &transition.target)
            .cloned()
            .or_else(|| state.initial.clone())
            .unwrap_or_else(|| initial_for(&state.shape));

        let next = match (&transition.form, &state.shape) {
            (Form::Toggle, Shape::Bool) => {
                crate::JsonValue::Bool(!current.as_bool().unwrap_or(false))
            }
            (Form::Clear, shape) => state.initial.clone().unwrap_or_else(|| initial_for(shape)),
            (Form::Cycle, Shape::Enum(members)) if !members.is_empty() => {
                let at = current
                    .as_str()
                    .and_then(|s| members.iter().position(|m| m == s))
                    .unwrap_or(0);
                crate::JsonValue::String(members[(at + 1) % members.len()].clone())
            }
            (Form::Set(source), _) => match source {
                SetSource::Payload => match payload {
                    Some(v) => v.clone(),
                    // No payload is a no-op rather than a silent clear: falling
                    // back to the initial would look like a deliberate reset.
                    None => continue,
                },
                // A path READS. Treating it as the payload meant
                // `n: set(config.answer)` wrote whatever the tap carried — or
                // nothing at all when it carried nothing.
                SetSource::Path(p) => match data_path(data, p) {
                    Some(v) => v,
                    None => continue,
                },
                SetSource::Token(t) | SetSource::Text(t) => crate::JsonValue::String(t.clone()),
                SetSource::Num(n) => crate::JsonValue::from(*n),
                SetSource::Bool(b) => crate::JsonValue::Bool(*b),
            },
            _ => continue,
        };
        store.set(instance_key, &transition.target, next);
        applied = true;
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
                        let root = root_of(p);
                        // Only another SOURCE is a fetch dependency. A path into
                        // card state is available before any fetch begins.
                        declared.iter().find(|d| **d == root).map(|_| root)
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
        reads.retain(|r| !component.params.contains(r));
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
        reads.retain(|r| !component.params.contains(r));
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
