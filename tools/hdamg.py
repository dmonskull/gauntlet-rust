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
    selectors=struct.unpack_from('<2h',r,0x30); o=struct.unpack_from('<3f',r,0x34); duration=struct.unpack_from('<f',r,0x40)[0]; color=struct.unpack_from('<I',r,0x4c)[0]
    print(f' {i:2} fl={fl:#x} next={nx} eff={eff!r} snd={snd!r} off={tuple(round(x,2) for x in o)} selectors={selectors} duration={duration:.2f} color={color:#010x} 08={r[8:16].hex()} 44={r[0x44:0x4c].hex()}')
off,n=ch['DAMG']
print('DAMG (kind flags blow | size radius unused duration missile_life trail_scale yaw cone pitch | offset | damage speed gravity | sfxx effect/hit/trail | next | start end hint)')
for i in range(n):
    r=d[off+i*0x58:off+(i+1)*0x58]
    kind,flags,blow=struct.unpack_from('<hHI',r,0)
    g=lambda o: struct.unpack_from('<f',r,o)[0]
    sh=struct.unpack_from('<3h',r,0x48)
    se=struct.unpack_from('<3h',r,0x50); nx=struct.unpack_from('<h',r,0x4e)[0]
    print(f' {i:2} k={kind} fl={flags:#06x} blow={blow:#x} | {g(8):g} {g(0xc):g} {g(0x10):g} {g(0x14):g} {g(0x18):g} {g(0x1c):g} {g(0x20):g} {g(0x24):g} {g(0x28):g}'
          f' | ({g(0x2c):g},{g(0x30):g},{g(0x34):g}) | {g(0x38):g} {g(0x3c):g}-{g(0x40):g} {g(0x44):g} | {sh} | {nx} | {se}')
off,n=ch['PDAT']
r=d[off:off+0x180]
sh=struct.unpack_from('<12h',r,0x0C)
names=['ATTPWRACLOSE','ATTPWRALOW','ATTPWRAMED','ATT360','ATTPWRATHROW','ATTPWRB','ATTPWRC','ATTPWRC2','COMBOACT1','COMBOACT3','?20','act7B']
print('PDAT', 'SFXX',struct.unpack_from('<h',r,0)[0],'DAMG',struct.unpack_from('<h',r,2)[0], ' '.join(f'{nm}={v}' for nm,v in zip(names,sh)))
