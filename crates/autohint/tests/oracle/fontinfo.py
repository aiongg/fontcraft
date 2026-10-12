"""Expected values for the private-dictionary tests of crates/autohint (src/fontinfo.rs).

Parses and merges an override set with otfautohint's fdTools (AFDKO 5.0.1). Run it with the
Python of an AFDKO installation:

    python -I crates/autohint/tests/oracle/fontinfo.py

A test tool only; nothing here ships.
"""
from afdko.otfautohint.fdTools import FDDict, parseFontInfoFile, mergeFDDicts
TEXT = """# overrides
begin FDDict ST_Smallcaps
    OrigEmSqUnits 1000
    FlexOK false
    BaselineOvershoot -20
    BaselineYCoord 0
    CapHeight 900
    CapOvershoot 20
    LcHeight 700
    LcOvershoot 15
    DescenderHeight -250
    DescenderOvershoot -10
    DominantV [236 267]
    DominantH [141 152]
    VCounterChars [m.sc n.sc]
end FDDict ST_Smallcaps

begin FDDict LM
    BaselineOvershoot -25
    BaselineYCoord 0
    CapHeight 950
    CapOvershoot 25
    DominantV [236]
    DominantH [141]
end FDDict LM

begin GlyphSet LM
    ^[Ll].*\\.sc$  ^m\\.sc$
end GlyphSet LM

begin GlyphSet ST_Smallcaps
    [Tt]\\S*\\.sc
end GlyphSet ST_Smallcaps
"""
base = FDDict(0, 'base')
for k, v in [('BaselineYCoord', 0), ('BaselineOvershoot', -12), ('CapHeight', 700), ('CapOvershoot', 12), ('BlueFuzz', 1)]:
    base.setInfo(k, v)
base.setInfo('DominantV', [80]); base.setInfo('DominantH', [70])
base.buildBlueLists()
m = {0: base}
names = ['a', 'l.sc', 'L.sc', 't.sc', 'T.sc', 'm.sc', 'x']
sel, final = parseFontInfoFile(m, TEXT, names, 800, -200, 'Test')
print(sel, final)
for i, d in m.items():
    print(i, d.DictName, d.BlueFuzz, d.FlexOK, d.DominantH, d.DominantV, d.VCounterChars, d.FontName)
    print('   blue', d.BlueValuesPairs)
    print('   other', d.OtherBlueValuesPairs)
print(mergeFDDicts([m[1], m[2]]))
