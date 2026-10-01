"""Reads constants from the game's main.dol by address (for decoding with the
decompile). R2/R13 are the small-data bases.

Usage: python3 tools/mydol.py f32:r2-5b44 f64:r2-5bf8 u32:80122628 cstr:80113c7c
  (kinds f32, f64, u32, i32, cstr; addresses hex, or r2±offset)
The game folder is $GDL_GAME (default ~/Desktop/GauntletDarkLegacy)."""
import os, struct, sys
DOL = os.path.join(os.environ.get('GDL_GAME', os.path.expanduser('~/Desktop/GauntletDarkLegacy')), 'sys', 'main.dol')
R2 = 0x8034D100
d = open(DOL, 'rb').read()
offs = struct.unpack('>18I', d[0:72]); addrs = struct.unpack('>18I', d[72:144]); sizes = struct.unpack('>18I', d[144:216])
def at(a):
    for o, s, n in zip(offs, addrs, sizes):
        if n and s <= a < s + n:
            return o + a - s
    raise ValueError(hex(a))
def u32(a): return struct.unpack('>I', d[at(a):at(a)+4])[0]
def i32(a): return struct.unpack('>i', d[at(a):at(a)+4])[0]
def f32(a): return struct.unpack('>f', d[at(a):at(a)+4])[0]
def f64(a): return struct.unpack('>d', d[at(a):at(a)+8])[0]
def cstr(a):
    o = at(a); e = d.index(b'\0', o); return d[o:e].decode('latin1')
def r2f(off): return f32(R2 + off)
def r2d(off): return f64(R2 + off)
if __name__ == '__main__':
    for arg in sys.argv[1:]:
        kind, a = arg.split(':')
        a = int(a, 16) if not a.startswith('r2') else R2 + int(a[2:], 16) if not a[2:].startswith('-') else R2 - int(a[3:], 16)
        print(arg, {'f32': f32, 'f64': f64, 'u32': lambda x: hex(u32(x)), 'i32': i32, 'cstr': cstr}[kind](a))
