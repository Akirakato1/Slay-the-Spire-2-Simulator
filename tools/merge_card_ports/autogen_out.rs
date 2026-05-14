// Auto-generated from cards.json by tools/merge_card_ports/autogen.py
// Per-card encodings -- conservative shape match only.
// Skipped cards fall through to the match-arm dispatch path or
// are not yet ported. See `// SKIP` comments for reasons.

        "Abrasive" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Canonical("ThornsPower".to_string()), target: Target::SelfPlayer }]),
        "Accuracy" => Some(vec![Effect::ApplyPower { power_id: "AccuracyPower".to_string(), amount: AmountSpec::Canonical("AccuracyPower".to_string()), target: Target::SelfPlayer }]),
        "AdaptiveStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Afterimage" => Some(vec![Effect::ApplyPower { power_id: "AfterimagePower".to_string(), amount: AmountSpec::Canonical("AfterimagePower".to_string()), target: Target::SelfPlayer }]),
        "Alignment" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "AllForOne" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Armaments" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Arsenal" => Some(vec![Effect::ApplyPower { power_id: "ArsenalPower".to_string(), amount: AmountSpec::Canonical("ArsenalPower".to_string()), target: Target::SelfPlayer }]),
        "AscendersBane" => Some(vec![]),
        "Assassinate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "Backflip" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Backstab" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BadLuck" => Some(vec![]),
        "BallLightning" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BansheesCry" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Barrage" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BattleTrance" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "BeatIntoShape" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Beckon" => Some(vec![]),
        "BelieveInYou" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "BiasedCognition" => Some(vec![Effect::ApplyPower { power_id: "BiasedCognitionPower".to_string(), amount: AmountSpec::Canonical("BiasedCognitionPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),
        "BlackHole" => Some(vec![Effect::ApplyPower { power_id: "BlackHolePower".to_string(), amount: AmountSpec::Canonical("BlackHolePower".to_string()), target: Target::SelfPlayer }]),
        "BladeOfInk" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Bludgeon" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Bolas" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Bombardment" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "BootSequence" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "BorrowedTime" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Break" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "BubbleBubble" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::ChosenEnemy }]),
        "Buffer" => Some(vec![Effect::ApplyPower { power_id: "BufferPower".to_string(), amount: AmountSpec::Canonical("BufferPower".to_string()), target: Target::SelfPlayer }]),
        "BulkUp" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "BundleOfJoy" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Burn" => Some(vec![]),
        "BurningPact" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Bury" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ByrdSwoop" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Caltrops" => Some(vec![Effect::ApplyPower { power_id: "ThornsPower".to_string(), amount: AmountSpec::Canonical("ThornsPower".to_string()), target: Target::SelfPlayer }]),
        "Catastrophe" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "CelestialMight" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Charge" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Clash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Claw" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Clumsy" => Some(vec![]),
        "ColdSnap" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Comet" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Compact" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "CompileDriver" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ConsumingShadow" => Some(vec![Effect::ApplyPower { power_id: "ConsumingShadowPower".to_string(), amount: AmountSpec::Canonical("ConsumingShadowPower".to_string()), target: Target::SelfPlayer }]),
        "Coolant" => Some(vec![Effect::ApplyPower { power_id: "CoolantPower".to_string(), amount: AmountSpec::Canonical("CoolantPower".to_string()), target: Target::SelfPlayer }]),
        "Coolheaded" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Countdown" => Some(vec![Effect::ApplyPower { power_id: "CountdownPower".to_string(), amount: AmountSpec::Canonical("CountdownPower".to_string()), target: Target::SelfPlayer }]),
        "CrashLanding" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "CrimsonMantle" => Some(vec![Effect::ApplyPower { power_id: "CrimsonMantlePower".to_string(), amount: AmountSpec::Canonical("CrimsonMantlePower".to_string()), target: Target::SelfPlayer }]),
        "Cruelty" => Some(vec![Effect::ApplyPower { power_id: "CrueltyPower".to_string(), amount: AmountSpec::Canonical("CrueltyPower".to_string()), target: Target::SelfPlayer }]),
        "CrushUnder" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "CurseOfTheBell" => Some(vec![]),
        "DaggerThrow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DanseMacabre" => Some(vec![Effect::ApplyPower { power_id: "DanseMacabrePower".to_string(), amount: AmountSpec::Canonical("DanseMacabrePower".to_string()), target: Target::SelfPlayer }]),
        "Dazed" => Some(vec![]),
        "DeadlyPoison" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::ChosenEnemy }]),
        "Deathbringer" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::AllEnemies }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::AllEnemies }]),
        "Debilitate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Debris" => Some(vec![]),
        "Debt" => Some(vec![]),
        "Decay" => Some(vec![]),
        "Deflect" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Defragment" => Some(vec![Effect::ApplyPower { power_id: "FocusPower".to_string(), amount: AmountSpec::Canonical("FocusPower".to_string()), target: Target::SelfPlayer }]),
        "DeprecatedCard" => Some(vec![]),
        "Devastate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DevourLife" => Some(vec![Effect::ApplyPower { power_id: "DevourLifePower".to_string(), amount: AmountSpec::Canonical("DevourLifePower".to_string()), target: Target::SelfPlayer }]),
        "Disintegration" => Some(vec![]),
        "Dismantle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Doubt" => Some(vec![]),
        "DrainPower" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "DramaticEntrance" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Dredge" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "DrumOfBattle" => Some(vec![Effect::ApplyPower { power_id: "DrumOfBattlePower".to_string(), amount: AmountSpec::Canonical("DrumOfBattlePower".to_string()), target: Target::SelfPlayer }]),
        "DualWield" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "DyingStar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "EchoingSlash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "EndOfDays" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::AllEnemies }]),
        "Enthralled" => Some(vec![]),
        "Envenom" => Some(vec![Effect::ApplyPower { power_id: "EnvenomPower".to_string(), amount: AmountSpec::Canonical("EnvenomPower".to_string()), target: Target::SelfPlayer }]),
        "EternalArmor" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "EvilEye" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Expertise" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Exterminate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "FallingStar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Fear" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "Feral" => Some(vec![Effect::ApplyPower { power_id: "FeralPower".to_string(), amount: AmountSpec::Canonical("FeralPower".to_string()), target: Target::SelfPlayer }]),
        "FightMe" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "FightThrough" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Finesse" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Finisher" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Fisticuffs" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "FlakCannon" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "FlashOfSteel" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Flechettes" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "FlickFlack" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "FocusedStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "FollowThrough" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Folly" => Some(vec![]),
        "Footwork" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }]),
        "ForegoneConclusion" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "ForgottenRitual" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "FranticEscape" => Some(vec![]),
        "Friendship" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Ftl" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "GammaBlast" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "GiantRock" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Glacier" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Glasswork" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Glitterstream" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "GoForTheEyes" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "GrandFinale" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Graveblast" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Greed" => Some(vec![]),
        "GuidingStar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Guilty" => Some(vec![]),
        "GunkUp" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Hailstorm" => Some(vec![Effect::ApplyPower { power_id: "HailstormPower".to_string(), amount: AmountSpec::Canonical("HailstormPower".to_string()), target: Target::SelfPlayer }]),
        "HandOfGreed" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HandTrick" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Hang" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Haze" => Some(vec![Effect::ApplyPower { power_id: "PoisonPower".to_string(), amount: AmountSpec::Canonical("PoisonPower".to_string()), target: Target::AllEnemies }]),
        "Headbutt" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Hegemony" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HeirloomHammer" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "HelixDrill" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Hemokinesis" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Hologram" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "HowlFromBeyond" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Hyperbeam" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "IAmInvincible" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "IceLance" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Impatience" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Impervious" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Infection" => Some(vec![]),
        "Inferno" => Some(vec![Effect::ApplyPower { power_id: "InfernoPower".to_string(), amount: AmountSpec::Canonical("InfernoPower".to_string()), target: Target::SelfPlayer }]),
        "Injury" => Some(vec![]),
        "Intercept" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Iteration" => Some(vec![Effect::ApplyPower { power_id: "IterationPower".to_string(), amount: AmountSpec::Canonical("IterationPower".to_string()), target: Target::SelfPlayer }]),
        "JackOfAllTrades" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Jackpot" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Juggernaut" => Some(vec![Effect::ApplyPower { power_id: "JuggernautPower".to_string(), amount: AmountSpec::Canonical("JuggernautPower".to_string()), target: Target::SelfPlayer }]),
        "KinglyKick" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KinglyPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Knockdown" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KnockoutBlow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "KnowThyPlace" => Some(vec![Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Leap" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Lethality" => Some(vec![Effect::ApplyPower { power_id: "LethalityPower".to_string(), amount: AmountSpec::Canonical("LethalityPower".to_string()), target: Target::SelfPlayer }]),
        "Lift" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "LightningRod" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "LightningRodPower".to_string(), amount: AmountSpec::Canonical("LightningRodPower".to_string()), target: Target::SelfPlayer }]),
        "Luminesce" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "LunarBlast" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MadScience" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "MakeItSo" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ManifestAuthority" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "MasterOfStrategy" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Maul" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Metamorphosis" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "MeteorShower" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::AllEnemies }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::AllEnemies }]),
        "MeteorStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MindRot" => Some(vec![]),
        "MinionDiveBomb" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MinionSacrifice" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "MinionStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Misery" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "MomentumStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "NegativePulse" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::AllEnemies }]),
        "NeowsFury" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Neurosurge" => Some(vec![Effect::ApplyPower { power_id: "NeurosurgePower".to_string(), amount: AmountSpec::Canonical("NeurosurgePower".to_string()), target: Target::SelfPlayer }]),
        "NeutronAegis" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "Normality" => Some(vec![]),
        "Null" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Oblivion" => Some(vec![Effect::ApplyPower { power_id: "DoomPower".to_string(), amount: AmountSpec::Canonical("DoomPower".to_string()), target: Target::ChosenEnemy }]),
        "Omnislice" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Outbreak" => Some(vec![Effect::ApplyPower { power_id: "OutbreakPower".to_string(), amount: AmountSpec::Canonical("OutbreakPower".to_string()), target: Target::SelfPlayer }]),
        "Outmaneuver" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Overclock" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "PactsEnd" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Parry" => Some(vec![Effect::ApplyPower { power_id: "ParryPower".to_string(), amount: AmountSpec::Canonical("ParryPower".to_string()), target: Target::SelfPlayer }]),
        "Parse" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "ParticleWall" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Patter" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "VigorPower".to_string(), amount: AmountSpec::Canonical("VigorPower".to_string()), target: Target::SelfPlayer }]),
        "Peck" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PhantomBlades" => Some(vec![Effect::ApplyPower { power_id: "PhantomBladesPower".to_string(), amount: AmountSpec::Canonical("PhantomBladesPower".to_string()), target: Target::SelfPlayer }]),
        "PhotonCut" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Pillage" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Pinpoint" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PoorSleep" => Some(vec![]),
        "Pounce" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Predator" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "PrepTime" => Some(vec![Effect::ApplyPower { power_id: "PrepTimePower".to_string(), amount: AmountSpec::Canonical("PrepTimePower".to_string()), target: Target::SelfPlayer }]),
        "Prepared" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Production" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Prophesize" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Prowess" => Some(vec![Effect::ApplyPower { power_id: "DexterityPower".to_string(), amount: AmountSpec::Canonical("DexterityPower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "PullFromBelow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Purity" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Radiate" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Rampage" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Reap" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Reave" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Reboot" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Rebound" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Reflect" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Reflex" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Refract" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Regret" => Some(vec![]),
        "Resonance" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::AllEnemies }]),
        "RipAndTear" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "RocketPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "RollingBoulder" => Some(vec![Effect::ApplyPower { power_id: "RollingBoulderPower".to_string(), amount: AmountSpec::Canonical("RollingBoulderPower".to_string()), target: Target::SelfPlayer }]),
        "Rupture" => Some(vec![Effect::ApplyPower { power_id: "StrengthPower".to_string(), amount: AmountSpec::Canonical("StrengthPower".to_string()), target: Target::SelfPlayer }]),
        "Salvo" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Scavenge" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Scrape" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SculptingStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Seance" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "SecondWind" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "SeekerStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SentryMode" => Some(vec![Effect::ApplyPower { power_id: "SentryModePower".to_string(), amount: AmountSpec::Canonical("SentryModePower".to_string()), target: Target::SelfPlayer }]),
        "SerpentForm" => Some(vec![Effect::ApplyPower { power_id: "SerpentFormPower".to_string(), amount: AmountSpec::Canonical("SerpentFormPower".to_string()), target: Target::SelfPlayer }]),
        "SevenStars" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Severance" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ShadowShield" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "ShadowStep" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Shame" => Some(vec![]),
        "Shatter" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "ShiningStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Shiv" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
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
        "SovereignBlade" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Sow" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "Speedster" => Some(vec![Effect::ApplyPower { power_id: "SpeedsterPower".to_string(), amount: AmountSpec::Canonical("SpeedsterPower".to_string()), target: Target::SelfPlayer }]),
        "Spinner" => Some(vec![Effect::ApplyPower { power_id: "SpinnerPower".to_string(), amount: AmountSpec::Canonical("SpinnerPower".to_string()), target: Target::SelfPlayer }]),
        "Spite" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SporeMind" => Some(vec![]),
        "Squash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "Stardust" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "Stomp" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "StoneArmor" => Some(vec![Effect::ApplyPower { power_id: "PlatingPower".to_string(), amount: AmountSpec::Canonical("PlatingPower".to_string()), target: Target::SelfPlayer }]),
        "Storm" => Some(vec![Effect::ApplyPower { power_id: "StormPower".to_string(), amount: AmountSpec::Canonical("StormPower".to_string()), target: Target::SelfPlayer }]),
        "Strangle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "SuckerPunch" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "Supercritical" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Suppress" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }, Effect::ApplyPower { power_id: "WeakPower".to_string(), amount: AmountSpec::Canonical("WeakPower".to_string()), target: Target::ChosenEnemy }]),
        "SweepingBeam" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::AllEnemies, hits: 1 }]),
        "SwordSage" => Some(vec![Effect::ApplyPower { power_id: "SwordSagePower".to_string(), amount: AmountSpec::Canonical("SwordSagePower".to_string()), target: Target::SelfPlayer }]),
        "Synthesis" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Tactician" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "TagTeam" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Taunt" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "VulnerablePower".to_string(), amount: AmountSpec::Canonical("VulnerablePower".to_string()), target: Target::ChosenEnemy }]),
        "TearAsunder" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TeslaCoil" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TheGambit" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "TheHunt" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "TheScythe" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ThinkingAhead" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Thrash" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "ThrummingHatchet" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Thunder" => Some(vec![Effect::ApplyPower { power_id: "ThunderPower".to_string(), amount: AmountSpec::Canonical("ThunderPower".to_string()), target: Target::SelfPlayer }]),
        "Toxic" => Some(vec![]),
        "Transfigure" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Turbo" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "UltimateDefend" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "UltimateStrike" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Undeath" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "Unrelenting" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Untouchable" => Some(vec![Effect::GainBlock { amount: AmountSpec::Canonical("Block".to_string()), target: Target::SelfPlayer }]),
        "UpMySleeve" => Some(vec![Effect::DrawCards { amount: AmountSpec::Canonical("Cards".to_string()) }]),
        "Uppercut" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Uproar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Veilpiercer" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Void" => Some(vec![]),
        "VoidForm" => Some(vec![Effect::ApplyPower { power_id: "VoidFormPower".to_string(), amount: AmountSpec::Canonical("VoidFormPower".to_string()), target: Target::SelfPlayer }]),
        "Volley" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::RandomEnemy, hits: 1 }]),
        "WasteAway" => Some(vec![]),
        "Whistle" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),
        "Wisp" => Some(vec![Effect::GainEnergy { amount: AmountSpec::Canonical("Energy".to_string()) }]),
        "Wound" => Some(vec![]),
        "WraithForm" => Some(vec![Effect::ApplyPower { power_id: "IntangiblePower".to_string(), amount: AmountSpec::Canonical("IntangiblePower".to_string()), target: Target::SelfPlayer }, Effect::ApplyPower { power_id: "WraithFormPower".to_string(), amount: AmountSpec::Canonical("WraithFormPower".to_string()), target: Target::SelfPlayer }]),
        "Writhe" => Some(vec![]),
        "WroughtInWar" => Some(vec![Effect::DealDamage { amount: AmountSpec::Canonical("Damage".to_string()), target: Target::ChosenEnemy, hits: 1 }]),

        // ===== Skipped (need shape-specific handling) =====
        // SKIP Accelerant: Power card with 0 canonical powers; unknown shape
        // SKIP Acrobatics: has richer match-arm in combat.rs; let it run
        // SKIP Adrenaline: Skill/Self shape with vars={'Energy', 'Cards'} powers=set() not recognized
        // SKIP Afterlife: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Aggression: Power card with 0 canonical powers; unknown shape
        // SKIP Alchemize: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Anger: has richer match-arm in combat.rs; let it run
        // SKIP Anointed: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Anticipate: Skill/Self shape with vars={'Power'} powers={'DexterityPower'} not recognized
        // SKIP Apotheosis: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Apparition: has richer match-arm in combat.rs; let it run
        // SKIP AshenStrike: Attack to single enemy without Damage var
        // SKIP Automation: Power card with 0 canonical powers; unknown shape
        // SKIP Barricade: has richer match-arm in combat.rs; let it run
        // SKIP BeaconOfHope: Power card with 0 canonical powers; unknown shape
        // SKIP BeatDown: unknown shape: type=Skill target=RandomEnemy vars={'Cards'} powers=set()
        // SKIP Begone: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP BigBang: Skill/Self shape with vars={'Energy', 'Forge', 'Cards', 'Stars'} powers=set() not recognized
        // SKIP BladeDance: has richer match-arm in combat.rs; let it run
        // SKIP BlightStrike: has richer match-arm in combat.rs; let it run
        // SKIP BloodWall: Skill/Self shape with vars={'Block', 'HpLoss'} powers=set() not recognized
        // SKIP Blur: Skill/Self shape with vars={'Block', 'Dynamic'} powers=set() not recognized
        // SKIP BodySlam: Attack to single enemy without Damage var
        // SKIP Bodyguard: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP BoneShards: AOE attack without Damage var
        // SKIP BouncingFlask: unknown shape: type=Skill target=RandomEnemy vars={'Repeat', 'Power'} powers={'PoisonPower'}
        // SKIP Brand: Skill/Self shape with vars={'HpLoss', 'Power'} powers={'StrengthPower'} not recognized
        // SKIP Breakthrough: has richer match-arm in combat.rs; let it run
        // SKIP BrightestFlame: Skill/Self shape with vars={'Energy', 'MaxHp', 'Cards'} powers=set() not recognized
        // SKIP BulletTime: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Bully: Attack to single enemy without Damage var
        // SKIP Bulwark: Skill/Self shape with vars={'Block', 'Forge'} powers=set() not recognized
        // SKIP Burst: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP ByrdonisEgg: unknown shape: type=Quest target=None vars=set() powers=set()
        // SKIP Calamity: Power card with 0 canonical powers; unknown shape
        // SKIP Calcify: has richer match-arm in combat.rs; let it run
        // SKIP CalculatedGamble: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP CallOfTheVoid: Power card with 0 canonical powers; unknown shape
        // SKIP Capacitor: Power card with 0 canonical powers; unknown shape
        // SKIP CaptureSpirit: Skill/AnyEnemy shape with vars={'Damage', 'Cards'} powers=set() not recognized
        // SKIP Cascade: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Chaos: Skill/Self shape with vars={'Repeat'} powers=set() not recognized
        // SKIP ChargeBattery: Skill/Self shape with vars={'Block', 'Energy'} powers=set() not recognized
        // SKIP ChildOfTheStars: Power card with 0 canonical powers; unknown shape
        // SKIP Chill: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Cinder: has richer match-arm in combat.rs; let it run
        // SKIP Cleanse: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP CloakAndDagger: has richer match-arm in combat.rs; let it run
        // SKIP CollisionCourse: has richer match-arm in combat.rs; let it run
        // SKIP Colossus: Skill/Self shape with vars={'Block', 'Dynamic'} powers=set() not recognized
        // SKIP Conflagration: AOE attack without Damage var
        // SKIP Conqueror: Skill/AnyEnemy shape with vars={'Forge'} powers=set() not recognized
        // SKIP Convergence: Skill/Self shape with vars={'Energy', 'Stars'} powers=set() not recognized
        // SKIP Coordinate: Skill/Self shape with vars={'Power'} powers={'StrengthPower'} not recognized
        // SKIP CorrosiveWave: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Corruption: Power card with 0 canonical powers; unknown shape
        // SKIP CreativeAi: Power card with 0 canonical powers; unknown shape
        // SKIP CrescentSpear: Attack to single enemy without Damage var
        // SKIP DaggerSpray: has richer match-arm in combat.rs; let it run
        // SKIP DarkEmbrace: Power card with 0 canonical powers; unknown shape
        // SKIP DarkShackles: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Darkness: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Dash: has richer match-arm in combat.rs; let it run
        // SKIP DeathMarch: Attack to single enemy without Damage var
        // SKIP DeathsDoor: Skill/Self shape with vars={'Block', 'Repeat'} powers=set() not recognized
        // SKIP DecisionsDecisions: Skill/Self shape with vars={'Repeat', 'Cards'} powers=set() not recognized
        // SKIP Delay: Skill/Self shape with vars={'Block', 'Energy'} powers=set() not recognized
        // SKIP Demesne: Power card with 0 canonical powers; unknown shape
        // SKIP DemonForm: has richer match-arm in combat.rs; let it run
        // SKIP DemonicShield: Skill/Self shape with vars={'CalculatedBlock', 'HpLoss', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP Dirge: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP Discovery: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Distraction: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP DodgeAndRoll: has richer match-arm in combat.rs; let it run
        // SKIP Dominate: Skill/AnyEnemy shape with vars={'Dynamic', 'Power'} powers={'VulnerablePower'} not recognized
        // SKIP DoubleEnergy: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Dualcast: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP EchoForm: Power card with 0 canonical powers; unknown shape
        // SKIP Eidolon: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP EnergySurge: unknown shape: type=Skill target=AllAllies vars={'Energy'} powers=set()
        // SKIP EnfeeblingTouch: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Enlightenment: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Entrench: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Entropy: Power card with 0 canonical powers; unknown shape
        // SKIP Equilibrium: Skill/Self shape with vars={'Block', 'Dynamic'} powers=set() not recognized
        // SKIP Eradicate: X-cost single-target attack (would need Repeat over hits)
        // SKIP EscapePlan: has richer match-arm in combat.rs; let it run
        // SKIP ExpectAFight: Skill/Self shape with vars={'Energy', 'Calculated', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP Expose: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP FanOfKnives: Power card with 0 canonical powers; unknown shape
        // SKIP Fasten: Power card with 0 canonical powers; unknown shape
        // SKIP Feed: has richer match-arm in combat.rs; let it run
        // SKIP FeedingFrenzy: Skill/Self shape with vars={'Power'} powers={'StrengthPower'} not recognized
        // SKIP FeelNoPain: Power card with 0 canonical powers; unknown shape
        // SKIP Fetch: Attack to single enemy without Damage var
        // SKIP FiendFire: has richer match-arm in combat.rs; let it run
        // SKIP FlameBarrier: Skill/Self shape with vars={'Block', 'Dynamic'} powers=set() not recognized
        // SKIP Flanking: Skill/AnyEnemy shape with vars=set() powers=set() not recognized
        // SKIP Flatten: Attack to single enemy without Damage var
        // SKIP ForbiddenGrimoire: Power card with 0 canonical powers; unknown shape
        // SKIP Fuel: Skill/Self shape with vars={'Energy', 'Cards'} powers=set() not recognized
        // SKIP Furnace: Power card with 0 canonical powers; unknown shape
        // SKIP Fusion: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP GangUp: Attack to single enemy without Damage var
        // SKIP GatherLight: Skill/Self shape with vars={'Block', 'Stars'} powers=set() not recognized
        // SKIP Genesis: Power card with 0 canonical powers; unknown shape
        // SKIP GeneticAlgorithm: Skill/Self shape with vars={'Block', 'Int'} powers=set() not recognized
        // SKIP Glimmer: Skill/Self shape with vars={'Dynamic', 'Cards'} powers=set() not recognized
        // SKIP GlimpseBeyond: unknown shape: type=Skill target=AllAllies vars={'Cards'} powers=set()
        // SKIP Glow: Skill/Self shape with vars={'Cards', 'Stars'} powers=set() not recognized
        // SKIP GoldAxe: Attack to single enemy without Damage var
        // SKIP GraveWarden: has richer match-arm in combat.rs; let it run
        // SKIP Guards: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP HammerTime: Power card with 0 canonical powers; unknown shape
        // SKIP Haunt: Power card with 0 canonical powers; unknown shape
        // SKIP Havoc: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP HeavenlyDrill: X-cost single-target attack (would need Repeat over hits)
        // SKIP HelloWorld: Power card with 0 canonical powers; unknown shape
        // SKIP Hellraiser: Power card with 0 canonical powers; unknown shape
        // SKIP HiddenCache: Skill/Self shape with vars={'Power', 'Stars'} powers={'StarNextTurnPower'} not recognized
        // SKIP HiddenDaggers: Skill/Self shape with vars={'Dynamic', 'Cards'} powers=set() not recognized
        // SKIP HiddenGem: Skill/Self shape with vars={'Int'} powers=set() not recognized
        // SKIP HighFive: AOE attack without Damage var
        // SKIP Hotfix: Skill/Self shape with vars={'Power'} powers={'FocusPower'} not recognized
        // SKIP HuddleUp: unknown shape: type=Skill target=AllAllies vars={'Cards'} powers=set()
        // SKIP Ignition: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP InfernalBlade: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP InfiniteBlades: Power card with 0 canonical powers; unknown shape
        // SKIP Invoke: Skill/Self shape with vars={'Energy', 'Summon'} powers=set() not recognized
        // SKIP Juggling: Power card with 0 canonical powers; unknown shape
        // SKIP KnifeTrap: Skill/AnyEnemy shape with vars={'Calculated', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP LanternKey: unknown shape: type=Quest target=Self vars=set() powers=set()
        // SKIP Largesse: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP LeadingStrike: has richer match-arm in combat.rs; let it run
        // SKIP LegSweep: has richer match-arm in combat.rs; let it run
        // SKIP LegionOfBone: unknown shape: type=Skill target=AllAllies vars={'Summon'} powers=set()
        // SKIP Loop: Power card with 0 canonical powers; unknown shape
        // SKIP MachineLearning: Power card with 0 canonical powers; unknown shape
        // SKIP Malaise: Skill/AnyEnemy shape with vars=set() powers=set() not recognized
        // SKIP Mangle: has richer match-arm in combat.rs; let it run
        // SKIP MasterPlanner: Power card with 0 canonical powers; unknown shape
        // SKIP Mayhem: Power card with 0 canonical powers; unknown shape
        // SKIP Melancholy: Skill/Self shape with vars={'Block', 'Energy'} powers=set() not recognized
        // SKIP MementoMori: Attack to single enemy without Damage var
        // SKIP Mimic: Skill/Self shape with vars={'CalculatedBlock', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP MindBlast: Attack to single enemy without Damage var
        // SKIP Mirage: Skill/Self shape with vars={'CalculatedBlock', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP Modded: Skill/Self shape with vars={'Repeat', 'Cards'} powers=set() not recognized
        // SKIP MoltenFist: has richer match-arm in combat.rs; let it run
        // SKIP MonarchsGaze: Power card with 0 canonical powers; unknown shape
        // SKIP Monologue: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP MultiCast: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Murder: Attack to single enemy without Damage var
        // SKIP NecroMastery: Power card with 0 canonical powers; unknown shape
        // SKIP Nightmare: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP NoEscape: Skill/AnyEnemy shape with vars={'Dynamic', 'Calculated', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP Nostalgia: Power card with 0 canonical powers; unknown shape
        // SKIP NotYet: Skill/Self shape with vars={'Heal'} powers=set() not recognized
        // SKIP NoxiousFumes: Power card with 0 canonical powers; unknown shape
        // SKIP Offering: Skill/Self shape with vars={'Energy', 'HpLoss', 'Cards'} powers=set() not recognized
        // SKIP OneTwoPunch: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Orbit: Power card with 0 canonical powers; unknown shape
        // SKIP Pagestorm: Power card with 0 canonical powers; unknown shape
        // SKIP PaleBlueDot: Power card with 0 canonical powers; unknown shape
        // SKIP Panache: Power card with 0 canonical powers; unknown shape
        // SKIP PanicButton: Skill/Self shape with vars={'Block', 'Dynamic'} powers=set() not recognized
        // SKIP PerfectedStrike: has richer match-arm in combat.rs; let it run
        // SKIP PiercingWail: Skill/AllEnemies with vars={'Dynamic'} powers=set() not recognized
        // SKIP PillarOfCreation: Power card with 0 canonical powers; unknown shape
        // SKIP PoisonedStab: has richer match-arm in combat.rs; let it run
        // SKIP Poke: Attack to single enemy without Damage var
        // SKIP PommelStrike: has richer match-arm in combat.rs; let it run
        // SKIP PreciseCut: Attack to single enemy without Damage var
        // SKIP PrimalForce: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Prolong: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Protector: Attack to single enemy without Damage var
        // SKIP PullAggro: Skill/Self shape with vars={'Block', 'Summon'} powers=set() not recognized
        // SKIP Putrefy: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Pyre: Power card with 0 canonical powers; unknown shape
        // SKIP Quadcast: Skill/Self shape with vars={'Repeat'} powers=set() not recognized
        // SKIP Quasar: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Rage: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Rainbow: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Rally: unknown shape: type=Skill target=AllAllies vars={'Block'} powers=set()
        // SKIP Rattle: Attack to single enemy without Damage var
        // SKIP Reanimate: Skill/Self shape with vars={'Summon'} powers=set() not recognized
        // SKIP ReaperForm: Power card with 0 canonical powers; unknown shape
        // SKIP RefineBlade: Skill/Self shape with vars={'Energy', 'Forge'} powers=set() not recognized
        // SKIP Relax: Skill/Self shape with vars={'Block', 'Energy', 'Cards'} powers=set() not recognized
        // SKIP Rend: Attack to single enemy without Damage var
        // SKIP Restlessness: Skill/Self shape with vars={'Energy', 'Cards'} powers=set() not recognized
        // SKIP Ricochet: has richer match-arm in combat.rs; let it run
        // SKIP RightHandHand: Attack to single enemy without Damage var
        // SKIP RoyalGamble: Skill/Self shape with vars={'Stars'} powers=set() not recognized
        // SKIP Royalties: Power card with 0 canonical powers; unknown shape
        // SKIP Sacrifice: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Scourge: Skill/AnyEnemy shape with vars={'Cards', 'Power'} powers={'DoomPower'} not recognized
        // SKIP Scrawl: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SecretTechnique: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SecretWeapon: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SeekingEdge: Power card with 0 canonical powers; unknown shape
        // SKIP SetupStrike: has richer match-arm in combat.rs; let it run
        // SKIP Shadowmeld: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP SharedFate: Skill/AnyEnemy shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP Shockwave: Skill/AllEnemies with vars={'Dynamic'} powers=set() not recognized
        // SKIP Shroud: Power card with 0 canonical powers; unknown shape
        // SKIP SicEm: Attack to single enemy without Damage var
        // SKIP SignalBoost: Skill/Self shape with vars={'Power'} powers={'SignalBoostPower'} not recognized
        // SKIP Skewer: X-cost single-target attack (would need Repeat over hits)
        // SKIP Snakebite: has richer match-arm in combat.rs; let it run
        // SKIP Snap: Attack to single enemy without Damage var
        // SKIP SoulStorm: Attack to single enemy without Damage var
        // SKIP SpectrumShift: Power card with 0 canonical powers; unknown shape
        // SKIP SpiritOfAsh: Power card with 0 canonical powers; unknown shape
        // SKIP Splash: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP SpoilsMap: unknown shape: type=Quest target=Self vars={'Gold'} powers=set()
        // SKIP SpoilsOfBattle: Skill/Self shape with vars={'Cards', 'Forge'} powers=set() not recognized
        // SKIP Spur: Skill/Self shape with vars={'Heal', 'Summon'} powers=set() not recognized
        // SKIP Squeeze: Attack to single enemy without Damage var
        // SKIP Stack: Skill/Self shape with vars={'CalculatedBlock', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP Stampede: Power card with 0 canonical powers; unknown shape
        // SKIP Stoke: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP StormOfSteel: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Stratagem: Power card with 0 canonical powers; unknown shape
        // SKIP Subroutine: Power card with 0 canonical powers; unknown shape
        // SKIP SummonForth: Skill/Self shape with vars={'Forge'} powers=set() not recognized
        // SKIP Sunder: has richer match-arm in combat.rs; let it run
        // SKIP Supermassive: Attack to single enemy without Damage var
        // SKIP Survivor: has richer match-arm in combat.rs; let it run
        // SKIP SweepingGaze: Random-target attack without Damage var
        // SKIP SwordBoomerang: has richer match-arm in combat.rs; let it run
        // SKIP Synchronize: Skill/Self shape with vars={'Calculated', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP Tank: Power card with 0 canonical powers; unknown shape
        // SKIP Tempest: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Terraforming: Skill/Self shape with vars={'Power'} powers={'VigorPower'} not recognized
        // SKIP TheBomb: Skill/Self shape with vars={'Dynamic'} powers=set() not recognized
        // SKIP TheSealedThrone: Power card with 0 canonical powers; unknown shape
        // SKIP TheSmith: Skill/Self shape with vars={'Forge'} powers=set() not recognized
        // SKIP TimesUp: Attack to single enemy without Damage var
        // SKIP ToolsOfTheTrade: Power card with 0 canonical powers; unknown shape
        // SKIP ToricToughness: Skill/Self shape with vars={'Block', 'Dynamic'} powers=set() not recognized
        // SKIP Tracking: Power card with 0 canonical powers; unknown shape
        // SKIP TrashToTreasure: Power card with 0 canonical powers; unknown shape
        // SKIP Tremble: has richer match-arm in combat.rs; let it run
        // SKIP TrueGrit: has richer match-arm in combat.rs; let it run
        // SKIP Tyranny: Power card with 0 canonical powers; unknown shape
        // SKIP Unleash: Attack to single enemy without Damage var
        // SKIP Unmovable: Power card with 0 canonical powers; unknown shape
        // SKIP Venerate: Skill/Self shape with vars={'Stars'} powers=set() not recognized
        // SKIP Vicious: Power card with 0 canonical powers; unknown shape
        // SKIP Voltaic: Skill/Self shape with vars={'Calculated', 'CalculationExtra', 'CalculationBase'} powers=set() not recognized
        // SKIP WellLaidPlans: Power card with 0 canonical powers; unknown shape
        // SKIP Whirlwind: X-cost AOE (Whirlwind shape — handled in earlier migration)
        // SKIP WhiteNoise: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Wish: Skill/Self shape with vars=set() powers=set() not recognized
        // SKIP Zap: Skill/Self shape with vars=set() powers=set() not recognized
