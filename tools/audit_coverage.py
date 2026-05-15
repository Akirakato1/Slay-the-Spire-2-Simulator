"""Audit the data-driven coverage of card_effects / relic_effects /
potion_effects against the full id tables. Emits a report of encoded /
unencoded items plus the primitives each encoded item uses.
"""
import json
import os
import re
import sys
from collections import Counter

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

def load_ids(json_path):
    with open(json_path, 'r', encoding='utf-8') as f:
        data = json.load(f)
    return [item['id'] for item in data]

def parse_legacy_match_arm_cards(combat_src):
    """Extract card-ids matched by combat.rs::dispatch_on_play (the legacy
    pre-Effect-VM dispatcher that the data table is gradually replacing).
    Returns a set of card ids that are still on the legacy path."""
    start = combat_src.find('fn dispatch_on_play(')
    if start < 0:
        return set()
    depth = 0
    i = combat_src.find('{', start)
    body_start = i + 1
    while i < len(combat_src):
        c = combat_src[i]
        if c == '{':
            depth += 1
        elif c == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    body = combat_src[body_start:i]
    # `"X" =>` or `"X" | "Y" =>` indicates an arm head; trailing `=>` plus
    # newline/whitespace before the body distinguishes it from `"Damage" =>`
    # variable accessors inside arm bodies.
    out = set()
    for m in re.finditer(r'^([\t ]*)("[A-Za-z_0-9]+"(?:\s*\|\s*"[A-Za-z_0-9]+")*)\s*=>',
                         body, re.M):
        for name in re.findall(r'"([A-Za-z_0-9]+)"', m.group(2)):
            # Only card-like strings (PascalCase, no special-purpose like vars).
            if name[0].isupper() and len(name) > 1 and not name.endswith('Power'):
                out.add(name)
    # Filter to those that are actually card ids (caller intersects with card_ids).
    return out


def parse_table(src, fn_name):
    """Extract (name, body) pairs from a `pub fn fn_name(...) { match id { ... } }` block."""
    start = src.find(f'pub fn {fn_name}(')
    if start < 0:
        return []
    # Find the function body close — last `}\n}` after start.
    depth = 0
    i = src.find('{', start)
    body_start = i + 1
    while i < len(src):
        c = src[i]
        if c == '{':
            depth += 1
        elif c == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    body = src[body_start:i]
    # Parse match arms: `"Name" =>` or `"Name" | "Other" =>`
    arms = []
    arm_re = re.compile(r'^([\t ]*)("[A-Za-z_0-9]+"(?:\s*\|\s*"[A-Za-z_0-9]+")*)\s*=>\s*(.+?)(?=^\1"[A-Za-z_0-9]+"|^\1_\s*=>|^\1\}\s*$)',
                        re.M | re.S)
    for m in arm_re.finditer(body):
        names_str = m.group(2)
        names = re.findall(r'"([A-Za-z_0-9]+)"', names_str)
        arm_body = m.group(3)
        for name in names:
            arms.append((name, arm_body))
    return arms

def primitives_in(body):
    """Return sorted unique Effect / AmountSpec / Condition / RelicHook variants used."""
    prims = set()
    for m in re.finditer(r'\b(Effect|AmountSpec|Condition|RelicHook|CardPoolRef)::([A-Za-z]+)', body):
        prims.add(f'{m.group(1)}::{m.group(2)}')
    return sorted(prims)

def extract_skipped(text):
    """Return set of names from `// SKIP Name: ...` comments."""
    return set(m.group(1) for m in re.finditer(r'//\s*SKIP\s+([A-Za-z_0-9]+)\s*:', text))

def main():
    with open(os.path.join(REPO, 'crates', 'sts2-sim', 'src', 'effects.rs'),
              'r', encoding='utf-8') as f:
        effects_src = f.read()

    card_ids = set(load_ids(os.path.join(REPO, 'crates', 'sts2-sim', 'data', 'cards.json')))
    relic_ids = set(load_ids(os.path.join(REPO, 'crates', 'sts2-sim', 'data', 'relics.json')))
    potion_ids = set(load_ids(os.path.join(REPO, 'crates', 'sts2-sim', 'data', 'potions.json')))

    card_arms = parse_table(effects_src, 'card_effects')
    relic_arms = parse_table(effects_src, 'relic_effects')
    potion_arms = parse_table(effects_src, 'potion_effects')
    run_state_arms = parse_table(effects_src, 'run_state_effects')
    monster_move_arms = parse_table(effects_src, 'monster_move_effects')

    card_encoded = set(n for n, _ in card_arms)
    relic_encoded = set(n for n, _ in relic_arms)
    potion_encoded = set(n for n, _ in potion_arms)
    run_state_encoded = set(n for n, _ in run_state_arms)
    relic_combined = relic_encoded | run_state_encoded

    # Cards still on the legacy combat.rs::dispatch_on_play match-arm
    # (pre-Effect-VM port). These ARE implemented at runtime, just not
    # data-driven yet.
    with open(os.path.join(REPO, 'crates', 'sts2-sim', 'src', 'combat.rs'),
              'r', encoding='utf-8') as f:
        combat_src = f.read()
    legacy_handled = parse_legacy_match_arm_cards(combat_src) & card_ids
    legacy_only = legacy_handled - card_encoded
    fully_handled = card_encoded | legacy_only

    # SKIP rationales (from inline comments in the autogen + manual blocks).
    skipped_cards = extract_skipped(effects_src) - card_encoded

    # Primitive frequency.
    prim_counts = Counter()
    for _, body in card_arms + relic_arms + potion_arms:
        for p in primitives_in(body):
            prim_counts[p] += 1

    # Coverage gap.
    relic_gap = sorted(relic_ids - relic_combined)
    potion_gap = sorted(potion_ids - potion_encoded)

    card_gap = sorted(card_ids - fully_handled)  # only truly unimplemented

    print('# Data-driven coverage audit')
    print()
    print(f'**Cards**:   data-table {len(card_encoded):>4}/{len(card_ids)}  ({100*len(card_encoded)/len(card_ids):.1f}%) +  legacy-match-arm-only {len(legacy_only)}  =  total handled {len(fully_handled)}/{len(card_ids)} ({100*len(fully_handled)/len(card_ids):.1f}%)')
    print(f'**Relics**:  relic_effects {len(relic_encoded)} +  run_state_effects {len(run_state_encoded)}  =  combined {len(relic_combined)}/{len(relic_ids)} ({100*len(relic_combined)/len(relic_ids):.1f}%)')
    print(f'**Potions**: encoded {len(potion_encoded):>4}/{len(potion_ids)}  ({100*len(potion_encoded)/len(potion_ids):.1f}%)  -> gap {len(potion_ids)-len(potion_encoded)}')
    print()
    print(f'Total entries in data tables: {len(card_arms)+len(relic_arms)+len(potion_arms)}')
    print()
    print('## Top primitives by use count')
    for prim, n in prim_counts.most_common(40):
        print(f'  {n:>4}  {prim}')

    print()
    print('## Unencoded cards (gap)')
    for cid in card_gap:
        print(f'  - {cid}')

    print()
    print('## Unencoded relics (gap)')
    for rid in relic_gap:
        print(f'  - {rid}')

    print()
    print('## Unencoded potions (gap)')
    for pid in potion_gap:
        print(f'  - {pid}')

if __name__ == '__main__':
    main()
