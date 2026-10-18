# A CFF2 table: a variation store with two VarData, a Private DICT with blended BlueValues and
# StdHW and a key CFF2 does not keep, charstrings with vsindex and blend, a hint mask after a
# blend, and subroutines that end with operands.
table CFF2
header head2 top
top dict @fda o12.36; @cs o17; @vs o24
gsubrs index2 cs:10 20 | cs:1 2 3
fda index2 dict:#p0 @p0 o18
vs raw 0032 0001 00000010 0002 00000020 00000028 0001 0002 0000 4000 4000 0000 2000 4000 0000 0000 0001 0000 0000 0000 0002 0000 0001
cs index2 cs:100 100 rmoveto 200 hlineto | cs:1 vsindex 10 20 5 6 7 8 2 blend rmoveto -107 callgsubr rmoveto | cs:5 10 20 1 blend hstemhm hintmask 10000000 0 0 rmoveto -107 callsubr
p0 dict -10 10 500 10 5 0 10 0 4 o23 o6; 50 6 1 o23 o10; 80 o11; 400 o20; @s0-@p0 o19
s0 index2 cs:100 0 rlineto

table maxp
all raw 00005000 0003

table post
all raw 00030000 00000000 FF9C 0032 00000000 00000000 00000000 00000000 00000000
