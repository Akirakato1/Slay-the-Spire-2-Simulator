"""Inject batch_r_*.txt into effects.rs::relic_effects.

Each entry in the batch file is multi-line:

    // C# evidence comment.
    "RelicName" => Some(vec![
        (RelicHook::AfterSideTurnStart { owner_side_only: true, first_turn_only: true },
         vec![Effect::ApplyPower { ... }]),
    ]),

We extract whole arms (from `"Name" => Some(vec![` to the matching closing
`]),` at the same brace depth) and inject before the `_ => None,` of
`relic_effects`.
"""

import os
import re
import sys

here = os.path.dirname(__file__)
repo = os.path.dirname(os.path.dirname(here))

ARM_START = re.compile(r'^"([A-Za-z_0-9]+)"\s*=>\s*Some\(vec!\[\s*$')


def parse_arms(text):
    """Yield (name, full_arm_text) tuples from a batch file.

    An arm starts on a line matching ARM_START and ends when we've seen
    the matching closing `]),` at depth 0 of the outermost Some(vec![...]).
    """
    lines = text.splitlines(keepends=True)
    i = 0
    while i < len(lines):
        line = lines[i].rstrip('\n').rstrip()
        m = ARM_START.match(line)
        if not m:
            i += 1
            continue
        name = m.group(1)
        # Accumulate lines until depth returns to zero of the outer Some(vec![ ... ]).
        # We're inside "Some(vec![" — that's 1 paren + 1 bracket. End condition:
        # close on `]),` after returning to start depth.
        acc = [lines[i]]
        # Track opening "[" / "]" balance starting at +1 for the opening vec![
        depth = 1  # +1 for the vec![ on the start line
        # Also count parens; the outermost Some(vec![ contributes +1 paren.
        paren = 1  # +1 for Some(
        i += 1
        while i < len(lines) and (depth > 0 or paren > 0):
            acc.append(lines[i])
            for ch in lines[i]:
                if ch == '[':
                    depth += 1
                elif ch == ']':
                    depth -= 1
                elif ch == '(':
                    paren += 1
                elif ch == ')':
                    paren -= 1
            i += 1
        yield name, ''.join(acc)


def main():
    batch_files = ['batch_r_1.txt', 'batch_r_2.txt', 'batch_r_3.txt', 'batch_r_4.txt', 'batch_r_5.txt']
    arms = []
    for fname in batch_files:
        p = os.path.join(here, fname)
        if not os.path.exists(p):
            continue
        with open(p, 'r', encoding='utf-8') as f:
            text = f.read()
        for name, body in parse_arms(text):
            arms.append((name, body))

    # Dedup — LAST-seen wins, so later batches override earlier ones.
    # Lets batch_r_N supersede batch_r_M's placeholder for the same relic.
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

    # Find the relic_effects function and its trailing _ => None,.
    fn_start = src.find('pub fn relic_effects(')
    if fn_start < 0:
        print('ERROR: relic_effects not found', file=sys.stderr)
        sys.exit(1)
    fn_end = src.find('\n}\n', fn_start)
    if fn_end < 0:
        print('ERROR: relic_effects end not found', file=sys.stderr)
        sys.exit(1)

    # Find the marker line within fn body.
    fn_body = src[fn_start:fn_end]
    marker = '\n        _ => None,'
    rel = fn_body.find(marker)
    if rel < 0:
        print('ERROR: marker `_ => None,` in relic_effects not found', file=sys.stderr)
        sys.exit(1)
    abs_idx = fn_start + rel

    # Strip any previous injection block between
    # `// ===== Manual relic ports (batch_r_1) =====` and the marker.
    prelude_marker = '// ===== Manual relic ports (batch_r_1) =====\n        // 22 hand-curated arms. Source: tools/merge_relic_ports/batch_r_1.txt.'
    if prelude_marker in src:
        prelude_idx = src.find(prelude_marker, fn_start)
        # Strip from end-of-prelude to abs_idx (re-injecting fresh).
        end_of_prelude = prelude_idx + len(prelude_marker)
        before = src[:end_of_prelude]
        after = src[abs_idx:]
        src = before + '\n\n' + after
        # Recompute abs_idx.
        fn_start = src.find('pub fn relic_effects(')
        fn_end = src.find('\n}\n', fn_start)
        fn_body = src[fn_start:fn_end]
        rel = fn_body.find(marker)
        abs_idx = fn_start + rel

    # Format the arms: indent each with 8 spaces (matching `match`).
    block_parts = []
    for name, body in deduped:
        text = body.rstrip()
        if not text.endswith(','):
            text += ','
        text += '\n'
        # Indent every line by 8 spaces.
        indented = '\n'.join('        ' + ln if ln.strip() else ln
                             for ln in text.splitlines())
        if not indented.endswith('\n'):
            indented += '\n'
        block_parts.append(indented)
    block = '\n'.join(block_parts) + '\n'

    new_src = src[:abs_idx] + '\n' + block + src[abs_idx:]
    with open(effects_path, 'w', encoding='utf-8') as f:
        f.write(new_src)

    print(f'OK: injected {len(deduped)} relic arms into relic_effects()')


if __name__ == '__main__':
    main()
