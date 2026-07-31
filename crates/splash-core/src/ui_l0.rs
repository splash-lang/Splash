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
    sink.into_report(Level::L0)
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
    sources: Vec<String>,
    states: Vec<StateDecl>,
    events: Vec<EventDecl>,
    copies: Vec<String>,
    components: Vec<Component>,
    views: Vec<View>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
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
#[derive(Clone, Debug, Eq, PartialEq)]
enum SetSource {
    /// `$value` — the payload from the element that raised the event.
    Payload,
    /// A `.token`, e.g. an enum member.
    Token(String),
    /// A string literal.
    Text(String),
    /// A bound path, resolved against the data the runtime injected.
    Path(String),
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
    Predicate(String),
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
                    if let Some(p) = self.dotted_name() {
                        card.sources.push(p);
                    }
                    self.skip_source_query();
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
                        card.copies.push(n);
                    }
                    self.skip_braced();
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
                // The index is itself a path; it must resolve like any other.
                let _ = self.dotted_name();
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
    fn skip_source_query(&mut self) {
        // name
        while self
            .peek()
            .is_some_and(|t| t.kind == Kind::Ident || t.is_punct("."))
        {
            self.at += 1;
        }
        if self.peek().is_some_and(|t| t.is_punct("(")) {
            self.skip_balanced("(", ")");
        }
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
                            _ => None,
                        };
                    }
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
        })
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
                                // `$value` — the payload.
                                self.at += 1;
                                let _ = self.ident();
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

        // `slot`
        if t.is_kw("slot") {
            self.at += 1;
            return Element {
                name: "slot".into(),
                line: t.line,
                column: t.column,
                ..Default::default()
            };
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
                if self.peek().is_some_and(|t| t.kind == Kind::Cmp) {
                    self.at += 1;
                    self.at += 1;
                    return Operand::Predicate(path);
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
        /// A number literal or a path; also admits a width token.
        Number,
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
                ("value", Number),
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
            &[("value", Number), ("format", Token(FORMAT)), ("tint", Path)],
        ),
        ("TextRow", &[("text", Text), ("width", TokenOrPath(WIDTH))]),
        (
            "TextCaption",
            &[
                ("text", Text),
                ("value", Number),
                ("glyph", Text),
                ("suffix", Text),
                ("unit", TokenOrPath(UNIT)),
                ("width", TokenOrPath(WIDTH)),
            ],
        ),
        (
            "TextValue",
            &[
                ("value", Number),
                ("unit", TokenOrPath(UNIT)),
                ("format", Token(FORMAT)),
                ("tint", Path),
            ],
        ),
        (
            "Tile",
            &[
                ("label", Text),
                ("value", Number),
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
}

// ──────────────────────────────────────────────────────────────────── validation ──

fn validate(card: &Card, sink: &mut Diagnostics) {
    // Components must not form a cycle — profile §6, condition 1. Without this
    // an L0 realization need not terminate.
    check_component_acyclicity(card, sink);
    validate_transitions(card, sink);

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
    check_event_batch(&card.events, &card.states, sink);
    for component in &card.components {
        check_event_batch(&component.events, &component.states, sink);
    }
}

fn check_event_batch(events: &[EventDecl], states: &[StateDecl], sink: &mut Diagnostics) {
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

fn check_component_acyclicity(card: &Card, sink: &mut Diagnostics) {
    for component in &card.components {
        let mut seen = vec![component.name.as_str()];
        let mut queue: Vec<&str> = component.instantiates.iter().map(|s| s.as_str()).collect();
        let mut steps = 0usize;
        while let Some(next) = queue.pop() {
            steps += 1;
            if steps > MAX_UI_L0_DECLARATIONS {
                break;
            }
            if next == component.name {
                sink.push(
                    component.line,
                    1,
                    format!(
                        "component {:?} is recursive; L0 requires an acyclic component graph",
                        component.name
                    ),
                );
                break;
            }
            if seen.contains(&next) {
                continue;
            }
            seen.push(next);
            if let Some(c) = card.components.iter().find(|c| c.name == next) {
                queue.extend(c.instantiates.iter().map(|s| s.as_str()));
            }
        }
    }
}

/// Names visible to one record. Profile §5.3: a component sees its props, its own
/// state and events, and card-scope `source` and `copy` — but never card `state`.
struct Scope {
    roots: Vec<String>,
    events: Vec<String>,
    /// Card state, tracked separately so reading it from a component is a
    /// specific diagnostic rather than "unknown name".
    forbidden_state: Vec<String>,
}

impl Scope {
    fn card(card: &Card) -> Self {
        let mut roots: Vec<String> = Vec::new();
        roots.extend(card.sources.iter().map(root_of));
        roots.extend(card.states.iter().map(|s| root_of(&s.path)));
        roots.push("copy".into());
        let events = card.events.iter().map(|e| e.name.clone()).collect();
        Self {
            roots,
            events,
            forbidden_state: Vec::new(),
        }
    }

    fn component(card: &Card, component: &Component) -> Self {
        let mut roots: Vec<String> = Vec::new();
        roots.extend(card.sources.iter().map(root_of));
        roots.push("copy".into());
        roots.extend(component.params.iter().cloned());
        roots.extend(component.states.iter().map(|s| root_of(&s.path)));
        let mut events: Vec<String> = component.events.iter().map(|e| e.name.clone()).collect();
        // Events arrive as props too — §5.4, outputs are event props.
        events.extend(component.params.iter().cloned());
        Self {
            roots,
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
        "" | "slot" => {}
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
        (Operand::Predicate(path), _) => check_path(path, scope, arg.line, arg.column, sink),
        _ => {}
    }

    let _ = card;
}

fn check_path(path: &str, scope: &Scope, line: usize, column: usize, sink: &mut Diagnostics) {
    if path.is_empty() {
        return;
    }
    let root = root_of(path);
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
            .unwrap_or_else(|| initial_for(&state.shape));
        frames.push((state.path.clone(), value));
    }
    let mut scope = ValueScope { frames, data };
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
}

impl ValueScope<'_> {
    fn lookup(&self, path: &str) -> Option<crate::JsonValue> {
        let mut segments = path.split('.');
        let root = segments.next()?;

        let mut current = match self.frames.iter().rev().find(|(n, _)| n == root) {
            Some((_, v)) => v.clone(),
            None => self.data.get(root)?.clone(),
        };
        for segment in segments {
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
    slots: Vec<Vec<Element>>,
}

impl Realizer<'_> {
    fn element(
        &mut self,
        element: &Element,
        scope: &mut ValueScope,
        key: &str,
        out: &mut Vec<UiNode>,
    ) {
        if self.nodes >= self.limits.max_nodes || key.len() > self.limits.max_depth * 32 {
            self.truncated = true;
            return;
        }

        match element.name.as_str() {
            "" => {}
            "slot" => {
                // Children are realized in the CALLER's scope, not the
                // component's — a child expression sees the call site's
                // bindings (§5.5). Taking the frame off the stack while
                // realizing prevents a component's own slot from re-entering.
                if let Some(children) = self.slots.pop() {
                    for (i, child) in children.iter().enumerate() {
                        self.element(child, scope, &format!("{key}/s{i}"), out);
                    }
                    self.slots.push(children);
                }
            }
            "for" => self.iteration(element, scope, key, out),
            "when" => {
                if self.guard_holds(element, scope) {
                    for (i, child) in element.children.iter().enumerate() {
                        self.element(child, scope, &format!("{key}/w{i}"), out);
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
        for (i, child) in element.children.iter().enumerate() {
            self.element(child, scope, &format!("{key}/{i}"), &mut node.children);
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
                    _ => crate::JsonValue::Null,
                };
                scope.frames.push((arg.name.clone(), value));
                bound += 1;
            }
        }
        let instance_key = format!("{key}/{}", component.name);
        self.live.push(instance_key.clone());
        self.slots.push(call.children.clone());

        // Local state resolves PER INSTANCE: from the store when it holds a
        // value for this key, otherwise from the declared initial. Two instances
        // of one component therefore never share a cell — the property the
        // component model exists to provide.
        let schema = schema_id(component);
        for state in &component.states {
            let live = self
                .store
                .filter(|s| s.schemas.get(&component.name).is_none_or(|k| *k == schema))
                .and_then(|s| s.get(&instance_key, &state.path))
                .cloned();
            scope.frames.push((
                state.path.clone(),
                live.or_else(|| state.initial.clone())
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

            for (i, child) in element.children.iter().enumerate() {
                self.element(child, scope, &format!("{key}[{item_key}]/{i}"), out);
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
        let right = match element.rhs.as_ref() {
            Some(Operand::Path(p)) => scope.lookup(p),
            Some(Operand::Token(t)) => Some(crate::JsonValue::String(
                t.trim_start_matches('.').to_string(),
            )),
            Some(Operand::Str(s)) => Some(crate::JsonValue::String(s.clone())),
            Some(Operand::Num(n)) => crate::JsonValue::from(*n).into(),
            _ => None,
        };

        // A comparison against a literal the parser did not retain cannot be
        // decided here; treat it as false rather than guess.
        let (Some(left), Some(right)) = (left, right) else {
            return cmp == "!=";
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
            Operand::Predicate(p) | Operand::Path(p) => match scope.lookup(p) {
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
                format!("{:?}", format!("{glyph}{body}{unit}"))
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
                    "{p}View{{ width: Fill height: Fit flow: Right align: Align{{y: 0.5}} spacing: 6"
                );
                children(node, depth, out);
                let _ = writeln!(out, "{p}}}");
            }
            "Panel" | "Card" => {
                let _ = writeln!(
                    out,
                    "{p}RoundedView{{ width: Fill height: Fit flow: Down new_batch: true \
                     draw_bg.color: {PANEL} draw_bg.border_radius: 14.0 \
                     margin: Inset{{top: 16}} padding: Inset{{left: 14 right: 14 top: 12 bottom: 12}}"
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
                let _ = writeln!(
                    out,
                    "{p}RoundedView{{ width: Fit height: Fit draw_bg.color: {PANEL} \
                     draw_bg.border_radius: 12.0 padding: Inset{{left: 10 right: 10 top: 5 bottom: 5}}"
                );
                let _ = writeln!(
                    out,
                    "{p}  TextCaption{{ text: {} draw_text.color: {SOFT} }}",
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
                    "{p}{role}{{{width} text: {body} draw_text.color: {colour}{size} }}"
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

    /// Drop every cell belonging to instances of `component`.
    fn reset_component(&mut self, component: &str) {
        let marker = format!("/{component}");
        self.cells.retain(|key, _| !key.contains(&marker));
    }

    /// Profile §5.8: a component whose state declarations changed gets a new
    /// schema id, and its live cells are dropped rather than migrated. Guessing
    /// that an old value still fits is how a rolled-back ledger feeds version-A
    /// state to version-B code.
    fn reconcile_schema(&mut self, component: &str, schema: u64) {
        match self.schemas.get(component) {
            Some(&known) if known == schema => {}
            Some(_) => {
                self.reset_component(component);
                self.schemas.insert(component.to_string(), schema);
            }
            None => {
                self.schemas.insert(component.to_string(), schema);
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
                Some((component.name.clone(), schema_id(component))),
            ),
            None => return false,
        },
        None => match card.events.iter().find(|e| e.name == event) {
            Some(d) => (d, &card.states, CARD_STATE_KEY, None),
            None => return false,
        },
    };

    if let Some((name, schema)) = schema_owner {
        store.reconcile_schema(&name, schema);
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
                SetSource::Payload | SetSource::Path(_) => match payload {
                    Some(v) => v.clone(),
                    // No payload is a no-op rather than a silent clear: falling
                    // back to the initial would look like a deliberate reset.
                    None => continue,
                },
                SetSource::Token(t) | SetSource::Text(t) => crate::JsonValue::String(t.clone()),
            },
            _ => continue,
        };
        store.set(instance_key, &transition.target, next);
        applied = true;
    }
    applied
}
