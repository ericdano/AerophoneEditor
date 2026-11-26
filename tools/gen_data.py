#!/usr/bin/env python3
"""Build rust/ae30-core/data/ae30.json from aerophoneCompanion's app/data.js.

aerophoneCompanion (MIT, https://github.com/wrongwaycorrigan/aerophoneCompanion)
parsed Roland's public AE-30/AE-20 MIDI Implementation, Parameter Guide and
Scene List PDFs into data.js. This script turns that into a smaller file the
Rust code loads: every block's parameters with their encoding, raw range,
label, widget kind, options and display format.

usage: python3 tools/gen_data.py <path to aerophoneCompanion/app/data.js>
"""
import json
import os
import re
import sys

src = sys.argv[1]
here = os.path.dirname(os.path.abspath(__file__))
out = os.path.join(here, '..', 'rust', 'ae30-core', 'data', 'ae30.json')

text = open(src, encoding='utf-8').read()
text = re.sub(r'^\s*window\.AE_DATA\s*=\s*', '', text).rstrip().rstrip(';')
D = json.loads(text)

SIZES = {v: k for k, v in D['sizes'].items()}


def encoding(code):
    """(bytes on the wire, bits per byte)."""
    name = SIZES[code]
    if name == 'PADDING':
        return (1, 7)
    m = re.fullmatch(r'INTEGER(\d+)x(\d+)', name)
    n, bits = int(m.group(1)), int(m.group(2))
    if n == 1:
        return (1, 7)  # 1xN: one byte holding an N-bit value
    return (n, bits)    # 2x4, 4x4, 8x4: nibbles; 2x7: two 7-bit bytes


# ---------------- display formats: JS expressions -> structured specs ----------------
NUM = r'(-?[\d.]+)'
LIST = r'(\[(?:"[^"]*"(?:,\s*)?)*\])'


def lin(add=0.0, mul=1.0, dec=0, sign=False, unit=''):
    return {'k': 'lin', 'add': add, 'mul': mul, 'dec': dec, 'sign': sign, 'unit': unit}


def unit_of(s):
    m = re.fullmatch(r"\s*(?:\+\s*'([^']*)')?\s*", s)
    if m is None:
        raise ValueError(s)
    return m.group(1) or ''


def fmt_spec(f):
    if f is None or f == 'value':
        return None
    f = f.strip()
    if f == "'D' + (100 - value) + ':' + value + 'W'":
        return {'k': 'drywet'}
    if f == "value === 64 ? 'C' : value < 64 ? 'L' + (64 - value) : (value - 64) + 'R'":
        return {'k': 'pan64'}
    if f == "value === 0 ? 'Center' : value < 0 ? 'L' + (-value) : 'R' + value":
        return {'k': 'panc'}
    m = re.fullmatch(LIST + r'\[value - (\d+)\]', f)
    if m:
        return {'k': 'list', 'items': json.loads(m.group(1)), 'base': int(m.group(2))}
    m = re.fullmatch(r'value < (\d+) \? ' + LIST + r'\[value\] : value - 1 \+ 1', f)
    if m:
        return {'k': 'list', 'items': json.loads(m.group(2)), 'base': 0}
    m = re.fullmatch(r'value > 127 \? ' + LIST + r'\[value - 128\] : value', f)
    if m:
        return {'k': 'list', 'items': json.loads(m.group(1)), 'base': 128}
    # signed, shifted: ((value - A) > 0 ? '+' : '') + ((value - A) * M) + unit  /  + (value - A) + unit
    m = re.fullmatch(r"\(\(value - " + NUM + r"\) > 0 \? '\+' : ''\) \+ \(\(value - \1\) \* " + NUM + r"\)(.*)", f)
    if m:
        return lin(add=-float(m.group(1)), mul=float(m.group(2)), sign=True, unit=unit_of(m.group(3)))
    m = re.fullmatch(r"\(\(value - " + NUM + r"\) > 0 \? '\+' : ''\) \+ \(value - \1\)(.*)", f)
    if m:
        return lin(add=-float(m.group(1)), sign=True, unit=unit_of(m.group(2)))
    m = re.fullmatch(r"\(value > 0 \? '\+' : ''\) \+ \(value / " + NUM + r"\)\.toFixed\((\d)\)(.*)", f)
    if m:
        return lin(mul=1 / float(m.group(1)), dec=int(m.group(2)), sign=True, unit=unit_of(m.group(3)))
    m = re.fullmatch(r"\(value > 0 \? '\+' : ''\) \+ value(.*)", f)
    if m:
        return lin(sign=True, unit=unit_of(m.group(1)))
    m = re.fullmatch(r'\(value \* ' + NUM + r'\)\.toFixed\((\d)\)(.*)', f)
    if m:
        return lin(mul=float(m.group(1)), dec=int(m.group(2)), unit=unit_of(m.group(3)))
    m = re.fullmatch(r'\(value / ' + NUM + r'\)\.toFixed\((\d)\)(.*)', f)
    if m:
        return lin(mul=1 / float(m.group(1)), dec=int(m.group(2)), unit=unit_of(m.group(3)))
    m = re.fullmatch(r'\(value \* ' + NUM + r'\)(.*)', f)
    if m:
        return lin(mul=float(m.group(1)), unit=unit_of(m.group(2)))
    m = re.fullmatch(r'\(value ([+-]) (-?\d+)\)(.*)', f)
    if m:
        a = float(m.group(2)) * (1 if m.group(1) == '+' else -1)
        return lin(add=a, unit=unit_of(m.group(3)))
    m = re.fullmatch(r'value(.*)', f)
    if m:
        return lin(unit=unit_of(m.group(1)))
    raise ValueError('unhandled format: ' + f)


# ---------------- tables ----------------
TABLES = D['map']['tables']
ROOTS = D['map']['roots']
# Sizes Roland's document gives where the companion padded or trimmed a table.
SIZE_OVERRIDE = {'Perf_common': 0x5A}  # MIDI Control: the unit sends 45 bytes (doc says 0x2E)
GENERIC_FX = re.compile(r'(MFX|Chorus|Reverb|Delay) Parameter \d+$')
KIND = {'Slider': 'slider', 'SelectBox': 'select', 'Switch': 'switch'}

catalog = D['catalog']


def base_name(n):
    return re.sub(r'_\d+$', '', n)


def build_table(root_name, tname, idx):
    rows = TABLES[idx]
    size = SIZE_OVERRIDE.get(tname)
    if size is None:
        last = rows[-1]
        size = last['a'] + encoding(last['s'])[0]
    params = []
    for p in rows:
        if p['a'] >= size:
            continue
        name = p['n']
        if not name or re.match(r'\(?reserved\)?$', name, re.I):
            continue
        nbytes, bits = encoding(p['s'])
        key = f'{root_name}%{tname}_N%{p["a"]}' if f'{root_name}%{tname}_N%{p["a"]}' in catalog else f'{root_name}%{tname}%{p["a"]}'
        c = catalog.get(key)
        vofs = p.get('o', 0)
        dmin, dmax = p.get('mn', 0), p.get('mx', 0)
        entry = {'off': p['a'], 'n': nbytes, 'bits': bits, 'name': name}
        if c:
            vofs = c.get('vofs', vofs)
            dmin, dmax = c.get('min', dmin), c.get('max', dmax)
            entry['label'] = re.sub(r'^-\s*', '', c['label'])
            entry['kind'] = KIND[c['kind']]
            if c.get('options'):
                entry['opts'] = c['options']
                dmin, dmax = 0, len(c['options']) - 1
            f = fmt_spec(c.get('format'))
            if f:
                entry['fmt'] = f
        if GENERIC_FX.search(name):
            entry['generic'] = True
        entry['vofs'] = vofs
        entry['min'], entry['max'] = dmin, dmax
        params.append(entry)
    # Effect-type specific parameters: '<root>%<table>%<off>%fx<type>'
    for k, c in catalog.items():
        parts = k.split('%')
        if len(parts) != 4 or parts[0] != root_name or base_name(parts[1].replace('_N', '')) != tname:
            continue
        off, fx = int(parts[2]), int(parts[3][2:])
        generic = next(e for e in params if e['off'] == off)
        entry = {'off': off, 'n': generic['n'], 'bits': generic['bits'], 'name': c['label'],
                 'label': c['label'], 'kind': KIND[c['kind']], 'fx': fx, 'vofs': c.get('vofs', 0),
                 'min': c.get('min', 0), 'max': c.get('max', 0)}
        if c.get('options'):
            entry['opts'] = c['options']
            entry['min'], entry['max'] = 0, len(c['options']) - 1
        f = fmt_spec(c.get('format'))
        if f:
            entry['fmt'] = f
        params.append(entry)
    return {'size': size, 'params': params}


tables = {}
layouts = {}
ROOT_KEYS = {'System': 'system', 'stp': 'setup', 'Temporary_Scene': 'scene', 'Temporary_Tone_PCM_(1)': 'tone'}
for root in ROOTS:
    key = ROOT_KEYS.get(root['n'])
    if not key:
        continue
    cat_root = 'Temporary_Tone_PCM' if key == 'tone' else root['n']
    rows = TABLES[root['c']]
    if 'c' not in rows[0]:            # the root is itself a parameter table (Setup)
        tables['stp'] = build_table(cat_root, 'stp', root['c'])
        layouts[key] = [{'name': 'stp', 'table': 'stp', 'off': 0}]
        continue
    lay = []
    for blk in rows:
        tname = base_name(blk['n'])
        if tname not in tables:
            tables[tname] = build_table(cat_root, tname, blk['c'])
        lay.append({'name': blk['n'], 'table': tname, 'off': blk['a']})
    layouts[key] = lay

# Assign "Function" lists: Off, CC 01..CC 95, then the named functions.
assign_opts = ['Off'] + [f'CC {i:02d}' for i in range(1, 96)] + D['assignFuncs']
for t in tables.values():
    for p in t['params']:
        if p['name'].endswith('Function') and 'opts' not in p and p['max'] == len(assign_opts) - 1:
            p['opts'] = assign_opts
            p['kind'] = 'select'

fx = {kind: [{'id': e['id'], 'name': e['text']} for e in lst] for kind, lst in D['fx'].items()}

result = {
    'source': 'Generated by tools/gen_data.py from aerophoneCompanion data.js (MIT); '
              'originally from Roland\'s AE-30/AE-20 MIDI Implementation, Parameter Guide and Scene List.',
    'layouts': layouts,
    'tables': tables,
    'fx': fx,
    'tones': D['tones'],
    'toneCats': D['toneCats'],
    'ccNames': D['ccNames'],
    'waves': D['waves'],
    'waveGroups': D['waveGroups'],
}
os.makedirs(os.path.dirname(out), exist_ok=True)
with open(out, 'w', encoding='utf-8') as fh:
    json.dump(result, fh, separators=(',', ':'), ensure_ascii=False)
n = sum(len(t['params']) for t in tables.values())
print(f'{out}: {len(tables)} tables, {n} parameters, {os.path.getsize(out) // 1024} KB')
