"""Compare the spot listings and the CFF literal view of `fontcraft-cli dump` with AFDKO.

    python -I spot_tx.py FONTCRAFT_CLI SPOT TX FONT... [--show N]
    python -I spot_tx.py --build-test-cff OUT.cff
    python -I spot_tx.py --listings FONTCRAFT_CLI SPOT TX TTX FONT... [--show N]

For each font and each listing the font has the table for, the script runs
`spot -ngid -t TAG=LEVEL FONT` (or `tx -dcf [-0|-1] [-b] FONT`) and
`fontcraft-cli dump FONT -t TAG=VIEW --format text [options]`, and prints one line per
case: `equal` or `different` with the first differing line. The last line gives the counts.

`--listings` compares the readable listings instead: `glyphs list` with `spot -ngid -n` (the
`[GID]=<NAME>` items, which spot prints on one line and FontCraft wraps), `glyphs list
--one-per-line` with `spot -ngid -nc` (lines), `glyphs list --columns gid,name,unicodes` with
the standard output of `tx -dump -4` (lines), and the (tag, checksum, offset, length) records of
`tables list` with those of `spot -T` and `ttx -l` (sets, since FontCraft lists the tables in
file order). Collections are left out of the table listings.

`--build-test-cff` writes the CFF table of the `cff_literal_view_writes_as_tx_does` test as a
bare CFF file, to run `tx -dcf` on it.

Expected differences: fonts whose glyphs have no names in `post` or CFF (spot prints glyph
indices, FontCraft the names fontTools makes up); `kern` format 0 subtables with a sentinel
pair (spot prints it); spot warnings, which go to standard output in some listings.
"""

import subprocess
import sys

# (tag, view, spot level, extra fontcraft-cli options)
SPOT = [
    ("cmap", "hex-gid", "5", []),
    ("cmap", "dec-gid", "6", []),
    ("cmap", "hex-name", "7", []),
    ("cmap", "dec-name", "8", []),
    ("cmap", "summary", "11", []),
    ("hmtx", "advances", "5", []),
    ("hmtx", "advances", "6", ["--units", "1000"]),
    ("hmtx", "advances-lsb", "7", []),
    ("hmtx", "advances-lsb", "8", ["--units", "1000"]),
    ("name", "raw", "2", []),
    ("name", "lines", "3", []),
    ("name", "described", "4", []),
    ("GSUB", "features", "5", []),
    ("GPOS", "features", "5", []),
    ("glyf", "points", "5", []),
    ("VORG", "names", "4", []),
    ("kern", "pairs", "5", []),
]

# (tx options, fontcraft-cli options)
TX = [
    ([], []),
    (["-0"], ["--cff-level", "offsets"]),
    (["-1"], ["--cff-level", "bytes"]),
    (["-b", "-T", "tCc"], ["--no-wrap", "--cff-parts", "tCc"]),
]


def run(cmd):
    p = subprocess.run(cmd, capture_output=True)
    return p.returncode, p.stdout.decode("latin-1")


def tables(path):
    data = open(path, "rb").read()
    start = 0
    if data[:4] == b"ttcf":
        start = int.from_bytes(data[12:16], "big")
    count = int.from_bytes(data[start + 4:start + 6], "big")
    return {data[start + 12 + 16 * i:start + 16 + 16 * i].decode("latin-1") for i in range(count)}


def compare(name, ours, theirs, show, counts):
    if ours == theirs:
        counts["equal"] += 1
        print(f"equal      {name}")
        return
    counts["different"] += 1
    a, b = ours.splitlines(), theirs.splitlines()
    for i in range(max(len(a), len(b))):
        x = a[i] if i < len(a) else "<end>"
        y = b[i] if i < len(b) else "<end>"
        if x != y:
            print(f"different  {name}: line {i + 1}\n  ours:   {x[:show]}\n  theirs: {y[:show]}")
            break


def build_test_cff(out):
    def index(items):
        if not items:
            return bytes([0, 0])
        offs = [1]
        for it in items:
            offs.append(offs[-1] + len(it))
        return bytes([0, len(items), 1]) + bytes(offs) + b"".join(items)

    def i32(v):
        return bytes([29]) + v.to_bytes(4, "big")

    g0 = bytes([248, 136, 14])
    g1 = bytes([248, 236, 139, 189, 18, 239, 189, 19, 0xC0, 149, 159, 21, 32, 10, 14])
    subr = bytes([255, 0, 1, 0x80, 0, 139, 5, 11])
    private = bytes([30, 0x0A, 0x03, 0x96, 0x25, 0xFF, 12, 9, 149, 19])
    top = bytes([139, 139, 248, 236, 249, 80, 5]) + i32(45) + bytes([15]) + i32(48) + bytes([17, 149]) + i32(72) + bytes([18])
    data = bytes([1, 0, 4, 1]) + index([b"T"]) + index([top]) + index([]) + index([])
    data += bytes([0, 0, 34]) + index([g0, g1]) + private + index([subr])
    open(out, "wb").write(data)


def records_spot(text):
    out = set()
    for line in text.splitlines():
        if line.startswith("[") and "={'" in line:
            tag, checksum, offset, length = line.split("={", 1)[1].rstrip("}").split(",")
            out.add((tag.strip("'").strip(), checksum.lower(), int(offset, 16), int(length, 16)))
    return out


def records_ttx(text):
    out = set()
    for line in text.splitlines():
        words = line.split()
        if len(words) >= 4 and words[1].startswith("0x"):
            tag, checksum, length, offset = words[:4]
            out.add((tag, checksum[2:].lower(), int(offset), int(length)))
    return out


def listings(argv, show):
    cli, spot, tx, ttx, fonts = argv[0], argv[1], argv[2], argv[3], argv[4:]
    counts = {"equal": 0, "different": 0, "failed": 0}

    def case(name, reference, ours, shape):
        code_a, theirs = run(reference)
        code_b, mine = run(ours)
        if code_a != 0 or code_b != 0:
            counts["failed"] += 1
            print(f"failed     {name}: reference {code_a}, ours {code_b}")
            return
        a, b = shape(mine), shape(theirs)
        if a == b:
            counts["equal"] += 1
            print(f"equal      {name}")
        elif isinstance(a, set):
            counts["different"] += 1
            print(f"different  {name}: ours only {sorted(a - b)[:3]}, theirs only {sorted(b - a)[:3]}"[:show])
        else:
            compare(name, "\n".join(a), "\n".join(b), show, counts)

    def items(text):
        return [w for line in text.splitlines()[1:] for w in line.split()]

    def lines(text):
        return [line.rstrip() for line in text.rstrip("\n").splitlines()]

    for font in fonts:
        glyphs = [cli, "glyphs", "list", font]
        case(f"{font} spot -n", [spot, "-ngid", "-n", font], glyphs, items)
        case(f"{font} spot -nc", [spot, "-ngid", "-nc", font], glyphs + ["--one-per-line"], lines)
        case(f"{font} tx -dump -4", [tx, "-dump", "-4", font], glyphs + ["--columns", "gid,name,unicodes"], lines)
        if open(font, "rb").read(4) == b"ttcf":
            continue
        case(f"{font} spot -T", [spot, "-T", font], [cli, "tables", "list", font], lambda t: records_spot(t) or records_ttx(t))
        case(f"{font} ttx -l", [ttx, "-l", font], [cli, "tables", "list", font], records_ttx)
    print(f"{counts['equal']} equal, {counts['different']} different, {counts['failed']} failed")
    return 1 if counts["different"] or counts["failed"] else 0


def main(argv):
    show = 160
    if "--show" in argv:
        i = argv.index("--show")
        show = int(argv[i + 1])
        del argv[i:i + 2]
    if argv[:1] == ["--listings"]:
        return listings(argv[1:], show)
    if argv[:1] == ["--build-test-cff"]:
        build_test_cff(argv[1])
        return 0
    cli, spot, tx, fonts = argv[0], argv[1], argv[2], argv[3:]
    counts = {"equal": 0, "different": 0, "failed": 0}
    for font in fonts:
        present = tables(font)
        cases = [(f"{t}={v}{' ' + ' '.join(o) if o else ''}", [spot, "-ngid", f"-t{t}={lvl}", font], ["-t", f"{t}={v}"] + o)
                 for t, v, lvl, o in SPOT if t in present]
        if "CFF " in present:
            cases += [(f"CFF=literal {' '.join(o)}", [tx, "-dcf"] + t + [font], ["-t", "CFF=literal"] + o) for t, o in TX]
        for name, reference, ours in cases:
            code_a, theirs = run(reference)
            code_b, mine = run([cli, "dump", font, "--format", "text"] + ours)
            if code_a != 0 or code_b != 0:
                counts["failed"] += 1
                print(f"failed     {font} {name}: reference {code_a}, ours {code_b}")
                continue
            compare(f"{font} {name}", mine.rstrip("\n"), theirs.rstrip("\n"), show, counts)
    print(f"{counts['equal']} equal, {counts['different']} different, {counts['failed']} failed")
    return 1 if counts["different"] or counts["failed"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
