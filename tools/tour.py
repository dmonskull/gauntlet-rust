"""Reads a tour's trigger list and game log (tour.sh) and prints, per
trigger, whether it switched on and whether the node it moves arrived."""
import re, sys
trig_file, log_file, level = sys.argv[1:4]
order = [int(l.split()[0]) for l in open(trig_file) if l.strip()]
ansi = re.compile(r'\x1b\[[0-9;]*m')
log = [ansi.sub('', l) for l in open(log_file, errors='replace')]
reg, on, fired, arrived, hops, nofloor = {}, set(), set(), [], {}, set()
for l in log:
    m = re.search(r'trigger (\d+) (0x[0-9a-f]+) flags (0x[0-9a-f]+) at .* -> node (Some\((\d+)\)|None) \((.*?)\) heights (\S+)/(\S+) id (-?\d+) next (-?\d+)', l)
    if m:
        reg[int(m[1])] = dict(sub=m[2], flags=m[3], node=m[5] and int(m[5]), name=m[6], off=float(m[7]), on=float(m[8]), id=m[9], next=m[10])
        continue
    m = re.search(r'trigger (\d+) \(subtype', l)
    if m: on.add(int(m[1])); continue
    m = re.search(r'trigger (\d+) \(id -?\d+\) fired', l)
    if m: fired.add(int(m[1])); continue
    m = re.search(r'mover (\d+) arrived', l)
    if m: arrived.append(int(m[1])); continue
    m = re.search(r'GDL_HOPS: no floor .* hop (\d+)', l)
    if m: nofloor.add(order[int(m[1])])
panics = [l for l in log if 'panicked' in l]
died = any('the hero has died' in l for l in log)
boxes = sum('message box:' in l for l in log)
exits = [l.split('exit to ')[1].strip() for l in log if 'exit to ' in l]
bad = 0
for idx in order:
    r = reg.get(idx)
    mover = r and r['off'] != r['on']
    state = 'NOFLOOR' if idx in nofloor else ('on' if idx in on else 'OFF')
    move = '' if not r else ('no node' if r['node'] is None else ('arrived' if r['node'] in arrived else 'NOT ARRIVED') if mover else 'no mover')
    if state != 'on' or move == 'NOT ARRIVED': bad += 1
    desc = f"{r['sub']} -> {r['name']} {r['off']}/{r['on']}" if r else 'unregistered'
    print(f"  {level} {idx}: {state:7} {move:11} {desc}")
last = max([int(m[1]) for l in log for m in [re.search(r'hop (\d+) to', l)] if m] or [-1])
notes = (', PANIC' if panics else '') + (', DIED' if died else '') + (f', left by exit to {exits[0]}' if exits else '') + (f', {boxes} boxes' if boxes else '')
print(f"{level}: {len(order)} triggers, {bad} to check, hops done {last + 1}/{len(order)}{notes}")
