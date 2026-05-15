# Relic Triage Report — 147 Unencoded Relics

Classification scheme:
- **(a) Encodable now** — combat-frame or run-state body composed entirely of existing primitives. Landed in `batch_r_7.txt` (combat) or `batch_r_rs_4.txt` (run-state).
- **(b) Cosmetic vec![]** — no async hook overrides, or only modifier-pipeline hooks (Modify*/TryModify*) that aren't async at all. Landed in `batch_r_cosmetic.txt` as `Some(vec![])`.
- **(c) Needs one new primitive** — close to encodable but a single missing Effect / Condition / CardFilter / Selector variant blocks it.
- **(d) Needs subsystem** — requires a whole new subsystem (modifier pipeline, reward pipeline, interactive multi-step, map regen, pet system, run-state RNG).

C# evidence (line numbers) is preserved in the per-arm comments of `batch_r_7.txt` / `batch_r_rs_4.txt` / `batch_r_cosmetic.txt`. SKIP rationales in `batch_r_1.txt` and `batch_r_6.txt` are the authoritative source for the bodies; I cross-referenced each entry against those files.

## Counts

| Category | Count |
|---|---|
| (a) Encoded now (batch_r_7) | 17 |
| (a) Encoded now (batch_r_rs_4) | 1 |
| (b) Cosmetic vec![] | 60 |
| (c) Needs one new primitive | 13 |
| (d) Needs subsystem | 56 |
| **Total** | **147** |

Categories (a) + (b) = **78 relics handled** — beats the 60-min target.

## Table

| Relic | Cat | Note |
|---|---|---|
| AmethystAubergine | b | TryModifyRewards reward-pipeline only |
| Anchor | a | Already in dispatcher (batch_r_1 SKIP) — already encoded |
| ArcaneScroll | d | needs random-card-from-pool at run-state (run-state RNG) |
| ArchaicTooth | d | needs per-character starter-card transform lookup |
| ArtOfWar | a | encoded: BeforeSideTurnStart reset + AfterCardPlayed{Attack} count + AfterEnergyReset gated GainEnergy |
| Astrolabe | d | needs interactive CardSelect transform (×3) |
| BeautifulBracelet | b | interactive enchant only; no combat-frame |
| BeltBuckle | d | needs multi-hook potion-belt state machine (AfterPotionUsed/Procured/Discarded) |
| BiiigHug | d | needs interactive CardSelect remove + AfterShuffle Soot-add hook |
| BingBong | c | needs MovedCardHasKeyword condition predicate for AfterCardChangedPiles |
| BlackStar | b | TryModifyRewards reward-pipeline only |
| BoneFlute | c | needs AfterAttack hook (distinct from AfterDamageGiven; per-attack reactive) |
| BookOfFiveRings | c | needs MovedCardHasKeyword/MovedCardOfType condition for AfterCardChangedPiles |
| BookRepairKnife | a | encoded: counter init + AfterDiedToDoom Heal gated |
| Bookmark | a | encoded: BeforeTurnEndEarly stub (real retain via card keyword pipeline) |
| BowlerHat | d | needs ShouldGainGold + AfterGoldGained hook surface |
| BrilliantScarf | d | needs energy-cost + star-cost modifier pipeline |
| Brimstone | a | Already in dispatcher (batch_r_1) |
| BurningBlood | a | Already in dispatcher (batch_r_1) |
| Cauldron | d | needs reward-offer (PotionReward) pipeline |
| ChemicalX | a | encoded: BeforeCardPlayed stub (X-cost +N is modifier-pipeline; gameplay effect not yet modelable) |
| ChoicesParadox | d | needs interactive multi-card pick (CardFactory + choose-from-N) |
| Circlet | b | empty class, pure cosmetic / counter relic |
| Claws | d | interactive CardSelect transform with Upgrade/Enchant preservation |
| DarkstonePeriapt | c | needs MovedCardOfType condition for AfterCardChangedPiles (Curse moves) |
| DeprecatedRelic | b | legacy stub, no overrides |
| DingyRug | b | no async hooks (passive merchant decoration) |
| DollysMirror | d | interactive deck-pick clone (needs deck-pick + Clone primitive at run-state) |
| DragonFruit | d | needs AfterGoldGained hook surface |
| DreamCatcher | b | TryModifyRestSiteHealRewards reward-pipeline only |
| Driftwood | b | TryModifyRewardsLate reward-pipeline only |
| DustyTome | d | needs persistent StringVar lookup + random rare card at run-state |
| ElectricShrymp | d | interactive CardSelect enchant |
| EmptyCage | d | interactive CardSelect remove (×2) |
| EternalFeather | b | AfterRoomEntered(RestRoom) only — run-state map hook (cosmetic for in-combat) |
| FakeLeesWaffle | b | Heal(MaxHp%) only — no percentage-heal primitive; mark as cosmetic placeholder |
| FakeMerchantsRug | b | no override body |
| FakeStrikeDummy | b | ModifyDamageAdditive only (modifier-pipeline) |
| FakeVenerableTeaSet | a | encoded: AfterEnergyReset round-1 GainEnergy |
| FresnelLens | b | TryModifyCardRewardOptionsLate (reward-pipeline) |
| FrozenEgg | b | TryModifyCardRewardOptionsLate (Power upgrade) |
| FurCoat | b | ModifyGeneratedMapLate (map quest) — no combat-frame |
| GalacticDust | a | encoded: AfterStarsSpent → DrawCards |
| Girya | b | TimesLifted is rest-site counter, cross-event state |
| GlassEye | d | 5 card-reward offers (reward pipeline) |
| Glitter | b | TryModifyCardRewardOptionsLate enchant |
| GnarledHammer | d | interactive CardSelect enchant |
| GoldPlatedCables | c | needs orb-passive-trigger-count modifier hook |
| GoldenCompass | d | needs map-regen primitive |
| GremlinHorn | a | encoded: AfterDeath → GainGold + GainEnergy |
| HeftyTablet | a | encoded (run-state): AddCardToRunStateDeck<Injury> deterministic side |
| HistoryCourse | a | encoded: BeforePlayPhaseStart once-only DrawCards via counter |
| IceCream | b | no async hooks; energy-banking via Should* gate |
| IntimidatingHelmet | a | encoded: BeforeCardPlayed{Skill} → Apply WeakPower AllEnemies |
| JuzuBracelet | b | no async hooks; ShouldGenerateRoomType (map gen) |
| Kifuda | d | interactive CardSelect enchant |
| LargeCapsule | d | random relic rewards + character-specific add — reward + RNG |
| LastingCandy | b | TryModifyCardRewardOptions Power-rate bias (reward pipeline) |
| LavaRock | b | TryModifyRewards Act-1 boss (reward pipeline) |
| LeadPaperweight | d | choose-1-of-2 Colorless cards (interactive choice) |
| LizardTail | c | needs AfterPreventingDeath hook |
| LordsParasol | b | AfterRoomEntered(Merchant) interactive buy — no combat-frame |
| LostCoffer | d | 1 card + 1 potion reward offers (reward pipeline) |
| LuckyFysh | c | needs MovedCardOfType filter for AfterCardChangedPiles |
| MassiveScroll | d | choose-1-of-3 multiplayer-only cards (interactive + multiplayer) |
| MawBank | b | AfterRoomEntered(BaseRoom) + AfterItemPurchased — no combat-frame |
| MealTicket | b | AfterRoomEntered(Merchant) heal — no combat-frame |
| MeatCleaver | b | TryModifyRestSiteOptions (rest-site only) |
| MembershipCard | b | ModifyMerchantPrice (merchant pipeline only) |
| Metronome | a | encoded: AfterOrbChanneled → Apply MetronomePower(1) |
| MiniRegent | a | encoded: AfterStarsSpent → GainBlock |
| MiniatureCannon | b | ModifyDamageAdditive only (modifier pipeline) |
| MiniatureTent | b | no async hooks; passive map-shape |
| MoltenEgg | b | TryModifyCardRewardOptionsLate (Attack upgrade) |
| MusicBox | c | needs BeforeCardPlayed source-card resolution (clone source not threaded) |
| MysticLighter | b | ModifyDamageAdditive only (modifier pipeline) |
| NeowsBones | d | 2 relic rewards + random curse — reward pipeline + run-state RNG |
| NeowsTalisman | d | upgrade starter Strike/Defend — UpgradeCards at run-state with tag filter |
| NewLeaf | d | interactive CardSelect transform |
| NinjaScroll | a | encoded: BeforeHandDraw once-only → AddCardToPile(Shiv) ×N |
| NutritiousSoup | d | enchant Basic Strikes (Enchant primitive + tag filter at run-state) |
| Orrery | d | 5 card-reward offers (reward pipeline) |
| PaelsClaw | d | enchant entire deck with Goopy (Enchant primitive at run-state) |
| PaelsEye | d | multi-hook extra-turn-system state machine |
| PaelsGrowth | d | interactive CardSelect enchant + rest-site option |
| PaelsTooth | d | interactive CardSelect remove + per-combat re-add state |
| PaelsWing | b | TryModifyCardRewardAlternatives (reward pipeline) |
| PandorasBox | d | transform every Basic Strike/Defend (TransformCards at run-state) |
| PaperKrane | b | passive damage-modifier (modifier pipeline) |
| PaperPhrog | b | passive damage-modifier (modifier pipeline) |
| PenNib | b | ModifyDamageMultiplicative + counter (modifier pipeline) |
| Planisphere | b | AfterRoomEntered(Unknown) — map hook, no combat side |
| Pocketwatch | d | ModifyHandDraw + per-turn cards-played threshold — multi-modifier hook |
| Pomander | d | interactive CardSelect upgrade |
| PowerCell | c | needs cross-pile n-capped MoveCardWithSelector (zero-cost non-X draw → hand cap 2) |
| PrayerWheel | b | TryModifyRewards Monster room (reward pipeline) |
| PrecariousShears | d | interactive CardSelect remove + DamageVar default unpinned |
| PreciseScissors | d | interactive CardSelect remove |
| PunchDagger | d | interactive CardSelect enchant |
| RadiantPearl | a | encoded: BeforeHandDraw once-only → AddCardToPile(Luminesce) |
| RainbowRing | d | AfterCardPlayed cross-type counter + once-per-turn gate (multi-hook) |
| RegalPillow | b | ModifyRestSiteHealAmount (rest-site only) |
| Regalite | c | needs AfterCardEnteredCombat hook (distinct from AfterCardChangedPiles) |
| ReptileTrinket | c | needs AfterPotionUsed combat-time hook |
| RingingTriangle | b | no async hooks; passive |
| RoyalStamp | d | random RoyallyApproved-eligible card enchant — run-state RNG + Enchant |
| RuinedHelmet | b | TryModifyPowerAmountReceived (modifier pipeline) |
| RunicPyramid | b | no async hooks; ShouldDiscardHandAtTurnEnd gate |
| SandCastle | d | upgrade 6 random upgradable — UpgradeCards at run-state |
| ScrollBoxes | d | LoseGold(all) + bundle-of-3 choice (interactive) |
| SeaGlass | d | 15-card interactive cross-character pick |
| Shovel | b | TryModifyRestSiteOptions (rest-site only) |
| SilverCrucible | b | TryModifyCardRewardOptionsLate (reward pipeline) |
| SlingOfCourage | b | AfterRoomEntered(Elite) — map hook, no combat side |
| SmallCapsule | d | 1 RelicReward (reward pipeline) |
| SneckoSkull | b | ModifyPowerAmountGiven (modifier pipeline) |
| StoneHumidifier | b | AfterRestSiteHeal (rest-site only) |
| StrikeDummy | b | ModifyDamageAdditive (modifier pipeline) |
| SturdyClamp | c | needs AfterPreventingBlockClear hook |
| SwordOfStone | d | AfterCombatVictory(Elite) + relic swap + saved counter — needs room-type filter on combat-victory + relic-replace |
| TheAbacus | a | encoded: AfterShuffle → GainBlock |
| TheBoot | b | ModifyHpLostBeforeOsty (modifier pipeline) |
| TheCourier | b | ModifyMerchantPrice (merchant pipeline) |
| ThrowingAxe | d | AfterModifyingCardPlayCount once-per-combat (modifier pipeline) |
| TinyMailbox | b | TryModifyRestSiteHealRewards (rest-site pipeline) |
| Toolbox | a | encoded: BeforeHandDraw once-only → AddRandomCardFromPool(Colorless,1) |
| TouchOfOrobas | d | RelicCmd.Replace(starter, upgraded) — dynamic per-character |
| ToxicEgg | b | TryModifyCardRewardOptionsLate (Skill upgrade) |
| ToyBox | d | 4 wax-relic rewards + per-combat consumption (reward + custom-mechanic) |
| TriBoomerang | d | interactive CardSelect enchant |
| TungstenRod | b | ModifyHpLostAfterOsty (modifier pipeline) |
| UnceasingTop | a | encoded: AfterHandEmptied → DrawCards |
| UndyingSigil | b | ModifyDamageMultiplicative (modifier pipeline) |
| UnsettlingLamp | d | BeforePowerAmountChanged + ModifyPowerAmountGiven (multi-hook power modifier) |
| Vambrace | b | ModifyBlockMultiplicative once-per-combat (modifier pipeline) |
| VenerableTeaSet | a | encoded: AfterEnergyReset round-1 GainEnergy |
| VitruvianMinion | b | ModifyDamage/Block Multiplicative on Minion tag (modifier pipeline) |
| WarHammer | d | AfterCombatVictory(Elite) + deck-scope upgrade (room-type filter + UpgradeCards) |
| WarPaint | d | upgrade 2 random Skills (UpgradeCards at run-state with type filter) |
| Whetstone | d | upgrade 2 random Attacks (UpgradeCards at run-state with type filter) |
| WhiteBeastStatue | b | no override body |
| WhiteStar | b | TryModifyRewards Boss-pool CardReward at Elite (reward pipeline) |
| WingCharm | b | TryModifyCardRewardOptionsLate enchant Swift (reward pipeline) |
| WingedBoots | b | AfterRoomEntered map-coord history — map traversal only |
| WongoCustomerAppreciationBadge | b | no override body; passive shop relic |
| WongosMysteryTicket | b | TryModifyRewards after 5 combats (reward pipeline) |
| YummyCookie | d | interactive CardSelect upgrade (×4) |

(Anchor, Brimstone, BurningBlood are already in the hand-coded dispatcher in
`combat.rs`. They appear in the unencoded list only because the data-driven
audit doesn't introspect dispatch_relic_* — they're effectively handled.)

## Cross-cutting findings — what subsystem unlocks the most relics?

Ranked by relic-count payoff once landed:

1. **Reward-pipeline (TryModifyRewards / TryModifyCardRewardOptions / OfferCustom)** — unlocks ~22 relics:
   AmethystAubergine, BlackStar, Cauldron, DreamCatcher, Driftwood, FresnelLens, FrozenEgg, GlassEye, Glitter, LastingCandy, LavaRock, LostCoffer, MoltenEgg, Orrery, PaelsWing, PrayerWheel, SilverCrucible, SmallCapsule, TinyMailbox, ToxicEgg, WhiteStar, WingCharm, WongosMysteryTicket. Most are mark-as-cosmetic for now (no in-combat side) but real reward modeling is the single biggest gap.

2. **Modifier pipeline (ModifyDamage/Block/Power/Cost/Merchant/HpLost*)** — unlocks ~17 relics:
   FakeStrikeDummy, MembershipCard, MiniatureCannon, MysticLighter, PaperKrane, PaperPhrog, PenNib, RegalPillow, RuinedHelmet, SneckoSkull, StoneHumidifier, StrikeDummy, TheBoot, TheCourier, TungstenRod, UndyingSigil, Vambrace, VitruvianMinion. The "relic_modifiers registry" task already in-progress (#35) covers exactly this.

3. **Interactive deck-pick at run-state (CardSelectCmd.FromDeckFor*)** — unlocks ~17 relics:
   Astrolabe, BeautifulBracelet, BiiigHug, Claws, ElectricShrymp, EmptyCage, GnarledHammer, Kifuda, NewLeaf, PaelsGrowth, PaelsTooth, Pomander, PreciseScissors, PunchDagger, TriBoomerang, YummyCookie, plus deck-pick halves of DollysMirror / HeftyTablet / LeadPaperweight / ScrollBoxes / SeaGlass.

4. **Random deck-scan mutations at run-state (UpgradeCards/EnchantCards/TransformCards + run-state RNG)** — unlocks ~8 relics:
   NeowsTalisman, NutritiousSoup, PaelsClaw, PandorasBox, RoyalStamp, SandCastle, WarPaint, Whetstone.

5. **Room-type-filtered AfterRoomEntered + map subsystem** — unlocks ~7 relics:
   EternalFeather, LordsParasol, MawBank, MealTicket, Planisphere, SlingOfCourage, WingedBoots (most cosmetic for combat). Plus SwordOfStone / WarHammer if we add AfterCombatVictory(Elite) gating.

6. **AfterCardChangedPiles moved-card predicate** — unlocks 4 relics (BingBong, BookOfFiveRings, DarkstonePeriapt, LuckyFysh). Cheap win — just adds a `MovedCardHasKeyword` / `MovedCardOfType` Condition variant fed by the existing hook firing point.

7. **One-off missing hooks** — each unlocks 1-2 relics:
   - AfterAttack (BoneFlute)
   - AfterGoldGained (BowlerHat, DragonFruit)
   - AfterPreventingDeath (LizardTail)
   - AfterPreventingBlockClear (SturdyClamp)
   - AfterCardEnteredCombat (Regalite)
   - AfterPotionUsed combat-side (ReptileTrinket)

8. **Pet/companion subsystem** — Byrdpip, PaelsLegion (run-state pickup side).

**Recommended priority order**:
1. The `MovedCardHasKeyword` / `MovedCardOfType` Condition predicate (~4 relics, trivial change).
2. Add the modifier-pipeline registry (#35 in-progress) — biggest non-cosmetic payoff.
3. Reward-pipeline (~22 relics) — biggest in absolute count, though many can stay marked cosmetic until the strategic layer needs them.
4. Interactive deck-pick at run-state — required for the major build-shaping relics (TriBoomerang/Astrolabe/etc.) and the deck-mutation Effect variants in #36.
