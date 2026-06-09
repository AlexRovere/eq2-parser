# -*- coding: utf-8 -*-
"""Extrait assets/spells.json depuis les fichiers JS de X:/dev/eq2/src/data/spells."""
import re, glob, json, os, sys

sys.stdout.reconfigure(encoding='utf-8')

SRC = r'X:/dev/eq2/src/data/spells/*.js'
OUT = r'X:/dev/eq2-parser/assets/spells.json'


def parse_dur(s):
    if s is None:
        return None
    s = s.strip().lower()
    if s == 'instant':
        return 0.0
    if (s.startswith('permanent') or s.startswith('until')
            or 'concentration' in s or s.startswith('toggle') or s in ('-', '')):
        return None
    tot = 0.0
    found = False
    for val, unit in re.findall(r'([0-9]*\.?[0-9]+)\s*(min|s|h)', s):
        found = True
        v = float(val)
        tot += v * 60 if unit == 'min' else (v * 3600 if unit == 'h' else v)
    return tot if found else None


def field(blk, key):
    m = (re.search(key + r"\s*:\s*'([^']*)'", blk)
         or re.search(key + r'\s*:\s*"([^"]*)"', blk))
    return m.group(1) if m else None


def main():
    spells = {}
    for f in glob.glob(SRC):
        cls = os.path.splitext(os.path.basename(f))[0]
        s = open(f, encoding='utf-8').read()
        for blk in re.findall(r'\{[^{}]*?name:\s*[\'"].*?\}', s, re.S):
            name = field(blk, 'name')
            if not name:
                continue
            spells[(cls, name)] = {
                'name': name, 'class': cls,
                'target': field(blk, 'target'),
                'mechanic': field(blk, 'mechanic'),
                'cast': parse_dur(field(blk, 'cast')),
                'recast': parse_dur(field(blk, 'recast')),
                'duration': parse_dur(field(blk, 'duration')),
                'damage_type': field(blk, 'damageType'),
            }
    out = sorted(spells.values(), key=lambda s: (s['class'], s['name']))
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    json.dump({'spells': out}, open(OUT, 'w', encoding='utf-8'),
              ensure_ascii=False, indent=0)
    dmg = [s for s in out if s['mechanic'] in ('dd', 'dot')]
    miss = [s for s in dmg if s['cast'] is None]
    print('extracted', len(out), '| damaging', len(dmg), '| missing cast', len(miss))
    print('--- warlock dd/dot ---')
    for s in out:
        if s['class'] == 'warlock' and s['mechanic'] in ('dd', 'dot'):
            print(f"  {s['name']:<22} cast={s['cast']} recast={s['recast']} "
                  f"dur={s['duration']} tgt={s['target']} {s['damage_type']}")


if __name__ == '__main__':
    main()
