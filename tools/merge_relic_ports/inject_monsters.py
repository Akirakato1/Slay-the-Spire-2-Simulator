"""Inject batch_m_*.txt into effects.rs::monster_move_effects.

Match keys are tuples `("MonsterId", "IntentName")`. Extract arms whose
LHS is a paren-tuple of two string literals; preserve body via depth-
balanced extraction.
"""

import os
import re
import sys

here = os.path.dirname(__file__)
repo = os.path.dirname(os.path.dirname(here))

ARM_START = re.compile(r'^\s*\(\s*"([A-Za-z_0-9]+)"\s*,\s*"([A-Za-z_0-9]+)"\s*\)\s*=>\s*Some\(vec!\[')


def parse_arms(text):
    lines = text.splitlines(keepends=True)
    out = []
    i = 0
    while i < len(lines):
        line = lines[i].rstrip('\n').rstrip()
        m = ARM_START.match(line)
        if not m:
            i += 1
            continue
        key = (m.group(1), m.group(2))
        acc = []
        depth = 0
        started = False
        while i < len(lines):
            l = lines[i]
            acc.append(l)
            for ch in l:
                if ch == '(':
                    depth += 1
                    started = True
                elif ch == ')':
                    depth -= 1
            i += 1
            if started and depth == 0:
                break
        out.append((key, ''.join(acc)))
    return out


def main():
    batch_files = ['batch_m_1.txt']
    arms = []
    for fname in batch_files:
        p = os.path.join(here, fname)
        if not os.path.exists(p):
            continue
        with open(p, 'r', encoding='utf-8') as f:
            text = f.read()
        arms.extend(parse_arms(text))

    # Dedup — last-seen wins. Keys are (monster, intent) tuples.
    by_key = {}
    order = []
    for k, b in arms:
        if k not in by_key:
            order.append(k)
        by_key[k] = b
    deduped = [(k, by_key[k]) for k in order]

    effects_path = os.path.join(repo, 'crates', 'sts2-sim', 'src', 'effects.rs')
    with open(effects_path, 'r', encoding='utf-8') as f:
        src = f.read()

    fn_start = src.find('pub fn monster_move_effects(')
    if fn_start < 0:
        print('ERROR: monster_move_effects not found', file=sys.stderr)
        sys.exit(1)
    fn_end = src.find('\n}\n', fn_start)
    if fn_end < 0:
        print('ERROR: monster_move_effects end not found', file=sys.stderr)
        sys.exit(1)
    fn_body = src[fn_start:fn_end]
    marker = '\n        _ => None,'
    rel = fn_body.find(marker)
    if rel < 0:
        print('ERROR: marker not found', file=sys.stderr)
        sys.exit(1)
    abs_idx = fn_start + rel

    # Strip any previous injection between prelude marker and _ => None,.
    prelude_marker = '// ===== Manual monster-move ports (batch_m_*) ====='
    if prelude_marker in src:
        prelude_idx = src.find(prelude_marker, fn_start)
        end_of_prelude = prelude_idx + len(prelude_marker)
        before = src[:end_of_prelude]
        after = src[abs_idx:]
        src = before + '\n\n' + after
        fn_start = src.find('pub fn monster_move_effects(')
        fn_end = src.find('\n}\n', fn_start)
        fn_body = src[fn_start:fn_end]
        rel = fn_body.find(marker)
        abs_idx = fn_start + rel
    else:
        ins = abs_idx
        prelude = '        // ===== Manual monster-move ports (batch_m_*) =====\n\n'
        src = src[:ins] + prelude + src[ins:]
        fn_start = src.find('pub fn monster_move_effects(')
        fn_end = src.find('\n}\n', fn_start)
        fn_body = src[fn_start:fn_end]
        rel = fn_body.find(marker)
        abs_idx = fn_start + rel

    # Format arms: indent every line to 8 spaces, ensure trailing comma.
    block_parts = []
    for _key, body in deduped:
        text = body.rstrip()
        if not text.endswith(','):
            text += ','
        text += '\n'
        indented = '\n'.join('        ' + ln.lstrip() if ln.strip() else ''
                             for ln in text.splitlines())
        if not indented.endswith('\n'):
            indented += '\n'
        block_parts.append(indented)
    block = '\n'.join(block_parts) + '\n'

    new_src = src[:abs_idx] + '\n' + block + src[abs_idx:]
    with open(effects_path, 'w', encoding='utf-8') as f:
        f.write(new_src)
    print(f'OK: injected {len(deduped)} monster-move arms')


if __name__ == '__main__':
    main()
