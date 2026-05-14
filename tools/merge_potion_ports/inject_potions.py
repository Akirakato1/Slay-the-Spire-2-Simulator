"""Inject batch_p_*.txt into effects.rs::potion_effects.

Each entry is either a single-line arm like:

    "BlockPotion" => Some(vec![Effect::GainBlock { ... }]),

or multi-line:

    "BottledPotential" => Some(vec![
        Effect::MoveCard { ... },
        Effect::Shuffle { ... },
        Effect::DrawCards { ... },
    ]),

We extract whole arms (depth-balanced parens) and inject before the
`_ => None,` of `potion_effects`.
"""

import os
import re
import sys

here = os.path.dirname(__file__)
repo = os.path.dirname(os.path.dirname(here))

ARM_START = re.compile(r'^"([A-Za-z_0-9]+)"\s*=>\s*Some\(vec!\[')


def parse_arms(text):
    """Yield (name, full_arm_text) tuples from a batch file."""
    lines = text.splitlines(keepends=True)
    i = 0
    while i < len(lines):
        line = lines[i].rstrip('\n').rstrip()
        m = ARM_START.match(line)
        if not m:
            i += 1
            continue
        name = m.group(1)
        # Accumulate paren-balanced text starting at `Some(`.
        # We balance `(` and `)` from the very first `Some(` until depth 0.
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
        yield name, ''.join(acc)


def main():
    batch_files = ['batch_p_1.txt']
    arms = []
    for fname in batch_files:
        p = os.path.join(here, fname)
        if not os.path.exists(p):
            continue
        with open(p, 'r', encoding='utf-8') as f:
            text = f.read()
        for name, body in parse_arms(text):
            arms.append((name, body))

    seen = set()
    deduped = []
    for n, b in arms:
        if n in seen:
            continue
        seen.add(n)
        deduped.append((n, b))

    effects_path = os.path.join(repo, 'crates', 'sts2-sim', 'src', 'effects.rs')
    with open(effects_path, 'r', encoding='utf-8') as f:
        src = f.read()

    fn_start = src.find('pub fn potion_effects(')
    if fn_start < 0:
        print('ERROR: potion_effects not found', file=sys.stderr)
        sys.exit(1)
    fn_end = src.find('\n}\n', fn_start)
    if fn_end < 0:
        print('ERROR: potion_effects end not found', file=sys.stderr)
        sys.exit(1)

    fn_body = src[fn_start:fn_end]
    marker = '\n        _ => None,'
    rel = fn_body.find(marker)
    if rel < 0:
        print('ERROR: marker `_ => None,` in potion_effects not found', file=sys.stderr)
        sys.exit(1)
    abs_idx = fn_start + rel

    # Strip any previous injection between the prelude marker and abs_idx.
    prelude_marker = '// ===== Manual potion ports (batch_p_1) =====\n        // 45 hand-curated arms. Source: tools/merge_potion_ports/batch_p_1.txt.'
    if prelude_marker in src:
        prelude_idx = src.find(prelude_marker, fn_start)
        end_of_prelude = prelude_idx + len(prelude_marker)
        before = src[:end_of_prelude]
        after = src[abs_idx:]
        src = before + '\n\n' + after
        fn_start = src.find('pub fn potion_effects(')
        fn_end = src.find('\n}\n', fn_start)
        fn_body = src[fn_start:fn_end]
        rel = fn_body.find(marker)
        abs_idx = fn_start + rel

    block_parts = []
    for name, body in deduped:
        text = body.rstrip()
        if not text.endswith(','):
            text += ','
        text += '\n'
        # Indent every line to 8 spaces.
        indented = '\n'.join('        ' + ln if ln.strip() else ln
                             for ln in text.splitlines())
        if not indented.endswith('\n'):
            indented += '\n'
        block_parts.append(indented)
    block = '\n'.join(block_parts) + '\n'

    new_src = src[:abs_idx] + '\n' + block + src[abs_idx:]
    with open(effects_path, 'w', encoding='utf-8') as f:
        f.write(new_src)

    print(f'OK: injected {len(deduped)} potion arms into potion_effects()')


if __name__ == '__main__':
    main()
