// Auto-generated from cards.json by tools/merge_card_ports/autogen.py
// Per-card encodings -- conservative shape match only.
// Skipped cards fall through to the match-arm dispatch path or
// are not yet ported. See `// SKIP` comments for reasons.

        "Abrasive" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Canonical("ThornsPower".to_string()), target: Target::SelfPlayer }]),
        "Accelerant" => Some(vec![Effect::ApplyPower { power_id: "AccelerantPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Accuracy" => Some(vec![Effect::ApplyPower { power_id: "AccuracyPower".to_string(), amount: AmountSpec::Canonical("AccuracyPower".to_string()), target: Target::SelfPlayer }]),
        "Afterimage" => Some(vec![Effect::ApplyPower { power_id: "AfterimagePower".to_string(), amount: AmountSpec::Canonical("AfterimagePower".to_string()), target: Target::SelfPlayer }]),
        "Aggression" => Some(vec![Effect::ApplyPower { power_id: "AggressionPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Alignment" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Armaments" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Arsenal" => Some(vec![Effect::ApplyPower { power_id: "ArsenalPower".to_string(), amount: AmountSpec::Canonical("ArsenalPower".to_string()), target: Target::SelfPlayer }]),
        "AscendersBane" => Some(vec![]),
        "Assassinate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "Automation" => Some(vec![Effect::ApplyPower { power_id: "AutomationPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Backflip" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Backstab" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BallLightning" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BansheesCry" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Barrage" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BattleTrance" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "BeaconOfHope" => Some(vec![Effect::ApplyPower { power_id: "BeaconOfHopePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "BeatIntoShape" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Beckon" => Some(vec![]),
        "BelieveInYou" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "BiasedCognition" => Some(vec![Effect::ApplyPower { power_id: "BiasedCognitionPower".to_string(), amount: AmountSpec::Canonical("BiasedCognitionPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),
        "BladeOfInk" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Bludgeon" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BootSequence" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "BorrowedTime" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Break" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "BubbleBubble" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::ChosenEnemy }]),
        "Buffer" => Some(vec![Effect::ApplyPower { power_id: "BufferPower".to_string(), amount: AmountSpec::Canonical("BufferPower".to_string()), target: Target::SelfPlayer }]),
        "BulkUp" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Burn" => Some(vec![]),
        "BurningPact" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Bury" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ByrdSwoop" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Calamity" => Some(vec![Effect::ApplyPower { power_id: "CalamityPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "CallOfTheVoid" => Some(vec![Effect::ApplyPower { power_id: "CallOfTheVoidPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Caltrops" => Some(vec![Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Canonical("ThornsPower".to_string()), target: Target::SelfPlayer }]),
        "Capacitor" => Some(vec![Effect::ApplyPower { power_id: "CapacitorPower".to_string(), amount: AmountSpec::Canonical("Repeat".to_string()), target: Target::SelfPlayer }]),
        "Catastrophe" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "ChildOfTheStars" => Some(vec![Effect::ApplyPower { power_id: "ChildOfTheStarsPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Clash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Clumsy" => Some(vec![]),
        "ColdSnap" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Comet" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Compact" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "ConsumingShadow" => Some(vec![Effect::ApplyPower { power_id: "ConsumingShadowPower".to_string(), amount: AmountSpec::Canonical("ConsumingShadowPower".to_string()), target: Target::SelfPlayer }]),
        "Coolant" => Some(vec![Effect::ApplyPower { power_id: "CoolantPower".to_string(), amount: AmountSpec::Canonical("CoolantPower".to_string()), target: Target::SelfPlayer }]),
        "Coolheaded" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Corruption" => Some(vec![Effect::ApplyPower { power_id: "CorruptionPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Countdown" => Some(vec![Effect::ApplyPower { power_id: "CountdownPower".to_string(), amount: AmountSpec::Canonical("CountdownPower".to_string()), target: Target::SelfPlayer }]),
        "CreativeAi" => Some(vec![Effect::ApplyPower { power_id: "CreativeAiPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Cruelty" => Some(vec![Effect::ApplyPower { power_id: "CrueltyPower".to_string(), amount: AmountSpec::Canonical("CrueltyPower".to_string()), target: Target::SelfPlayer }]),
        "CrushUnder" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "CurseOfTheBell" => Some(vec![]),
        "DaggerThrow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DanseMacabre" => Some(vec![Effect::ApplyPower { power_id: "DanseMacabrePower".to_string(), amount: AmountSpec::Canonical("DanseMacabrePower".to_string()), target: Target::SelfPlayer }]),
        "DarkEmbrace" => Some(vec![Effect::ApplyPower { power_id: "DarkEmbracePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Dazed" => Some(vec![]),
        "DeadlyPoison" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::ChosenEnemy }]),
        "Deathbringer" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::AllEnemies }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::AllEnemies }]),
        "Debilitate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Debris" => Some(vec![]),
        "Debt" => Some(vec![]),
        "Decay" => Some(vec![]),
        "Deflect" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Defragment" => Some(vec![Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),
        "Demesne" => Some(vec![Effect::ApplyPower { power_id: "DemesnePower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "DeprecatedCard" => Some(vec![]),
        "Devastate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DevourLife" => Some(vec![Effect::ApplyPower { power_id: "DevourLifePower".to_string(), amount: AmountSpec::Canonical("DevourLifePower".to_string()), target: Target::SelfPlayer }]),
        "Disintegration" => Some(vec![]),
        "Dismantle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Doubt" => Some(vec![]),
        "DrainPower" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DramaticEntrance" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "DrumOfBattle" => Some(vec![Effect::ApplyPower { power_id: "DrumOfBattlePower".to_string(), amount: AmountSpec::Canonical("DrumOfBattlePower".to_string()), target: Target::SelfPlayer }]),
        "EchoForm" => Some(vec![Effect::ApplyPower { power_id: "EchoFormPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "EchoingSlash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Enthralled" => Some(vec![]),
        "Entropy" => Some(vec![Effect::ApplyPower { power_id: "EntropyPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Envenom" => Some(vec![Effect::ApplyPower { power_id: "EnvenomPower".to_string(), amount: AmountSpec::Canonical("EnvenomPower".to_string()), target: Target::SelfPlayer }]),
        "EternalArmor" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "Expertise" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Exterminate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "FallingStar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "FanOfKnives" => Some(vec![Effect::ApplyPower { power_id: "FanOfKnivesPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Fasten" => Some(vec![Effect::ApplyPower { power_id: "FastenPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Fear" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "FeelNoPain" => Some(vec![Effect::ApplyPower { power_id: "FeelNoPainPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Feral" => Some(vec![Effect::ApplyPower { power_id: "FeralPower".to_string(), amount: AmountSpec::Canonical("FeralPower".to_string()), target: Target::SelfPlayer }]),
        "Finesse" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "FlashOfSteel" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "FlickFlack" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "FocusedStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "FollowThrough" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Folly" => Some(vec![]),
        "Footwork" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }]),
        "ForbiddenGrimoire" => Some(vec![Effect::ApplyPower { power_id: "ForbiddenGrimoirePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "ForegoneConclusion" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "FranticEscape" => Some(vec![]),
        "Friendship" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Furnace" => Some(vec![Effect::ApplyPower { power_id: "FurnacePower".to_string(), amount: AmountSpec::Canonical("Forge".to_string()), target: Target::SelfPlayer }]),
        "Genesis" => Some(vec![Effect::ApplyPower { power_id: "GenesisPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "GiantRock" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Glacier" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Glasswork" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Glitterstream" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "GoForTheEyes" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "GrandFinale" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Graveblast" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Greed" => Some(vec![]),
        "Guilty" => Some(vec![]),
        "GunkUp" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Hailstorm" => Some(vec![Effect::ApplyPower { power_id: "HailstormPower".to_string(), amount: AmountSpec::Canonical("HailstormPower".to_string()), target: Target::SelfPlayer }]),
        "HammerTime" => Some(vec![Effect::ApplyPower { power_id: "HammerTimePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "HandOfGreed" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HandTrick" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Haunt" => Some(vec![Effect::ApplyPower { power_id: "HauntPower".to_string(), amount: AmountSpec::Canonical("HpLoss".to_string()), target: Target::SelfPlayer }]),
        "Haze" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::AllEnemies }]),
        "Hegemony" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HeirloomHammer" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HelloWorld" => Some(vec![Effect::ApplyPower { power_id: "HelloWorldPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Hellraiser" => Some(vec![Effect::ApplyPower { power_id: "HellraiserPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Hemokinesis" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HowlFromBeyond" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "IAmInvincible" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Impatience" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Impervious" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Infection" => Some(vec![]),
        "Inferno" => Some(vec![Effect::ApplyPower { power_id: "InfernoPower".to_string(), amount: AmountSpec::Canonical("InfernoPower".to_string()), target: Target::SelfPlayer }]),
        "InfiniteBlades" => Some(vec![Effect::ApplyPower { power_id: "InfiniteBladesPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Injury" => Some(vec![]),
        "Intercept" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Iteration" => Some(vec![Effect::ApplyPower { power_id: "IterationPower".to_string(), amount: AmountSpec::Canonical("IterationPower".to_string()), target: Target::SelfPlayer }]),
        "Juggernaut" => Some(vec![Effect::ApplyPower { power_id: "JuggernautPower".to_string(), amount: AmountSpec::Canonical("JuggernautPower".to_string()), target: Target::SelfPlayer }]),
        "Juggling" => Some(vec![Effect::ApplyPower { power_id: "JugglingPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "KinglyKick" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KinglyPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Knockdown" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KnockoutBlow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KnowThyPlace" => Some(vec![Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Leap" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Lethality" => Some(vec![Effect::ApplyPower { power_id: "LethalityPower".to_string(), amount: AmountSpec::Canonical("LethalityPower".to_string()), target: Target::SelfPlayer }]),
        "Lift" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "LightningRod" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "LightningRodPower".to_string(), amount: AmountSpec::Canonical("LightningRodPower".to_string()), target: Target::SelfPlayer }]),
        "Loop" => Some(vec![Effect::ApplyPower { power_id: "LoopPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Luminesce" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "LunarBlast" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MachineLearning" => Some(vec![Effect::ApplyPower { power_id: "MachineLearningPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "MadScience" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "MakeItSo" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ManifestAuthority" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "MasterOfStrategy" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "MasterPlanner" => Some(vec![Effect::ApplyPower { power_id: "MasterPlannerPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Mayhem" => Some(vec![Effect::ApplyPower { power_id: "MayhemPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Metamorphosis" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "MeteorShower" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::AllEnemies }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::AllEnemies }]),
        "MeteorStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MindRot" => Some(vec![]),
        "MinionDiveBomb" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MinionSacrifice" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "MinionStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MomentumStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MonarchsGaze" => Some(vec![Effect::ApplyPower { power_id: "MonarchsGazePower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "NecroMastery" => Some(vec![Effect::ApplyPower { power_id: "NecroMasteryPower".to_string(), amount: AmountSpec::Canonical("Summon".to_string()), target: Target::SelfPlayer }]),
        "NegativePulse" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::AllEnemies }]),
        "NeowsFury" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "NeutronAegis" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "Normality" => Some(vec![]),
        "Nostalgia" => Some(vec![Effect::ApplyPower { power_id: "NostalgiaPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "NoxiousFumes" => Some(vec![Effect::ApplyPower { power_id: "NoxiousFumesPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Null" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Oblivion" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::ChosenEnemy }]),
        "Orbit" => Some(vec![Effect::ApplyPower { power_id: "OrbitPower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Outbreak" => Some(vec![Effect::ApplyPower { power_id: "OutbreakPower".to_string(), amount: AmountSpec::Canonical("OutbreakPower".to_string()), target: Target::SelfPlayer }]),
        "Outmaneuver" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "PactsEnd" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Pagestorm" => Some(vec![Effect::ApplyPower { power_id: "PagestormPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "PaleBlueDot" => Some(vec![Effect::ApplyPower { power_id: "PaleBlueDotPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Panache" => Some(vec![Effect::ApplyPower { power_id: "PanachePower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Parry" => Some(vec![Effect::ApplyPower { power_id: "ParryPower".to_string(), amount: AmountSpec::Canonical("ParryPower".to_string()), target: Target::SelfPlayer }]),
        "Parse" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "ParticleWall" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Peck" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PhantomBlades" => Some(vec![Effect::ApplyPower { power_id: "PhantomBladesPower".to_string(), amount: AmountSpec::Canonical("PhantomBladesPower".to_string()), target: Target::SelfPlayer }]),
        "PillarOfCreation" => Some(vec![Effect::ApplyPower { power_id: "PillarOfCreationPower".to_string(), amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Pinpoint" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PoorSleep" => Some(vec![]),
        "Pounce" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Predator" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PrepTime" => Some(vec![Effect::ApplyPower { power_id: "PrepTimePower".to_string(), amount: AmountSpec::Canonical("PrepTimePower".to_string()), target: Target::SelfPlayer }]),
        "Prepared" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Production" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Prophesize" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Protector" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Prowess" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Purity" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Pyre" => Some(vec![Effect::ApplyPower { power_id: "PyrePower".to_string(), amount: AmountSpec::Canonical("Energy".to_string()), target: Target::SelfPlayer }]),
        "Reap" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ReaperForm" => Some(vec![Effect::ApplyPower { power_id: "ReaperFormPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Reave" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Rebound" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Reflect" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Reflex" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Regret" => Some(vec![]),
        "RipAndTear" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "RocketPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "RollingBoulder" => Some(vec![Effect::ApplyPower { power_id: "RollingBoulderPower".to_string(), amount: AmountSpec::Canonical("RollingBoulderPower".to_string()), target: Target::SelfPlayer }]),
        "Royalties" => Some(vec![Effect::ApplyPower { power_id: "RoyaltiesPower".to_string(), amount: AmountSpec::Canonical("Gold".to_string()), target: Target::SelfPlayer }]),
        "Rupture" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Scrape" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SculptingStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Seance" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "SecondWind" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "SeekerStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SeekingEdge" => Some(vec![Effect::ApplyPower { power_id: "SeekingEdgePower".to_string(), amount: AmountSpec::Canonical("Forge".to_string()), target: Target::SelfPlayer }]),
        "SentryMode" => Some(vec![Effect::ApplyPower { power_id: "SentryModePower".to_string(), amount: AmountSpec::Canonical("SentryModePower".to_string()), target: Target::SelfPlayer }]),
        "SerpentForm" => Some(vec![Effect::ApplyPower { power_id: "SerpentFormPower".to_string(), amount: AmountSpec::Canonical("SerpentFormPower".to_string()), target: Target::SelfPlayer }]),
        "SevenStars" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "ShadowShield" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "ShadowStep" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Shame" => Some(vec![]),
        "Shatter" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "ShiningStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Shiv" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Shroud" => Some(vec![Effect::ApplyPower { power_id: "ShroudPower".to_string(), amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "ShrugItOff" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Skim" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "SleightOfFlesh" => Some(vec![Effect::ApplyPower { power_id: "SleightOfFleshPower".to_string(), amount: AmountSpec::Canonical("SleightOfFleshPower".to_string()), target: Target::SelfPlayer }]),
        "Slice" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Slimed" => Some(vec![]),
        "Sloth" => Some(vec![]),
        "Smokestack" => Some(vec![Effect::ApplyPower { power_id: "SmokestackPower".to_string(), amount: AmountSpec::Canonical("SmokestackPower".to_string()), target: Target::SelfPlayer }]),
        "Sneaky" => Some(vec![Effect::ApplyPower { power_id: "SneakyPower".to_string(), amount: AmountSpec::Canonical("SneakyPower".to_string()), target: Target::SelfPlayer }]),
        "SolarStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Soot" => Some(vec![]),
        "Soul" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "SoulStorm" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SovereignBlade" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Sow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "SpectrumShift" => Some(vec![Effect::ApplyPower { power_id: "SpectrumShiftPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Speedster" => Some(vec![Effect::ApplyPower { power_id: "SpeedsterPower".to_string(), amount: AmountSpec::Canonical("SpeedsterPower".to_string()), target: Target::SelfPlayer }]),
        "Spinner" => Some(vec![Effect::ApplyPower { power_id: "SpinnerPower".to_string(), amount: AmountSpec::Canonical("SpinnerPower".to_string()), target: Target::SelfPlayer }]),
        "SpiritOfAsh" => Some(vec![Effect::ApplyPower { power_id: "SpiritOfAshPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "SporeMind" => Some(vec![]),
        "Squash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "Squeeze" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Stampede" => Some(vec![Effect::ApplyPower { power_id: "StampedePower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Stardust" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "Stomp" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "StoneArmor" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "Storm" => Some(vec![Effect::ApplyPower { power_id: "StormPower".to_string(), amount: AmountSpec::Canonical("StormPower".to_string()), target: Target::SelfPlayer }]),
        "Strangle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Stratagem" => Some(vec![Effect::ApplyPower { power_id: "StratagemPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Subroutine" => Some(vec![Effect::ApplyPower { power_id: "SubroutinePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "SuckerPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Supercritical" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Supermassive" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Suppress" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "SweepingBeam" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "SwordSage" => Some(vec![Effect::ApplyPower { power_id: "SwordSagePower".to_string(), amount: AmountSpec::Canonical("SwordSagePower".to_string()), target: Target::SelfPlayer }]),
        "Synthesis" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Tactician" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "TagTeam" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Tank" => Some(vec![Effect::ApplyPower { power_id: "TankPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Taunt" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "TearAsunder" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TeslaCoil" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TheGambit" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "TheHunt" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TheScythe" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TheSealedThrone" => Some(vec![Effect::ApplyPower { power_id: "TheSealedThronePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "ThinkingAhead" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Thrash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ThrummingHatchet" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Thunder" => Some(vec![Effect::ApplyPower { power_id: "ThunderPower".to_string(), amount: AmountSpec::Canonical("ThunderPower".to_string()), target: Target::SelfPlayer }]),
        "TimesUp" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ToolsOfTheTrade" => Some(vec![Effect::ApplyPower { power_id: "ToolsOfTheTradePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Toxic" => Some(vec![]),
        "Tracking" => Some(vec![Effect::ApplyPower { power_id: "TrackingPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Transfigure" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "TrashToTreasure" => Some(vec![Effect::ApplyPower { power_id: "TrashToTreasurePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Tyranny" => Some(vec![Effect::ApplyPower { power_id: "TyrannyPower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "UltimateDefend" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "UltimateStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Unleash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("CalculatedDamage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Unmovable" => Some(vec![Effect::ApplyPower { power_id: "UnmovablePower".to_string(), amount: AmountSpec::Fixed(1), target: Target::SelfPlayer }]),
        "Untouchable" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "UpMySleeve" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Veilpiercer" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Vicious" => Some(vec![Effect::ApplyPower { power_id: "ViciousPower".to_string(), amount: AmountSpec::Canonical("Cards".to_string()), target: Target::SelfPlayer }]),
        "Void" => Some(vec![]),
        "VoidForm" => Some(vec![Effect::ApplyPower { power_id: "VoidFormPower".to_string(), amount: AmountSpec::Canonical("VoidFormPower".to_string()), target: Target::SelfPlayer }]),
        "Volley" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "WasteAway" => Some(vec![]),
        "WellLaidPlans" => Some(vec![Effect::ApplyPower { power_id: "WellLaidPlansPower".to_string(), amount: AmountSpec::Canonical("Dynamic".to_string()), target: Target::SelfPlayer }]),
        "Whistle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Wisp" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Wound" => Some(vec![]),
        "WraithForm" => Some(vec![Effect::ApplyPower { power_id: "IntangiblePower".to_string(), amount: AmountSpec::Canonical("IntangiblePower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "WraithFormPower".to_string(), amount: AmountSpec::Canonical("WraithFormPower".to_string()), target: Target::SelfPlayer }]),
        "Writhe" => Some(vec![]),
        "WroughtInWar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),

        // ===== Skipped (need shape-specific handling) =====
        // SKIP Acrobatics: has richer match-arm in combat.rs; let it run
        // SKIP AdaptiveStrike: has richer match-arm in combat.rs; let it run
        // SKIP Adrenaline: Skill/Self shape with vars={'Energy', 'Cards'} powers=set() not recognized
        // SKIP Afterlife: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Alchemize: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP AllForOne: has richer match-arm in combat.rs; let it run
        // SKIP Anger: has richer match-arm in combat.rs; let it run
        // SKIP Anointed: has richer match-arm in combat.rs; let it run
        // SKIP Anticipate: Skill/Self shape with vars={'Power'} powers={'DexterityPower'} not recognized
        // SKIP Apotheosis: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Apparition: has richer match-arm in combat.rs; let it run
        // SKIP AshenStrike: has richer match-arm in combat.rs; let it run
        // SKIP BadLuck: has richer match-arm in combat.rs; let it run
        // SKIP Barricade: has richer match-arm in combat.rs; let it run
        // SKIP BeatDown: has richer match-arm in combat.rs; let it run
        // SKIP Begone: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP BigBang: Skill/Self shape with vars={'Energy', 'Cards', 'Stars', 'Forge'} powers=set() not recognized
        // SKIP BlackHole: has richer match-arm in combat.rs; let it run
        // SKIP BladeDance: has richer match-arm in combat.rs; let it run
        // SKIP BlightStrike: has richer match-arm in combat.rs; let it run
        // SKIP BloodWall: Skill/Self shape with vars={'HpLoss', 'Block'} powers=set() not recognized
        // SKIP Blur: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP BodySlam: has richer match-arm in combat.rs; let it run
        // SKIP Bodyguard: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Bolas: has richer match-arm in combat.rs; let it run
        // SKIP Bombardment: has richer match-arm in combat.rs; let it run
        // SKIP BoneShards: AOE attack without Damage var
        // SKIP BouncingFlask: unknown shape: type=Skill target=RandomEnemy vars={'Power', 'Repeat'} powers={'PoisonPower'}
        // SKIP Brand: Skill/Self shape with vars={'HpLoss', 'Power'} powers={'StrengthPower'} not recognized
        // SKIP Breakthrough: has richer match-arm in combat.rs; let it run
        // SKIP BrightestFlame: Skill/Self shape with vars={'MaxHp', 'Energy', 'Cards'} powers=set() not recognized
        // SKIP BulletTime: has richer match-arm in combat.rs; let it run
        // SKIP Bully: has richer match-arm in combat.rs; let it run
        // SKIP Bulwark: Skill/Self shape with vars={'Block', 'Forge'} powers=set() not recognized
        // SKIP BundleOfJoy: has richer match-arm in combat.rs; let it run
        // SKIP Burst: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP ByrdonisEgg: unknown shape: type=Quest target=None vars=set() powers=set()
        // SKIP Calcify: has richer match-arm in combat.rs; let it run
        // SKIP CalculatedGamble: has richer match-arm in combat.rs; let it run
        // SKIP CaptureSpirit: Skill/AnyEnemy shape with vars={'Cards', 'Damage'} powers=set() not recognized
        // SKIP Cascade: has richer match-arm in combat.rs; let it run
        // SKIP CelestialMight: has richer match-arm in combat.rs; let it run
        // SKIP Chaos: Skill/Self shape with vars={'Repeat'} powers=set() not recognized
        // SKIP Charge: has richer match-arm in combat.rs; let it run
        // SKIP ChargeBattery: Skill/Self shape with vars={'Energy', 'Block'} powers=set() not recognized
        // SKIP Chill: has richer match-arm in combat.rs; let it run
        // SKIP Cinder: has richer match-arm in combat.rs; let it run
        // SKIP Claw: has richer match-arm in combat.rs; let it run
        // SKIP Cleanse: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP CloakAndDagger: has richer match-arm in combat.rs; let it run
        // SKIP CollisionCourse: has richer match-arm in combat.rs; let it run
        // SKIP Colossus: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP CompileDriver: has richer match-arm in combat.rs; let it run
        // SKIP Conflagration: has richer match-arm in combat.rs; let it run
        // SKIP Conqueror: Skill/AnyEnemy shape with vars={'Forge'} powers=set() not recognized
        // SKIP Convergence: Skill/Self shape with vars={'Energy', 'Stars'} powers=set() not recognized
        // SKIP Coordinate: Skill/Self shape with vars={'Power'} powers={'StrengthPower'} not recognized
        // SKIP CorrosiveWave: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP CrashLanding: has richer match-arm in combat.rs; let it run
        // SKIP CrescentSpear: has richer match-arm in combat.rs; let it run
        // SKIP CrimsonMantle: has richer match-arm in combat.rs; let it run
        // SKIP DaggerSpray: has richer match-arm in combat.rs; let it run
        // SKIP DarkShackles: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Darkness: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Dash: has richer match-arm in combat.rs; let it run
        // SKIP DeathMarch: has richer match-arm in combat.rs; let it run
        // SKIP DeathsDoor: has richer match-arm in combat.rs; let it run
        // SKIP DecisionsDecisions: has richer match-arm in combat.rs; let it run
        // SKIP Delay: Skill/Self shape with vars={'Energy', 'Block'} powers=set() not recognized
        // SKIP DemonForm: has richer match-arm in combat.rs; let it run
        // SKIP DemonicShield: Skill/Self shape with vars={'HpLoss', 'CalculationExtra', 'CalculationBase', 'CalculatedBlock'} powers=set() not recognized
        // SKIP Dirge: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Discovery: has richer match-arm in combat.rs; let it run
        // SKIP Distraction: has richer match-arm in combat.rs; let it run
        // SKIP DodgeAndRoll: has richer match-arm in combat.rs; let it run
        // SKIP Dominate: has richer match-arm in combat.rs; let it run
        // SKIP DoubleEnergy: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Dredge: has richer match-arm in combat.rs; let it run
        // SKIP DualWield: has richer match-arm in combat.rs; let it run
        // SKIP Dualcast: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP DyingStar: has richer match-arm in combat.rs; let it run
        // SKIP Eidolon: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP EndOfDays: has richer match-arm in combat.rs; let it run
        // SKIP EnergySurge: unknown shape: type=Skill target=AllAllies vars={'Energy'} powers=set()
        // SKIP EnfeeblingTouch: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Enlightenment: has richer match-arm in combat.rs; let it run
        // SKIP Entrench: has richer match-arm in combat.rs; let it run
        // SKIP Equilibrium: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP Eradicate: has richer match-arm in combat.rs; let it run
        // SKIP EscapePlan: has richer match-arm in combat.rs; let it run
        // SKIP EvilEye: has richer match-arm in combat.rs; let it run
        // SKIP ExpectAFight: has richer match-arm in combat.rs; let it run
        // SKIP Expose: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Feed: has richer match-arm in combat.rs; let it run
        // SKIP FeedingFrenzy: Skill/Self shape with vars={'Power'} powers={'StrengthPower'} not recognized
        // SKIP Fetch: Attack to single enemy without Damage var
        // SKIP FiendFire: has richer match-arm in combat.rs; let it run
        // SKIP FightMe: has richer match-arm in combat.rs; let it run
        // SKIP FightThrough: has richer match-arm in combat.rs; let it run
        // SKIP Finisher: has richer match-arm in combat.rs; let it run
        // SKIP Fisticuffs: has richer match-arm in combat.rs; let it run
        // SKIP FlakCannon: has richer match-arm in combat.rs; let it run
        // SKIP FlameBarrier: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP Flanking: Skill/AnyEnemy shape with vars=set() powers=set() not recognized
        // SKIP Flatten: Attack to single enemy without Damage var
        // SKIP Flechettes: has richer match-arm in combat.rs; let it run
        // SKIP ForgottenRitual: has richer match-arm in combat.rs; let it run
        // SKIP Ftl: has richer match-arm in combat.rs; let it run
        // SKIP Fuel: Skill/Self shape with vars={'Energy', 'Cards'} powers=set() not recognized
        // SKIP Fusion: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP GammaBlast: has richer match-arm in combat.rs; let it run
        // SKIP GangUp: has richer match-arm in combat.rs; let it run
        // SKIP GatherLight: Skill/Self shape with vars={'Block', 'Stars'} powers=set() not recognized
        // SKIP GeneticAlgorithm: has richer match-arm in combat.rs; let it run
        // SKIP Glimmer: Skill/Self shape with vars={'Dynamic', 'Cards'} powers=set() not recognized
        // SKIP GlimpseBeyond: unknown shape: type=Skill target=AllAllies vars={'Cards'} powers=set()
        // SKIP Glow: has richer match-arm in combat.rs; let it run
        // SKIP GoldAxe: has richer match-arm in combat.rs; let it run
        // SKIP GraveWarden: has richer match-arm in combat.rs; let it run
        // SKIP Guards: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP GuidingStar: has richer match-arm in combat.rs; let it run
        // SKIP Hang: has richer match-arm in combat.rs; let it run
        // SKIP Havoc: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Headbutt: has richer match-arm in combat.rs; let it run
        // SKIP HeavenlyDrill: has richer match-arm in combat.rs; let it run
        // SKIP HelixDrill: has richer match-arm in combat.rs; let it run
        // SKIP HiddenCache: Skill/Self shape with vars={'Power', 'Stars'} powers={'StarNextTurnPower'} not recognized
        // SKIP HiddenDaggers: Skill/Self shape with vars={'Dynamic', 'Cards'} powers=set() not recognized
        // SKIP HiddenGem: Skill/Self shape with vars={'Int'} powers=set() not recognized
        // SKIP HighFive: AOE attack without Damage var
        // SKIP Hologram: has richer match-arm in combat.rs; let it run
        // SKIP Hotfix: Skill/Self shape with vars={'Power'} powers={'FocusPower'} not recognized
        // SKIP HuddleUp: unknown shape: type=Skill target=AllAllies vars={'Cards'} powers=set()
        // SKIP Hyperbeam: has richer match-arm in combat.rs; let it run
        // SKIP IceLance: has richer match-arm in combat.rs; let it run
        // SKIP Ignition: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP InfernalBlade: has richer match-arm in combat.rs; let it run
        // SKIP Invoke: Skill/Self shape with vars={'Summon', 'Energy'} powers=set() not recognized
        // SKIP JackOfAllTrades: has richer match-arm in combat.rs; let it run
        // SKIP Jackpot: has richer match-arm in combat.rs; let it run
        // SKIP KnifeTrap: has richer match-arm in combat.rs; let it run
        // SKIP LanternKey: unknown shape: type=Quest target=Self vars=set() powers=set()
        // SKIP Largesse: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP LeadingStrike: has richer match-arm in combat.rs; let it run
        // SKIP LegSweep: has richer match-arm in combat.rs; let it run
        // SKIP LegionOfBone: has richer match-arm in combat.rs; let it run
        // SKIP Malaise: Skill/AnyEnemy shape with vars=set() powers=set() not recognized
        // SKIP Mangle: has richer match-arm in combat.rs; let it run
        // SKIP Maul: has richer match-arm in combat.rs; let it run
        // SKIP Melancholy: Skill/Self shape with vars={'Energy', 'Block'} powers=set() not recognized
        // SKIP MementoMori: has richer match-arm in combat.rs; let it run
        // SKIP Mimic: has richer match-arm in combat.rs; let it run
        // SKIP MindBlast: has richer match-arm in combat.rs; let it run
        // SKIP Mirage: has richer match-arm in combat.rs; let it run
        // SKIP Misery: has richer match-arm in combat.rs; let it run
        // SKIP Modded: has richer match-arm in combat.rs; let it run
        // SKIP MoltenFist: has richer match-arm in combat.rs; let it run
        // SKIP Monologue: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP MultiCast: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Murder: has richer match-arm in combat.rs; let it run
        // SKIP Neurosurge: has richer match-arm in combat.rs; let it run
        // SKIP Nightmare: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP NoEscape: has richer match-arm in combat.rs; let it run
        // SKIP NotYet: Skill/Self shape with vars={'Heal'} powers=set() not recognized
        // SKIP Offering: has richer match-arm in combat.rs; let it run
        // SKIP Omnislice: has richer match-arm in combat.rs; let it run
        // SKIP OneTwoPunch: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Overclock: has richer match-arm in combat.rs; let it run
        // SKIP PanicButton: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP Patter: has richer match-arm in combat.rs; let it run
        // SKIP PerfectedStrike: has richer match-arm in combat.rs; let it run
        // SKIP PhotonCut: has richer match-arm in combat.rs; let it run
        // SKIP PiercingWail: Skill/AllEnemies with vars={'Dynamic'} powers=set() not recognized
        // SKIP Pillage: has richer match-arm in combat.rs; let it run
        // SKIP PoisonedStab: has richer match-arm in combat.rs; let it run
        // SKIP Poke: Attack to single enemy without Damage var
        // SKIP PommelStrike: has richer match-arm in combat.rs; let it run
        // SKIP PreciseCut: has richer match-arm in combat.rs; let it run
        // SKIP PrimalForce: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Prolong: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP PullAggro: has richer match-arm in combat.rs; let it run
        // SKIP PullFromBelow: has richer match-arm in combat.rs; let it run
        // SKIP Putrefy: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Quadcast: Skill/Self shape with vars={'Repeat'} powers=set() not recognized
        // SKIP Quasar: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Radiate: has richer match-arm in combat.rs; let it run
        // SKIP Rage: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Rainbow: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Rally: has richer match-arm in combat.rs; let it run
        // SKIP Rampage: has richer match-arm in combat.rs; let it run
        // SKIP Rattle: has richer match-arm in combat.rs; let it run
        // SKIP Reanimate: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Reboot: has richer match-arm in combat.rs; let it run
        // SKIP RefineBlade: Skill/Self shape with vars={'Energy', 'Forge'} powers=set() not recognized
        // SKIP Refract: has richer match-arm in combat.rs; let it run
        // SKIP Relax: has richer match-arm in combat.rs; let it run
        // SKIP Rend: has richer match-arm in combat.rs; let it run
        // SKIP Resonance: has richer match-arm in combat.rs; let it run
        // SKIP Restlessness: has richer match-arm in combat.rs; let it run
        // SKIP Ricochet: has richer match-arm in combat.rs; let it run
        // SKIP RightHandHand: Attack to single enemy without Damage var
        // SKIP RoyalGamble: Skill/Self shape with vars={'Stars'} powers=set() not recognized
        // SKIP Sacrifice: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Salvo: has richer match-arm in combat.rs; let it run
        // SKIP Scavenge: has richer match-arm in combat.rs; let it run
        // SKIP Scourge: has richer match-arm in combat.rs; let it run
        // SKIP Scrawl: has richer match-arm in combat.rs; let it run
        // SKIP SecretTechnique: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SecretWeapon: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SetupStrike: has richer match-arm in combat.rs; let it run
        // SKIP Severance: has richer match-arm in combat.rs; let it run
        // SKIP Shadowmeld: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP SharedFate: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Shockwave: Skill/AllEnemies with vars={'Dynamic'} powers=set() not recognized
        // SKIP SicEm: Attack to single enemy without Damage var
        // SKIP SignalBoost: Skill/Self shape with vars={'Power'} powers={'SignalBoostPower'} not recognized
        // SKIP Skewer: X-cost single-target attack (would need Repeat over hits)
        // SKIP Snakebite: has richer match-arm in combat.rs; let it run
        // SKIP Snap: Attack to single enemy without Damage var
        // SKIP Spite: has richer match-arm in combat.rs; let it run
        // SKIP Splash: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SpoilsMap: unknown shape: type=Quest target=Self vars={'Gold'} powers=set()
        // SKIP SpoilsOfBattle: Skill/Self shape with vars={'Forge', 'Cards'} powers=set() not recognized
        // SKIP Spur: Skill/Self shape with vars={'Summon', 'Heal'} powers=set() not recognized
        // SKIP Stack: Skill/Self shape with vars={'CalculationExtra', 'CalculationBase', 'CalculatedBlock'} powers=set() not recognized
        // SKIP Stoke: has richer match-arm in combat.rs; let it run
        // SKIP StormOfSteel: has richer match-arm in combat.rs; let it run
        // SKIP SummonForth: Skill/Self shape with vars={'Forge'} powers=set() not recognized
        // SKIP Sunder: has richer match-arm in combat.rs; let it run
        // SKIP Survivor: has richer match-arm in combat.rs; let it run
        // SKIP SweepingGaze: Random-target attack without Damage var
        // SKIP SwordBoomerang: has richer match-arm in combat.rs; let it run
        // SKIP Synchronize: Skill/Self shape with vars={'CalculationExtra', 'CalculationBase', 'Calculated'} powers=set() not recognized
        // SKIP Tempest: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Terraforming: Skill/Self shape with vars={'Power'} powers={'VigorPower'} not recognized
        // SKIP TheBomb: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP TheSmith: Skill/Self shape with vars={'Forge'} powers=set() not recognized
        // SKIP ToricToughness: Skill/Self shape with vars={'Dynamic', 'Block'} powers=set() not recognized
        // SKIP Tremble: has richer match-arm in combat.rs; let it run
        // SKIP TrueGrit: has richer match-arm in combat.rs; let it run
        // SKIP Turbo: has richer match-arm in combat.rs; let it run
        // SKIP Undeath: has richer match-arm in combat.rs; let it run
        // SKIP Unrelenting: has richer match-arm in combat.rs; let it run
        // SKIP Uppercut: has richer match-arm in combat.rs; let it run
        // SKIP Uproar: has richer match-arm in combat.rs; let it run
        // SKIP Venerate: Skill/Self shape with vars={'Stars'} powers=set() not recognized
        // SKIP Voltaic: Skill/Self shape with vars={'CalculationExtra', 'CalculationBase', 'Calculated'} powers=set() not recognized
        // SKIP Whirlwind: X-cost AOE (Whirlwind shape -- handled in earlier migration)
        // SKIP WhiteNoise: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Wish: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Zap: Skill/Self shape with vars=set() powers=set() not recognized
