"""Inject batch_v2_1/2/3 into crates/sts2-sim/src/effects.rs::card_effects.

Each batch file contains lines like:
        "CardName" => Some(vec![ ... ]),

with optional leading-space indentation and inline `// comment` evidence
trails or `// SKIP CardName: reason` lines.

We extract all match arms (one per line by convention), re-indent to 8
spaces, and emit a block injected before the trailing `_ => None,` of
`card_effects`.

If a CardName is already present in effects.rs::card_effects, we DROP the
batch entry (existing wins). This protects the smaller hand-ported set
that lives above the autogen block.
"""

import os
import re
import sys

here = os.path.dirname(__file__)
repo = os.path.dirname(os.path.dirname(here))

ARM_START_RE = re.compile(r'^\s*"(?P<name>[A-Za-z_0-9]+)"\s*=>\s*Some\(vec!\[')
ARM_NAME_ONLY_RE = re.compile(r'"(?P<name>[A-Za-z_0-9]+)"')


def extract_arms(text):
    """Return ordered list of (name, full_arm_text). Supports both
    single-line `"X" => Some(vec![...]),` and multi-line arms with
    paren-balanced bodies. Skips entries inside `// SKIP X: ...` comments."""
    out = []
    seen = set()
    lines = text.splitlines(keepends=True)
    i = 0
    while i < len(lines):
        line = lines[i]
        m = ARM_START_RE.match(line.rstrip('\n').rstrip())
        if not m:
            i += 1
            continue
        name = m.group('name')
        if name in seen:
            i += 1
            continue
        seen.add(name)
        acc = [line]
        # Balance Some( ... ) ... starting from this line. Track parens
        # and brackets together; the arm ends when we return to the
        # starting depth.
        depth = 0
        started = False
        for ch in line:
            if ch in '([':
                depth += 1
                started = True
            elif ch in ')]':
                depth -= 1
        i += 1
        while i < len(lines) and not (started and depth == 0):
            l = lines[i]
            acc.append(l)
            for ch in l:
                if ch in '([':
                    depth += 1
                    started = True
                elif ch in ')]':
                    depth -= 1
            i += 1
        body = ''.join(acc).rstrip()
        if not body.endswith(','):
            body += ','
        # Re-indent every line to 8 spaces.
        indented = '\n'.join('        ' + ln.lstrip() if ln.strip() else ''
                             for ln in body.splitlines())
        out.append((name, indented))
    return out


def collect_existing(effects_src):
    """Cards already present as match arms in card_effects()."""
    names = set()
    # Limit to within fn card_effects ... .
    start = effects_src.find('pub fn card_effects(')
    end = effects_src.find('\n}\n', start)
    region = effects_src[start:end] if start >= 0 and end >= 0 else effects_src
    for m in ARM_NAME_ONLY_RE.finditer(region):
        names.add(m.group('name'))
    return names


def main():
    batch_files = ['batch_v2_1.txt', 'batch_v2_2.txt', 'batch_v2_3.txt', 'batch_v2_4.txt', 'batch_v3.txt', 'batch_v4.txt', 'batch_v5.txt', 'batch_v6.txt', 'batch_v7.txt', 'batch_v8.txt', 'batch_v9.txt']
    all_arms = []
    for fname in batch_files:
        p = os.path.join(here, fname)
        with open(p, 'r', encoding='utf-8') as f:
            text = f.read()
        all_arms.extend(extract_arms(text))

    # Dedup across batches (first-seen wins).
    seen = set()
    deduped = []
    for name, arm in all_arms:
        if name in seen:
            continue
        seen.add(name)
        deduped.append((name, arm))

    effects_path = os.path.join(repo, 'crates', 'sts2-sim', 'src', 'effects.rs')
    with open(effects_path, 'r', encoding='utf-8') as f:
        src = f.read()

    # Strip any previously-injected v2 block so re-running is idempotent.
    # MUST happen before collect_existing, otherwise the script would treat
    # its own previous output as "existing" and drop everything.
    prev_pattern = re.compile(
        r'\n\s*// ===== Manual v2 card ports.*?(?=        _ => None,\s*\n\s*\}\s*\n\}\s*\n)',
        re.S,
    )
    src = prev_pattern.sub('', src)

    existing = collect_existing(src)
    # Drop batch entries that already exist in card_effects().
    fresh = [(n, a) for (n, a) in deduped if n not in existing]
    dropped = [(n, a) for (n, a) in deduped if n in existing]

    # Find the LAST `_ => None,` followed by `}\n}\n` (end of card_effects fn).
    # card_effects function ends with `_ => None,\n    }\n}` — find the
    # right occurrence.
    fn_start = src.find('pub fn card_effects(')
    if fn_start < 0:
        print('ERROR: card_effects not found', file=sys.stderr)
        sys.exit(1)
    fn_end = src.find('\n}\n', fn_start)
    if fn_end < 0:
        print('ERROR: card_effects end not found', file=sys.stderr)
        sys.exit(1)

    # Within fn body, find the trailing `_ => None,`.
    fn_body = src[fn_start:fn_end]
    none_marker = '        _ => None,'
    rel = fn_body.rfind(none_marker)
    if rel < 0:
        print('ERROR: trailing `_ => None,` not found', file=sys.stderr)
        sys.exit(1)
    abs_idx = fn_start + rel

    prelude = (
        '\n        // ===== Manual v2 card ports (batches v2_1..v2_3) =====\n'
        f'        // {len(fresh)} hand-curated arms covering Acrobatics..Rattle.\n'
        '        // Source: tools/merge_card_ports/batch_v2_*.txt.\n'
        '        // SKIPs documented in those files.\n\n'
    )
    block = prelude + '\n'.join(arm for _, arm in fresh) + '\n\n'
    new_src = src[:abs_idx] + block + src[abs_idx:]

    with open(effects_path, 'w', encoding='utf-8') as f:
        f.write(new_src)

    print(f'OK: injected {len(fresh)} new card arms ({len(dropped)} dropped as duplicates)')
    if dropped:
        for n, _ in dropped[:10]:
            print(f'  - dropped duplicate: {n}')


if __name__ == '__main__':
    main()
