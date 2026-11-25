"""Group the differences of an oracle run by cause (M2.11).

    fontcraft-cli oracle --corpus --instances --differences 10000 --json > instances.json
    python -I classify.py instances.json

Works on the JSON of any `oracle` run. Prints each class of difference, a table and its key with
the glyph, lookup and location names replaced by `*`, with the number of sources and differences and
the sources; then the sources that were not compared, by message. Two sources with the same set of
classes usually share a cause, so the sets are listed too.
"""

import argparse
import collections
import json
import re
import sys


def cls(table, key):
    """`key` with the names in it replaced by `*`."""
    key = re.sub(r"^(glyf|hmtx|vmtx|CFF) \S+ ", r"\1 * ", key)
    key = re.sub(r"^lookup \d+$", "lookup *", key)
    key = re.sub(r"^(cmap|post glyphName|name) .*$", r"\1 *", key)
    key = re.sub(r" at (instance|midpoint|master|virtual master) .*$", r" at \1 *", key)
    return f"{table.strip()}: {key}"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("report")
    parser.add_argument("--sources", type=int, default=6, help="sources named per class")
    args = parser.parse_args()
    with open(args.report, encoding="utf-8") as f:
        reports = json.load(f)["sources"]
    classes = collections.defaultdict(lambda: collections.defaultdict(list))
    sets = collections.defaultdict(list)
    failed = collections.defaultdict(list)
    for r in reports:
        if r["outcome"] != "compared":
            failed[(r["outcome"], "; ".join(r.get("messages", []))[:160])].append(r["source"])
            continue
        mine = set()
        for t in r["tables"]:
            if t["status"] != "different":
                continue
            for d in t["differences"]:
                c = cls(t["tag"], d["key"])
                mine.add(c)
                classes[c][r["source"]].append(d)
        if mine:
            sets[tuple(sorted(mine))].append(r["source"])
    different = sum(len(v) for v in sets.values())
    print(f"{len(reports)} compared or tried, {different} with differences, {sum(map(len, failed.values()))} not compared")
    print("\nclasses")
    for c, by_source in sorted(classes.items(), key=lambda kv: -len(kv[1])):
        total = sum(map(len, by_source.values()))
        print(f"  {c}: {len(by_source)} sources, {total} differences")
        for source, ds in list(by_source.items())[: args.sources]:
            d = ds[0]
            print(f"      {source}: {d['key']}: ours {d['ours'][:70]!r}, theirs {d['theirs'][:70]!r}")
    print("\nsources by their set of classes")
    for key, sources in sorted(sets.items(), key=lambda kv: -len(kv[1])):
        print(f"  {len(sources)}: {', '.join(key)}")
        for s in sources:
            print(f"      {s}")
    if failed:
        print("\nnot compared")
        for (outcome, message), sources in failed.items():
            print(f"  {len(sources)} {outcome}: {message}")
    sys.exit(0)


if __name__ == "__main__":
    main()
