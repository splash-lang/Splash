#!/usr/bin/env python3
"""
Mutation-test the L0 validator: which of its rules does the suite actually hold?

A passing test proves the code does what the test says. It does NOT prove the
test would notice if the code stopped. Three review rounds on this branch found
guarantees that were false within hours and one test of mine that could not fail
at all, so "the suite is green" has repeatedly meant less than it appeared to.

This answers the harder question mechanically. For each diagnostic the validator
can emit — each one being some rule the profile claims — suppress it, run the
suite, and see whether anything goes red.

    SURVIVED  nothing failed. That rule is unprotected: it could regress
              silently, exactly like the `initial` fix whose regression test
              turned out to check the wrong artifact.
    caught    at least one test failed. The rule is genuinely held.

Suppression happens in Diagnostics::push rather than at each call site, so the
mutation is uniform and cannot accidentally change control flow.

    ./mutate.py            # every rule
    ./mutate.py <substr>   # only rules whose message matches
"""

import re
import subprocess
import sys
from pathlib import Path

SRC = Path(__file__).parent / "crates/splash-core/src/ui_l0.rs"

# Rules this harness CANNOT test, verified by hand instead. Suppressing a
# message only disables a rule when the message IS the enforcement; where a rule
# also sets the level or `valid` directly, the mutation is a no-op and the rule
# reports as unprotected while being perfectly well held.
#
#   let / while / arithmetic  enforced by the LEVEL a card is assigned, which
#                             `every_construct_outside_l0_raises_the_level`
#                             asserts directly.
#   header duplicates         check_header writes report.diagnostics itself
#                             rather than going through Diagnostics::push, so
#                             the hook never sees it. Verified by disabling
#                             `report.valid = false` and watching
#                             `a_contradictory_duplicate_header_field_is_rejected`
#                             go red.
#
# Isolating that second one changed the test: the original used `# level: L2`
# then `# level: L0`, and the retained L2 tripped the level-vs-derived check —
# so the card was refused by a DIFFERENT rule and the test passed with the
# contradiction check removed entirely. That is the failure this harness exists
# to find, and it found it in a test written the same day.
KNOWN_ARTIFACTS = [
    "`let` is not in L0",
    "`while` is not in L0",
    "arithmetic is not in L0",
    "the header declares level twice",
    "the header declares profile twice",
]

# The hook: a line inside Diagnostics::push that drops the targeted message.
ANCHOR = """    fn push(&mut self, line: usize, column: usize, message: String) {
"""
INJECT = '        if message.contains(%s) { return; }\n'


def messages():
    """Distinct diagnostic literals, as proxies for the rules they enforce."""
    src = SRC.read_text()
    found = set()
    for m in re.finditer(r'"((?:[^"\\\n]|\\.){12,90})"', src):
        t = m.group(1)
        if any(k in t for k in ("is not", "must", "cannot", "takes a", "has no",
                                "needs a", "may not", "is recursive", "only",
                                "does not accept", "is an event", "renders a value",
                                "declares level", "declares profile", "writes")):
            found.add(t)
    # The runtime message is formatted, so match on its longest LITERAL run —
    # the text between placeholders. Using only the prefix dropped every message
    # that starts with one, e.g. "{root:?} is not a declared name".
    out = {}
    for t in found:
        runs = [r.strip() for r in re.split(r"\{[^}]*\}", t)]
        longest = max(runs, key=len) if runs else ""
        # Trim a trailing fragment that would not appear verbatim once formatted.
        if len(longest) >= 12:
            out.setdefault(longest, t)
    return sorted(out.items())


def run_tests():
    """True if the suite passes."""
    r = subprocess.run(
        ["cargo", "test", "-q", "-p", "splash-core", "--test", "ui_profile_l0"],
        cwd=SRC.parents[3], capture_output=True, text=True,
    )
    return r.returncode == 0


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else None
    original = SRC.read_text()
    assert ANCHOR in original, "Diagnostics::push signature moved"

    targets = [(p, f) for p, f in messages() if not only or only in f]
    print(f"{len(targets)} rules to check\n")

    survived, caught, broken = [], [], []
    try:
        for i, (prefix, full) in enumerate(targets, 1):
            literal = '"%s"' % prefix.replace("\\", "\\\\").replace('"', '\\"')
            SRC.write_text(original.replace(ANCHOR, ANCHOR + INJECT % literal, 1))
            ok = run_tests()
            label = full[:66]
            artifact = any(a in full for a in KNOWN_ARTIFACTS)
            if ok and artifact:
                caught.append(label)
                print(f"  [{i:2}/{len(targets)}] held*     {label}")
            elif ok:
                survived.append(label)
                print(f"  [{i:2}/{len(targets)}] SURVIVED  {label}")
            else:
                caught.append(label)
                print(f"  [{i:2}/{len(targets)}] caught    {label}")
    finally:
        SRC.write_text(original)

    print(f"\n{'=' * 72}")
    print(f"held by a test: {len(caught)}     UNPROTECTED: {len(survived)}")
    print("(* held by a level or validity assertion this harness cannot reach;"
          " see KNOWN_ARTIFACTS)")
    if survived:
        print("\nRules no test would notice regressing:\n")
        for s in survived:
            print(f"  - {s}")
    if broken:
        print(f"\ncompile failures (mutation invalid): {len(broken)}")
    return 1 if survived else 0


if __name__ == "__main__":
    sys.exit(main())
