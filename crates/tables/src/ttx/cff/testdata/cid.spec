# A CID-keyed CFF font: ROS, CIDCount, a format 2 charset of CIDs, FDSelect format 3 over two
# font dicts, each with its own Private DICT and local subroutines, and a key the font dicts do
# not keep.
table CFF
header raw 01 00 04 01
names index str:Test-CID
top index dict:391 392 0 o12.30; 5 o12.34; -50 -120 1000 880 o5; 0 o16; @charset o15; @fds o12.37; @fda o12.36; @cs o17
strings index str:Adobe | str:Identity | str:Test-CID-Alpha | str:Test-CID-Beta
gsubrs index
charset raw 02 0001 0003
fds raw 03 0002 0000 00 0003 01 0005
fda index dict:393 o12.38; r0.001 0 0 r0.001 0 0 o12.7; #p0 @p0 o18 | dict:394 o12.38; 388 o4; 1 o12.1; #p1 @p1 o18
cs index cs:endchar | cs:100 100 rmoveto -107 callsubr endchar | cs:50 50 rmoveto endchar | cs:200 0 0 rmoveto -107 callsubr endchar | cs:endchar
p0 dict -20 20 600 20 o6; 40 o10; @s0-@p0 o19
s0 index cs:0 100 rlineto return
p1 dict 300 o20; 0 o21; @s1-@p1 o19
s1 index cs:100 0 rlineto return | cs:7 7 rlineto return

table maxp
all raw 00005000 0005
