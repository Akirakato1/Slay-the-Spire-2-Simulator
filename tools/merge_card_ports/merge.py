"""
Merge the 4 card-port agent outputs into a clean Rust card_effects() registry.

The agents produced ~430 cards' worth of effect-list encodings, but with
consistent arity errors:
  - Effect::ChannelOrb / SummonOsty / DamageFromOsty / Forge / AutoplayFromDraw /
    GainGold / ApplyKeywordToCards / TransformCards / SetCardCost / ChangeOrbSlots
    used without their required fields
  - Selector::Random / PlayerInteractive / FirstMatching taking AmountSpec
    where the schema requires i32
  - CardFilter::Not — not a real variant
  - AmountSpec::BranchedOnUpgrade with i32 instead of AmountSpec subfields

Strategy: aggressive regex-based filtering. Any match arm whose body contains
a forbidden pattern becomes `// SKIP CardName: <reason>` instead.

Output is a Rust source fragment ready to paste into effects.rs::card_effects.
"""

import re
import sys
import os

# Heuristic forbidden patterns; an arm hitting any of these is skipped.
FORBIDDEN = [
    (r'\bEffect::ChannelOrb\b(?!\s*\{)', 'ChannelOrb needs orb_id'),
    (r'\bEffect::SummonOsty\b(?!\s*\{)', 'SummonOsty needs osty_id'),
    (r'\bEffect::DamageFromOsty\b(?!\s*\{)', 'DamageFromOsty needs amount+target'),
    (r'\bEffect::Forge\b(?!\s*\{)', 'Forge needs amount'),
    (r'\bEffect::AutoplayFromDraw\b(?!\s*\{)', 'AutoplayFromDraw needs n'),
    (r'\bEffect::GainGold\b(?!\s*\{)', 'GainGold needs amount'),
    (r'\bEffect::ChangeOrbSlots\b(?!\s*\{)', 'ChangeOrbSlots needs delta'),
    (r'\bEffect::ApplyKeywordToCards\b(?!\s*\{)', 'ApplyKeywordToCards needs keyword+from+selector'),
    (r'\bEffect::TransformCards\b(?!\s*\{)', 'TransformCards needs from+selector'),
    (r'\bEffect::SetCardCost\b(?!\s*\{)', 'SetCardCost needs from+selector+cost+scope'),
    (r'\bChannelOrb\s*\{\s*orb_id:\s*"Random"', 'ChannelOrb random-orb stub'),
    (r'\bSelector::Random\s*\{\s*n:\s*AmountSpec::', 'Selector::Random needs i32'),
    (r'\bSelector::PlayerInteractive\s*\{\s*n:\s*AmountSpec::', 'Selector::PlayerInteractive needs i32'),
    (r'\bSelector::FirstMatching\s*\{\s*n:\s*AmountSpec::', 'Selector::FirstMatching needs i32'),
    (r'\bCardFilter::Not\b', 'CardFilter::Not does not exist'),
    (r'\bCardFilter::OfRarity\b', 'CardFilter::OfRarity does not exist'),
    (r'\bAmountSpec::BranchedOnUpgrade\s*\{\s*base:\s*\d', 'BranchedOnUpgrade fields are i32 — but only when used wrongly inside AmountSpec'),
    (r'/\*\s*STUB', 'inline stub comment indicates incomplete arity'),
    (r'/\*\s*XEnergy\s*\*/', 'placeholder XEnergy comment'),
    (r'\bEffect::MoveCard\s*\{[^}]*\}\s*,?[^\n]*\)\s*,?', None),  # MoveCard validity checked separately
    (r'\bEffect::EvokeNextOrb\s*\{', 'EvokeNextOrb takes no fields'),
    (r'\bEffect::TriggerOrbPassive\s*\{', 'TriggerOrbPassive takes no fields'),
    (r'\bEffect::EndTurn\s*\{', 'EndTurn takes no fields'),
    (r'\bEffect::DiscardHand\s*\{', 'DiscardHand takes no fields'),
    (r'\bEffect::KillSelf\s*\{', 'KillSelf takes no fields'),
    (r'\bEffect::CompleteQuest\s*\{', 'CompleteQuest takes no fields'),
    (r'\bEffect::GenerateRandomPotion\s*\{', 'GenerateRandomPotion takes no fields'),
    (r'\bEffect::FillPotionSlots\s*\{', 'FillPotionSlots takes no fields'),
]

# Validate that MoveCard always has from + to + selector (the agents sometimes
# wrote MoveCard with only from + selector).
MOVECARD_BAD_RE = re.compile(
    r'Effect::MoveCard\s*\{\s*from:\s*Pile::\w+\s*,\s*selector:',
    re.M,
)

# Arm parser — naive, but robust enough for the agent output format.
# Matches: "CardName" => Some(vec![ ... ]),
# Or: // SKIP CardName: reason
# Or: "CardName" | "Other" => Some(vec![...]),
ARM_RE = re.compile(
    r'(?P<arm>("(?P<name>[A-Za-z_0-9]+)"\s*=>\s*Some\(vec!\[.*?\]\)\s*,?))\s*$',
    re.M | re.S,
)

SKIP_RE = re.compile(r'//\s*SKIP\s+(?P<name>[A-Za-z_0-9]+)\s*:\s*(?P<reason>.+)$', re.M)


def parse_batch(text):
    """Return list of (card_name, body_text, kind) where kind is 'arm' or 'skip'."""
    # Strip HTML escapes from the agent text (some emit &gt; etc.).
    text = text.replace('&gt;', '>').replace('&lt;', '<').replace('&amp;', '&')

    out = []
    seen = set()

    # First find all SKIP comments
    for m in SKIP_RE.finditer(text):
        name = m.group('name')
        if name in seen:
            continue
        seen.add(name)
        out.append((name, None, 'skip', m.group('reason').strip()))

    # Then find all match arms
    for m in ARM_RE.finditer(text):
        name = m.group('name')
        if name in seen:
            continue
        seen.add(name)
        body = m.group('arm')
        out.append((name, body, 'arm', None))

    return out


def is_arm_clean(body):
    """Return (clean: bool, reason: str)."""
    for pat, reason in FORBIDDEN:
        if reason is None:
            continue
        if re.search(pat, body):
            return False, reason
    # MoveCard arity check (from + to + selector)
    if 'Effect::MoveCard' in body:
        # Must have both 'from:' and 'to:' inside the MoveCard struct.
        for m in re.finditer(r'Effect::MoveCard\s*\{([^}]*)\}', body, re.S):
            inner = m.group(1)
            if 'from:' not in inner or 'to:' not in inner:
                return False, 'MoveCard missing to: field'
    # AmountSpec::BranchedOnUpgrade subfields must be i32, the agent sometimes
    # wrote AmountSpec::XEnergy or other AmountSpec there.
    for m in re.finditer(
        r'AmountSpec::BranchedOnUpgrade\s*\{\s*base:\s*([^,]+),\s*upgraded:\s*([^,}]+)',
        body,
    ):
        for f in (m.group(1), m.group(2)):
            f = f.strip()
            if 'AmountSpec' in f or '/*' in f:
                return False, 'BranchedOnUpgrade subfields must be i32'
    return True, ''


def main():
    here = os.path.dirname(__file__)
    batches = []
    for name in ('batch_a.txt', 'batch_b.txt', 'batch_c.txt', 'batch_d.txt'):
        p = os.path.join(here, name)
        with open(p, 'r', encoding='utf-8') as f:
            batches.append(f.read())

    all_entries = []  # list of (name, body_or_None, kind, reason)
    for text in batches:
        all_entries.extend(parse_batch(text))

    # Dedup by name (first-seen wins).
    seen = set()
    deduped = []
    for e in all_entries:
        n = e[0]
        if n in seen:
            continue
        seen.add(n)
        deduped.append(e)

    # Produce the Rust output.
    print("// Auto-merged card encodings from 4 parallel agents.")
    print("// Generated by tools/merge_card_ports/merge.py")
    print()

    n_clean = 0
    n_skipped_arity = 0
    n_skipped_reason = 0
    for name, body, kind, reason in deduped:
        if kind == 'skip':
            print(f'        // SKIP {name}: {reason}')
            n_skipped_reason += 1
            continue
        # body is a full match arm; strip trailing comma/whitespace
        clean, why = is_arm_clean(body)
        if not clean:
            print(f'        // SKIP {name}: arity-fix needed ({why})')
            n_skipped_arity += 1
            continue
        # Indent the body
        indented = '\n'.join('        ' + line if line.strip() else line
                             for line in body.splitlines())
        # Ensure trailing comma
        if not indented.rstrip().endswith(','):
            indented = indented.rstrip() + ','
        print(indented)
        n_clean += 1

    print(file=sys.stderr)
    print(f'STATS: clean={n_clean} skipped_arity={n_skipped_arity} skipped_reason={n_skipped_reason}',
          file=sys.stderr)
    print(f'TOTAL: {n_clean + n_skipped_arity + n_skipped_reason}', file=sys.stderr)


if __name__ == '__main__':
    main()
