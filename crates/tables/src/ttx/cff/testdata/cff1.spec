# A name-keyed CFF font, written block by block (see `tests.rs` for the format): Top DICT with
# strings, reals and arrays, a format 1 charset with a repeated name, a format 1 encoding with a
# supplement, a Private DICT with local subroutines, hint masks of one and two bytes, a counter
# mask, 16.16 numbers, an unknown operator ending a charstring, operands left after the last
# operator, and subroutines that are never called. A VORG table with an out-of-range glyph and a
# repeated glyph.
table CFF
header raw 01 00 04 01
names index str:Test-Regular
top index dict:391 o0; 392 o1; 393 o12.0; 394 o2; 395 o3; 388 o4; 1 o12.1; r-12.5 o12.2; -150 o12.3; 60 o12.4; r5E-4 0 r1.5E-4 r5E-4 0 0 o12.7; 4000000 o13; -10 -250 1100 900 o5; 1 2 3 o14; 397 o12.21; @charset o15; @enc o16; #priv @priv o18; @cs o17
strings index str:1.000 | str:Notice text | str:Copyright line one\n   line two\nend | str:Test Regular | str:Test | str:a.alt | str:/OrigFontType /TrueType def
gsubrs index cs:10 20 rlineto return | cs:30 40 rlineto return
charset raw 01 0001 00 0022 01 0022 00 018C 00
enc raw 81 02 20 00 41 01 01 C0 0022
cs index cs:500 50 50 rmoveto 400 hlineto -105 callsubr -105 callsubr endchar | cs:-100 endchar | cs:0 50 300 50 hstemhm 100 80 vstemhm hintmask 11100000 50 50 rmoveto -107 callsubr hintmask 01000000 0 300 rlineto f1.5 f-0.25 rlineto -107 callgsubr endchar | cs:10 20 hstem 30 40 cntrmask 11000000 0 0 rmoveto -105 callsubr f0.1 f-2 rlineto 1 2 3 4 5 6 7 8 9 10 11 12 50 flex endchar | cs:0 10 20 10 40 10 60 10 80 10 hstemhm 0 10 20 10 40 10 60 10 vstemhm hintmask 1111111110000000 10 10 rmoveto 5 x02 6 6 rlineto endchar | cs:50 50 rmoveto 100 200
priv dict -15 15 500 15 o6; -250 10 o7; r0.0375 o12.9; 50 o10; 80 o11; 50 5 o12.12; 1 o12.14; 500 o20; 600 o21; @subrs-@priv o19
subrs index cs:100 0 rlineto return | cs:5 5 rlineto return | cs:0 100 rlineto return

table maxp
all raw 00005000 0006

table VORG
all raw 0001 0000 0370 0004 0004 0384 0001 0366 0009 035C 0001 036B
