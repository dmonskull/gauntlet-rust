"""Dumps a hero class's PDATA/<class>.WAD: its SFXX (effect/sound) records,
its 0x58-byte DAMG attack records and the PDAT's per-action DAMG indices
(docs/chunk-files.md "Player stats", docs/coop.md "Co-op combos").

Usage: python3 tools/hdamg.py WAR   (game folder: $GDL_GAME)"""
import os,struct,sys
GAME=os.environ.get('GDL_GAME', os.path.expanduser('~/Desktop/GauntletDarkLegacy'))
def load(cls):
    p=f'{GAME}/Gauntlet/PDATA/{cls}.WAD'
    d=open(p,'rb').read()
    diro,cnt=struct.unpack_from('<II',d,0)
    ch={}
    for i in range(cnt):
        tag=d[diro+i*16:diro+i*16+4][::-1].decode()
        off,c1,c2=struct.unpack_from('<III',d,diro+i*16+4)
        ch[tag]=(off,c1)
    return d,ch
cls=sys.argv[1] if len(sys.argv)>1 else 'WAR'
d,ch=load(cls)
off,n=ch['SFXX']
print('SFXX')
for i in range(n):
    r=d[off+i*0x50:off+(i+1)*0x50]
    fl,nx=struct.unpack_from('<Ii',r,0)
    eff=r[0x10:0x20].split(b'\0')[0].decode('latin1'); snd=r[0x20:0x30].split(b'\0')[0].decode('latin1')
    o=struct.unpack_from('<3f',r,0x30); life,scale=struct.unpack_from('<2f',r,0x3c); size=struct.unpack_from('<f',r,0x4c)[0]
    print(f' {i:2} fl={fl:#x} next={nx} eff={eff!r} snd={snd!r} off={tuple(round(x,2) for x in o)} life={life:.2f} scale={scale:.2f} size={size:.2f} 08={r[8:16].hex()} 44={r[0x44:0x4c].hex()}')
off,n=ch['DAMG']
print('DAMG')
for i in range(n):
    r=d[off+i*0x58:off+(i+1)*0x58]
    kind,flags=struct.unpack_from('<hH',r,0)
    fl=struct.unpack_from('<20f',r,4)
    sh=struct.unpack_from('<4h',r,0x50)
    words=' '.join(f'{x:.3g}' for x in fl)
    print(f' {i:2} kind={kind} flags={flags:#06x} f[04..4f]={words} | 50..57={sh}')
off,n=ch['PDAT']
r=d[off:off+0x180]
print('PDAT 0..0x28 shorts', struct.unpack_from('<20h',r,0))
