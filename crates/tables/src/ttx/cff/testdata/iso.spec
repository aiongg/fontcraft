# A name-keyed CFF font on the predefined ISOAdobe charset (charset offset 0), with fewer glyphs
# than the charset names, and the defaults of every Top DICT and Private DICT key.
table CFF
header raw 01 00 04 01
names index str:Test-Iso
top index dict:0 o15; #priv @priv o18; @cs o17
strings index
gsubrs index
cs index cs:endchar | cs:-50 endchar | cs:10 20 rmoveto endchar
priv dict 500 o20

table maxp
all raw 00005000 0003
