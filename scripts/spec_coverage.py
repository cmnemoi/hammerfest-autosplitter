"""Which spec ids have no test, and which tests name an id no spec declares.

A behaviour counts as covered when its id appears in three places: a spec
page, a test, and the code that implements it. See `docs/specs/`.

The specs declare two kinds of id, and they are counted apart:

    a rule       `reader::a-known-world`        no dot before the `::`
    a criterion  `reader.find::a-parallel-world`   a dot before the `::`

A rule states a design decision. A criterion is a situation that proves one.
So `memory-reader.md` proves its six rules through its seventeen criteria, and
zero rules with an `@spec` is the expected shape there, not a gap.
`level-crossings.md` tests its rules directly, and has no criteria at all.

Run it with `mise run spec-coverage`.
"""

import pathlib
import re
import sys

ID = re.compile(r"\b([a-z]+(?:\.[a-z]+)?::[a-z0-9-]+)\b")
ROOT = pathlib.Path(__file__).resolve().parent.parent


def ids_in_specs() -> dict[str, set[str]]:
    """Every id each spec page declares."""
    found: dict[str, set[str]] = {}
    for page in sorted((ROOT / "docs" / "specs").glob("*.md")):
        found[page.name] = set(ID.findall(page.read_text(encoding="utf-8")))
    return found


def ids_in_code() -> set[str]:
    """Every id an `@spec` marker names, in a test or in the code."""
    marker = re.compile(r"@spec\s+(\S+)")
    found: set[str] = set()
    for source in (ROOT / "src").rglob("*.rs"):
        found.update(marker.findall(source.read_text(encoding="utf-8")))
    return found


def main() -> int:
    specs = ids_in_specs()
    code = ids_in_code()
    declared = {name for page in specs.values() for name in page}

    for page, names in specs.items():
        print(page)
        for kind, group in (
            ("criteria", {n for n in names if "." in n.split("::")[0]}),
            ("rules", {n for n in names if "." not in n.split("::")[0]}),
        ):
            if not group:
                continue
            missing = sorted(group - code)
            print(f"    {len(group) - len(missing)} of {len(group)} {kind} have an @spec")
            for name in missing:
                print(f"        no @spec  {name}")

    dangling = sorted(code - declared)
    if dangling:
        print("\n@spec markers that no spec declares:")
        for name in dangling:
            print(f"    no spec  {name}")

    # Never fails the build. The gap is the point of the report, and today it
    # is wide on purpose: see `docs/internals/testing-the-memory-reader.md`.
    return 0


if __name__ == "__main__":
    sys.exit(main())
