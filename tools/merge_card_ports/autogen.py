"""
Auto-generate effects::card_effects() Rust match arms from cards.json.

Strategy: pattern-match on (card_type, target_type, canonical_vars) and
emit a safe Effect list. Cards that don't fit a recognized shape are
emitted as `// SKIP CardName: <reason>` for later manual handling.

This is the conservative path — only cards with crisp shape get encoded,
but every emitted arm WILL compile against the existing Effect VM.
"""

import json
import os
import sys

ALREADY_MIGRATED = {
    'StrikeIronclad', 'StrikeSilent', 'StrikeDefect', 'StrikeRegent', 'StrikeNecrobinder',
    'DefendIronclad', 'DefendSilent', 'DefendDefect', 'DefendRegent', 'DefendNecrobinder',
    'Bash', 'Neutralize', 'Thunderclap', 'IronWave', 'TwinStrike', 'Inflame', 'Bloodletting',
    'Defile', 'Defy', 'CosmicIndifference', 'CloakOfStars', 'AstralPulse', 'BeamCell',
    'BoostAway',
}

# Cards with existing rich match-arm dispatch in combat.rs that the
# shape-match autogen would oversimplify. Leaving these out lets the
# match-arm path keep running. Discovered empirically when an auto-
# encoding caused a regression test to fail.
HAS_MATCH_ARM_KEEP_AS_FALLTHROUGH = {
    'Anger',                  # clones self into discard
    'BlightStrike',           # Doom = damage dealt (post-modify)
    'Breakthrough',           # LoseHp + AOE damage
    'Cinder',                 # exhaust random hand card
    'CollisionCourse',        # adds Debris to hand
    'DaggerSpray',            # multi-hit AOE
    'DemonForm',              # Power-VM-bound behavior (turn-start Strength)
    'Feed',                   # ChangeMaxHp on kill
    'FiendFire',              # exhaust hand + per-card hit
    'GraveWarden',            # Souls to draw
    'Mangle',                 # ManglePower + StrengthLoss
    'MoltenFist',             # exhaust + vuln stack
    'BladeDance',             # adds Shivs
    'CloakAndDagger',         # block + Shivs
    'LeadingStrike',          # damage + Shivs
    'PommelStrike',           # damage + draw
    'Ricochet',               # random multi-hit
    'SetupStrike',            # SetupStrikePower self-buff
    'Snakebite',              # damage + Poison rider
    'SwordBoomerang',         # random hits, count is canonical
    'Tremble',                # Vulnerable on target (no power_id var alone)
    'TrueGrit',               # block + exhaust upgrade branch
    'LegSweep',               # block + Weak on target
    'PerfectedStrike',        # deck-scan scaling
    'Barricade',              # power w/ persistent ShouldClearBlock
    'Apparition',             # IntangiblePower self
    'Survivor',                # "Cards" var means cards-to-discard, not draw — auto-shape misinterprets
    'Acrobatics',              # Same: draws then discards
    'DodgeAndRoll',            # Same shape mis-interpretation
    'EscapePlan',              # conditional on draw
    'Calcify',                 # Block scaling per-turn, hand-coded better
    'Dash',                    # block + damage (auto-shape doesn't compose)
    'PoisonedStab',            # damage + Poison
    'Sunder',                  # damage + energy-on-kill
    'BodySlam',                # damage = self block (needs SelfBlock AmountSpec)
    'Bully',                   # damage scales with target Vulnerable
    'Conflagration',           # damage = attacks-played-this-turn * X
    'PerfectedStrike',         # damage = N * Strike-tag count in deck
    'Entrench',                # doubles current block
    'Mimic',                   # block = target's current block
    'MindBlast',               # damage = draw pile size
    'Rend',                    # damage scales by target debuff count
    'GoldAxe',                 # damage scales by cards played this turn
    'Finisher',                # hit count = attacks played this turn
    'FlakCannon',              # hit count = status count
    'Flechettes',              # hit count = skill count in hand
    'GangUp',                  # damage scales by teammate attacks
    'HelixDrill',              # hit count = energy spent this turn
    'Murder',                  # damage scales by card draws
    'MementoMori',             # damage scales by discards this turn
    'PullFromBelow',           # hit count = ethereals played
    'Radiate',                 # hit count = stars gained
    'Rattle',                  # hit count = osty attacks
    'CrescentSpear',           # scales by star-cost cards in state
    'NoEscape',                # Doom = target Doom / 10
    'Dominate',                # Strength = target Vulnerable
    'Hang',                    # power max with current
    'Restlessness',            # only-card-in-hand conditional
    'PreciseCut',              # negative hand-size scaling
    'DeathMarch',              # damage scales by draws this turn
    'DeathsDoor',              # block multiple if Doom applied
    'Pillage',                 # draw until non-attack
    'Fisticuffs',              # block = damage dealt
    'Mirage',                  # block = sum enemy Poison
    'Misery',                  # debuff clone-and-spread
    'Omnislice',               # transitive teammate splash
    'Eradicate',               # X-hits attack
    'HeavenlyDrill',           # X-cost hits doubled if X>=4
    'ExpectAFight',            # energy = attack count in hand
    'Ftl',                     # draw based on plays-this-turn
    'EvilEye',                 # block 2x if exhaust this turn
    'ForgottenRitual',         # energy if card exhausted this turn
    'CalculatedGamble',        # draw = hand size pre-discard
    'CompileDriver',           # draw = orb-types in queue
    'Maul',                    # self-buff own DamageVar
    'Rampage',                 # self-buff own DamageVar
    'Claw',                    # buff all Claws in deck
    'GeneticAlgorithm',        # persistent self-buff
    'KnifeTrap',               # auto-play all Shivs in exhaust
    'LegionOfBone',            # per-ally Osty
    'Rally',                   # per-teammate block
    'Enlightenment',           # set cost on hand
    'Discovery',               # random card-from-pool choice
    'Distraction',             # random skill-from-pool
    'BulletTime',              # set hand free + NoDraw
    'BundleOfJoy',             # 3 random colorless
    'JackOfAllTrades',         # random colorless
    'Jackpot',                 # random 0-cost generation
    'InfernalBlade',           # random attack-from-pool
    'AshenStrike',             # damage = exhaust pile count
    'Charge',                  # transform 2 draw to MinionDiveBomb
    'Cascade',                 # AutoplayFromDraw with X+1 hits
    'Chill',                   # channel FrostOrb per enemy
    'CrashLanding',            # fill hand to 10 with Debris
    'CrimsonMantle',           # increment own SelfDamage
    'CelestialMight',          # hit count via canonical
    'Bombardment',             # BeforeHandDraw auto-replay
    'Bolas',                   # BeforeHandDraw re-add
    'Stoke',                   # generate per exhausted
    'StormOfSteel',            # shiv count = hand size
    'Scrawl',                  # draw = 10 - hand size
    'Severance',               # async stub
    'BeatDown',                # play random attacks from discard
    'DecisionsDecisions',      # auto-play chosen skill 3 times
    'Headbutt',                # discard pick to draw top
    'Anointed',                # move all Rare to hand
    'AllForOne',               # filter cost==0 attacks
    'BadLuck',                 # turn-end-in-hand only
    'AdaptiveStrike',          # clone-with-cost-0
    'DualWield',               # clone chosen hand card
    'Dredge',                  # discard interactive pick × 3
    'PhotonCut',               # damage + interactive put-back
    'DyingStar',               # damage + StrengthLoss AOE
    'EndOfDays',               # Doom on all (PowerCmd.Apply works)
    'FightMe',                 # damage + both-sides Strength
    'FightThrough',            # block + 2 Wounds
    'Glow',                    # stars + draw + next-turn-draw
    'GammaBlast',              # damage + Weak + Vulnerable
    'GuidingStar',             # damage + draw
    'Hologram',                # block + discard pick
    'Hyperbeam',               # AOE + lose Focus
    'IceLance',                # damage + 3 channels
    'Modded',                  # +orb slot + draw
    'Neurosurge',              # energy + draw + NeurosurgePower
    'Offering',                # lose-hp + energy + draw
    'Overclock',               # draw + burn
    'Patter',                  # block + Vigor
    'PoisonedStab',            # damage + Poison
    'PullAggro',               # summon Osty + block
    'Refract',                 # damage 2-hit + channel
    'Resonance',               # str-self + neg-str-all
    'Salvo',                   # damage + Retain
    'Scavenge',                # exhaust + energy next turn
    'Scourge',                 # Doom + draw
    'Reboot',                  # discard all + shuffle + draw
    'Relax',                   # block + draw-next + energy-next
    'Spite',                   # conditional on lost-HP-this-turn
    'Turbo',                   # energy + add Void
    'Undeath',                 # block + clone self
    'Unrelenting',             # damage + FreeAttack
    'Uppercut',                # damage + Weak + Vulnerable
    'Uproar',                  # damage + AutoplayFromDraw
    'BlackHole',               # zero-var Power
}


def var_names(card):
    """List of (kind, generic, base_value) from canonical_vars."""
    return [(v.get('kind'), v.get('generic'), v.get('base_value'))
            for v in card.get('canonical_vars', [])]


def has_var(card, generic_name):
    """Does the card have a canonical var with the given generic/kind name?"""
    for k, g, _ in var_names(card):
        if g == generic_name or k == generic_name:
            return True
    return False


def has_power_var(card, power_id):
    for k, g, _ in var_names(card):
        if k == 'Power' and g == power_id:
            return True
    return False


def named_int_kinds(card):
    """Just the 'kind' field for non-Power vars (e.g., 'Damage', 'Block', 'Hits')."""
    return {k for k, g, _ in var_names(card) if k and k != 'Power'}


# Map of {target_type, has_only_damage, ...} → Effect template.
def encode_card(card):
    """Return (rust_body | None, skip_reason | None)."""
    cid = card['id']
    ctype = card.get('card_type')
    ttype = card.get('target_type')
    pool = card.get('pool')
    keywords = set(card.get('keywords', []))
    tags = set(card.get('tags', []))
    has_x = card.get('has_energy_cost_x')
    vars_ = var_names(card)
    var_kinds = {k for k, _, _ in vars_ if k}
    var_powers = {g for k, _, g in [(k, k, g) for k, g, _ in vars_] if k == 'Power'}
    # Re-derive var_powers cleanly:
    var_powers = {g for k, g, _ in vars_ if k == 'Power'}

    # === Trivial empty bodies (status/curse with no OnPlay) ===
    if ctype in ('Status', 'Curse'):
        # Most status/curse cards have OnPlay = nothing
        # (their behavior is in OnTurnEndInHand / passive hooks).
        # Encode as empty effect list.
        # Exceptions: Slimed has DrawCards in some versions but we treat as
        # empty for the conservative auto-encode.
        return 'Some(vec![])', None

    # === Power-type cards: OnPlay = single ApplyPower<XPower> ===
    if ctype == 'Power' and ttype in ('Self', 'AnyPlayer'):
        # Need exactly one Power canonical var to use this shape.
        if len(var_powers) == 1:
            (pid,) = var_powers
            return (
                f'Some(vec![Effect::ApplyPower {{ '
                f'power_id: "{pid}".to_string(), '
                f'amount: AmountSpec::Canonical("{pid}".to_string()), '
                f'target: Target::SelfPlayer }}])',
                None,
            )
        # Two-power Power cards: Apply both to self (Abrasive: Thorns+Dex,
        # Prowess: Strength+Dex, etc.)
        if len(var_powers) >= 2 and len(var_powers) <= 3:
            steps = []
            for pid in sorted(var_powers):
                steps.append(
                    f'Effect::ApplyPower {{ '
                    f'power_id: "{pid}".to_string(), '
                    f'amount: AmountSpec::Canonical("{pid}".to_string()), '
                    f'target: Target::SelfPlayer }}'
                )
            return f'Some(vec![{", ".join(steps)}])', None
        # Zero-power Power card: the power id is the card name + "Power".
        # C# convention is `MayhemPower`, `BarricadePower`, etc. The card's
        # canonical vars carry the amount under a key like "Power" or
        # "Cards" (whatever the C# CanonicalVars getter names it).
        if len(var_powers) == 0:
            pid = f'{cid}Power'
            # Pick the most-likely amount var. Prefer kinds in this order.
            amount_var = None
            for prefer in ('Power', 'Strength', 'Dexterity', 'Focus', 'Cards',
                           'Energy', 'Amount', 'Stars', 'Block', 'Dynamic',
                           'StrengthPower'):
                if prefer in var_kinds:
                    amount_var = prefer
                    break
            if amount_var is None and var_kinds:
                # Fall back to first var alphabetically — deterministic but arbitrary.
                amount_var = sorted(var_kinds)[0]
            amount_spec = (
                f'AmountSpec::Canonical("{amount_var}".to_string())'
                if amount_var else
                'AmountSpec::Fixed(1)'
            )
            return (
                f'Some(vec![Effect::ApplyPower {{ '
                f'power_id: "{pid}".to_string(), '
                f'amount: {amount_spec}, '
                f'target: Target::SelfPlayer }}])',
                None,
            )
        return None, f'Power card with {len(var_powers)} canonical powers; unknown shape'

    # Helper: pick the damage-amount var (Damage or CalculatedDamage).
    def pick_dmg_var():
        for k in ('Damage', 'CalculatedDamage'):
            if k in var_kinds:
                return k
        return None

    # === Attack to single enemy ===
    if ctype == 'Attack' and ttype == 'AnyEnemy':
        if has_x:
            return None, 'X-cost single-target attack (would need Repeat over hits)'
        dvar = pick_dmg_var()
        if dvar and 'Damage' in var_kinds:
            damage_part = (
                f'Effect::DealDamage {{ '
                f'amount: AmountSpec::Canonical("Damage".to_string()), '
                f'target: Target::ChosenEnemy, hits: 1 }}'
            )
            # Common riders: Damage + ApplyPower on target
            riders = []
            for p in sorted(var_powers):
                if p in (
                    'VulnerablePower', 'WeakPower', 'FrailPower', 'PoisonPower',
                    'BurnedPower', 'BlindedPower',
                ):
                    # Use the matching numeric var name (e.g. "Vulnerable", "Weak").
                    # Conventionally the var is named without "Power" suffix.
                    base = p.replace('Power', '')
                    if base in var_kinds:
                        riders.append(
                            f'Effect::ApplyPower {{ power_id: "{p}".to_string(), '
                            f'amount: AmountSpec::Canonical("{base}".to_string()), '
                            f'target: Target::ChosenEnemy }}'
                        )
                    else:
                        # Power-id keyed amount
                        riders.append(
                            f'Effect::ApplyPower {{ power_id: "{p}".to_string(), '
                            f'amount: AmountSpec::Canonical("{p}".to_string()), '
                            f'target: Target::ChosenEnemy }}'
                        )
            # Self block as rider (e.g. IronWave shape — but we already migrated IronWave;
            # other cards like Dash: Block + Damage)
            if 'Block' in var_kinds:
                # IronWave shape: block first, then damage.
                block_part = (
                    f'Effect::GainBlock {{ '
                    f'amount: AmountSpec::Canonical("Block".to_string()), '
                    f'target: Target::SelfPlayer }}'
                )
                return f'Some(vec![{block_part}, {damage_part}{", " + ", ".join(riders) if riders else ""}])', None
            return f'Some(vec![{damage_part}{", " + ", ".join(riders) if riders else ""}])', None
        if dvar:  # CalculatedDamage path
            return (
                f'Some(vec![Effect::DealDamage {{ '
                f'amount: AmountSpec::Canonical("{dvar}".to_string()), '
                f'target: Target::ChosenEnemy, hits: 1 }}])',
                None,
            )
        return None, 'Attack to single enemy without Damage var'

    # === Attack to all enemies ===
    if ctype == 'Attack' and ttype == 'AllEnemies':
        if has_x:
            return None, 'X-cost AOE (Whirlwind shape -- handled in earlier migration)'
        dvar_aoe = pick_dmg_var()
        if 'Damage' in var_kinds:
            damage_part = (
                f'Effect::DealDamage {{ '
                f'amount: AmountSpec::Canonical("Damage".to_string()), '
                f'target: Target::AllEnemies, hits: 1 }}'
            )
            riders = []
            for p in sorted(var_powers):
                if p in ('VulnerablePower', 'WeakPower', 'FrailPower', 'PoisonPower'):
                    base = p.replace('Power', '')
                    if base in var_kinds:
                        riders.append(
                            f'Effect::ApplyPower {{ power_id: "{p}".to_string(), '
                            f'amount: AmountSpec::Canonical("{base}".to_string()), '
                            f'target: Target::AllEnemies }}'
                        )
                    else:
                        riders.append(
                            f'Effect::ApplyPower {{ power_id: "{p}".to_string(), '
                            f'amount: AmountSpec::Canonical("{p}".to_string()), '
                            f'target: Target::AllEnemies }}'
                        )
            return f'Some(vec![{damage_part}{", " + ", ".join(riders) if riders else ""}])', None
        if dvar_aoe:
            return (
                f'Some(vec![Effect::DealDamage {{ '
                f'amount: AmountSpec::Canonical("{dvar_aoe}".to_string()), '
                f'target: Target::AllEnemies, hits: 1 }}])',
                None,
            )
        return None, 'AOE attack without Damage var'

    # === Attack to random enemy ===
    if ctype == 'Attack' and ttype == 'RandomEnemy':
        dvar_r = pick_dmg_var()
        if dvar_r:
            return (
                f'Some(vec![Effect::DealDamage {{ '
                f'amount: AmountSpec::Canonical("{dvar_r}".to_string()), '
                f'target: Target::RandomEnemy, hits: 1 }}])',
                None,
            )
        return None, 'Random-target attack without Damage var'

    # === Skill: self-target block ===
    if ctype == 'Skill' and ttype in ('Self', 'AnyPlayer', 'AnyAlly'):
        # Pure block skill
        if var_kinds == {'Block'}:
            return (
                f'Some(vec![Effect::GainBlock {{ '
                f'amount: AmountSpec::Canonical("Block".to_string()), '
                f'target: Target::SelfPlayer }}])',
                None,
            )
        # Block + Cards (Backflip / ShrugItOff)
        if var_kinds == {'Block', 'Cards'}:
            return (
                f'Some(vec!['
                f'Effect::GainBlock {{ amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }}, '
                f'Effect::DrawCards {{ amount: AmountSpec::Canonical("Cards".to_string()) }}'
                f'])',
                None,
            )
        # Pure draw
        if var_kinds == {'Cards'}:
            return (
                f'Some(vec![Effect::DrawCards {{ '
                f'amount: AmountSpec::Canonical("Cards".to_string()) }}])',
                None,
            )
        # Pure energy
        if var_kinds == {'Energy'}:
            return (
                f'Some(vec![Effect::GainEnergy {{ '
                f'amount: AmountSpec::Canonical("Energy".to_string()) }}])',
                None,
            )
        # Block + single Power on self
        if 'Block' in var_kinds and len(var_powers) == 1:
            (pid,) = var_powers
            block_part = (
                f'Effect::GainBlock {{ amount: AmountSpec::Canonical("Block".to_string()), '
                f'target: Target::SelfPlayer }}'
            )
            power_part = (
                f'Effect::ApplyPower {{ power_id: "{pid}".to_string(), '
                f'amount: AmountSpec::Canonical("{pid}".to_string()), '
                f'target: Target::SelfPlayer }}'
            )
            return f'Some(vec![{block_part}, {power_part}])', None
        # Self power application (e.g. Inflame as Skill not Power)
        if var_kinds == set() and len(var_powers) == 1:
            (pid,) = var_powers
            return (
                f'Some(vec![Effect::ApplyPower {{ '
                f'power_id: "{pid}".to_string(), '
                f'amount: AmountSpec::Canonical("{pid}".to_string()), '
                f'target: Target::SelfPlayer }}])',
                None,
            )
        return None, f'Skill/Self shape with vars={var_kinds} powers={var_powers} not recognized'

    # === Skill: target enemy (apply debuffs) ===
    if ctype == 'Skill' and ttype == 'AnyEnemy':
        # Pure debuff on target
        if len(var_powers) == 1 and not var_kinds - {'Power'}:
            (pid,) = var_powers
            return (
                f'Some(vec![Effect::ApplyPower {{ '
                f'power_id: "{pid}".to_string(), '
                f'amount: AmountSpec::Canonical("{pid}".to_string()), '
                f'target: Target::ChosenEnemy }}])',
                None,
            )
        # Apply two debuffs (e.g. Tremble = Vulnerable)
        if len(var_powers) >= 1 and not var_kinds - {'Power'}:
            steps = []
            for p in sorted(var_powers):
                base = p.replace('Power', '')
                amount_var = base if base in var_kinds else p
                steps.append(
                    f'Effect::ApplyPower {{ power_id: "{p}".to_string(), '
                    f'amount: AmountSpec::Canonical("{amount_var}".to_string()), '
                    f'target: Target::ChosenEnemy }}'
                )
            return f'Some(vec![{", ".join(steps)}])', None
        # Block + debuff on target (Defy shape — already migrated, but covers more cards)
        if 'Block' in var_kinds and len(var_powers) == 1:
            (pid,) = var_powers
            base = pid.replace('Power', '')
            amount_var = base if base in var_kinds else pid
            return (
                f'Some(vec!['
                f'Effect::GainBlock {{ amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }}, '
                f'Effect::ApplyPower {{ power_id: "{pid}".to_string(), amount: AmountSpec::Canonical("{amount_var}".to_string()), target: Target::ChosenEnemy }}'
                f'])',
                None,
            )
        return None, f'Skill/AnyEnemy shape with vars={var_kinds} powers={var_powers} not recognized'

    # === Skill: all enemies (mass debuff) ===
    if ctype == 'Skill' and ttype == 'AllEnemies':
        if len(var_powers) >= 1:
            steps = []
            for p in sorted(var_powers):
                base = p.replace('Power', '')
                amount_var = base if base in var_kinds else p
                steps.append(
                    f'Effect::ApplyPower {{ power_id: "{p}".to_string(), '
                    f'amount: AmountSpec::Canonical("{amount_var}".to_string()), '
                    f'target: Target::AllEnemies }}'
                )
            return f'Some(vec![{", ".join(steps)}])', None
        return None, f'Skill/AllEnemies with vars={var_kinds} powers={var_powers} not recognized'

    return None, f'unknown shape: type={ctype} target={ttype} vars={var_kinds} powers={var_powers}'


def main():
    here = os.path.dirname(os.path.dirname(os.path.dirname(__file__)))
    cards_path = os.path.join(here, 'crates', 'sts2-sim', 'data', 'cards.json')
    with open(cards_path, 'r', encoding='utf-8') as f:
        cards = json.load(f)

    encoded = []  # (cid, body)
    skipped = []  # (cid, reason)

    for c in cards:
        cid = c['id']
        if cid in ALREADY_MIGRATED:
            continue
        if cid in HAS_MATCH_ARM_KEEP_AS_FALLTHROUGH:
            skipped.append((cid, 'has richer match-arm in combat.rs; let it run'))
            continue
        body, reason = encode_card(c)
        if body is not None:
            encoded.append((cid, body))
        else:
            skipped.append((cid, reason))

    # Output Rust source (ASCII-only — Windows console hates em-dashes).
    print('// Auto-generated from cards.json by tools/merge_card_ports/autogen.py')
    print('// Per-card encodings -- conservative shape match only.')
    print('// Skipped cards fall through to the match-arm dispatch path or')
    print('// are not yet ported. See `// SKIP` comments for reasons.')
    print()
    for cid, body in encoded:
        print(f'        "{cid}" => {body},')
    print()
    print('        // ===== Skipped (need shape-specific handling) =====')
    for cid, reason in skipped:
        print(f'        // SKIP {cid}: {reason}')

    print(file=sys.stderr)
    print(f'STATS: encoded={len(encoded)} skipped={len(skipped)} total={len(encoded) + len(skipped)}',
          file=sys.stderr)


if __name__ == '__main__':
    main()
