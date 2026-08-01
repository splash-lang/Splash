# Splash UI Profile — Level 0

**Status:** **implemented.** Normative producer contract for generated UI source at Level 0.
Parser, validator, realizer and instance store live in `splash-core::ui_l0`; 254 tests, three
reference cards, and a card rendered on a OnePlus 6T through an unmodified host.
**Relationship to [Grammar v0.2](grammar.md):** a *sibling* canonical profile, not a subset of
the workflow language. `check_syntax` rejects UI source and `eval_vm_compatibility` must not
receive generated source, so neither canonical entry point admits a generated card. This
profile is the third path, and `check_ui_l0` is its entry point.

---

## 1. What Level 0 is for

A generated card is untrusted source. The workflow profile answers this by refusing UI
constructs; the compatibility parser answers it by refusing untrusted input. L0 answers it a
third way: **admit UI constructs, but admit nothing that could reach a capability.**

Sources are resolved by the runtime *before* realization and injected as data. There is no
dangerous call to block, because no call can be written.

**L0 needs no evaluator at all.** Earlier drafts of this section said an L0 *evaluator* is
handed a host surface with no modules, and that a bounded evaluator — heap, stack, call-depth,
instruction and time limits — remains mandatory. Implementing the profile showed that to be
wrong in an interesting direction: **L0 has no expression form, so there is nothing to
evaluate.** Realization is a pure walk over the parsed tree with data substituted. The
reference implementation contains no reference to the VM whatsoever.

What that changes:

| | Earlier claim | Actual |
|---|---|---|
| Host surface | empty | **absent** — no evaluator to hand one to |
| Instruction limit | mandatory | not applicable |
| Heap / stack / call-frame caps | mandatory | not applicable |
| Bounds still required | — | node count, nesting depth, collection length — all traversal bounds |

So authority confinement is structural in the strongest available sense: not a sandbox that
blocks calls, and not an evaluator with an empty module table, but **no execution machinery in
the path at all**.

**What is still not bought.** Two properties are decidable at L0 — membership in the grammar,
and absence of authority-bearing operations. **Termination is not a consequence of the grammar
alone**; §6 states the five conditions that make it hold, and traversal bounds enforce the rest.

---

## 2. Grammar

```ebnf
source        = header , { declaration } ;

header        = "#" , "ledger" , ident , "@" , semver , { meta-line } ;
meta-line     = "#" , ident , ":" , value ;          (* level, model, profile *)

declaration   = source-decl | state-decl | event-decl | copy-decl
              | component-decl | view-decl ;

source-decl   = "source" , path , capability-query ;
state-decl    = "state"  , path , "{" , shape-spec , "}" ;
event-decl    = "event"  , ident , transition-block ;
copy-decl     = "copy"   , path , "{" , "class" , ":" , provenance , "," , locale-map , "}" ;
component-decl= "component" , Ident , "(" , [ params ] , ")" , "{" , { local } , view-body , "}" ;
view-decl     = "view"   , path , view-body ;

local         = state-decl | event-decl ;

view-body     = element ;
element       = construct | iteration | guard | reference | slot | fill ;
construct     = Constructor , [ "(" , [ arguments ] , ")" ] , [ block ] ;
slot          = "slot" , [ ident ] ;                 (* in a component body *)
fill          = "into" , ident , block ;             (* at a call site *)
reference     = path ;                               (* splices another view record *)
iteration     = "for" , ident , [ "," , ident ] , "in" , path , "key" , path , block ;
guard         = "when" , predicate , block ;
block         = "{" , { element } , "}" ;

arguments     = argument , { "," , argument } ;
argument      = ident , ":" , operand ;
operand       = path | literal | predicate ;         (* NO expressions *)

predicate     = path , [ comparator , ( path | literal | token ) ] ;
comparator    = "==" | "!=" | "<" | "<=" | ">" | ">=" ;

path          = root , { "." , ident | "." , integer | "[" , path , "]" }
                     , [ "." , "$state" ] ;          (* source lifecycle, §5.9 *)
root          = ident ;

transition-block = "{" , transition , { "," , transition } , "}" ;
transition    = path , ":" , total-form ;
total-form    = "set" , "(" , ( literal | token | path | "$value" ) , ")"
                          (* literal = string | number | "true" | "false" *)
              | "toggle"                              (* boolean state only *)
              | "cycle" , "(" , token , { "," , token } , ")"
              | "clear" ;                             (* restore declared initial *)

shape-spec    = "shape" , ":" , shape , [ "," , "initial" , ":" , ( literal | path ) ]
                              , [ "," , "keep" , ":" , "true" ] ;
shape         = "text" | "number" | "bool" | "enum" , "[" , ident , { "," , ident } , "]"
              | "record" | "collection" | "event" ;   (* prop shapes, §5.2 *)
provenance    = "vocabulary" | "user-copy" | "model-copy" ;
```

### 2.0 Amendments found by the reference cards

The grammar above already incorporates eight corrections that the three reference cards in
`octos-one/docs/l0/` forced. They are listed because the process matters: the cards were
written against the first draft, and **every construct they needed that the draft lacked is a
defect in this document, not in the cards.**

| # | Missing | Needed by | Resolution |
|---|---|---|---|
| 1 | element referencing another `view` record | all three — `root` splices named blocks | `reference = path` |
| 2 | `initial` from the environment | weather — `units` defaults by locale | `initial : literal \| path` |
| 3 | constant index into a collection | news — `stories.0` is the lead story | `"." integer` in `path` |
| 4 | index by a state value | news, stock — `stories[selected]` | `"[" path "]"` in `path` |
| 5 | loop index binding | news — the rank column | optional second binder: `for s, i in …` |
| 6 | predicate as an argument value | stock — `active: range == d1` | `operand` admits `predicate` |
| 7 | event payload at the call site | news, stock — which row was tapped | `value:` argument, §3.1 |
| 8 | sub-collection of a source | news — feed skips the lead | **not granted** — see below |
| 9 | bare identifiers are ambiguous | both components — `width: rank` is a token, `text: position` is a path | tokens marked lexically, §2.2 |
| 10 | a bare boolean guard | weather — `when expanded { … }` | `predicate` admits a lone path |

**None of these introduces an expression.** Indexing is a total lookup, a predicate as an
operand is the form `when` already admits, and a loop index is supplied by the iterator. The
authority property and the absence of arithmetic both survive.

### 2.2 Tokens are lexically marked

`width: rank` and `text: position` are both `ident : ident`, but the first names a layout token
and the second a bound path. Resolving this by scope — *a path if it names a binding, else a
token* — would be a trap: declaring a prop named `fill` would silently change what
`width: fill` meant everywhere in that component.

**Tokens carry a leading dot.** `width: .fill`, `format: .money`, `align: .center`. A bare
identifier is always a path.

```ebnf
operand       = path | literal | token | predicate ;
token         = "." , ident ;
```

An earlier draft resolved this by argument shape instead — *each argument accepts either a
token set or a value, so the argument decides what a bare name means.* **That rule is
withdrawn**, because it fails on any argument admitting both, and one does: `unit: units`
follows card state while `unit: .pct` is fixed. Marking tokens removes the ambiguity
lexically, so the catalog only has to say which tokens are **legal** rather than disambiguate
what a name **means**.

The ambiguity surfaced only once components existed, because a component's props are the first
bindings whose names the card author chooses freely — both reference cards had to rename a
prop, `day` → `item` and `rank` → `position`, before it was visible at all. The `unit` case
surfaced one step later still, while cataloguing arguments. Each pass found what the previous
one could not have.

**Enum members are tokens too.** A `shape: enum[c, f]` declaration *defines* the set with bare
names; every use is dotted — `cycle(.c, .f)`, `initial: .m1`, `range == .d1`. Same rule as any
other token: a bare identifier is always a path, so an enum member can never be shadowed by a
binding of the same name.

The normative argument contract is [`ui-l0-constructors.toml`](ui-l0-constructors.toml):
which constructors exist, which arguments each admits, and which tokens are legal per
argument.

**#8 was refused.** The news card wanted `stories.rest` to skip the lead story. Granting a
derived sub-collection would be the thin end of computation — `rest` invites `filter`, which
invites a predicate expression, which invites arithmetic. The card instead binds
`sys.news(...)` twice, or the source declares `lead` and `rest` as fields. **The runtime
shapes the data; the view does not reshape it.**

### 2.1 What is absent, and deliberately

| Construct | Status | Why |
|---|---|---|
| arithmetic, string concatenation | **absent** | any expression can hide a fabricated fact (§5) |
| function definition, call | **absent** | would reintroduce unbounded work |
| `while`, unbounded `for` | **absent** | iteration is only over a declared collection |
| `ui.<id>.<method>` | **absent** | imperative widget commands are L2 |
| module access of any kind | **absent** | this is the authority property |
| assignment outside a transition | **absent** | state changes only through declared events |

**`operand` admits no expressions.** `Text(value: now.temp)` is legal; `Text(value: now.temp * 9 / 5 + 32)` is not. Unit conversion and formatting belong to the runtime (§4).

---

## 3. Transitions are total by construction

This is the point where an earlier draft of this design was wrong, and the correction is
load-bearing.

A transition written as an expression — `expanded: !expanded`, or
`units: units == c ? f : c` — **is computation**, however small. Declared storage does not
make it total or bounded, and once expressions are admitted the grammar has to answer for
recursion, unbounded numerics, and event cascades.

L0 therefore has no expression form in transitions. It has four **total forms**:

| Form | Applies to | Semantics |
|---|---|---|
| `set($value)` | any | the payload the raising element carried in its `value:` argument |
| `set(.token)` | `enum` | a declared member |
| `set("text")` | `text` | a literal |
| `set(path)` | any | a bound path, resolved from injected data |
| `toggle` | `bool` | the other value |
| `cycle(.a, .b, …)` | `enum` | successor, wrapping; members must be declared in the shape |
| `clear` | any | restore the declared `initial` |

A `set($value)` raised with no payload is a **no-op**, not a clear: falling back to the
initial would read as a deliberate reset.

Each is total on its declared shape and terminates in constant time. `units: cycle(c, f)`
replaces the ternary; `expanded: toggle` replaces the negation.

An event may name several transitions, applied **atomically as one batch** — every write is
staged and the batch commits only if all of them resolve, so a transition that cannot be
computed leaves the store exactly as it was rather than half-applied.

**A payload is checked at dispatch.** `set($value)` is the one form whose value arrives from
outside the card, so it is the one the declared shape cannot decide statically; a payload that
does not fit the target state is refused and the batch does not commit. Every other `set`
spelling is decided at check time against the declared shape. A single-assignment
rule would be wrong: production code already fires two state writes from one gesture, and the
stock reference card's `back` does the same — clearing both `selected` and `range`.

### 3.1 The event payload

An element that raises an event supplies its payload at the call site:

```
Row(on_tap: open_quote, value: m.ticker)
```

`on_tap` names a declared event; `value` is the payload the transition receives as `$value`.
Both are ordinary operands, so a payload can be a bound path or a literal but never a computed
one. An event whose transitions do not mention `$value` ignores any payload.

This replaces the string-tagging approach used by the current runtime, where a card's identity
is prepended to its event name by rewriting generated source. A card that constructs its event
name dynamically escapes that rewrite; here the event is a declared name resolved by the
runtime, so there is nothing to construct.

---

## 4. Sources, and the no-facts rule

A `source` body is a **query the runtime executes**, never a call the view makes. Views cannot
invoke it; they bind to its result.

```
source place  sys.geocode(name: state.city)
source now    sys.weather(lat: place.lat, lon: place.lon, fields: [temp, hi, lo, cond])
```

The evaluator never sees `sys.geocode`. The runtime resolves it, injects the result as data,
and the view reads `place.name`.

**A literal in a data position must be `vocabulary` or `source-derived`.** This is decidable
at L0 precisely because `view` is a typed tree — a `model-copy` literal bound to a data
argument is a structural violation, not a judgement call. It is the property that makes
`"34 mph"` concatenated onto a live value unrepresentable rather than merely discouraged.

**What is actually enforced, and what is not.** Both halves matter, and only one is complete:

| Position | Rule |
|---|---|
| A `value` argument (`TextHero`, `TextStat`, `TextValue`, `TextCaption`, `Tile`) | Must be bound. A literal — string, number or token — is refused. `TextHero(value: "34 mph")` is rejected |
| Any `path`-kind argument (the drawing widgets) | Must be bound. `TempBar(lo: 5)` is rejected |
| `copy.x`, by any route | Refused when `x` is declared `model-copy` — named in a view, passed as a prop, used as a state initial, or written by a transition. A `copy.x` that is not declared at all is refused too |
| A **raw string literal** in a `text`, `label` or `glyph` position | **Accepted** |

The last row is a deliberate decision, not an oversight. A raw literal carries no declared
provenance, and by this section's own logic that makes it model-copy — so `TextTitle(text:
"Revenue: $41.2M")` passes, and a model that simply writes the string inline is unaffected by
the rule. Refusing it would mean every human-readable string is declared `copy`, which is a
real ergonomic cost and buys less than it appears to: of the eleven literals in the reference
cards, six are `glyph` symbols (`‹ ↑ ↓ ≈`) that are closed presentation vocabulary rather than
language, and translating them is meaningless.

So the guarantee this section provides is narrower than its first paragraph implies. **A
fabricated number cannot reach a numeric position — by any route, including laundered through
a chain of component props — and fabricated prose can reach a text position, but only as a raw
literal.** Text that is *declared* model-authored is refused wherever it tries to surface. The first is the failure mode that produced the six shipped bugs this rule exists
for — an invented `condition`, an invented `wmin`/`wmax` — and it is closed. The second is open
by choice, and would need retained provenance on every string, not a broader argument category,
to close.

Implicit dependencies are declared like any other source:

```
source env.locale  sys.locale()
source env.theme   sys.theme()
```

Anything reachable without a declared source is a defect by construction — which is also what
makes an L0 view's dependency set readable without evaluating it.

---

## 5. Components and instance identity

```
component ForecastRow(day) {
  state expanded { shape: bool, initial: false }
  event toggle   { expanded: toggle }
  view  Row {
          Text(role: Row, text: day.dayname)
          TempBar(lo: day.lo, hi: day.hi, min: week.min_lo, max: week.max_hi)
          when expanded { Detail(day: day) }
        }
}
```

Local state is declared in the ledger; **its values live only in the runtime.**

### 5.1 Identity is a path, not a position

An instance is identified by the tuple

```
( card namespace, parent instance identity, instantiation site,
  component name and version, vector of enclosing loop keys )
```

Recursive by construction, because a static declaration path is not enough: two `TripPanel`
instances each containing an unkeyed `Toggle` must not share one state cell.

**This is not React's rule.** React associates state with position in the realized tree, where
component type and keys both affect preservation. L0 binds to a declared path so that
identity survives a ledger edit that inserts an element above.

**Neither the card namespace nor the version is currently a key segment.** Keys begin at the
root view and append component names; the schema id is held in a separate map keyed by component
name. So the tuple above describes the intended identity, and the implementation covers the
instantiation site and enclosing loop keys only. One consequence worth knowing: two cards
sharing one store can collide, because card state lives under a single fixed key.

**"Version" means the STATE SCHEMA version** (§5.8), not the component's whole definition.
Using the definition digest would contradict §5.8 directly: a view-only edit would change
identity and reset state, which is precisely what §5.8 promises it does not do. The two are
different instruments — the schema id decides whether live state still fits, and the closure
digest (§7) tells a host that a definition was replaced at all.

**What path-based identity does and does not survive.** Insertion of a differently named
sibling is stable, because siblings are numbered within each element name. Identity moves when
the PATH moves: reordering two same-name siblings, inserting another instance of the same
component above, adding or removing an enclosing guard, loop, slot or view reference, and
changing a loop's key expression.

Only the element name and its ordinal enter a segment, so editing a guard's *condition* —
`when a` to `when b`, both true — leaves the path untouched and the state survives. An earlier
version of this paragraph said any change to an enclosing guard moved identity; that is wrong,
and wrong in the direction that matters, since it would have justified not looking. Fully edit-stable identity needs a persistent site id in the source or
edit provenance from the ledger; neither exists yet, and the numbering is an improvement on
tree position rather than a solution.

### 5.2 Props

```ebnf
params        = param , { "," , param } ;
param         = ident , ":" , shape , [ "=" , literal ] ;
```

Props are **typed by the same shape vocabulary as state**, may carry a literal default, and
are **immutable inside the component**. A prop is an input, not storage; a component that
wants to change something declares local state or raises an event.

```
component ForecastRow(day: record, unit: enum[c, f] = c, on_pick: event)
```

`record` and `collection` are added to the shape vocabulary for props, since a row receives a
whole item rather than a scalar.

### 5.3 Scope: what a component may reference

| May read | May not read |
|---|---|
| its own props | card `state` — pass it as a prop |
| its own `state` and `event` | another component's locals |
| card-scope `source` | — |
| card-scope `copy` | — |

**Sources and copy are readable; state is not.** Both are immutable and already declared
dependencies the tracker sees, so reading them creates nothing implicit. Card state is what
drives reconciliation, so it must arrive explicitly as a prop or the dependency becomes
invisible — the failure §6.2 warns about.

This is why `week.min_lo` may be read directly by a forecast row, while `units` must be passed.

### 5.4 Outputs are event props

A component raises an event the parent declared, passed in as a prop:

```
component StoryRow(story: record, on_open: event) {
  view Row(on_tap: on_open, value: story.id) { … }
}

view latest  Panel { for s, i in feed key s.id { StoryRow(story: s, on_open: open_story) } }
```

**This needs no new mechanism.** An event name is a path, `operand` already admits paths, and
the transition runs in the scope where it was *declared* — the parent's — so a child cannot
reach state it was not handed a route to. It is React's callback prop with the scoping made
explicit.

A component may only name events it declares locally or receives as props. There is no way to
raise an arbitrary event by constructing its name, because there is no expression form.

### 5.5 Slots

A single anonymous slot, marked in the component body:

```
component Panel(title: text) {
  view Col { TextCaption(text: title) slot }
}

Panel(title: "Details") { Tile(…) Tile(…) }
```

Children are realized in the parent's scope, not the component's — a child expression sees the
call site's bindings.

A component may also offer **named** slots, and a call site directs children at one with `into`:

```
component Framed(title: text) {
  view Col { TextCaption(text: title)  slot header  Rule()  slot }
}

Framed(title: "Today") {
  into header { TextRow(text: "above the rule") }
  TextRow(text: "below it")
}
```

Children written outside any `into` fill the anonymous slot, so a component with one slot needs
no ceremony and every card written before named slots existed is unchanged.

**An `into` naming a slot the component does not offer is rejected.** Its children would
otherwise render nowhere, which is the same silent-drop failure as a binding that resolves to
nothing — the card looks deliberate and the content is simply gone.

### 5.6 What is refused, and why

**Hoisted or shared state.** If two components need the same state it lives at card scope and
arrives as props, with the event declared at card scope too. Hoisting would raise an ownership
question the grammar cannot answer, and a Context-like mechanism reintroduces exactly the
invisible dependencies §6.2 exists to prevent.

**Cross-field invariants are not expressible.** Two date pickers cannot enforce
start-before-end through transitions, because total forms cannot compare. This is a real limit,
not an oversight. Where an invariant matters it belongs in the **shape**, enforced by the
runtime:

```
state start { shape: date, initial: today, max: end }
```

**That example is illustrative, not grammatical.** `date` is not one of §2's shapes, `today`
is not a literal form, and `max:` is not part of `shape-spec` — it sketches where such an
invariant would belong if the shape vocabulary grew, and nothing in the profile accepts it
today. It is retained because the *limit* it describes is real: total forms cannot compare, so
a cross-field invariant has nowhere to live at L0.

If that is insufficient, the card is L1 — and should say so rather than smuggling a
comparison into a transition.

**Per-component sources.** A component may not declare its own `source`. Sources are card-scope
and resolved before evaluation; a component instantiated *n* times would fan out *n* fetches,
with *n* determined by data. That breaks the bound §6.2 requires, and makes fetch volume a
function of a collection the model does not control.

### 5.7 Lifecycle

There are no lifecycle hooks — §2.2 rejects `useEffect` and its dependency arrays. But the
runtime needs defined behaviour, and identity (§5.1) decides all of it:

| Event | Behaviour |
|---|---|
| **Mount** — identity appears | local state initialized from `initial` |
| **Update** — identity persists, props differ | view re-realizes; **local state persists** |
| **Unmount** — identity disappears | local state discarded |

Realization reports every instance key it produced. A host passes them to `prune` after each
pass; without it the store grows for the life of the app, and a key that reappears inherits a
stale value.

**Duplicate keys are an error at realization**, surfaced as a diagnostic — never coalesced.
Silent coalescing would make two rows share one state cell, which is the failure mode keys
exist to prevent. A missing key on an iteration is a grammar error.

### 5.8 State schema versioning

A component's **state schema id** is derived from its `state` declarations — names, shapes and
initials — and forms part of instance identity (§5.1).

| Definition change | Effect |
|---|---|
| view or props only | schema id unchanged → **local state persists** |
| any `state` declaration added, removed, renamed or retyped | schema id changes → **new identity → state reset** |

**Reset is the default; preservation is opt-in**, never inferred. A ledger edit that adds a
field cannot be assumed compatible with live values, and guessing is how a rolled-back ledger
ends up feeding version-A state to version-B code — at which point last-known-good is no longer
known good.

**The opt-in is `keep: true`** on a state declaration:

```
state expanded { shape: bool, initial: false, keep: true }
```

A cell that opts in survives a schema change **only while its own shape is unchanged**. Adding
an unrelated field elsewhere in the component preserves it; retyping `expanded` itself resets it
regardless. Without that second condition `keep` would be precisely the "assume the old value
still fits" that this section exists to prevent — it would just have moved the guess from the
runtime to the card author, where it is no more reliable and considerably less visible.

Reset therefore remains the default in both directions: a component that says nothing behaves
exactly as before, and a component that opts in still resets on any change to the field it
opted in for.

**Consequence worth stating plainly:** folding the ledger reconstructs the *program*, not the
visible application state. Local state is disposable UI state — scroll positions, expansion
flags. Anything that must survive a definition change belongs in a `source` or in card state
with a declared migration, not in a component.

### 5.9 Source lifecycle

Every source carries a lifecycle, readable as `<source>.$state`, whose value is one of four
tokens:

| Token | Meaning |
|---|---|
| `.pending` | never resolved |
| `.ready` | resolved and current |
| `.stale` | resolved, but past its freshness budget |
| `.failed` | the fetch failed |

```
when now.$state == .pending { TextRow(text: copy.loading) }
when now.$state == .failed  { TextRow(text: copy.offline) }
```

This exists because **absence is not a state.** Without it a card can only ask whether a value
arrived, so "still loading", "the network is down" and "this field genuinely has no value" all
render the same em dash. The card then looks like working software displaying real data, which
is the worst of the three outcomes.

`$state` must be the last segment of a path, and is available **only on a source** — a local
state is not fetched and has no pending or failed. Both are rejected rather than resolving to
nothing.

The host reports the lifecycle; the card never computes it.

**The fallback, and what it assumes.** When the host reports no `$status`, a source with a
value is treated as `.ready` and one without as `.pending`. That is an inference, and this
section has just argued that absence cannot carry it — so it is sound *only* under an explicit
host contract: **a source key is present if and only if it resolved successfully and is
current.** A host that caches a stale value without reporting status will have it read as
`.ready`; one that represents a successful empty result by omitting the key will have it read
as `.pending`.

The fallback therefore exists for hosts that do not track status at all, and any host that can
distinguish these cases must report `$status` rather than rely on it. `.stale` and `.failed`
are not derivable from data by construction, which is the whole reason status reporting exists;
the fallback does not change that, it only declines to guess for the two cases that are
derivable under the invariant above.

**Cadence and freshness budget belong to the capability, not the card.** A card names what it
needs; how often `sys.weather` is worth re-fetching is a property of weather, and duplicating it
per card is how two cards on one screen disagree about how old their data is.

---

## 6. What makes L0 terminate

Grammar membership alone does not bound execution. These conditions do, and an implementation
must enforce all of them:

1. **The component graph is acyclic.** No recursive or mutually recursive components.
2. **Iteration is over a runtime-provided collection**, whose length the runtime bounds before
   injection.
3. **Transitions are total forms** (§3), so a single event applies in constant time per key.
4. **Event cascades are prohibited** — a transition may not trigger another event.
5. **Node count and nesting depth are capped** at realization.

With all five, an L0 realization terminates. **Without them the grammar does not**, which is
why "L0 is decidable" is a claim about authority and grammar membership, not about
termination.

Condition 4 holds **because no admitted transition form raises an event** — `set`, `toggle`,
`cycle` and `clear` all write a state cell and nothing else, and a transition target is
validated against declared state, so an event cannot be named as a write target either.

Stating that more carefully than an earlier draft did: `Form` carries a fifth variant,
`NotTotal`, for anything outside those four, and the dispatcher ends in a catch-all rather than
matching exhaustively. **So the compiler would not force a review if a variant were added.**
The property is true of the forms that exist; it is not structurally protected. Any new `Form`
variant must be checked against this condition by hand, and one that reads a path must exclude
event-typed paths explicitly.

An earlier version of this section claimed the stronger property that "no total form can name
an event". That was false when written, because component prop types were discarded and
`set(out)` was accepted where `out` was an event prop. It is now true — prop shapes survive
parsing, and event-shaped props are excluded from the readable set — but it is true by a check
rather than by the grammar, which is the distinction worth keeping. **Any new `Form` variant
must be checked against this condition**, and any form that reads a path must exclude
event-typed ones explicitly, because nothing in the type system will do it for you.

A test enumerates the syntaxes a cascade could take — `emit e`, `e()`, `n: set(1) then e`, a
transition targeting an event — and asserts each is refused. That test is a tripwire, not the
proof.

---

## 7. Level determination

A card's level is the **maximum** required by any record, resolved over the **transitive
component closure** — not by inspecting records locally:

```
uses any construct outside §2                  →  L1 or L2
instantiates a component whose level is higher →  that level
otherwise                                      →  L0
```

Level is declared in the header and checked at append. A record needing a wider grammar is
rejected until the level is explicitly raised; escalation is never silent. **Component
versions must be pinned**, or an L0 card can be moved to L2 by a definition it references
being replaced.

The check therefore returns a **closure digest**: every component the card declares, paired
with a hash of its definition — param names, state (path, shape, initial, keep), events and
view body — sorted by name. A host stores this beside the approved level. A digest that moved
means the level was derived from a definition that no longer exists and must be re-derived
before the card is accepted, rather than inherited from the record it no longer matches.

The digest sorts **components**, so reordering declarations at the top level is not a version
change — a digest that moved on a cosmetic edit would make pinning useless in the way that
matters, since hosts would see it move constantly and learn to ignore it. Order *within* a
component is hashed, so moving a param, state or event does register as a change.

**Known limits, stated so the digest is not trusted past them.** It hashes every declared
component rather than the closure reachable from the root view, so an unused definition causes
a false repin. And it is 64-bit FNV-1a, which is a change *detector* and not an adversarial
boundary: it is fit for noticing that a definition moved, and unfit for resisting a chosen
collision. Pinning is also report material — nothing in the runtime yet stores or compares it.

It also omits a loop's `key` expression, whether an element is a view reference, parameter
defaults, and the bodies of views a component references — so a card can change what it renders
without the digest moving. Parameter and state *shapes* are covered as of the change that made
prop shapes survive parsing; before that `x: number` → `x: text` was invisible to both the digest and §5.8's
schema id, so a live string could survive into number-shaped state.

Three token tests are not sufficient and an earlier draft that used them was both incomplete
and self-contradictory. Level must be an effect judgment over the closure.

**"Maximum" is a scan, not a search.** A classifier that returns at the first construct outside
L0 reports whichever appears earliest in the source, not the highest. The nav card demonstrates
it: arithmetic on line 55 precedes `fn tick()` on line 80, so a short-circuiting classifier
calls it L1 — a card with 65 imperative widget commands. The implementation must examine every
construct and keep the widest.

---

## 8. Open questions

| # | Question |
|---|---|
| ~~1~~ | ~~Whether `when` needs `else`~~ — **settled: no.** Two complementary guards suffice and both reference cards that branch already use them (`when selected == ""` / `when selected != ""`). An `else` would need the negation of the guard, and there is no expression form to negate with; adding one for this alone would buy nothing a second `when` does not already express |
| ~~2~~ | ~~Whether `enum` shapes need ordering guarantees for `cycle`~~ — **settled: yes, declaration order is normative.** `cycle` advances to the successor in the enum's declared order and wraps, which is what keeps it total and O(1). A `cycle(…)` listing a different order is **rejected**, not reordered: otherwise the card would say one order and the runtime would run another |
| ~~3~~ | ~~How a `source` declares refresh cadence, staleness and error surfaces~~ — **settled in §5.9.** Staleness and error are a four-token lifecycle the host reports and the card reads as `<source>.$state`. Cadence is not a card concern: it belongs to the capability, since how often a fetch is worth repeating is a property of the data, not of the card displaying it |
| ~~4~~ | ~~A bounded `format()`~~ — **settled**: `format:` is a declared token and the runtime applies it. A card that wrote `"$"` or `"41.2M"` itself would be authoring a fact, and currency, grouping and compaction are locale-dependent besides |
| ~~5~~ | ~~Named slots~~ — **settled in §5.5.** `slot name` in the component, `into name { … }` at the call site, anonymous slot unchanged as the default. An `into` naming a slot that does not exist is rejected rather than silently dropping its children |
| ~~6~~ | ~~An explicit state migration~~ — **settled in §5.8.** `keep: true` opts a cell out of the schema-change reset, and only while that field's own shape is unchanged |
| ~~7~~ | ~~The closed token sets per constructor argument~~ — **settled**: `docs/ui-l0-constructors.toml` is the normative catalog, and tests assert the two agree in both directions — constructor names and shared token sets, and for source capabilities the arguments too |

All seven are now settled. What replaces them is not a shorter list of questions but a
different kind of work: the profile is specified and implemented, and the remaining gaps are
in the surrounding system rather than in the language.

**Known gaps, stated plainly.** `makepad::lower` still lives in `splash-core` and belongs in a
backend crate. `StockPlot` and `AqiContour` lower without their data arrays. L1 and L2 are
specified only as "what L0 excludes" — neither is implemented, and the level classifier can
therefore name them but not check them.

Component contracts were question 1 and are now specified in §5.2–§5.8, validated by adding
components to two reference cards. That exercise produced amendment #9, which nothing had
surfaced before — props are the first bindings a card author names freely, so the
token/path ambiguity was invisible until one existed.

The reference cards in `octos-one/docs/l0/` remain the evidence: any construct they need that
this document lacks is a defect here rather than in the cards. Two of the settlements above came
from that rule rather than from argument — question 1 was answered by noticing both branching
cards had already written the complementary-guard form without anyone specifying it, and
question 7 by the cards exhausting the token sets in the course of being written.
