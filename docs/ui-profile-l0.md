# Splash UI Profile — Level 0

**Status:** proposed. Normative producer contract for generated UI source at Level 0.
**Relationship to [Grammar v0.2](grammar.md):** a *sibling* canonical profile, not a subset
of the workflow language. `check_syntax` rejects UI source today, and
`eval_vm_compatibility` accepts it but must not receive generated source — so there is
currently no supported path for LLM-authored UI. This profile is that path, at its narrowest.

---

## 1. What Level 0 is for

A generated card is untrusted source. The workflow profile answers this by refusing UI
constructs; the compatibility parser answers it by refusing untrusted input. L0 answers it a
third way: **admit UI constructs, but admit nothing that could reach a capability.**

The consequence is that an L0 evaluator is handed a host surface with **no** modules — no
`sys.*`, no `fs`, `run` or `net`. Sources are resolved by the runtime *before* evaluation and
injected as data (`set_json_global`). There is no dangerous call to block, because no call
can be written.

**What this does and does not buy.** Two properties are decidable at L0: membership in the
grammar, and absence of authority-bearing operations. **Termination and bounded state are
*not* consequences of the grammar alone** — §6 states what makes them hold. A bounded
evaluator (heap, stack, call-depth, instruction and time limits) remains mandatory at every
level; L0 removes the need for an *OS authority* boundary, not for resource containment.

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
element       = construct | iteration | guard | reference ;
construct     = Constructor , [ "(" , [ arguments ] , ")" ] , [ block ] ;
reference     = path ;                               (* splices another view record *)
iteration     = "for" , ident , [ "," , ident ] , "in" , path , "key" , path , block ;
guard         = "when" , predicate , block ;
block         = "{" , { element } , "}" ;

arguments     = argument , { "," , argument } ;
argument      = ident , ":" , operand ;
operand       = path | literal | predicate ;         (* NO expressions *)

predicate     = path , comparator , ( path | literal ) ;
comparator    = "==" | "!=" | "<" | "<=" | ">" | ">=" ;

path          = root , { "." , ident | "." , integer | "[" , path , "]" } ;
root          = ident ;

transition-block = "{" , transition , { "," , transition } , "}" ;
transition    = path , ":" , total-form ;
total-form    = "set" , "(" , ( literal | path | "$value" ) , ")"
              | "toggle"                              (* boolean state only *)
              | "cycle" , "(" , literal , { "," , literal } , ")"
              | "clear" ;                             (* restore declared initial *)

shape-spec    = "shape" , ":" , shape , [ "," , "initial" , ":" , ( literal | path ) ] ;
shape         = "text" | "number" | "bool" | "enum" , "[" , ident , { "," , ident } , "]" ;
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

**None of these introduces an expression.** Indexing is a total lookup, a predicate as an
operand is the form `when` already admits, and a loop index is supplied by the iterator. The
authority property and the absence of arithmetic both survive.

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
| `set(v)` | any | assign a literal, a bound path, or the event payload `$value` |
| `toggle` | `bool` | the other value |
| `cycle(a, b, …)` | `enum` | successor, wrapping; members must be declared in the shape |
| `clear` | any | restore the declared `initial` |

Each is total on its declared shape and terminates in constant time. `units: cycle(c, f)`
replaces the ternary; `expanded: toggle` replaces the negation.

An event may name several transitions, applied **atomically as one batch**. A single-assignment
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

### 5.2 Unresolved

The following are **not specified here** and must be before this profile is implementable:
child-to-parent outputs, slots/children, shared or hoisted state, per-component source
declarations, mount/update/unmount semantics, duplicate-key behaviour, and typed props with
defaults. §7 records this.

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

Three token tests are not sufficient and an earlier draft that used them was both incomplete
and self-contradictory. Level must be an effect judgment over the closure.

---

## 8. Open questions

| # | Question |
|---|---|
| 1 | Component contracts (§5.2) — outputs, slots, shared state, lifecycle |
| 2 | Whether `when` needs `else`, or whether two complementary guards suffice |
| 3 | Whether `enum` shapes need ordering guarantees for `cycle` |
| 4 | How a `source` declares refresh cadence, staleness and error surfaces |
| 5 | Whether the profile should admit a bounded `format(path, style)` for dates and units, or keep all formatting in the runtime |

Question 5 is the one most likely to force a change: the three reference cards in
`octos-one/docs/l0/` are the evidence, and any construct they need that §2 lacks is a defect
in this document rather than in the cards.
