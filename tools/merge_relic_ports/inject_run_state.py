"""Inject batch_r_rs_*.txt into effects.rs::run_state_effects.

Same depth-balanced-paren extraction as inject_relics.py, but targeting
the run-state registry (which returns
`Option<Vec<(RunStateHook, Vec<Effect>)>>`).
"""

import os
import re
import sys

here = os.path.dirname(__file__)
repo = os.path.dirname(os.path.dirname(here))

ARM_START = re.compile(r'^"([A-Za-z_0-9]+)"\s*=>\s*Some\(vec!\[')


def parse_arms(text):
    lines = text.splitlines(keepends=True)
    i = 0
    out = []
    while i < len(lines):
        line = lines[i].rstrip('\n').rstrip()
        m = ARM_START.match(line)
        if not m:
            i += 1
            continue
        name = m.group(1)
        acc = [lines[i]]
        depth = 0
        started = False
        while i < len(lines):
            l = lines[i]
            if i > len(acc) - 1 + (len(lines) - len(acc)):
                # safety
                pass
            for ch in l:
                if ch == '(':
                    depth += 1
                    started = True
                elif ch == ')':
                    depth -= 1
            i += 1
            if started and depth == 0:
                break
            if i < len(lines):
                acc.append(lines[i] if i < len(lines) else '')
        # acc[0] is the start line; the loop already started counting parens
        # from that line. Re-collect including ALL lines until depth returns.
        # Simpler: re-scan from scratch.
        # Restart logic — use the simpler proven script:
        yield name, ''.join(acc)


# Simpler implementation copied from inject_relics.py
def parse_arms2(text):
    lines = text.splitlines(keepends=True)
    out = []
    i = 0
    while i < len(lines):
        line = lines[i].rstrip('\n').rstrip()
        m = ARM_START.match(line)
        if not m:
            i += 1
            continue
        name = m.group(1)
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
        out.append((name, ''.join(acc)))
    return out


def main():
    batch_files = ['batch_r_rs_1.txt', 'batch_r_rs_2.txt', 'batch_r_rs_3.txt', 'batch_r_rs_4.txt']
    arms = []
    for fname in batch_files:
        p = os.path.join(here, fname)
        if not os.path.exists(p):
            continue
        with open(p, 'r', encoding='utf-8') as f:
            text = f.read()
        arms.extend(parse_arms2(text))

    # Dedup — last-seen wins.
    by_name = {}
    order = []
    for n, b in arms:
        if n not in by_name:
            order.append(n)
        by_name[n] = b
    deduped = [(n, by_name[n]) for n in order]

    effects_path = os.path.join(repo, 'crates', 'sts2-sim', 'src', 'effects.rs')
    with open(effects_path, 'r', encoding='utf-8') as f:
        src = f.read()

    fn_start = src.find('pub fn run_state_effects(')
    if fn_start < 0:
        print('ERROR: run_state_effects not found', file=sys.stderr)
        sys.exit(1)
    fn_end = src.find('\n}\n', fn_start)
    if fn_end < 0:
        print('ERROR: run_state_effects end not found', file=sys.stderr)
        sys.exit(1)
    fn_body = src[fn_start:fn_end]
    marker = '\n        _ => None,'
    rel = fn_body.find(marker)
    if rel < 0:
        print('ERROR: marker not found', file=sys.stderr)
        sys.exit(1)
    abs_idx = fn_start + rel

    # Strip any previous injection.
    prelude_marker = '// ===== Manual run-state ports (batch_r_rs_*) ====='
    if prelude_marker in src:
        prelude_idx = src.find(prelude_marker, fn_start)
        end_of_prelude = prelude_idx + len(prelude_marker)
        before = src[:end_of_prelude]
        after = src[abs_idx:]
        src = before + '\n\n' + after
        fn_start = src.find('pub fn run_state_effects(')
        fn_end = src.find('\n}\n', fn_start)
        fn_body = src[fn_start:fn_end]
        rel = fn_body.find(marker)
        abs_idx = fn_start + rel
    else:
        # Insert prelude marker line ABOVE the trailing _ => None,.
        ins = abs_idx
        prelude = '        // ===== Manual run-state ports (batch_r_rs_*) =====\n\n'
        src = src[:ins] + prelude + src[ins:]
        fn_start = src.find('pub fn run_state_effects(')
        fn_end = src.find('\n}\n', fn_start)
        fn_body = src[fn_start:fn_end]
        rel = fn_body.find(marker)
        abs_idx = fn_start + rel

    # Format arms — indent to 8 spaces.
    block_parts = []
    for name, body in deduped:
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
    print(f'OK: injected {len(deduped)} run-state relic arms')


if __name__ == '__main__':
    main()
