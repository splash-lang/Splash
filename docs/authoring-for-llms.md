# Authoring Splash as an LLM

You were trained on an enormous amount of JavaScript and Python. **Splash is
neither.** Under load you will reach for their idioms by reflex, and the result
either fails to parse or fails to run. This page is the antidote: every rule
below was *measured* against `splash check` (syntax) and the runtime (semantics)
on the canonical **workflow profile** — not inferred from the grammar. The exact
diagnostic the tools emit is quoted so you recognize it in the check→fix loop.

> This is the workflow/tool-orchestration profile. The generated-UI (card)
> profile is a sibling with **different** rules — do not carry rules between
> them. When unsure, run `splash check <file>`; it is effect-free and authoritative.

## Rule 0 — the one that bites hardest: blocks are newline-delimited

A one-line block `{ statement }` is **invalid**. The statement inside a block
must end with a newline (or a `;`) before the closing `}`.

```
// REJECTED  ->  expected a newline or `;` after a statement   (at the `}`)
if x == 1 { x = 10 }
fn add(a, b) { return a + b }

// OK
if x == 1 {
    x = 10
}
fn add(a, b) {
    return a + b
}
// OK (explicit terminator)
if x == 1 { x = 10; }
```

This applies to every block: `fn`, `if`, `elif`, `else`, `for`, `while`, `loop`,
`try`, `catch`. It is the single most common JS reflex that breaks Splash.

## Rule 1 — syntax the parser rejects outright

Each of these stops at `splash check` with the quoted message.

| You will write (JS/Python) | Diagnostic | Write instead |
| --- | --- | --- |
| `cond ? a : b` | ``operator `?` is not part of the canonical Splash profile`` | `if` is an expression: `let x = if cond { a } else { b }` |
| `if(cond, a, b)` (ternary-as-call) | `expected ) after a parenthesized expression` | same — use an `if` expression |
| `[...a, b]` | ``operator `...` is not part of the canonical Splash profile`` | `array.concat(a, [b])`, or `array.push(a, b)` |
| `x => x + 1` | ``operator `=>` is not part of the canonical Splash profile`` | `|x| x + 1` (bar-delimited lambda) |
| `a === b` / `a !== b` | ``operator `===` is not part of the canonical Splash profile`` | `a == b` / `a != b` |
| `` `hi ${x}` `` (template) | `unsupported character ...` | `"hi " + x` (string `+`) |
| `else if` (two words) | `expected a newline or ; after a statement` | `elif` |
| `const x` / `var x` | rejected | `let x` (reassign with `=`, `+=`, `-=`) |
| `new Date()` | rejected | no `new`; get time/etc. from a host tool |
| `function f() {}` | rejected | `fn f() { ... }` |
| `/ab+c/` (regex) | `expected an expression` | no regex; use `mod.std.text` literal ops |
| `for (let i = 0; i < n; i++)` | `expected a binding identifier after for` | `for i in array.range(0, n)` |
| `import x` / `from x import y` | `expected a newline or ; after a statement` | `use mod.tool` — effects come from registered tools |
| `try:` / `except:` (Python) | `expected an expression` | `try { ... } catch { ... }` |

## Rule 2 — parses, but dies at runtime (the sneakier class)

`splash check` will **not** catch these — they are syntactically valid. They
fail only when evaluated, so they surface at execution with a runtime message.
The mental model to fix them is at the bottom.

| You will write | Runtime diagnostic | Write instead |
| --- | --- | --- |
| `Math.max(a, b)` | `variable Math not found in scope` | `use mod.std.math` → `math.max(a, b)` |
| `JSON.parse(s)` | `variable JSON not found in scope` | `use mod.std.json` → `json.parse(s)`, or `s.parse_json()` |
| `console.log(x)` | `variable console not found in scope` | there is no stdout — a step's final value **is** its output |
| `null` / `undefined` | `variable null not found in scope` | `nil` |
| `require("x")` | `variable require not found in scope` | `use mod.tool`; capabilities are host-registered |
| `arr.push(x)` | `method push not found on array` | `use mod.std.array` → `array.push(arr, x)` |
| `s.trim()` | `method trim not found on string` | `use mod.std.text` → `text.trim(s)` |
| `s.toUpperCase()` | `method toUpperCase not found on string` | `text.upper(s)` |

**The mental shift:** collection, text, and math operations are **module
functions that take the value as their first argument**, not methods hanging off
the value. It is `array.push(list, x)`, `text.trim(s)`, `math.max(a, b)` — never
`list.push(x)`, `s.trim()`, `Math.max(...)`. The *only* methods that exist on a
value are `.parse_json()`, `.to_json()`, `.to_bytes()`, and `.await()` (on a
promise). Everything else is a module function.

## Rule 3 — how you actually do things

- **Effects / tools.** `use mod.tool`, then `tool.call("name", "text-arg")` or
  `tool.call_json("name", {field: value})`. Async: `tool.start("name", arg).await()`.
  A tool exists only if the host registered it and the workflow step was granted
  a lease for it; otherwise the call is denied. There is no ambient I/O, no file
  system, no network except through a granted tool.
- **Standard library** (frozen, no I/O): `mod.std.math`, `mod.std.json`,
  `mod.std.text`, `mod.std.array`, `mod.std.object`, `mod.std.assert`. Import with
  `use mod.std.array` and call `array.<fn>(value, ...)`.
- **`if`/`elif`/`else` and `try`/`catch` produce a value** — you can assign them:
  `let x = if c { 1 } else { 2 }`. **`for`/`while`/`loop` do NOT** produce a value;
  put an explicit `nil` (or a result expression) after the loop.
- **`&&` and `||` work** (unlike some earlier profiles — measured valid here).
- **`+` is string concatenation whenever either operand is a string.** `3 + "5"`
  is `"35"`, not `8`. Arithmetic `+` requires both operands numeric.
- **Dataflow between steps.** A step reads host input as `workflow.input.<field>`
  and a prior step's returned value as `workflow.outputs.<stepId>.<field>`. Its
  own final expression becomes its output.

## Rule 4 — verify before you hand it over

Never ship generated source unchecked; the host will reject it anyway.

```sh
splash check <file.splash>          # JSON diagnostics with line + column, nonzero on any violation
splash workflow-review <draft.json> # per-step syntax status for a multi-step plan
splash format <file.splash>         # canonical spelling
```

Run `check`, read each `line`/`column`/`message`, fix it, regenerate, re-check
until `valid` is `true`. Rule 2's runtime errors survive `check`; they appear
when the host executes the step and show up in the audit as a failed call — feed
that message back and fix it the same way.
