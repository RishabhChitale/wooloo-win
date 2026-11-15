#!/usr/bin/env python3
"""Independent check of docs/sync-vectors.json, derived from docs/sync-protocol.md only."""
import json, re, sys, os
D = json.load(open(os.path.join(os.path.dirname(__file__), '..', 'docs', 'sync-vectors.json')))
fails = []; counts = {}

def esc(s):
    o = ['"']
    for c in s:
        if c == '"': o.append('\\"')
        elif c == '\\': o.append('\\\\')
        elif c == '\n': o.append('\\n')
        elif c == '\t': o.append('\\t')
        elif c == '\r': o.append('\\r')
        elif c == '\b': o.append('\\b')
        elif c == '\f': o.append('\\f')
        elif ord(c) < 0x20: o.append('\\u%04x' % ord(c))
        else: o.append(c)
    o.append('"'); return ''.join(o)

def canon(v):
    if v is None: return 'null'
    if v is True: return 'true'
    if v is False: return 'false'
    if isinstance(v, str): return esc(v)
    if isinstance(v, (int, float)): return repr(v)
    if isinstance(v, list): return '[' + ','.join(canon(x) for x in v) + ']'
    return '{' + ','.join(esc(k) + ':' + canon(v[k]) for k in sorted(v)) + '}'

def hk(h): return (h['wall'], h['counter'])
def rk(r): return (hk(r['hlc']), r['node'].encode(), canon(r['value']).encode())

NODE = re.compile(r'^[a-z0-9-]{1,64}$')
UUID = re.compile(r'^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}$')
def text(s, lo, hi):
    return isinstance(s, str) and lo <= len(s) <= hi and not re.search('[\x00-\x1f\x7f]', s)
def num(v): return isinstance(v, (int, float)) and not isinstance(v, bool) and v == v and abs(v) != float('inf')

def valid(op):
    try:
        k, f, v, h, n = op['key'], op['field'], op['value'], op['hlc'], op['node']
    except KeyError: return False
    if not isinstance(n, str) or not NODE.match(n): return False
    if not (isinstance(h, dict) and all(isinstance(h.get(x), int) and not isinstance(h.get(x), bool) and h[x] >= 0 for x in ('wall', 'counter'))): return False
    if not isinstance(k, str): return False
    if k.startswith('space:'):
        if not UUID.match(k[6:]): return False
        if f == 'name': return text(v, 1, 200)
        if f == 'category': return v is None or text(v, 1, 64)
        if f == 'color': return v is None or (isinstance(v, str) and re.fullmatch(r'#[0-9a-f]{6}', v) is not None)
        if f == 'order': return num(v)
        if f == 'notes':
            return v is None or (isinstance(v, str) and len(v.encode()) <= 65536 and not re.search('[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]', v))
        if f == 'pinned_tabs': return v is None or (isinstance(v, list) and len(v) <= 64 and all(text(x, 1, 200) for x in v))
        if f == 'host': return isinstance(v, str) and NODE.match(v) is not None
        if f == 'herdr_id': return isinstance(v, str) and re.fullmatch(r'[A-Za-z0-9_.:-]{1,64}', v) is not None
        if f == 'deleted': return isinstance(v, bool)
        return False
    if k.startswith('unitcat:'):
        return f == 'category' and text(k[8:], 1, 200) and (v is None or text(v, 1, 64))
    return False

def merge(ops, now, maxf=D['max_future_ms']):
    doc = {}; last = (0, 0); rej = 0
    for op in ops:
        if not valid(op) or op['hlc']['wall'] > now + maxf: rej += 1; continue
        last = max(last, hk(op['hlc']))
        p = (op['key'], op['field'])
        if p not in doc or rk(op) > rk(doc[p]): doc[p] = op
    return doc, last, rej

def chk(cat, name, ok, detail=''):
    counts[cat] = counts.get(cat, 0) + 1
    if not ok: fails.append((cat, name, detail))

for v in D['canonical']: chk('canonical', repr(v['value'])[:30], canon(v['value']) == v['text'], canon(v['value']))
for v in D['hlc']:
    last = (v['start']['wall'], v['start']['counter'])
    for i, s in enumerate(v['steps']):
        if s['op'] == 'tick':
            last = (s['now'], 0) if s['now'] > last[0] else (last[0], last[1] + 1)
        else:
            r = hk(s['remote']); last = max(last, r)
        chk('hlc', f"{v['name']}#{i}", last == (s['expect']['wall'], s['expect']['counter']), last)
for v in D['compare']:
    w = 'equal' if rk(v['a']) == rk(v['b']) else 'a' if rk(v['a']) > rk(v['b']) else 'b'
    chk('compare', v['name'], w == v['winner'], w)
for v in D['validate']: chk('validate', v['name'], valid(v['op']) == v['valid'], valid(v['op']))
for v in D['merge']:
    doc, last, rej = merge(v['ops'], v['now'])
    regs = sorted(doc.values(), key=lambda r: (r['key'].encode(), r['field'].encode()))
    chk('merge', v['name'] + ' regs', regs == v['expect_registers'])
    chk('merge', v['name'] + ' rejected', rej == v['expect_rejected'], rej)
    chk('merge', v['name'] + ' clock', last == (v['expect_clock']['wall'], v['expect_clock']['counter']), last)
    live = sorted(k[6:] for k in {p[0] for p in doc if p[0].startswith('space:')}
                  if not (doc.get((k, 'deleted')) or {}).get('value') is True)
    chk('merge', v['name'] + ' live', live == sorted(v['expect_live_spaces']), live)
for v in D['unitcat_export']:
    out = {r['key'][8:]: r['value'] for r in v['registers'] if r['key'].startswith('unitcat:') and r['value'] is not None}
    chk('unitcat_export', v['name'], out == v['expect'], out)
for v in D['unitcat_import']:
    snap, f = v['snapshot'], v['file']
    if f is None: ops, ns = [], snap
    else:
        ok = lambda n, c: text(n, 1, 200) and text(c, 1, 64)
        fv = {n: c for n, c in f.items() if ok(n, c)}; sv = {n: c for n, c in snap.items() if ok(n, c)}
        ops = [(n, c) for n, c in fv.items() if sv.get(n) != c] + [(n, None) for n in sv if n not in fv]
        ops = [{'key': 'unitcat:' + n, 'field': 'category', 'value': c} for n, c in sorted(ops)]
        ns = fv  # spec s7: snapshot is the valid entries only
    chk('unitcat_import', v['name'] + ' ops', ops == v['expect_ops'], ops)
    chk('unitcat_import', v['name'] + ' snap', ns == v['expect_snapshot'], ns)

def sort_regs(d): return sorted(d.values(), key=lambda r: (r['key'].encode(), r['field'].encode()))

def exchange_model(v):
    """Spec s8: the responder merges what it received, replies with its registers that differ
    from or are missing in the received set; the initiator applies the reply."""
    now = v['now']
    def raw(ops):  # a replica's stored document is trusted as loaded (no validation, no skew guard)
        d = {}
        for o in ops:
            p = (o['key'], o['field'])
            if p not in d or rk(o) > rk(d[p]): d[p] = o
        return d
    a = raw(v['initiator']); b0 = raw(v['responder'])
    recv = {(o['key'], o['field']): o for o in v['initiator']}
    b = dict(b0); acc = rej = 0
    for op in v['initiator']:
        if not valid(op) or op['hlc']['wall'] > now + D['max_future_ms']: rej += 1; continue
        acc += 1; p = (op['key'], op['field'])
        if p not in b or rk(op) > rk(b[p]): b[p] = op
    reply = [r for r in sort_regs(b) if recv.get((r['key'], r['field'])) != r]
    for r in reply:
        p = (r['key'], r['field'])
        if valid(r) and r['hlc']['wall'] <= now + D['max_future_ms'] and (p not in a or rk(r) > rk(a[p])): a[p] = r
    return reply, acc, rej, sort_regs(a), sort_regs(b)
for v in D['exchange']:
    reply, acc, rej, ai, bi = exchange_model(v)
    chk('exchange', v['name'] + ' reply', reply == v['expect_reply'])
    chk('exchange', v['name'] + ' counts', (acc, rej) == (v['expect_accepted'], v['expect_rejected']), (acc, rej))
    chk('exchange', v['name'] + ' initiator', ai == v['expect_initiator'])
    chk('exchange', v['name'] + ' responder', bi == v['expect_responder'])

def binding_model(v):
    """Spec s3 host binding pass (planning only; minted spaces are None)."""
    me = v['node']; doc, _, _ = merge(v['registers'], 1 << 62)
    def val(k, f): return (doc.get((k, f)) or {}).get('value')
    spaces = sorted({k for k, _ in doc if k.startswith('space:')}, key=lambda k: k[6:].encode())
    live = [k for k in spaces if val(k, 'deleted') is not True and val(k, 'host') == me]
    ws = v['workspaces']; ids = {w['herdr_id'] for w in ws}
    used = set(); out = {}
    for w in ws:                      # step 1: bound by herdr_id
        c = [k for k in live if val(k, 'herdr_id') == w['herdr_id'] and k not in used]
        if c: used.add(c[0]); out[w['herdr_id']] = ('bound', c[0][6:])
    for w in ws:                      # step 2: rebind by name; step 3: mint
        if w['herdr_id'] in out: continue
        c = [k for k in live if k not in used and val(k, 'herdr_id') not in ids and val(k, 'name') == w['label']]
        if c: used.add(c[0]); out[w['herdr_id']] = ('rebound', c[0][6:])
        else: out[w['herdr_id']] = ('minted', None)
    res = [{'herdr_id': w['herdr_id'], 'label': w['label'], 'kind': out[w['herdr_id']][0], 'space': out[w['herdr_id']][1]} for w in ws]
    return res, [k[6:] for k in live if k not in used]
for v in D['binding']:
    res, orph = binding_model(v)
    chk('binding', v['name'] + ' plan', res == v['expect'], res)
    chk('binding', v['name'] + ' orphans', orph == v['expect_orphans'], orph)
HV = re.compile(r'[A-Za-z0-9+.-]{1,32}')
def hello_model(line, own):
    """Spec s8: t is hello, proto is the integer 1, node is a valid id other than ours;
    herdr_version is optional and any invalid value counts as absent."""
    m = json.loads(line); n = m.get('node'); p = m.get('proto')
    if m.get('t') != 'hello' or isinstance(p, bool) or p != 1 or not isinstance(p, int): return False, None, None
    if not (isinstance(n, str) and re.fullmatch(r'[a-z0-9-]{1,64}', n)) or n == own: return False, None, None
    hv = m.get('herdr_version')
    return True, n, hv if isinstance(hv, str) and HV.fullmatch(hv) else None
def hello_wire(node, hv):
    m = {'t': 'hello', 'proto': 1, 'node': node}
    if hv is not None: m['herdr_version'] = hv
    return json.dumps(m, sort_keys=True, separators=(',', ':'), ensure_ascii=False)
for v in D.get('hello', []):
    got = hello_model(v['line'], v['own'])
    chk('hello', v['name'], got == (v['expect_ok'], v['expect_peer'], v['expect_herdr_version']), got)
    if v['wire']: chk('hello', v['name'] + ' wire', hello_wire(v['expect_peer'], v['expect_herdr_version']) == v['line'])
# key_order: every wire line is valid JSON whose members are NOT in sorted order, and the ops still mean the same register
for v in D.get('key_order', []):
    ls = v['initiator_lines'] + v['responder_lines']
    objs = [json.loads(l) for l in ls]
    chk('key_order', v['name'] + ' unsorted', all(list(o) != sorted(o) for o in objs))
    chk('key_order', v['name'] + ' op', [{k: x for k, x in objs[1].items() if k != 't'}] == v['expect_responder'], objs[1])
    chk('key_order', v['name'] + ' end', objs[2].get('count') == v['expect_accepted'] and objs[4].get('accepted') == v['expect_accepted'])
print({k: c for k, c in counts.items()})
for f in fails: print('MISMATCH', f)
print('FAILS', len(fails)); sys.exit(1 if fails else 0)
