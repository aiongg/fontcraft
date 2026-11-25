"""The PostScript autohinter against otfautohint (AFDKO 5.0.1) on the corpus.

Every corpus source is built as a static CFF font twice: without autohinting, and with Autohint on.
otfautohint (default options) hints the first build, and each glyph that build leaves unhinted is
compared with the second build's: the hstem and vstem lists, the hint masks with the number of
segments drawn before each, the counter masks, and the segments drawn as flex. Identical
alignment zones are merged before otfautohint runs, since it rejects them, and a font without
enough zones or without stem widths is hinted with `--no-zones-stems`. Sources whose builds
have the same outlines and the same Private dict count once. Run it with the Python of an AFDKO
installation:

    python -I crates/autohint/tests/oracle/corpus.py --cli target/release/fontcraft-cli \\
        --corpus corpus --out DIR

The report (`report.json`, and the totals on standard output) gives each differing glyph's class:

  * `round`: the same stems, masks and flex but for edges one unit apart
  * `stems`, `masks`, `counters`, `flex`: the first of these that differs
  * `outline`: the two fonts draw different segments, so nothing else compares

`--time SOURCE INSTANCE` times that instance's builds with and without autohinting.
`--reference FONT...` takes compiled CFF fonts, strips their hints, hints them with otfautohint
twice (the second time from a font written again by fontTools) and compares the two, a check of
this script without a FontCraft build.

A test tool only; nothing here ships.
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from fontTools.misc.psCharStrings import T2OutlineExtractor
from fontTools.pens.recordingPen import RecordingPen
from fontTools.ttLib import TTFont

SUFFIXES = {'.glyphs', '.glyphspackage', '.designspace', '.ufo', '.ufoz'}
# The instance built of a source with many: (end of the source's path, instance name).
INSTANCE = [
    ('inter/src/Inter-Roman.glyphspackage', 'Regular'),
    ('inter/src/Inter-Italic.glyphspackage', 'Italic'),
    ('oswald/sources/Oswald.glyphs', 'Regular'),
]
TIMEOUT_S = 1800
MAX_LISTED = 200


def sources(corpus):
    out, stack = [], [corpus.resolve()]
    while stack:
        for p in sorted(stack.pop().iterdir()):
            if p.suffix in SUFFIXES:
                out.append(p)
            elif p.is_dir() and p.name != '.git':
                stack.append(p)
    return sorted(out)


def compile_cff(cli, source, instance, autohint, folder):
    """(path, warnings) of the first static CFF font, or (None, why)."""
    params = {'formats': ['otf'], 'timestamp': 0, 'versionStamp': False, 'subroutinize': False, 'skipFeatures': True,
              'instances': [instance] if instance else True, 'autohint': autohint, 'destination': str(folder)}
    try:
        r = subprocess.run([str(cli), 'run', '--in', str(source), '--cmd', 'font.compile', '--params', json.dumps(params)],
                           capture_output=True, text=True, timeout=TIMEOUT_S, check=False)
    except subprocess.TimeoutExpired:
        return None, 'timed out'
    for line in r.stdout.splitlines():
        try:
            fonts = json.loads(line)['result']['fonts']
        except (ValueError, KeyError, TypeError):
            continue
        if fonts and fonts[0]['files']:
            return Path(fonts[0]['files'][0]['path']), fonts[0].get('warnings', [])
    return None, (r.stderr.strip().splitlines() or ['no static font'])[-1][:200]


def otfautohint(program, source, out, tmp, *options):
    """otfautohint's message when it fails. The program writes temporary files, into `tmp`."""
    out.unlink(missing_ok=True)
    tmp.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, TMPDIR=str(tmp))
    try:
        r = subprocess.run([str(program), '-p', '1', *options, '-o', str(out), str(source)], capture_output=True, text=True, timeout=TIMEOUT_S, check=False, env=env)
    except subprocess.TimeoutExpired:
        return 'otfautohint timed out'
    if not out.exists() and not options and any(w in r.stderr + r.stdout for w in ('BlueValues', 'StemSnap')):
        # FontCraft hints a font without usable zones or stems as this option does.
        return otfautohint(program, source, out, tmp, '--no-zones-stems')
    if not out.exists():
        return 'otfautohint: ' + ' '.join((r.stderr + r.stdout).split())[-200:]
    return ''


class Hints(T2OutlineExtractor):
    """What a charstring says besides its outline."""

    def __init__(self, charstring):
        private = charstring.private
        super().__init__(RecordingPen(), getattr(private, 'Subrs', []), charstring.globalSubrs,
                         private.nominalWidthX, private.defaultWidthX)
        self.kind, self.h, self.v, self.masks, self.counters, self.flexes = 'v', [], [], [], [], []
        self.execute(charstring)

    def countHints(self, index):
        args = self.popallWidth()
        self.hintCount += len(args) // 2
        at, stems = 0, self.h if self.kind == 'h' else self.v
        for a, b in zip(args[::2], args[1::2]):
            at += a
            stems.append((round(at, 3), round(at + b, 3)))
            at += b

    def op_hstem(self, index):
        self.kind = 'h'
        self.countHints(index)

    op_hstemhm = op_hstem

    def op_vstem(self, index):
        self.kind = 'v'
        self.countHints(index)

    op_vstemhm = op_vstem

    def segments(self):
        """Moves, lines and curves drawn so far."""
        return sum(1 for name, _ in self.pen.value if name in ('moveTo', 'lineTo', 'curveTo'))

    def doMask(self, index, into):
        self.kind = 'v'
        if not self.hintMaskBytes:
            self.countHints(index)
            self.hintMaskBytes = (self.hintCount + 7) // 8
        data, index = self.callingStack[-1].getBytes(index, self.hintMaskBytes)
        into.append((self.segments(), data.hex()))
        return data, index

    def op_hintmask(self, index):
        return self.doMask(index, self.masks)

    def op_cntrmask(self, index):
        return self.doMask(index, self.counters)

    def flexed(name):
        def op(self, index):
            self.flexes.append(self.segments())
            return getattr(T2OutlineExtractor, name)(self, index)
        return op

    op_hflex, op_flex, op_hflex1, op_flex1 = flexed('op_hflex'), flexed('op_flex'), flexed('op_hflex1'), flexed('op_flex1')
    del flexed

    def empty(self):
        return not (self.h or self.v or self.masks or self.counters)


def outline(hints):
    return [(n, tuple((round(x, 2), round(y, 2)) for x, y in a)) for n, a in hints.pen.value if n in ('moveTo', 'lineTo', 'curveTo')]


def charstrings(path):
    font = TTFont(path)
    top = font['CFF '].cff.topDictIndex[0]
    if hasattr(top, 'ROS'):
        raise ValueError('CID-keyed')
    strings = top.CharStrings
    return font, {name: Hints(strings[name]) for name in font.getGlyphOrder()}


def private_key(font):
    p = font['CFF '].cff.topDictIndex[0].Private
    return json.dumps({k: v for k, v in sorted(p.rawDict.items()) if k != 'Subrs'}, default=str)


def merge_zones(font):
    """Merges identical alignment zones; True when there were any."""
    private, changed = font['CFF '].cff.topDictIndex[0].Private, False
    for name in ('BlueValues', 'OtherBlues', 'FamilyBlues', 'FamilyOtherBlues'):
        values = private.rawDict.get(name)
        if values:
            # Zones are stored as differences in the file and as positions in fontTools.
            pairs = list(dict.fromkeys(zip(values[::2], values[1::2])))
            if len(pairs) * 2 != len(values):
                setattr(private, name, [v for p in pairs for v in p])
                private.rawDict[name] = getattr(private, name)
                changed = True
    return changed


def difference(ours, theirs):
    """The class of the first difference of two glyphs, and it; None when they are the same."""
    if outline(ours) != outline(theirs):
        return 'outline', ''
    a, b = (ours.h, ours.v), (theirs.h, theirs.v)
    if a != b:
        flat = lambda s: [e for stems in s for stem in stems for e in stem]
        same_shape = [len(x) for x in a] == [len(x) for x in b]
        if same_shape and max(abs(x - y) for x, y in zip(flat(a), flat(b))) <= 1:
            return 'round', f'{a} / {b}'
        return 'stems', f'{a} / {b}'
    for kind, x, y in (('counters', ours.counters, theirs.counters), ('masks', ours.masks, theirs.masks), ('flex', ours.flexes, theirs.flexes)):
        if x != y:
            return kind, f'{x} / {y}'
    return None


def measure(args, source, rel):
    key = hashlib.sha256(rel.encode()).hexdigest()[:12]
    folder = args.out / 'fonts' / key
    instance = next((name for end, name in INSTANCE if str(source).endswith(end)), None)
    plain, why = compile_cff(args.cli, source, instance, False, folder / 'plain')
    if plain is None:
        return {'source': rel, 'skip': f'no build: {why}'}
    ours, why = compile_cff(args.cli, source, instance, True, folder / 'autohint')
    if ours is None:
        return {'source': rel, 'skip': f'no autohinted build: {why}'}
    try:
        font, before = charstrings(plain)
        _, mine = charstrings(ours)
    except Exception as e:  # noqa: BLE001
        return {'source': rel, 'skip': f'not read: {e}'}
    blank = {n for n, g in before.items() if g.empty()}
    key = hashlib.sha256((repr(sorted((n, outline(g)) for n, g in before.items())) + private_key(font)).encode()).hexdigest()
    fed = plain
    if merge_zones(font):
        fed = folder / 'merged.otf'
        font.save(fed)
    if why := otfautohint(args.otfautohint, fed, folder / 'reference.otf', args.out / 'tmp'):
        return {'source': rel, 'skip': why, 'key': key}
    _, theirs = charstrings(folder / 'reference.otf')
    differing = {}
    for name in sorted(blank):
        if name in mine and name in theirs and (d := difference(mine[name], theirs[name])):
            differing[name] = d
    return {'source': rel, 'key': key, 'glyphs': len(blank), 'own hints': len(before) - len(blank), 'differing': differing, 'warnings': why or []}


def run(args):
    corpus = args.corpus.resolve()
    files = [s for s in sources(args.corpus) if not args.only or args.only in str(s)]
    with ThreadPoolExecutor(args.jobs) as pool:
        results = list(pool.map(lambda s: measure(args, s, str(s.relative_to(corpus))), files))
    seen, fonts, skipped = {}, [], Counter()
    for r in results:
        if 'skip' in r:
            skipped[r['skip'].split(':')[0]] += 1
        elif r['key'] in seen:
            r['skip'] = f"same font as {seen[r['key']]}"
            skipped['same font'] += 1
        else:
            seen[r['key']] = r['source']
            fonts.append(r)
    (args.out / 'report.json').write_text(json.dumps(results, indent=1))
    total, classes = sum(r['glyphs'] for r in fonts), Counter(d[0] for r in fonts for d in r['differing'].values())
    for r in fonts:
        if r['differing']:
            by = Counter(d[0] for d in r['differing'].values())
            print(f"{r['source']}: {len(r['differing'])} of {r['glyphs']} glyphs differ {dict(by)}, first: {next(iter(r['differing'].items()))}"[:400])
    for why, n in skipped.most_common():
        print(f'skipped {n}: {why}')
    same = total - sum(classes.values())
    print(f'{len(fonts)} fonts of {len(files)} sources, {total} glyphs, identical {same} = {100 * same / max(total, 1):.2f}%, differing {dict(classes)}')


def timing(args):
    source, instance = args.time
    for autohint in (False, True, False, True):
        start = time.time()
        path, why = compile_cff(args.cli, Path(source), instance, autohint, args.out / 'time')
        print('autohint' if autohint else 'plain', f'{time.time() - start:.2f} s', path or why)


def reference(args):
    for path in args.reference:
        work = args.out / path.stem
        work.mkdir(parents=True, exist_ok=True)
        font = TTFont(path)
        font['CFF '].cff.desubroutinize()
        font['CFF '].cff.remove_hints()
        merged = merge_zones(font)
        font.save(work / 'plain.otf')
        start = time.time()
        why = otfautohint(args.otfautohint, work / 'plain.otf', work / 'reference.otf', args.out / 'tmp')
        spent = time.time() - start
        if why:
            print(path.name, why)
            continue
        TTFont(work / 'plain.otf').save(work / 'again.otf')
        otfautohint(args.otfautohint, work / 'again.otf', work / 'reference2.otf', args.out / 'tmp')
        _, plain = charstrings(work / 'plain.otf')
        _, a = charstrings(work / 'reference.otf')
        _, b = charstrings(work / 'reference2.otf')
        assert all(g.empty() for g in plain.values())
        same = sum(difference(a[n], b[n]) is None for n in a)
        hinted = [g for g in a.values() if not g.empty()]
        print(f"{path.name}: {len(a)} glyphs, {len(hinted)} hinted, {sum(len(g.h) for g in hinted)} hstems, {sum(len(g.v) for g in hinted)} vstems, "
              f"{sum(len(g.masks) for g in hinted)} masks, {sum(len(g.counters) for g in hinted)} counter masks, {sum(len(g.flexes) for g in a.values())} flexes, "
              f"zones merged: {merged}, {spent:.1f} s; run twice: {same} of {len(a)} the same")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--cli', type=Path, default=Path('target/release/fontcraft-cli'))
    ap.add_argument('--corpus', type=Path, default=Path('corpus'))
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--otfautohint', type=Path, default=Path(sys.executable).parent / 'otfautohint')
    ap.add_argument('--only', help='sources whose path holds this text')
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--time', nargs=2, metavar=('SOURCE', 'INSTANCE'))
    ap.add_argument('--reference', nargs='+', type=Path, metavar='FONT')
    args = ap.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    if args.reference:
        reference(args)
    elif args.time:
        timing(args)
    else:
        run(args)


if __name__ == '__main__':
    main()
