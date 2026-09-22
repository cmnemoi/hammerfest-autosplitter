"""Turns a capture into a fixture a Rust test can replay.

A capture is made for a human: `metadata.json` holds 124 regions with their
digests and their holes, and the bytes are gzipped. A test wants neither JSON
nor gzip, because both would cost the crate a dependency for one test.

So this script writes two files instead:

    heap.bin.gz  the bytes of the regions that are kept, concatenated
    index.txt    the plugin range, the state the game was in, the region map

`index.txt` is read by `src/replay.rs` with `split_whitespace`, and nothing
else. The bytes are gzipped at level 9: the file is written once and read
often, so the slowest level is the right one. It buys a third over level 1.

Keeping every region costs 85 MiB, which stays out of git. `--keep` names the
regions that carry the object graph, and the others become zeros: they are
still in the map, at their address and their size, so the search walks the
same heap. It just finds nothing in them.

    mise run replay-fixture -- main-world
    mise run replay-fixture -- main-world --keep 0x4793b400000 0xf811400000

The Rust test says which regions to keep. It prints them when it runs on the
whole capture.
"""

import argparse
import gzip
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
MIB = 1 << 20


def load(fixture: pathlib.Path):
    meta = json.loads((fixture / "metadata.json").read_text(encoding="utf-8"))
    path = fixture / meta["heap"]["file"]
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rb") as stream:
        return meta, stream.read()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("name", help="the capture under fixtures/")
    ap.add_argument("--keep", nargs="*", default=None,
                    help="base addresses of the regions to keep, in hex. "
                         "Every other region becomes zeros. The default keeps "
                         "them all.")
    ap.add_argument("--out", default=None,
                    help="where to write. Default: fixtures/replay/<name>")
    args = ap.parse_args()

    fixture = ROOT / "fixtures" / args.name
    if not (fixture / "metadata.json").is_file():
        print(f"no capture at {fixture}", file=sys.stderr)
        return 1

    meta, stream = load(fixture)
    keep = {int(k, 16) for k in args.keep} if args.keep is not None else None

    out = pathlib.Path(args.out) if args.out else ROOT / "fixtures" / "replay" / args.name
    out.mkdir(parents=True, exist_ok=True)

    lines = []
    plugin = meta["plugin"]
    lines.append("plugin %s %s" % (plugin["base"], hex(int(plugin["base"], 16) + plugin["size"])))

    state = meta["state_before"]
    lines.append("state level=%d set=%s dim=%d game_mode=%s"
                 % (state["level"], state["set"], state["dim"], state["game_mode"]))

    stored = 0
    with gzip.open(out / "heap.bin.gz", "wb", compresslevel=9) as blob:
        for region in meta["heap"]["regions"]:
            base = int(region["base"], 16)
            size = region["size"]
            if keep is not None and base not in keep:
                # Still in the map, at its address, and empty.
                lines.append("region %s %d -" % (region["base"], size))
                continue
            blob.write(stream[region["offset"]:region["offset"] + size])
            lines.append("region %s %d %d" % (region["base"], size, stored))
            stored += size

    (out / "index.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")

    kept = sum(1 for line in lines if line.startswith("region") and not line.endswith("-"))
    total = sum(1 for line in lines if line.startswith("region"))
    print("%s -> %s" % (args.name, out))
    packed = (out / "heap.bin.gz").stat().st_size
    print("  %d regions of %d kept, %.1f MiB, %.1f MiB gzipped"
          % (kept, total, stored / MIB, packed / MIB))
    return 0


if __name__ == "__main__":
    sys.exit(main())
