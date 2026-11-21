"""Nova's only binary asset: the font, at its ASCII positions in a 4 KiB
pattern table. Every other tile is drawn as text in art.nst and encoded by
the compiler."""
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..', 'scripts', 'art'))
from tiles import Table

table = Table()
table.font('c')
with open(os.path.join(os.path.dirname(__file__), 'assets', 'font.chr'), 'wb') as f:
    f.write(bytes(table.data()[:4096]))
print('wrote assets/font.chr')
