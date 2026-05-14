# Effect Vocabulary

Closed primitive vocabulary for the data-driven port of cards, relics, potions,
and monster moves. Derived from a survey of the C# decompile under
`sts2_decompiled\sts2\MegaCrit\sts2\Core\Models\{Cards,Relics,Potions,Monsters}\`
and `Core\MonsterMoves\`.

The thesis (decided 2026-05-14, see plan §0.2a): every card / relic / potion /
monster-move body is a composition over a fixed set of primitive operations.
Once each primitive is implemented once in Rust, content becomes pure data
(JSON tuples of `(condition?, primitive, args)` steps). Even single-use
primitives — Headbutt's pick-from-discard, EchoingSlash's repeat-until-no-kills,
EntropicBrew's fill-potion-slots — are still primitives. They get implemented
once and never again.

**Status legend**: ✅ implemented in Rust core — 🟡 partial — ❌ missing.

---

## 1. Card OnPlay vocabulary

Survey: all 577 .cs files in `Models/Cards/`.

The C# OnPlay bodies are written in a fluent-Cmd style:
`DamageCmd.Attack(amt).FromCard(card).Targeting(target).WithHitCount(n).WithMultiplier(...)`.
Once cosmetic chains (`WithHitFx`, `TriggerAnim`, `Cmd.Wait`) are dropped, the
underlying primitive vocabulary is small.

### 1.1 Damage primitives

| primitive | status | freq | notes |
|---|---|---:|---|
| `DealDamage { amount, target=SingleEnemy }` | ✅ | 193 | `combat.rs::deal_damage`. The dominant attack form. |
| `DealDamage { amount, target=AllEnemies }` | ✅ | 28 | Loop existing primitive over `hittable_enemies`. |
| `DealDamage { amount, target=RandomEnemy, reroll_dead }` | ❌ | 7 | SwordBoomerang. Need RNG-keyed per-hit target selection. |
| `DealDamage { amount, hits: int }` (multi-hit, same target) | ✅ | 36 | Implemented via loop; needs primitive-level expression for VM. |
| `DealDamage { amount = f(state), multiplier_fn }` | 🟡 | 43 | PerfectedStrike + Conflagration done as bespoke arms; need generic `AmountSpec::Scaled`. |
| `DealDamage` with `BeforeDamage(asyncDelegate)` (per-hit callback) | ❌ | 6 | FiendFire's exhaust-then-damage idiom; Hyperbeam; Flatten. |
| `CreatureCmd.Damage(target, amount, props=Unblockable\|Unpowered)` (non-attack direct dmg) | 🟡 | 18 | Bloodletting works; need general `DirectDamage { props }`. |
| `RepeatUntilNoKills(attack_payload)` | ❌ | 1 | EchoingSlash. Single card but a true primitive. |
| `Kill { target }` | ❌ | 2 | Sacrifice. Direct kill bypassing damage. |

### 1.2 Block / HP primitives

| primitive | status | freq | notes |
|---|---|---:|---|
| `GainBlock { target=Self, amount }` | ✅ | 77 | `combat.rs::gain_block`. |
| `GainBlock { amount = f(state) }` (e.g. Sacrifice: `Osty.MaxHp * 2`) | ❌ | rare | Generalize amount-spec. |
| `LoseBlock { target, amount }` | ❌ | 1 | Sunder-style. |
| `Heal { target, amount }` | ✅ | 2 | `combat.rs::heal_creature`. |
| `GainMaxHp { target, amount }` | ✅ | 1 | Feed. `change_max_hp`. |
| `LoseMaxHp { target, amount }` | 🟡 | 1 | Inverse via `change_max_hp(-n)`. |
| `SetCurrentHp { target, amount_or_pct }` | ❌ | 0 in cards (used in LizardTail relic) | |

### 1.3 Power application

| primitive | status | freq | notes |
|---|---|---:|---|
| `ApplyPower<T> { target, amount }` (generic, T = Power id) | ✅ | 269 | `combat.rs::apply_power`. Single primitive — `T` is data. |
| `ModifyPowerAmount<T> { target, delta }` (direct mutate) | ❌ | 2 | Adrenaline-style. |
| `RemovePower<T> { target }` | ❌ | 1 | Cleanse-style. |

### 1.4 Pile / card-flow primitives

| primitive | status | freq | notes |
|---|---|---:|---|
| `DrawCards { n }` | ✅ | 52 | `combat.rs::draw_cards`. |
| `AddCardToPile { card_id, pile, position, upgrade? }` (token gen) | ✅ | 23 | `combat.rs::add_card_to_pile`. AdaptiveStrike, Anger, Discovery. |
| `AddGeneratedCardsToCombat { cards[], pile }` (plural) | 🟡 | 9 | Loop over scalar. |
| `MoveCard { card_ref, to_pile, position }` (existing card, not generated) | ❌ | 23 | Anointed, Headbutt. Different from AddCardToPile. |
| `RemoveFromDeck { card_ref }` | ❌ | 3 | Dismantle. |
| `AutoPlayFromDrawPile { card_ref }` | ❌ | 3 | Mayhem-adjacent. |
| `Shuffle { pile }` | ❌ | 1 | Recycle. |
| `Exhaust { card_ref }` | 🟡 | 14 | `exhaust_random_card_in_hand` for random; need targeted form. |
| `ExhaustRandomInHand { n }` | ✅ | rare | Cinder, TrueGrit. |
| `Discard { card_ref }` | ❌ | 8 | Acrobatics. |
| `AutoPlay { card_ref }` | ❌ | 8 | Mockingbird / Echo-style — plays another card immediately. |
| `UpgradeCard { card_ref }` | ❌ | 21 | Armaments. In-combat upgrade. |
| `Transform { card_ref, to_card_id? }` | ❌ | 6 | TransformCard. |
| `Enchant { card_ref, enchantment }` | ❌ | 1 | Single use. |
| `ApplyKeyword { card_ref, kw }` | ❌ | 2 | Add Ethereal / Exhaust / Retain / Innate at runtime. |
| `SetCardCostThisCombat / ThisTurn / UntilPlayed { card_ref, cost }` | ❌ | 12 | Discovery and family. |
| `PromptPlayerToSelect { source: PileType\|GeneratedList, filter, count, then_effect }` | ❌ | 39 | `CardSelectCmd.From*` family. Player choice. |

### 1.5 Resource primitives

| primitive | status | freq | notes |
|---|---|---:|---|
| `GainEnergy { amount }` | ✅ | 23 | Bloodletting. `combat.rs`. |
| `LoseEnergy { amount }` | ❌ | 1 | Debt. |
| `GainGold { amount }` | ❌ | 2 | HandOfGreed, Alchemize. |
| `LoseGold { amount }` | ❌ | 1 | Debt. |
| `GainStars { amount }` | ❌ | 9 | GatherLight + Watcher-style cards. |
| `GenerateRandomPotion { slot }` | ❌ | 1 | Alchemize. |
| `EndTurn` | ❌ | 1 | FranticEscape. |
| `CompleteQuest` | ❌ | 1 | RoyalGamble. Quest-card progression. |

### 1.6 Orb primitives (Defect)

| primitive | status | freq | notes |
|---|---|---:|---|
| `ChannelOrb<T>` | ❌ | 23 | Generic, T = orb id. |
| `EvokeNextOrb` | ❌ | 5 | MultiCast. |
| `TriggerOrbPassive` | ❌ | 2 | Recycle-passive. |
| `AddOrbSlots { n }` | ❌ | 2 | Capacitor. |
| `RemoveOrbSlots { n }` | ❌ | 1 | |

### 1.7 Osty / Forge primitives (StS2-specific)

| primitive | status | freq | notes |
|---|---|---:|---|
| `SummonOsty { osty_id }` | ❌ | 9 | Companion-summon cards. |
| `DamageFromOsty { amount, target }` | ❌ | 19 | Damage attributed to companion (Protector). |
| `Forge { ... }` | ❌ | 10 | Smith-forge primitive. |

### 1.8 X-cost / repeat conventions

- `XCostExpands(effect_per_x)` — Whirlwind, Skewer, MultiCast, Cascade, Dirge,
  Eradicate, HeavenlyDrill, Malaise, Tempest, Volley. Resolves via
  `ResolveEnergyXValue()` → integer N → repeat the inner effect N times.
- Delayed effects (`AtEndOfTurn`, `NextTurn`) are **always** encoded as a Power
  applied at OnPlay time. The Power's turn-start/turn-end hook does the work.
  So `OnPlay` for these cards reduces to a bare `ApplyPower<XPower>`.

### 1.9 Top 7 primitives cover ~85% of all cards

Pareto frontier: `ApplyPower` + `Attack(single/multi/all)` + `GainBlock` +
`Draw` + `GainEnergy` + `ExhaustRandomInHand` + `AddCardToPile`. All seven are
already implemented in the Rust core. The long tail of one-offs (about 18
primitives, ❌ above) is the remaining wiring work.

---

## 2. Relic hooks + bodies

Survey: all 294 .cs files in `Models/Relics/`.

StS2 relics do NOT use `RegisterHook(...)`. Every hook is an `override` on
`RelicModel`. The vocabulary splits into two layers: **(a)** trigger points the
relic subscribes to and **(b)** primitives invoked inside.

### 2.1 Hook trigger points (~50 distinct)

| trigger | status | scope | example_relics |
|---|---|---|---|
| `BeforeCombatStart` | ✅ | combat | Anchor, Akabeko, BagOfMarbles, BronzeScales, DataDisk |
| `AfterCombatVictory` | ✅ | combat | BurningBlood, BlackBlood, BeltBuckle |
| `AfterCombatEnd` (non-victory branch) | ❌ | combat | DiamondDiadem (counter reset), HappyFlower |
| `BeforeSideTurnStart` | ❌ | turn | ArtOfWar, Pendulum, RainbowRing (reset per-turn counter) |
| `AfterSideTurnStart` | ✅ | turn | Brimstone, MiniRegent, EmberTea, Candelabra |
| `BeforeTurnEndVeryEarly`, `BeforeTurnEnd`, `AfterTurnEnd` | 🟡 | turn | ArtOfWar, Pocketwatch |
| `BeforeCardPlayed`, `AfterCardPlayed` | ❌ | card | RainbowRing, BrilliantScarf, IronClub, Kunai, LetterOpener |
| `ShouldDraw`, `ShouldPlay`, `ShouldFlush`, `ShouldClearBlock` | ❌ | gate | TheBoot, Calipers |
| `ModifyCardPlayCount`, `ModifyHandDraw`, `ModifyXValue` | ❌ | modify | Pocketwatch, BagOfPreparation, ChemicalX |
| `ModifyDamageAdditive`, `ModifyDamageMultiplicative` | ✅ | damage | StrikeDummy, PenNib |
| `ModifyBlockMultiplicative` | 🟡 | combat | Calipers-style |
| `ModifyHpLostBeforeOsty`, `ModifyHpLostAfterOsty` | ❌ | damage | BeatingRemnant, TungstenRod-pattern |
| `AfterDamageReceived` | ✅ | damage | LavaLamp (flag) |
| `ShouldDieLate`, `AfterDiedToDoom` | ❌ | health | LizardTail |
| `AfterRoomEntered` | ❌ | floor | MawBank, MealTicket, RegalPillow, NewLeaf, LavaLamp |
| `AfterObtained` (one-shot on pickup) | ❌ | run | Pear, PandorasBox, ArcaneScroll, Astrolabe, BiiigHug |
| `TryModifyRestSiteOptions`, `TryModifyRestSiteHealRewards`, `ModifyRestSiteHealAmount` | ❌ | rest | RegalPillow, Coffee-Dripper-style |
| `AfterItemPurchased`, `ShouldRefillMerchantEntry`, `ModifyMerchantPrice`, `ModifyMerchantCardCreationResults` | ❌ | shop | MawBank, merchant-discount relics |
| `ShouldGainGold`, `ShouldProcurePotion`, `ShouldForcePotionReward` | ❌ | gate | Sozu, Ectoplasm |
| `TryModifyRewards`, `TryModifyCardRewardOptionsLate`, `TryModifyCardBeingAddedToDeck` | ❌ | reward | AmethystAubergine, FrozenEgg, MoltenEgg, ToxicEgg, Omamori |
| `ModifyMaxEnergy` | ❌ | run | BloodSoakedRose, Bread, Ectoplasm, Sozu |
| `ModifyOrbPassiveTriggerCounts` | ❌ | orb | Defect-orb relic |
| `ModifyGeneratedMap` | ❌ | map | WingedBoots |

**Cosmetic / system** (`AfterCloned`, `ShouldFlashOnPlayer`, `IsAllowed`,
`IsAllowedInShops`, `IsStackable`, `IsUsedUp`): static metadata, not effect-list
runtime. Lift these into relic-data JSON, not VM ops.

### 2.2 Primitives invoked in relic hook bodies

| primitive | status | freq | notes |
|---|---|---:|---|
| `ApplyPower<T>` | ✅ | 35 | Most-used relic primitive. |
| `GainBlock` | ✅ | 19 | Anchor pattern. |
| `DealDamage` | ✅ | 16 | CharonsAshes, Crossbow, GremlinHorn. |
| `Heal` | ✅ | 15 | BurningBlood, MealTicket. |
| `GainEnergy` | ✅ | 15 | ArtOfWar, Candelabra. |
| `GainMaxHp` / `LoseMaxHp` / `SetCurrentHp` | 🟡 | 14 | Pear (+10), LizardTail (revive %), etc. |
| `UpgradeCard` | ❌ | 22 | RazorTooth, ArchaicTooth, FrozenEgg. |
| `EnchantCard` | ❌ | 6 | BeautifulBracelet. |
| `ApplyKeyword` | ❌ | 4 | JossPaper. |
| `TransformCard` / `TransformToRandom` | ❌ | 7 | PandorasBox. |
| `AddCardToPile` (incl. AddCurseToDeck) | ✅ | 27 | `combat.rs`. ArcaneScroll, BigHat. |
| `RemoveFromDeck` | ❌ | 6 | Astrolabe, BiiigHug. |
| `DrawCards` | ✅ | 7 | Pendulum, OrnamentalFan. |
| `GainGold` / `LoseGold` | ❌ | 9 | MawBank. |
| `GainStars` | ❌ | 3 | StarCost relics. |
| `GainMaxPotionCount` | ❌ | 3 | AlchemicalCoffer, PotionBelt. |
| `ProcurePotion` | ❌ | 4 | AlchemicalCoffer. |
| `ObtainRelic` / `ReplaceRelic` / `MeltRelic` | ❌ | 5 | ToyBox, BurningSticks→Calm. |
| `SummonOsty` | ❌ | 3 | BoundPhylactery. |
| `ChannelOrb` / `AddOrbSlots` / `OrbPassive` | ❌ | 5 | Defect relics. |
| `ForgeCard` | ❌ | 1 | |
| `OfferRewardCustom` | ❌ | 7 | One-off boss / event rewards. |
| `PromptPlayerToSelect` (various `CardSelectCmd.*` variants) | ❌ | 30+ | BeautifulBracelet, BiiigHug, Whetstone. Collapses to one primitive. |

### 2.3 Per-relic-instance state shapes (5 templates cover ~90%)

1. **bool flag** — `_usedThisCombat`, `_usedThisTurn`, `_wasTriggered`,
   `_isActivating`. Used by ~30 relics.
2. **int counter (turn / combat / run scope)** — `_cardsPlayedThisTurn`,
   `_attacksPlayed`, `_turnsSeen`, `_combatsLeft`, `_timesLifted`. ~20 relics.
3. **decimal accumulator** — BeatingRemnant `_damageReceivedThisTurn`,
   BowlerHat `_pendingBonusGold`.
4. **CardModel reference** — MusicBox `CardBeingPlayed`, PenNib `AttackToDouble`,
   ArchaicTooth `StarterCard`.
5. **act-index** — FurCoat `_furCoatActIndex`, GoldenCompass `_goldenPathAct`,
   PumpkinCandle `_pumpkinActiveAct`. Per-act activation gating.

The `MonsterState.counters: HashMap<String, i32>` we already have for monsters
(`hardened_shell_taken`, `vigor_snapshot`, etc.) is the right model — extend
the same pattern to a `RelicState` per-relic-instance side-table.

### 2.4 Condition vocabulary in relic hooks

- `cardPlay.Card.Owner == base.Owner` (MP filter — always present, collapses to no-op in solo)
- `cardPlay.Card.Type == Attack | Skill | Power | Curse` — card-type predicate
- `cardPlay.Card.Tags.Contains(CardTag.Shiv)` — tag predicate
- `cardPlay.Card.EnergyCost.CostsX` / `HasStarCostX`
- `cardPlay.Resources.EnergyValue >= N`
- `cardPlay.IsAutoPlay`
- `props.IsPoweredAttack()` — gates damage modifiers
- `room is MerchantRoom | RestRoom`
- `RelicModel.IsBeforeAct3TreasureChest(runState)` — act gating
- `combatState.HittableEnemies.Any(...)`
- `CombatManager.Instance.History.CardPlaysFinished.Any(e => HappenedThisTurn && Type == Attack)` — turn-history scan
- `card.IsBasicStrikeOrDefend && card.IsRemovable` — basic-card filter
- `c.IsUpgradable` — eligibility gate

No `Random(p)` coin flips. RNG-driven effects always pull from a named stream
(`Rng.X.NextItem<T>`) — not from raw probability.

### 2.5 Off-combat relics

About 25 relics operate entirely outside combat:
- **+MaxHp on pickup**: Pear (+10), Strawberry (+7), Mango (+14), BigMushroom,
  ChosenCheese, plus the food-relic family.
- **Deck mutation on pickup**: PandorasBox (transform basics to random),
  ArcaneScroll, Astrolabe, BiiigHug (player-prompted edits), Whetstone (upgrade
  N), WarPaint (upgrade N skills), BeautifulBracelet (enchant N with Swift).
- **Floor / shop hooks**: MawBank, MealTicket, RegalPillow, NewLeaf, Bread,
  Girya (gym counter).
- **Run-level gates**: Sozu (blocks potions), Ectoplasm (blocks gold), Omamori
  (blocks curses entering deck), Egg relics (auto-upgrade rewards of a type).
- **Map mutation**: WingedBoots.

These hit ~10 different trigger points; the bodies use the same effect
primitives as in-combat relics plus `GainMaxHp`, `GainGold`, `GainStars`,
`ObtainRelic`, `GainMaxPotionCount`.

---

## 3. Potion OnUse vocabulary

Survey: all 64 .cs files in `Models/Potions/`.

| primitive | status | freq | example |
|---|---|---:|---|
| `ApplyPower<T>` | ✅ | 19 | StrengthPotion, FlexPotion, RegenPotion, VulnerablePotion. Dominant verb. |
| `DealDamage` | ✅ | 4 | FirePotion (20), ExplosiveAmpoule (AOE), FoulPotion (AOE), PotionShapedRock |
| `GainBlock` | ✅ | 2 | BlockPotion, Fortifier (`target.Block * 2`, props=Unpowered), ShipInABottle |
| `GainEnergy` | ✅ | 3 | EnergyPotion, CureAll, RadiantTincture |
| `DrawCards` | ✅ | 5 | SwiftPotion, Clarity, BottledPotential, SneckoOil, GlowwaterPotion |
| `Heal { amount = pct * MaxHp }` | 🟡 | 2 | FairyInABottle (30%), BloodPotion |
| `GainMaxHp` | 🟡 | 1 | FruitJuice |
| `GainGold` (potion thrown at merchant) | ❌ | 1 | FoulPotion (3-way context dispatch) |
| `GainStars` | ❌ | 1 | StarPotion |
| `ForgeCard` | ❌ | 1 | KingsCourage |
| `GenerateCardChoice { type_filter, count, free_this_turn }` | ❌ | 5 | AttackPotion, SkillPotion, PowerPotion, ColorlessPotion |
| `GenerateCardsToHand { pool, count, upgraded? }` | ❌ | 1 | CosmicConcoction (3 upgraded colorless) |
| `CreateCardInHand { card_id, count }` | ❌ | 2 | PotOfGhouls (Souls), CunningPotion (Shivs upgraded) |
| `ChannelOrb { orb_id, count_to_fill_queue }` | ❌ | 1 | EssenceOfDarkness |
| `AddOrbSlots { n }` | ❌ | 1 | PotionOfCapacity |
| `DiscardAndDraw` (player-chosen) | ❌ | 1 | GamblersBrew |
| `ExhaustHand` | ❌ | 2 | GlowwaterPotion (all), Ashwater (chosen) |
| `UpgradeAllInHand` | ❌ | 1 | BlessingOfTheForge |
| `MoveCardToHand { from_pile, choose, free? }` | ❌ | 2 | DropletOfPrecognition (from draw), LiquidMemories (from discard, free) |
| `AutoplayFromDraw { n }` | ❌ | 1 | DistilledChaos |
| `SetCardFreeThisCombat { player_choose 1 }` | ❌ | 1 | TouchOfInsanity |
| `RandomizeHandCost { range, draw_first? }` | ❌ | 1 | SneckoOil |
| `IncrementCardReplayCount { filter=Tag.Strike }` | ❌ | 1 | SoldiersStew |
| `FillPotionSlots` | ❌ | 1 | EntropicBrew |
| `SummonOsty` | ❌ | 1 | BoneBrew |
| `OnPreventedDeath` (passive trigger hook) | ❌ | 1 | FairyInABottle (auto-usage) |

### 3.1 Target-type vocabulary
`Self`, `AnyPlayer`, `AnyEnemy`, `AllEnemies`, `TargetedNoCreature` (FoulPotion
merchant branch). No `RandomEnemy` or `ChoosePileCard` at the potion level
(pile-choice is implemented as `Self` + inline `CardSelectCmd` call).

### 3.2 Amount-source patterns
Fixed literal · `DynamicVars[key].BaseValue` (universal access pattern) ·
`target.MaxHp * pct` (FairyInABottle, BloodPotion) · `target.Block * 2`
(Fortifier) · `OrbQueue.Capacity` (EssenceOfDarkness) · loop-until-slots-full
with named RNG (EntropicBrew) · `Rng.X.NextInt(0,3)` (SneckoOil).

---

## 4. Monster move vocabulary

Survey: ~95 monster files in `Models/Monsters/` + the
`Core/MonsterMoves/MonsterMoveStateMachine/` infrastructure.

### 4.1 Move-payload primitives (~12 total)

| primitive | status | freq | notes |
|---|---|---:|---|
| `DealDamage { target=Player, amount }` | ✅ | 120+ | Default attack move. |
| `DealDamage { hits: int }` (multi-hit) | ✅ | 30+ | `.WithHitCount(n)`. |
| `GainBlock { target=Self, amount, props=Move }` | ✅ | 15 | Defend moves. |
| `ApplyPower<T> { target=Self }` | ✅ | 35 | Buff moves (Strength, Vigor, …). Can pass negative delta (Toadpole removes own Thorns). |
| `ApplyPower<T> { target=Player }` | ✅ | 40 | Debuff (Weak / Frail / Vulnerable / Poison). |
| `AddCardToPile { target=Player, card_id, count, pile=Discard }` | ✅ | 10 | Status-card debuff (Wound, Burn, Slimed, Dazed). |
| `SummonMonster { spawn_id, slot, +MinionPower }` | ❌ | 5 | LivingFog, Fabricator, Ovicopter, Doormaker. |
| `Heal { target=Self, amount * Players.Count }` | ❌ | 3 | TestSubject Revive, KnowledgeDemon, WaterfallGiant. |
| `SetMaxHp + HealToFull` (phase shift) | ❌ | 2 | TestSubject Revive, Doormaker DramaticOpen. |
| `KillSelf` | ❌ | 1 | GasBomb Explode (DeathBlowIntent). |
| `RemovePower<T> { target=Self }` | 🟡 | rare | Doormaker phase swap, TestSubject phase 3, SlumberingBeetle wake. |
| `SetMoveImmediate(state)` (force phase) | 🟡 | 3 | TestSubject TriggerDead, Queen→Enraged on Amalgam death, Doormaker. Triggered by event hooks, not state-machine flow. |

### 4.2 Intent-selection vocabulary (~8 patterns)

1. **Deterministic chain** — `moveState.FollowUpState = otherState`. Doormaker,
   LivingFog, Vantom, PhrogParasite (alternating).
2. **WeightedRandom** — `RandomBranchState.AddBranch(state, cooldown, repeat_type, weight_fn)`.
   Axebot, FlailKnight, Inklet, Fabricator, ScrollOfBiting.
3. **MoveRepeatType** — `CannotRepeat`, `CanRepeatXTimes(n)`, `CanRepeatForever`.
4. **ConditionalBranch** — `ConditionalBranchState.AddState(state, () => predicate)`.
   - On position: `SlotName == "first"` (Myte, Nibbit).
   - On allies alive: `GetAllyCount() > 0` (LivingShield, CorpseSlug, KinPriest, Fabricator).
   - On power: `HasPower<AsleepPower>` (Lagavulin), `HasPower<SlumberPower>` (SlumberingBeetle).
5. **OneTimeFirstMove(opener)** — `MustPerformOnceBeforeTransitioning = true`.
6. **StarterMoveSeed (Rng-indexed)** — `StarterMoveIdx % N` chooses opener.
   ScrollOfBiting, CorpseSlug.
7. **Event-driven phase shifts** — subscribe to `Creature.Died`, `AfterCurrentHpChanged`,
   `PowerApplied`. Used by all multi-phase bosses.

### 4.3 On-spawn payloads (~5 patterns)

Pattern: `await base.AfterAddedToRoom(); await PowerCmd.Apply<X>(self, amt, …);`

- **ApplyPower(self, X)** — by far the dominant on-spawn (60+ monsters).
- **ApplyPower(opponents, X)** — Rocket grants Surrounded to the player.
- **SetMaxAndCurrentHp(∞)** — Doormaker "door form" gimmick.
- **SubscribeToEvent** — TestSubject (PowerApplied/PowerRemoved), SoulNexus
  (Died), Crusher/Rocket (AfterCurrentHpChanged).
- **InitializeRunState / cache sibling** — Queen finds Amalgam, FabricatorNormal
  position init.

### 4.4 On-death payloads

`MonsterModel.AfterDeath` is almost universally cosmetic. Real death-rattle
gameplay lives in **Power classes** (InfestedPower, HardToKillPower,
PoisonPower) that subscribe to the host's death event from inside the power.

**Implication**: do NOT add an "on death effect list" primitive at the monster
layer. Instead, the **Power model** needs an `OnHostDeath` hook in the
power-VM. Cleaner factoring; matches the C# layout exactly.

A few monsters subscribe to `Creature.Died` directly inside `AfterAddedToRoom`
(SlumberingBeetle, SoulNexus) — but only for VFX cleanup. The only real
gameplay death-rattle in surveyed bosses is KinPriest's `AllFollowerDeathResponse`
which fires when ALL allies die (not when itself dies), and Queen's amalgam-death
phase shift (also not self-death).

---

## 5. Cross-cutting: AmountSpec

Every numeric arg to every primitive resolves through one of these computation modes:

| `AmountSpec` variant | description | examples |
|---|---|---|
| `Fixed(i32)` | Literal | StrengthPotion(2), FirePotion(20) |
| `CanonicalVar(name)` | `DynamicVars[name].BaseValue` (data-driven) | Universal access pattern; baseline value lives in card/relic/potion/monster data table |
| `Upgraded(name)` | Same accessor, post-`OnUpgrade` data delta | Universal; recoverable from data |
| `BranchedOnUpgrade(base, upgraded)` | `if IsUpgraded { upgraded } else { base }` | TrueGrit, MultiCast |
| `XEnergy` | `ResolveEnergyXValue()` (resolves to player's current energy or X-cost) | Whirlwind, Skewer, Eradicate, MultiCast, etc. |
| `CardCountInPile(pile, predicate)` | Count of cards in pile matching predicate | FiendFire (hand count), PerfectedStrike (deck-strikes count), Anointed (pile filter) |
| `CardPlayHistoryCount(predicate, scope: Turn/Combat)` | Count of card-play history entries matching predicate | Conflagration (attacks-played-this-turn), Spite (lost-HP-this-turn) |
| `PowerAmount<P>(target)` | `target.GetPowerAmount<P>()` | Bully/BulletTime scaling |
| `HpFraction(target, num, denom)` | `target.MaxHp * pct` | FairyInABottle (30%), BloodPotion |
| `BlockAmount(target, mult)` | `target.Block * k` | Fortifier (×2) |
| `OwnerHp / OwnerLostHp / OwnerMaxHp` | direct field read | BodySlam-style cards |
| `Multiplied(base_spec, k_fn)` | `WithMultiplier(card, _ => k)` | PerfectedStrike, Protector (Osty.MaxHp×2) |
| `RandomInRange(rng_stream, lo, hi)` | RNG-keyed | SneckoOil (NextInt 0..3) |
| `LoopUntilFull(rng_stream)` | Generate-until-cap | EntropicBrew |
| `ScaledByPlayerCount(base)` | `... * combatState.Players.Count` | TestSubject moves, KnowledgeDemon heal (multiplayer-scaled) |

This `AmountSpec` enum, plus a primitive-id enum, IS the closed vocabulary for
data-driven cards/relics/potions/monster-moves. About 14 amount-spec variants;
about 50 primitive-ids across all layers (after dedup).

---

## 6. Condition vocabulary (cross-cutting)

Distinct guards inside any effect step:

- `IsUpgraded`
- `IsPoweredAttack` (relic damage modifiers only)
- `CardType == Attack | Skill | Power | Curse | Status`
- `CardTag.Contains(tag)` (Shiv, Strike, ...)
- `Card.EnergyCost.CostsX` / `HasStarCostX`
- `Resources.EnergyValue >= N`
- `Owner == self.Owner` (MP filter)
- `target.HasPower<P>()`
- `LostHpThisTurn(creature)` (Spite-family scan over history)
- `AttackKilledSomething(attackCommand)` (Feed, HandOfGreed)
- `target.AllPowersAllowDeath()` (fatal-eligibility check; Feed/HandOfGreed)
- `target.Powers.All(p => p.ShouldOwnerDeathTriggerFatal())`
- `room is MerchantRoom | RestRoom`
- `IsBeforeAct3TreasureChest(runState)`
- `combatState.HittableEnemies.Count() > 0` / `Any(...)`
- `pile.Cards.Count == n` / `> 0`
- `c.IsUpgradable`
- `History.CardPlaysFinished.Any/Count(filter)`
- `card.IsBasicStrikeOrDefend && card.IsRemovable`
- `Owner.IsOstyAlive` / `IsOstyMissing`

About 20 distinct condition predicates. The same closed set covers cards,
relics, potions, and monster intent-selection branches.

---

## 7. Per-power lifecycle hooks (deferred sub-survey)

Cards like Mayhem, SetupStrike, Stampede, Storm, Strangle, DemonForm, Barricade,
WraithForm have an almost empty OnPlay: just `PowerCmd.Apply<XPower>`. The real
behavior lives inside the Power class as turn-start / draw / damage hooks.

This means **after implementing the card OnPlay primitives, a separate but
analogous Power VM is needed** for hook bodies on `PowerModel`. Hook points
observed inside powers (this is a subset; needs a follow-up survey):

- `OnTurnStart(side)` — DemonForm (gain Strength), Doom (check HP threshold)
- `OnTurnEnd(side)` — Frail / Weak / Vulnerable duration tick, Vigor drain,
  TempStrength cleanup
- `OnCardPlayed(card)` — Mayhem (on-draw-into-hand), SetupStrike (on-attack)
- `BeforeDamageGiven / AfterDamageGiven` — Thorns reflect, PaperCuts max-HP loss
- `BeforeDamageReceived / AfterDamageReceived` — CurlUp grant block, Skittish
  flag, Shriek flag
- `ModifyDamageAdditive / Multiplicative` — Strength, Weak, Vulnerable, Frail
- `OnHostDeath` — InfestedPower (death-rattle spawn), HardToKillPower
- `OnHostSpawn` — already covered by `AfterAddedToRoom`
- `ShouldClearBlock` — Barricade, Burrowed (keep block across turns)

The current Rust core wires ~20 of these hook points across ~30 powers (see
combat.rs `fire_*_hook` functions and `tick_*_powers` callers). Closure on the
power-VM is a follow-up after the effect-VM for cards/relics/potions lands.

---

## 8. Estimated implementation closure

After cross-referencing all surveys with `crates/sts2-sim/src/combat.rs`:

**Already in Rust core (✅)**: ~15 primitives.
- DealDamage (single + multi-hit + AllEnemies), GainBlock, ApplyPower<T>,
  DrawCards, AddCardToPile, ExhaustRandomInHand, change_max_hp,
  GainEnergy, Heal, damage_creature (with hp_loss_cap for HardenedShell),
  fire_thorns / fire_after_damage_received hooks, tick_duration_debuffs,
  tick_temporary_strength_powers, monster turn dispatch.

**Partial / generalize (🟡)**: ~8 primitives.
- ScaledByPileCount, ScaledByHistoryCount, BranchedOnUpgrade, DirectDamage
  with props, UpgradeCard-on-existing-card, AddCardToPile plural form,
  ModifyHpLost, LoseBlock.

**Missing — implement once each (❌)**: ~35–45 primitives.
- Card-flow tail: MoveCard, RemoveFromDeck, AutoPlayFromDrawPile,
  Shuffle, Discard, AutoPlay, EnchantCard, TransformCard, ApplyKeyword,
  SetCardCost, PromptPlayerToSelect.
- Resource tail: LoseEnergy, GainGold, LoseGold, GainStars, GenerateRandomPotion,
  EndTurn, CompleteQuest, GainMaxPotionCount.
- Orb tail: ChannelOrb<T>, EvokeNextOrb, TriggerOrbPassive, AddOrbSlots,
  RemoveOrbSlots.
- Osty / Forge: SummonOsty, DamageFromOsty, ForgeCard.
- Relic-specific: ObtainRelic, ReplaceRelic, MeltRelic, OfferRewardCustom.
- Monster-specific: SummonMonster, KillSelf, SetMaxHp+HealFull, RemovePower<T>,
  SetMoveImmediate.
- Per-instance state: RelicState `counters: HashMap<String, i32>` (mirror
  MonsterState pattern).
- New trigger points: ~25 missing relic hooks (BeforeSideTurnStart,
  AfterRoomEntered, AfterObtained, AfterCardPlayed, ModifyMaxEnergy,
  TryModifyCardRewardOptionsLate, ShouldGainGold, etc.).
- Rare card primitives: RepeatUntilNoKills (EchoingSlash), BeforeDamage
  per-hit callback (FiendFire family), Kill (Sacrifice).

**Estimate** (each new primitive: 30 min – 2 hr including a spec-derived
test): ~30–70 hours of primitive-wiring work. After that, encoding every
card/relic/potion as a JSON effect list is a single bulk session (auto-classify
~80% via extractor pattern-matching; hand-classify the ~20% tail).

Validation bar unchanged: spec-derived tests are the floor; C# oracle-diff is
the real bar (see [[oracle-diff-is-the-real-bar]]).

---

## 9. Constraint: observer layer is a pure function of the data

The Effect VM data model is also the schema the **observer layer / RL feature
extractor** is keyed by. `crates/sts2-sim/src/features.rs::card_features`
and `relic_features` must compute their feature vectors **directly from
the item's effect-list, amount-specs, conditions, keywords, and rarity** —
never from a per-card hand-curated lookup, never by matching on card id.

Concretely, every `Effect` enum variant corresponds to one or more feature
columns. Adding a new card adds a new *data row* (its effect-list) but
no new feature column.

Implications for design:
- The agent sees cards as embeddings derived from the same primitive
  vocabulary the simulator runs on. No "card name" channel.
- A balance patch (Strike 6 → 7) changes the data, changes the embedding,
  and the agent adapts in-place — no retraining.
- A novel new card that is a composition of existing primitives produces
  a new embedding the agent has seen the building blocks of — partial
  generalization for free.
- A genuinely new mechanic (a primitive the game adds that the vocabulary
  does not yet contain) is the only case requiring a Rust core change AND
  agent retraining. That's intentional and rare.
- If a card needs special-case logic to be ported correctly, that's a
  signal that a primitive is missing from the vocabulary — extend the
  vocabulary, don't special-case the card.

This is why the vocabulary-first port matters for RL, not just for porting
efficiency. See memory [[feedback-observer-layer-pure-function]].
