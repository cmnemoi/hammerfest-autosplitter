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

Run it with `mise run spec-coverage`. `--self-check` runs its own tests, and
`mise run test` calls that.
"""

import pathlib
import re
import sys

ID = re.compile(r"\b([a-z]+(?:\.[a-z]+)?::[a-z0-9-]+)\b")
ANCHOR = re.compile(r"\{#([a-z]+(?:\.[a-z]+)?::[a-z0-9-]+)\}")
MARKER = re.compile(r"@spec\s+(\S+)")
ROOT = pathlib.Path(__file__).resolve().parent.parent


def ids_in_text(text: str) -> set[str]:
    """Every id a page declares.

    A page declares an id in two places, and nowhere else: an anchor, for a
    rule, and a table row, for a criterion. Everything else is prose, and prose
    names Rust paths that look exactly like ids -- `asr::timer`,
    `timer::start`. Reading those as rules invents rules no test can ever
    cover.
    """
    found: set[str] = set()
    for line in text.splitlines():
        found.update(ANCHOR.findall(line))
        if line.lstrip().startswith("|"):
            found.update(ID.findall(line))
    return found


def ids_in_specs() -> dict[str, set[str]]:
    """Every id each spec page declares."""
    found: dict[str, set[str]] = {}
    for page in sorted((ROOT / "docs" / "specs").glob("*.md")):
        found[page.name] = ids_in_text(page.read_text(encoding="utf-8"))
    return found


def markers_in_text(text: str) -> set[str]:
    """Every id an `@spec` marker names."""
    return set(MARKER.findall(text))


def ids_in_code() -> set[str]:
    """Every id an `@spec` marker names, in a test or in the code."""
    found: set[str] = set()
    for source in (ROOT / "src").rglob("*.rs"):
        found.update(markers_in_text(source.read_text(encoding="utf-8")))
    return found


def self_check() -> int:
    """The report decides what the test net looks like, so it is held too."""
    page = """
### A rule

`{#pacing::a-failed-scan-waits-longer-each-time}`

The loop calls `timer::start` and `asr::timer` lives in the runtime.

| id | given | then |
| --- | --- | --- |
| `reader.find::an-orphan-game` | a game and no manager | the game is found |
"""
    declared = ids_in_text(page)

    assert "pacing::a-failed-scan-waits-longer-each-time" in declared, "an anchor declares a rule"
    assert "reader.find::an-orphan-game" in declared, "a table row declares a criterion"
    assert "timer::start" not in declared, "prose names a Rust path, not a rule"
    assert "asr::timer" not in declared, "prose names a Rust path, not a rule"

    covered = markers_in_text("/// @spec pacing::a-failed-scan-waits-longer-each-time")
    assert covered == {"pacing::a-failed-scan-waits-longer-each-time"}, (
        "a marker in the code covers the id it names"
    )

    print("spec_coverage: 5 checks ok")
    return 0


def main() -> int:
    if "--self-check" in sys.argv:
        return self_check()

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
