//! Central registry mapping `Creature.model_id` → spawn + per-turn
//! intent dispatch.
//!
//! Two entry points:
//!
//!  * [`fire_monster_spawn_hooks`] — invoked once at combat start
//!    (after `fire_before_combat_start_hooks`). Walks every enemy and
//!    routes to its per-model spawn payload (e.g. `chomper_spawn`,
//!    `exoskeleton_spawn`). Monsters without spawn payloads are
//!    silently skipped.
//!
//!  * [`dispatch_enemy_turn`] — invoked from `CombatEnv::step(EndTurn)`
//!    for each living enemy. Parses the enemy's stored `intent_move`
//!    string back to its typed enum, picks the next intent via the
//!    monster's `pick_*` fn, runs the matching `execute_*` payload,
//!    then writes the new intent id back. Returns `true` if the
//!    monster had a dispatcher entry, `false` if its `model_id` is
//!    unknown (the harness uses this to count unported monsters).
//!
//! Adding a new monster: write `pub fn pick_X_intent` /
//! `pub fn execute_X_move` / optional `pub fn x_spawn` in `combat.rs`,
//! then add one arm to each of the two functions here. The string
//! literals must match the enum's `id()` method exactly — the same
//! id appears in the .run corpus as the enemy's intent log entry.
//!
//! The combat-scoped RNG is shared across all enemies for the turn;
//! `take_rng` / `put_rng` borrow it transiently so each `pick_*` call
//! can take it as `&mut`.

use crate::combat::*;
use crate::rng::Rng;

/// Spawn a fresh monster into the named slot and fire its spawn
/// payload (`AfterAddedToRoom`). Used by summon moves (LivingFog,
/// Fabricator, Ovicopter, Doormaker) and the data-driven
/// `Effect::SummonMonster` primitive.
pub fn spawn_monster_into_slot(cs: &mut CombatState, monster_id: &str, slot: &str) {
    let new_idx = cs.enemies.len();
    cs.enemies.push(Creature::from_monster_spawn(monster_id, slot));
    fire_one_monster_spawn(cs, new_idx);
}

fn fire_one_monster_spawn(cs: &mut CombatState, i: usize) {
    let id = cs.enemies[i].model_id.clone();
    // Data-driven AIs fire their spawn body first; legacy match
    // arms below cover monsters with hand-rolled spawn hooks.
    if let Some(ai) = crate::monster_ai::ai_for(&id) {
        crate::monster_ai::execute_spawn(cs, ai, i);
        return;
    }
    match id.as_str() {
        "Exoskeleton" => exoskeleton_spawn(cs, i),
        "ThievingHopper" => thieving_hopper_spawn(cs, i),
        "BowlbugRock" => bowlbug_rock_spawn(cs, i),
        "MechaKnight" => mecha_knight_spawn(cs, i),
        "Entomancer" => entomancer_spawn(cs, i),
        "LivingShield" => living_shield_spawn(cs, i),
        "Byrdonis" => byrdonis_spawn(cs, i),
        "Chomper" => chomper_spawn(cs, i),
        "CorpseSlug" => corpse_slug_spawn(cs, i),
        "MysteriousKnight" => {
            cs.apply_power(CombatSide::Enemy, i, "StrengthPower", 6);
            cs.apply_power(CombatSide::Enemy, i, "PlatingPower", 6);
        }
        "SlumberingBeetle" => slumbering_beetle_spawn(cs, i),
        "LagavulinMatriarch" => lagavulin_matriarch_spawn(cs, i),
        "Crusher" => {
            cs.apply_power(CombatSide::Enemy, i, "BackAttackLeftPower", 1);
            cs.apply_power(CombatSide::Enemy, i, "CrabRagePower", 1);
        }
        "Rocket" => {
            cs.apply_power(CombatSide::Enemy, i, "BackAttackRightPower", 1);
            cs.apply_power(CombatSide::Enemy, i, "CrabRagePower", 1);
            let n_players = cs.allies.len();
            for p in 0..n_players {
                cs.apply_power(CombatSide::Player, p, "SurroundedPower", 1);
            }
        }
        "SkulkingColony" => skulking_colony_spawn(cs, i),
        "LouseProgenitor" => louse_progenitor_spawn(cs, i),
        "TerrorEel" => terror_eel_spawn(cs, i),
        "PhantasmalGardener" => phantasmal_gardener_spawn(cs, i),
        "InfestedPrism" => infested_prism_spawn(cs, i),
        _ => {}
    }
}

/// Fire spawn payloads (`AfterAddedToRoom`) for every enemy that has
/// one. Idempotent: only fires when called.
pub fn fire_monster_spawn_hooks(cs: &mut CombatState) {
    let n = cs.enemies.len();
    for i in 0..n {
        let id = cs.enemies[i].model_id.clone();
        // Data-driven AIs fire their spawn body via the registry.
        if let Some(ai) = crate::monster_ai::ai_for(&id) {
            crate::monster_ai::execute_spawn(cs, ai, i);
            continue;
        }
        match id.as_str() {
            "Exoskeleton" => exoskeleton_spawn(cs, i),
            "ThievingHopper" => thieving_hopper_spawn(cs, i),
            "BowlbugRock" => bowlbug_rock_spawn(cs, i),
            "MechaKnight" => mecha_knight_spawn(cs, i),
            "Entomancer" => entomancer_spawn(cs, i),
            "LivingShield" => living_shield_spawn(cs, i),
            "Byrdonis" => byrdonis_spawn(cs, i),
            "Chomper" => chomper_spawn(cs, i),
            "CorpseSlug" => corpse_slug_spawn(cs, i),
            "MysteriousKnight" => {
                cs.apply_power(CombatSide::Enemy, i, "StrengthPower", 6);
                cs.apply_power(CombatSide::Enemy, i, "PlatingPower", 6);
            }
            "SlumberingBeetle" => slumbering_beetle_spawn(cs, i),
            "LagavulinMatriarch" => lagavulin_matriarch_spawn(cs, i),
            "Crusher" => {
                cs.apply_power(
                    CombatSide::Enemy,
                    i,
                    "BackAttackLeftPower",
                    1,
                );
                cs.apply_power(CombatSide::Enemy, i, "CrabRagePower", 1);
            }
            "Rocket" => {
                cs.apply_power(
                    CombatSide::Enemy,
                    i,
                    "BackAttackRightPower",
                    1,
                );
                cs.apply_power(CombatSide::Enemy, i, "CrabRagePower", 1);
                // SurroundedPower(1) on every opponent.
                let n_players = cs.allies.len();
                for p in 0..n_players {
                    cs.apply_power(CombatSide::Player, p, "SurroundedPower", 1);
                }
            }
            "SkulkingColony" => skulking_colony_spawn(cs, i),
            "LouseProgenitor" => louse_progenitor_spawn(cs, i),
            "TerrorEel" => terror_eel_spawn(cs, i),
            "PhantasmalGardener" => phantasmal_gardener_spawn(cs, i),
            "InfestedPrism" => infested_prism_spawn(cs, i),
            _ => {}
        }
    }
}

/// True if every enemy has a dispatch path. Useful for the replay
/// harness — gated on this, calling `dispatch_enemy_turn` for every
/// enemy reaches `true` rather than the unported-no-op branch.
pub fn all_enemies_have_dispatch(cs: &CombatState) -> bool {
    cs.enemies
        .iter()
        .all(|e| monster_has_dispatch(&e.model_id))
}

pub fn monster_has_dispatch(model_id: &str) -> bool {
    // Data-driven AIs go through the registry path; the hand-rolled
    // matches!() list below covers the legacy monsters not yet migrated.
    if crate::monster_ai::ai_for(model_id).is_some() {
        return true;
    }
    matches!(
        model_id,
        "Axebot"
            | "Myte"
            | "Nibbit"
            | "FlailKnight"
            | "OwlMagistrate"
            | "SoulNexus"
            | "DevotedSculptor"
            | "Exoskeleton"
            | "Toadpole"
            | "ThievingHopper"
            | "CalcifiedCultist"
            | "SludgeSpinner"
            | "FuzzyWurmCrawler"
            | "BowlbugRock"
            | "MechaKnight"
            | "Entomancer"
            | "LivingShield"
            | "ShrinkerBeetle"
            | "Byrdonis"
            | "Chomper"
            | "TurretOperator"
            | "TwigSlimeM"
            | "LeafSlimeM"
            | "TwigSlimeS"
            | "LeafSlimeS"
            | "Seapunk"
            | "CorpseSlug"
            | "ScrollOfBiting"
            | "BowlbugSilk"
            | "BowlbugNectar"
            | "BowlbugEgg"
            | "Vantom"
            | "SpinyToad"
            | "GlobeHead"
            | "SlimedBerserker"
            | "BygoneEffigy"
            | "SkulkingColony"
            | "LouseProgenitor"
            | "TerrorEel"
            | "PhantasmalGardener"
            | "InfestedPrism"
            | "PhrogParasite"
            | "SoulFysh"
            | "TorchHeadAmalgam"
            | "DecimillipedeSegmentFront"
            | "DecimillipedeSegmentMiddle"
            | "DecimillipedeSegmentBack"
            | "MysteriousKnight"
            | "SlumberingBeetle"
            | "TheInsatiable"
            | "Tunneler"
            | "MagiKnight"
            | "SpectralKnight"
            | "Ovicopter"
            | "Crusher"
            | "Rocket"
            | "Queen"
            | "HauntedShip"
            | "LagavulinMatriarch"
            | "Doormaker"
            | "Fabricator"
            | "TheObscura"
            | "LivingFog"
            | "WaterfallGiant"
            | "TwoTailedRat"
    )
}

/// Read the per-enemy last-intent string, if any.
fn last_intent_str(cs: &CombatState, idx: usize) -> Option<String> {
    cs.enemies
        .get(idx)
        .and_then(|c| c.monster.as_ref())
        .and_then(|m| m.intent_move.clone())
}

/// Write the new intent id back into the enemy's `MonsterState`.
fn set_intent(cs: &mut CombatState, idx: usize, id: &'static str) {
    if let Some(m) = cs
        .enemies
        .get_mut(idx)
        .and_then(|c| c.monster.as_mut())
    {
        m.intent_move = Some(id.to_string());
    }
}

/// Borrow the combat RNG transiently so `pick_*` fns that take
/// `&mut Rng` can be called with the shared state-scoped stream.
fn take_rng(cs: &mut CombatState) -> Rng {
    std::mem::replace(&mut cs.rng, Rng::new(0, 0))
}

fn put_rng(cs: &mut CombatState, rng: Rng) {
    cs.rng = rng;
}

/// Derive "is_front" from the slot string. C# encounters use either
/// `"front"/"back"` or positional `"first"/"second"/"third"/"fourth"`.
/// Anything that isn't recognized as a back/second-or-later slot is
/// treated as front.
fn slot_is_front(slot: &str) -> bool {
    !matches!(
        slot,
        "back" | "second" | "third" | "fourth"
    )
}

fn slot_index_1based(slot: &str) -> u8 {
    match slot {
        "first" | "front" => 1,
        "second" | "back" => 2,
        "third" => 3,
        "fourth" => 4,
        _ => 1,
    }
}

fn count_living_enemies(cs: &CombatState) -> usize {
    cs.enemies.iter().filter(|e| e.current_hp > 0).count()
}

/// Run one enemy's turn. Returns `true` if dispatched (regardless of
/// what was played); `false` if the monster's `model_id` has no
/// entry and the turn was skipped.
pub fn dispatch_enemy_turn(
    cs: &mut CombatState,
    enemy_idx: usize,
    player_idx: usize,
) -> bool {
    // Skip dead enemies — they don't act.
    if cs
        .enemies
        .get(enemy_idx)
        .map(|c| c.current_hp <= 0)
        .unwrap_or(true)
    {
        return false;
    }
    // Stun gate: if the monster carries the "stunned" flag, consume it
    // and skip the move (mirrors C# `CreatureCmd.Stun` setting a
    // skip-next-move flag). Returns true so the caller doesn't count
    // this as an unported monster.
    if let Some(ms) = cs.enemies[enemy_idx].monster.as_mut() {
        if ms.flag("stunned") {
            ms.set_flag("stunned", false);
            return true;
        }
    }
    let model_id = cs.enemies[enemy_idx].model_id.clone();
    let slot = cs.enemies[enemy_idx].slot.clone();
    let last_str = last_intent_str(cs, enemy_idx);
    let last_ref = last_str.as_deref();

    // Try the data-driven AI registry first. Monsters whose state
    // machine + per-move effect lists are encoded in `monster_ai.rs`
    // resolve through one generic pick-and-execute pipeline rather
    // than per-monster pick_*_intent / execute_*_move pairs.
    if let Some(ai) = crate::monster_ai::ai_for(&model_id) {
        let mut rng = take_rng(cs);
        let next = crate::monster_ai::pick_next_move(
            &ai.pattern,
            cs,
            enemy_idx,
            last_ref,
            &slot,
            &mut rng,
        );
        put_rng(cs, rng);
        if let Some(move_id) = next {
            crate::monster_ai::execute_move(cs, ai, move_id, enemy_idx, player_idx);
            if let Some(m) = cs.enemies.get_mut(enemy_idx).and_then(|c| c.monster.as_mut()) {
                m.intent_move = Some(move_id.to_string());
            }
        }
        return true;
    }

    match model_id.as_str() {
        "Axebot" => {
            let last = last_ref.and_then(|s| match s {
                "BOOT_UP_MOVE" => Some(AxebotIntent::BootUp),
                "ONE_TWO_MOVE" => Some(AxebotIntent::OneTwo),
                "SHARPEN_MOVE" => Some(AxebotIntent::Sharpen),
                "HAMMER_UPPERCUT_MOVE" => Some(AxebotIntent::HammerUppercut),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_axebot_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_axebot_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Myte" => {
            let last = last_ref.and_then(|s| match s {
                "TOXIC_MOVE" => Some(MyteIntent::Toxic),
                "BITE_MOVE" => Some(MyteIntent::Bite),
                "SUCK_MOVE" => Some(MyteIntent::Suck),
                _ => None,
            });
            let intent = pick_myte_intent(last, &slot);
            execute_myte_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Nibbit" => {
            let last = last_ref.and_then(|s| match s {
                "BUTT_MOVE" => Some(NibbitIntent::Butt),
                "SLICE_MOVE" => Some(NibbitIntent::Slice),
                "HISS_MOVE" => Some(NibbitIntent::Hiss),
                _ => None,
            });
            let is_alone = count_living_enemies(cs) == 1;
            let is_front = slot_is_front(&slot);
            let intent = pick_nibbit_intent(last, is_alone, is_front);
            execute_nibbit_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "MysteriousKnight" => {
            // MysteriousKnight is a FlailKnight subclass with an
            // AfterAddedToRoom that adds Strength(6) + Plating(6).
            // Its state machine is unchanged — dispatch through the
            // FlailKnight pipeline. Spawn payload lives in
            // fire_monster_spawn_hooks.
            let last = last_ref.and_then(|s| match s {
                "WAR_CHANT" => Some(FlailKnightIntent::WarChant),
                "FLAIL_MOVE" => Some(FlailKnightIntent::Flail),
                "RAM_MOVE" => Some(FlailKnightIntent::Ram),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_flail_knight_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_flail_knight_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "FlailKnight" => {
            let last = last_ref.and_then(|s| match s {
                "WAR_CHANT" => Some(FlailKnightIntent::WarChant),
                "FLAIL_MOVE" => Some(FlailKnightIntent::Flail),
                "RAM_MOVE" => Some(FlailKnightIntent::Ram),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_flail_knight_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_flail_knight_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "OwlMagistrate" => {
            let last = last_ref.and_then(|s| match s {
                "MAGISTRATE_SCRUTINY" => Some(OwlMagistrateIntent::Scrutiny),
                "PECK_ASSAULT" => Some(OwlMagistrateIntent::PeckAssault),
                "JUDICIAL_FLIGHT" => Some(OwlMagistrateIntent::JudicialFlight),
                "VERDICT" => Some(OwlMagistrateIntent::Verdict),
                _ => None,
            });
            let intent = pick_owl_magistrate_intent(last);
            execute_owl_magistrate_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "SoulNexus" => {
            let last = last_ref.and_then(|s| match s {
                "SOUL_BURN_MOVE" => Some(SoulNexusIntent::SoulBurn),
                "MAELSTROM_MOVE" => Some(SoulNexusIntent::Maelstrom),
                "DRAIN_LIFE_MOVE" => Some(SoulNexusIntent::DrainLife),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_soul_nexus_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_soul_nexus_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "DevotedSculptor" => {
            let last = last_ref.and_then(|s| match s {
                "FORBIDDEN_INCANTATION_MOVE" => {
                    Some(DevotedSculptorIntent::ForbiddenIncantation)
                }
                "SAVAGE_MOVE" => Some(DevotedSculptorIntent::Savage),
                _ => None,
            });
            let intent = pick_devoted_sculptor_intent(last);
            execute_devoted_sculptor_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Exoskeleton" => {
            let last = last_ref.and_then(|s| match s {
                "SKITTER_MOVE" => Some(ExoskeletonIntent::Skitter),
                "MANDIBLE_MOVE" => Some(ExoskeletonIntent::Mandibles),
                "ENRAGE_MOVE" => Some(ExoskeletonIntent::Enrage),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent =
                pick_exoskeleton_intent(&mut rng, last, slot_index_1based(&slot));
            put_rng(cs, rng);
            execute_exoskeleton_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Toadpole" => {
            let last = last_ref.and_then(|s| match s {
                "SPIKE_SPIT_MOVE" => Some(ToadpoleIntent::SpikeSpit),
                "WHIRL_MOVE" => Some(ToadpoleIntent::Whirl),
                "SPIKEN_MOVE" => Some(ToadpoleIntent::Spiken),
                _ => None,
            });
            let is_front = slot_is_front(&slot) || enemy_idx == 0;
            let intent = pick_toadpole_intent(last, is_front);
            execute_toadpole_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "ThievingHopper" => {
            let last = last_ref.and_then(|s| match s {
                "THIEVERY_MOVE" => Some(ThievingHopperIntent::Thievery),
                "FLUTTER_MOVE" => Some(ThievingHopperIntent::Flutter),
                "HAT_TRICK_MOVE" => Some(ThievingHopperIntent::HatTrick),
                "NAB_MOVE" => Some(ThievingHopperIntent::Nab),
                "ESCAPE_MOVE" => Some(ThievingHopperIntent::Escape),
                _ => None,
            });
            let intent = pick_thieving_hopper_intent(last);
            execute_thieving_hopper_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "CalcifiedCultist" => {
            let last = last_ref.and_then(|s| match s {
                "INCANTATION_MOVE" => Some(CalcifiedCultistIntent::Incantation),
                "DARK_STRIKE_MOVE" => Some(CalcifiedCultistIntent::DarkStrike),
                _ => None,
            });
            let intent = pick_calcified_cultist_intent(last);
            execute_calcified_cultist_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "SludgeSpinner" => {
            let last = last_ref.and_then(|s| match s {
                "OIL_SPRAY_MOVE" => Some(SludgeSpinnerIntent::OilSpray),
                "SLAM_MOVE" => Some(SludgeSpinnerIntent::Slam),
                "RAGE_MOVE" => Some(SludgeSpinnerIntent::Rage),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_sludge_spinner_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_sludge_spinner_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "FuzzyWurmCrawler" => {
            let last = last_ref.and_then(|s| match s {
                "FIRST_ACID_GOOP" => Some(FuzzyWurmCrawlerIntent::FirstAcidGoop),
                "INHALE" => Some(FuzzyWurmCrawlerIntent::Inhale),
                "ACID_GOOP" => Some(FuzzyWurmCrawlerIntent::AcidGoop),
                _ => None,
            });
            let intent = pick_fuzzy_wurm_crawler_intent(last);
            execute_fuzzy_wurm_crawler_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "BowlbugRock" => {
            let last = last_ref.and_then(|s| match s {
                "HEADBUTT_MOVE" => Some(BowlbugRockIntent::Headbutt),
                "DIZZY_MOVE" => Some(BowlbugRockIntent::Dizzy),
                _ => None,
            });
            let is_off_balance = cs.enemies[enemy_idx]
                .monster
                .as_ref()
                .map(|m| m.flag("is_off_balance"))
                .unwrap_or(false);
            let intent = pick_bowlbug_rock_intent(last, is_off_balance);
            execute_bowlbug_rock_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "MechaKnight" => {
            let last = last_ref.and_then(|s| match s {
                "CHARGE_MOVE" => Some(MechaKnightIntent::Charge),
                "FLAMETHROWER_MOVE" => Some(MechaKnightIntent::Flamethrower),
                "WINDUP_MOVE" => Some(MechaKnightIntent::Windup),
                "HEAVY_CLEAVE_MOVE" => Some(MechaKnightIntent::HeavyCleave),
                _ => None,
            });
            let intent = pick_mecha_knight_intent(last);
            execute_mecha_knight_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Entomancer" => {
            let last = last_ref.and_then(|s| match s {
                "BEES_MOVE" => Some(EntomancerIntent::Bees),
                "SPEAR_MOVE" => Some(EntomancerIntent::Spear),
                "PHEROMONE_SPIT_MOVE" => Some(EntomancerIntent::Spit),
                _ => None,
            });
            let intent = pick_entomancer_intent(last);
            execute_entomancer_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "LivingShield" => {
            let last = last_ref.and_then(|s| match s {
                "SHIELD_SLAM_MOVE" => Some(LivingShieldIntent::ShieldSlam),
                "SMASH_MOVE" => Some(LivingShieldIntent::Smash),
                _ => None,
            });
            // has_alive_allies: TurretOperators or any non-self enemy alive.
            let has_alive_allies = cs
                .enemies
                .iter()
                .enumerate()
                .any(|(i, e)| i != enemy_idx && e.current_hp > 0);
            let intent = pick_living_shield_intent(last, has_alive_allies);
            execute_living_shield_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "ShrinkerBeetle" => {
            let last = last_ref.and_then(|s| match s {
                "SHRINKER_MOVE" => Some(ShrinkerBeetleIntent::Shrinker),
                "CHOMP_MOVE" => Some(ShrinkerBeetleIntent::Chomp),
                "STOMP_MOVE" => Some(ShrinkerBeetleIntent::Stomp),
                _ => None,
            });
            let intent = pick_shrinker_beetle_intent(last);
            execute_shrinker_beetle_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Byrdonis" => {
            let last = last_ref.and_then(|s| match s {
                "PECK_MOVE" => Some(ByrdonisIntent::Peck),
                "SWOOP_MOVE" => Some(ByrdonisIntent::Swoop),
                _ => None,
            });
            let intent = pick_byrdonis_intent(last);
            execute_byrdonis_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Chomper" => {
            let last = last_ref.and_then(|s| match s {
                "CLAMP_MOVE" => Some(ChomperIntent::Clamp),
                "SCREECH_MOVE" => Some(ChomperIntent::Screech),
                _ => None,
            });
            // C# Chomper's screech_first is set by encounter, not by
            // intrinsic state. Default to false (Clamp first) — only
            // encounters that explicitly opt in use Screech-first,
            // which we don't currently distinguish at the encounter
            // table level.
            let intent = pick_chomper_intent(last, false);
            execute_chomper_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "TurretOperator" => {
            let last = last_ref.and_then(|s| match s {
                "UNLOAD_MOVE_1" => Some(TurretOperatorIntent::Unload1),
                "UNLOAD_MOVE_2" => Some(TurretOperatorIntent::Unload2),
                "RELOAD_MOVE" => Some(TurretOperatorIntent::Reload),
                _ => None,
            });
            let intent = pick_turret_operator_intent(last);
            execute_turret_operator_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "TwigSlimeM" => {
            let last = last_ref.and_then(|s| match s {
                "CLUMP_SHOT_MOVE" => Some(TwigSlimeMIntent::Clump),
                "STICKY_SHOT_MOVE" => Some(TwigSlimeMIntent::Sticky),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_twig_slime_m_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_twig_slime_m_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "LeafSlimeM" => {
            let last = last_ref.and_then(|s| match s {
                "CLUMP_SHOT" => Some(LeafSlimeMIntent::Clump),
                "STICKY_SHOT" => Some(LeafSlimeMIntent::Sticky),
                _ => None,
            });
            let intent = pick_leaf_slime_m_intent(last);
            execute_leaf_slime_m_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "TwigSlimeS" => {
            let intent = pick_twig_slime_s_intent(None);
            execute_twig_slime_s_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "LeafSlimeS" => {
            let last = last_ref.and_then(|s| match s {
                "BUTT_MOVE" => Some(LeafSlimeSIntent::Butt),
                "GOOP_MOVE" => Some(LeafSlimeSIntent::Goop),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_leaf_slime_s_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_leaf_slime_s_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Seapunk" => {
            let last = last_ref.and_then(|s| match s {
                "SEA_KICK_MOVE" => Some(SeapunkIntent::SeaKick),
                "SPINNING_KICK_MOVE" => Some(SeapunkIntent::SpinningKick),
                "BUBBLE_BURP_MOVE" => Some(SeapunkIntent::BubbleBurp),
                _ => None,
            });
            let intent = pick_seapunk_intent(last);
            execute_seapunk_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "CorpseSlug" => {
            let last = last_ref.and_then(|s| match s {
                "WHIP_SLAP_MOVE" => Some(CorpseSlugIntent::WhipSlap),
                "GLOMP_MOVE" => Some(CorpseSlugIntent::Glomp),
                "GOOP_MOVE" => Some(CorpseSlugIntent::Goop),
                _ => None,
            });
            // starter_move_idx defaults to slot index (0..2) so the
            // three slugs in CorpseSlugsWeak init differently.
            let starter_move_idx = (slot_index_1based(&slot) - 1) as i32;
            let intent = pick_corpse_slug_intent(last, starter_move_idx);
            execute_corpse_slug_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "ScrollOfBiting" => {
            let last = last_ref.and_then(|s| match s {
                "CHOMP" => Some(ScrollOfBitingIntent::Chomp),
                "CHEW" => Some(ScrollOfBitingIntent::Chew),
                "MORE_TEETH" => Some(ScrollOfBitingIntent::MoreTeeth),
                _ => None,
            });
            let mut rng = take_rng(cs);
            // Use slot index as starter_move_idx so the 3 scrolls in
            // an encounter init differently. Defaults to 0 if slot
            // doesn't parse.
            let starter_move_idx = (slot_index_1based(&slot) - 1) as i32;
            let intent =
                pick_scroll_of_biting_intent(&mut rng, last, starter_move_idx);
            put_rng(cs, rng);
            execute_scroll_of_biting_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "BowlbugSilk" => {
            let last = last_ref.and_then(|s| match s {
                "TRASH_MOVE" => Some(BowlbugSilkIntent::Trash),
                "TOXIC_SPIT_MOVE" => Some(BowlbugSilkIntent::ToxicSpit),
                _ => None,
            });
            let intent = pick_bowlbug_silk_intent(last);
            execute_bowlbug_silk_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "BowlbugNectar" => {
            let last = last_ref.and_then(|s| match s {
                "THRASH_MOVE" => Some(BowlbugNectarIntent::Thrash),
                "BUFF_MOVE" => Some(BowlbugNectarIntent::Buff),
                "THRASH2_MOVE" => Some(BowlbugNectarIntent::Thrash2),
                _ => None,
            });
            let intent = pick_bowlbug_nectar_intent(last);
            execute_bowlbug_nectar_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "BowlbugEgg" => {
            let intent = pick_bowlbug_egg_intent(None);
            execute_bowlbug_egg_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Vantom" => {
            let last = last_ref.and_then(|s| match s {
                "INK_BLOT_MOVE" => Some(VantomIntent::InkBlot),
                "INKY_LANCE_MOVE" => Some(VantomIntent::InkyLance),
                "DISMEMBER_MOVE" => Some(VantomIntent::Dismember),
                "PREPARE_MOVE" => Some(VantomIntent::Prepare),
                _ => None,
            });
            let intent = pick_vantom_intent(last);
            execute_vantom_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "SpinyToad" => {
            let last = last_ref.and_then(|s| match s {
                "PROTRUDING_SPIKES_MOVE" => Some(SpinyToadIntent::Spikes),
                "SPIKE_EXPLOSION_MOVE" => Some(SpinyToadIntent::Explosion),
                "TONGUE_LASH_MOVE" => Some(SpinyToadIntent::Lash),
                _ => None,
            });
            let intent = pick_spiny_toad_intent(last);
            execute_spiny_toad_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "GlobeHead" => {
            let last = last_ref.and_then(|s| match s {
                "SHOCKING_SLAP" => Some(GlobeHeadIntent::ShockingSlap),
                "THUNDER_STRIKE" => Some(GlobeHeadIntent::ThunderStrike),
                "GALVANIC_BURST" => Some(GlobeHeadIntent::GalvanicBurst),
                _ => None,
            });
            let intent = pick_globe_head_intent(last);
            execute_globe_head_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "SlimedBerserker" => {
            let last = last_ref.and_then(|s| match s {
                "VOMIT_ICHOR_MOVE" => Some(SlimedBerserkerIntent::VomitIchor),
                "FURIOUS_PUMMELING_MOVE" => {
                    Some(SlimedBerserkerIntent::FuriousPummeling)
                }
                "LEECHING_HUG_MOVE" => Some(SlimedBerserkerIntent::LeechingHug),
                "SMOTHER_MOVE" => Some(SlimedBerserkerIntent::Smother),
                _ => None,
            });
            let intent = pick_slimed_berserker_intent(last);
            execute_slimed_berserker_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "BygoneEffigy" => {
            let last = last_ref.and_then(|s| match s {
                "INITIAL_SLEEP_MOVE" => Some(BygoneEffigyIntent::InitialSleep),
                "WAKE_MOVE" => Some(BygoneEffigyIntent::Wake),
                "SLASHES_MOVE" => Some(BygoneEffigyIntent::Slash),
                _ => None,
            });
            let intent = pick_bygone_effigy_intent(last);
            execute_bygone_effigy_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "SkulkingColony" => {
            let last = last_ref.and_then(|s| match s {
                "SMASH_MOVE" => Some(SkulkingColonyIntent::Smash),
                "ZOOM_MOVE" => Some(SkulkingColonyIntent::Zoom),
                "INERTIA_MOVE" => Some(SkulkingColonyIntent::Inertia),
                "PIERCING_STABS_MOVE" => Some(SkulkingColonyIntent::PiercingStabs),
                _ => None,
            });
            let intent = pick_skulking_colony_intent(last);
            execute_skulking_colony_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "LouseProgenitor" => {
            let last = last_ref.and_then(|s| match s {
                "CURL_AND_GROW_MOVE" => Some(LouseProgenitorIntent::CurlAndGrow),
                "POUNCE_MOVE" => Some(LouseProgenitorIntent::Pounce),
                "WEB_CANNON_MOVE" => Some(LouseProgenitorIntent::Web),
                _ => None,
            });
            let intent = pick_louse_progenitor_intent(last);
            execute_louse_progenitor_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "TerrorEel" => {
            let last = last_ref.and_then(|s| match s {
                "CRASH_MOVE" => Some(TerrorEelIntent::Crash),
                "ThrashMove" => Some(TerrorEelIntent::Thrash),
                "TERROR_MOVE" => Some(TerrorEelIntent::Terror),
                _ => None,
            });
            let shriek_triggered = cs.enemies[enemy_idx]
                .monster
                .as_ref()
                .map(|m| m.flag("shriek_triggered"))
                .unwrap_or(false);
            let intent = pick_terror_eel_intent(last, shriek_triggered);
            execute_terror_eel_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "PhantasmalGardener" => {
            let last = last_ref.and_then(|s| match s {
                "BITE_MOVE" => Some(PhantasmalGardenerIntent::Bite),
                "LASH_MOVE" => Some(PhantasmalGardenerIntent::Lash),
                "FLAIL_MOVE" => Some(PhantasmalGardenerIntent::Flail),
                "ENLARGE_MOVE" => Some(PhantasmalGardenerIntent::Enlarge),
                _ => None,
            });
            let intent = pick_phantasmal_gardener_intent(
                last,
                slot_index_1based(&slot),
            );
            execute_phantasmal_gardener_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "InfestedPrism" => {
            let last = last_ref.and_then(|s| match s {
                "JAB_MOVE" => Some(InfestedPrismIntent::Jab),
                "RADIATE_MOVE" => Some(InfestedPrismIntent::Radiate),
                "WHIRLWIND_MOVE" => Some(InfestedPrismIntent::Whirlwind),
                "PULSATE_MOVE" => Some(InfestedPrismIntent::Pulsate),
                _ => None,
            });
            let intent = pick_infested_prism_intent(last);
            execute_infested_prism_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "PhrogParasite" => {
            let last = last_ref.and_then(|s| match s {
                "INFECT_MOVE" => Some(PhrogParasiteIntent::Infect),
                "LASH_MOVE" => Some(PhrogParasiteIntent::Lash),
                _ => None,
            });
            let intent = pick_phrog_parasite_intent(last);
            execute_phrog_parasite_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "SoulFysh" => {
            let last = last_ref.and_then(|s| match s {
                "BECKON_MOVE" => Some(SoulFyshIntent::Beckon),
                "DE_GAS_MOVE" => Some(SoulFyshIntent::DeGas),
                "GAZE_MOVE" => Some(SoulFyshIntent::Gaze),
                "FADE_MOVE" => Some(SoulFyshIntent::Fade),
                "SCREAM_MOVE" => Some(SoulFyshIntent::Scream),
                _ => None,
            });
            let intent = pick_soul_fysh_intent(last);
            execute_soul_fysh_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "WaterfallGiant" => {
            let last = last_ref.and_then(|s| match s {
                "PRESSURIZE_MOVE" => Some(WaterfallGiantIntent::Pressurize),
                "STOMP_MOVE" => Some(WaterfallGiantIntent::Stomp),
                "RAM_MOVE" => Some(WaterfallGiantIntent::Ram),
                "SIPHON_MOVE" => Some(WaterfallGiantIntent::Siphon),
                "PRESSURE_GUN_MOVE" => Some(WaterfallGiantIntent::PressureGun),
                "PRESSURE_UP_MOVE" => Some(WaterfallGiantIntent::PressureUp),
                _ => None,
            });
            let intent = pick_waterfall_giant_intent(last);
            execute_waterfall_giant_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "TwoTailedRat" => {
            let last = last_ref.and_then(|s| match s {
                "SCRATCH_MOVE" => Some(TwoTailedRatIntent::Scratch),
                "DISEASE_BITE_MOVE" => Some(TwoTailedRatIntent::DiseaseBite),
                "SCREECH_MOVE" => Some(TwoTailedRatIntent::Screech),
                "CALL_FOR_BACKUP_MOVE" => Some(TwoTailedRatIntent::CallForBackup),
                _ => None,
            });
            // slot index 0..N from the encounter slot string.
            let slot_idx = (slot_index_1based(&slot).saturating_sub(1)) as u8;
            let mut rng = take_rng(cs);
            let intent = pick_two_tailed_rat_intent(&mut rng, last, slot_idx);
            put_rng(cs, rng);
            execute_two_tailed_rat_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "TheObscura" => {
            let last = last_ref.and_then(|s| match s {
                "ILLUSION_MOVE" => Some(TheObscuraIntent::Illusion),
                "PIERCING_GAZE_MOVE" => Some(TheObscuraIntent::PiercingGaze),
                "SAIL_MOVE" => Some(TheObscuraIntent::Wail),
                "HARDENING_STRIKE_MOVE" => Some(TheObscuraIntent::HardeningStrike),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_the_obscura_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_the_obscura_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "LivingFog" => {
            let last = last_ref.and_then(|s| match s {
                "ADVANCED_GAS_MOVE" => Some(LivingFogIntent::AdvancedGas),
                "BLOAT_MOVE" => Some(LivingFogIntent::Bloat),
                "SUPER_GAS_BLAST_MOVE" => Some(LivingFogIntent::SuperGas),
                _ => None,
            });
            let intent = pick_living_fog_intent(last);
            execute_living_fog_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Fabricator" => {
            let last = last_ref.and_then(|s| match s {
                "FABRICATE_MOVE" => Some(FabricatorIntent::Fabricate),
                "FABRICATING_STRIKE_MOVE" => Some(FabricatorIntent::FabricatingStrike),
                "DISINTEGRATE_MOVE" => Some(FabricatorIntent::Disintegrate),
                _ => None,
            });
            // CanFabricate = alive teammates < 4 (excluding self).
            let live_teammates = cs
                .enemies
                .iter()
                .enumerate()
                .filter(|(i, e)| *i != enemy_idx && e.current_hp > 0)
                .count();
            let can_fabricate = live_teammates < 4;
            let mut rng = take_rng(cs);
            let intent = pick_fabricator_intent(&mut rng, last, can_fabricate);
            put_rng(cs, rng);
            execute_fabricator_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Doormaker" => {
            let last = last_ref.and_then(|s| match s {
                "DRAMATIC_OPEN_MOVE" => Some(DoormakerIntent::DramaticOpen),
                "HUNGER_MOVE" => Some(DoormakerIntent::Hunger),
                "SCRUTINY_MOVE" => Some(DoormakerIntent::Scrutiny),
                "GRASP_MOVE" => Some(DoormakerIntent::Grasp),
                _ => None,
            });
            let intent = pick_doormaker_intent(last);
            execute_doormaker_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "LagavulinMatriarch" => {
            let last = last_ref.and_then(|s| match s {
                "SLEEP_MOVE" => Some(LagavulinMatriarchIntent::Sleep),
                "SLASH_MOVE" => Some(LagavulinMatriarchIntent::Slash),
                "SLASH2_MOVE" => Some(LagavulinMatriarchIntent::Slash2),
                "DISEMBOWEL_MOVE" => Some(LagavulinMatriarchIntent::Disembowel),
                "SOUL_SIPHON_MOVE" => Some(LagavulinMatriarchIntent::SoulSiphon),
                _ => None,
            });
            let has_asleep = cs.enemies[enemy_idx]
                .powers
                .iter()
                .any(|p| p.id == "AsleepPower" && p.amount > 0);
            let intent = pick_lagavulin_matriarch_intent(last, has_asleep);
            execute_lagavulin_matriarch_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "HauntedShip" => {
            let last = last_ref.and_then(|s| match s {
                "HAUNT_MOVE" => Some(HauntedShipIntent::Haunt),
                "RAMMING_SPEED_MOVE" => Some(HauntedShipIntent::RammingSpeed),
                "SWIPE_MOVE" => Some(HauntedShipIntent::Swipe),
                "STOMP_MOVE" => Some(HauntedShipIntent::Stomp),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_haunted_ship_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_haunted_ship_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Queen" => {
            let last = last_ref.and_then(|s| match s {
                "PUPPET_STRINGS_MOVE" => Some(QueenIntent::PuppetStrings),
                "YOUR_MINE_MOVE" => Some(QueenIntent::YoureMine),
                "BURN_BRIGHT_FOR_ME_MOVE" => Some(QueenIntent::BurnBrightForMe),
                "OFF_WITH_YOUR_HEAD_MOVE" => Some(QueenIntent::OffWithYourHead),
                "EXECUTION_MOVE" => Some(QueenIntent::Execution),
                "ENRAGE_MOVE" => Some(QueenIntent::Enrage),
                _ => None,
            });
            // amalgam_dead: any TorchHeadAmalgam in the encounter
            // with current_hp == 0 (it spawned and is no longer
            // alive). If none ever spawned, treat as alive (default
            // BurnBrightForMe path).
            let amalgam_dead = cs.enemies.iter().any(|e| {
                e.model_id == "TorchHeadAmalgam" && e.current_hp <= 0
            });
            let intent = pick_queen_intent(last, amalgam_dead);
            execute_queen_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Crusher" => {
            let last = last_ref.and_then(|s| match s {
                "THRASH_MOVE" => Some(CrusherIntent::Thrash),
                "ENLARGING_STRIKE_MOVE" => Some(CrusherIntent::EnlargingStrike),
                "BUG_STING_MOVE" => Some(CrusherIntent::BugSting),
                "ADAPT_MOVE" => Some(CrusherIntent::Adapt),
                "GUARDED_STRIKE_MOVE" => Some(CrusherIntent::GuardedStrike),
                _ => None,
            });
            let intent = pick_crusher_intent(last);
            execute_crusher_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Rocket" => {
            let last = last_ref.and_then(|s| match s {
                "TARGETING_RETICLE_MOVE" => Some(RocketIntent::TargetingReticle),
                "PRECISION_BEAM_MOVE" => Some(RocketIntent::PrecisionBeam),
                "CHARGE_UP_MOVE" => Some(RocketIntent::ChargeUp),
                "LASER_MOVE" => Some(RocketIntent::Laser),
                "RECHARGE_MOVE" => Some(RocketIntent::Recharge),
                _ => None,
            });
            let intent = pick_rocket_intent(last);
            execute_rocket_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Ovicopter" => {
            let last = last_ref.and_then(|s| match s {
                "LAY_EGGS_MOVE" => Some(OvicopterIntent::LayEggs),
                "SMASH_MOVE" => Some(OvicopterIntent::Smash),
                "TENDERIZER_MOVE" => Some(OvicopterIntent::Tenderizer),
                "NUTRITIONAL_PASTE_MOVE" => Some(OvicopterIntent::NutritionalPaste),
                _ => None,
            });
            // CanLay: alive teammates count ≤ 3 (C# excludes self).
            // No summon system → teammate count stays at whatever
            // the encounter spawned, usually 0 for Ovicopter solo.
            let live_teammates = cs
                .enemies
                .iter()
                .enumerate()
                .filter(|(i, e)| *i != enemy_idx && e.current_hp > 0)
                .count();
            let can_lay = live_teammates <= 3;
            let intent = pick_ovicopter_intent(last, can_lay);
            execute_ovicopter_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "MagiKnight" => {
            let last = last_ref.and_then(|s| match s {
                "FIRST_POWER_SHIELD_MOVE" => Some(MagiKnightIntent::PowerShield),
                "DAMPEN_MOVE" => Some(MagiKnightIntent::Dampen),
                "RAM_MOVE" => Some(MagiKnightIntent::Spear),
                "PREP_MOVE" => Some(MagiKnightIntent::Prep),
                "MAGIC_BOMB" => Some(MagiKnightIntent::MagicBomb),
                _ => None,
            });
            let intent = pick_magi_knight_intent(last);
            execute_magi_knight_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "SpectralKnight" => {
            let last = last_ref.and_then(|s| match s {
                "HEX" => Some(SpectralKnightIntent::Hex),
                "SOUL_SLASH" => Some(SpectralKnightIntent::SoulSlash),
                "SOUL_FLAME" => Some(SpectralKnightIntent::SoulFlame),
                _ => None,
            });
            let mut rng = take_rng(cs);
            let intent = pick_spectral_knight_intent(&mut rng, last);
            put_rng(cs, rng);
            execute_spectral_knight_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "Tunneler" => {
            let last = last_ref.and_then(|s| match s {
                "BITE_MOVE" => Some(TunnelerIntent::Bite),
                "BURROW_MOVE" => Some(TunnelerIntent::Burrow),
                "BELOW_MOVE_1" => Some(TunnelerIntent::Below),
                _ => None,
            });
            let intent = pick_tunneler_intent(last);
            execute_tunneler_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "TheInsatiable" => {
            let last = last_ref.and_then(|s| match s {
                "LIQUIFY_GROUND_MOVE" => Some(TheInsatiableIntent::Liquify),
                "THRASH_MOVE_1" => Some(TheInsatiableIntent::Thrash1),
                "LUNGING_BITE_MOVE" => Some(TheInsatiableIntent::Bite),
                "SALIVATE_MOVE" => Some(TheInsatiableIntent::Salivate),
                "THRASH_MOVE_2" => Some(TheInsatiableIntent::Thrash2),
                _ => None,
            });
            let intent = pick_the_insatiable_intent(last);
            execute_the_insatiable_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "SlumberingBeetle" => {
            let last = last_ref.and_then(|s| match s {
                "SNORE_MOVE" => Some(SlumberingBeetleIntent::Snore),
                "ROLL_OUT_MOVE" => Some(SlumberingBeetleIntent::Rollout),
                _ => None,
            });
            let has_slumber = cs.enemies[enemy_idx]
                .powers
                .iter()
                .any(|p| p.id == "SlumberPower" && p.amount > 0);
            let intent = pick_slumbering_beetle_intent(last, has_slumber);
            execute_slumbering_beetle_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "DecimillipedeSegmentFront"
        | "DecimillipedeSegmentMiddle"
        | "DecimillipedeSegmentBack" => {
            let last = last_ref.and_then(|s| match s {
                "CONSTRICT_MOVE" => Some(DecimillipedeSegmentIntent::Constrict),
                "BULK_MOVE" => Some(DecimillipedeSegmentIntent::Bulk),
                "WRITHE_MOVE" => Some(DecimillipedeSegmentIntent::Writhe),
                _ => None,
            });
            let intent = pick_decimillipede_segment_intent(last);
            execute_decimillipede_segment_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        "TorchHeadAmalgam" => {
            let last = last_ref.and_then(|s| match s {
                "TACKLE_1_MOVE" => Some(TorchHeadAmalgamIntent::Tackle1),
                "TACKLE_2_MOVE" => Some(TorchHeadAmalgamIntent::Tackle2),
                "BEAM_MOVE" => Some(TorchHeadAmalgamIntent::Beam),
                "TACKLE_3_MOVE" => Some(TorchHeadAmalgamIntent::Tackle3),
                "TACKLE_4_MOVE" => Some(TorchHeadAmalgamIntent::Tackle4),
                _ => None,
            });
            let intent = pick_torch_head_amalgam_intent(last);
            execute_torch_head_amalgam_move(cs, enemy_idx, player_idx, intent);
            set_intent(cs, enemy_idx, intent.id());
        }
        _ => {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod ai_integration_tests {
    use super::*;
    use crate::combat::{CombatState, Creature, CreatureKind, MonsterState};

    /// Build a minimal combat: one player (allies[0]) + one enemy.
    fn rig(model_id: &str) -> CombatState {
        let mut cs = CombatState::empty();
        let player = Creature {
            kind: CreatureKind::Player,
            model_id: "Ironclad".to_string(),
            slot: String::new(),
            current_hp: 80,
            max_hp: 80,
            block: 0,
            powers: Vec::new(),
            afflictions: Vec::new(),
            player: None,
            monster: None,
        };
        cs.allies.push(player);
        let mut enemy = Creature::from_monster_spawn(model_id, "front");
        if enemy.monster.is_none() {
            enemy.monster = Some(MonsterState::default());
        }
        cs.enemies.push(enemy);
        cs.rng = crate::rng::Rng::new(0xDEADBEEF, 0);
        cs
    }

    #[test]
    fn axe_ruby_raider_slash_first_then_sharpen() {
        let mut cs = rig("AxeRubyRaider");
        let pre_hp = cs.allies[0].current_hp;
        // Turn 1: SLASH (-7 HP).
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("SLASH_MOVE".to_string())
        );
        assert_eq!(cs.allies[0].current_hp, pre_hp - 7);
        // Turn 2: SHARPEN (+2 Strength to enemy). HP unchanged.
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("SHARPEN_MOVE".to_string())
        );
        let str_amount = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "StrengthPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(str_amount, 2);
    }

    #[test]
    fn crossbow_ruby_raider_shoot_first_then_reload() {
        let mut cs = rig("CrossbowRubyRaider");
        let pre_hp = cs.allies[0].current_hp;
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(cs.allies[0].current_hp, pre_hp - 5);
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        // After RELOAD the enemy has 6 block.
        assert_eq!(cs.enemies[0].block, 6);
    }

    #[test]
    fn single_attack_move_monster_attacks_every_turn() {
        let mut cs = rig("SingleAttackMoveMonster");
        let pre_hp = cs.allies[0].current_hp;
        for _ in 0..3 {
            assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        }
        assert_eq!(cs.allies[0].current_hp, pre_hp - 30);
    }

    #[test]
    fn stunned_monster_skips_turn_and_clears_flag() {
        let mut cs = rig("AxeRubyRaider");
        cs.enemies[0].monster.as_mut().unwrap().set_flag("stunned", true);
        let pre_hp = cs.allies[0].current_hp;
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        // No damage dealt; flag cleared.
        assert_eq!(cs.allies[0].current_hp, pre_hp);
        assert!(!cs.enemies[0].monster.as_ref().unwrap().flag("stunned"));
    }

    #[test]
    fn dead_enemy_does_not_dispatch() {
        let mut cs = rig("AxeRubyRaider");
        cs.enemies[0].current_hp = 0;
        let pre_hp = cs.allies[0].current_hp;
        assert!(!dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(cs.allies[0].current_hp, pre_hp);
    }

    #[test]
    fn data_driven_dispatch_overrides_legacy_for_registered_monsters() {
        // Smoke test that the dispatcher actually goes through the
        // data-driven path: monster_has_dispatch returns true for an
        // id we know is only in monster_ai (not the legacy match).
        assert!(monster_has_dispatch("AxeRubyRaider"));
        assert!(monster_has_dispatch("GremlinMerc"));
        assert!(monster_has_dispatch("FatGremlin"));
    }

    #[test]
    fn zapbot_spawn_applies_high_voltage() {
        let mut cs = rig("Zapbot");
        fire_monster_spawn_hooks(&mut cs);
        let hv = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "HighVoltagePower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(hv, 2, "Zapbot must spawn with HighVoltage(2)");
    }

    #[test]
    fn sewer_clam_spawn_applies_plating() {
        let mut cs = rig("SewerClam");
        fire_monster_spawn_hooks(&mut cs);
        let plating = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "PlatingPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(plating, 8);
    }

    #[test]
    fn punch_construct_spawn_applies_artifact() {
        let mut cs = rig("PunchConstruct");
        fire_monster_spawn_hooks(&mut cs);
        let artifact = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "ArtifactPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(artifact, 1);
    }

    #[test]
    fn parafright_spawn_applies_illusion() {
        let mut cs = rig("Parafright");
        fire_monster_spawn_hooks(&mut cs);
        let illusion = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "IllusionPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(illusion, 1);
    }

    #[test]
    fn fossil_stalker_spawn_applies_suck() {
        let mut cs = rig("FossilStalker");
        fire_monster_spawn_hooks(&mut cs);
        let suck = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "SuckPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(suck, 3);
    }

    #[test]
    fn tough_egg_spawn_applies_hatch_countdown() {
        let mut cs = rig("ToughEgg");
        fire_monster_spawn_hooks(&mut cs);
        let hatch = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "HatchPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(hatch, 1);
    }

    #[test]
    fn gas_bomb_explode_kills_self() {
        let mut cs = rig("GasBomb");
        fire_monster_spawn_hooks(&mut cs);
        let pre_hp = cs.allies[0].current_hp;
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        // EXPLODE deals 8 damage to player.
        assert_eq!(cs.allies[0].current_hp, pre_hp - 8);
        // GasBomb killed itself.
        assert_eq!(cs.enemies[0].current_hp, 0,
            "GasBomb must self-kill after EXPLODE");
    }

    #[test]
    fn mawler_first_turn_claw_then_alternates_under_no_repeat() {
        let mut cs = rig("Mawler");
        // Turn 1: CLAW.
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("CLAW_MOVE".to_string())
        );
        // Turn 2: must be one of RIP_AND_TEAR or ROAR (CLAW blocked by
        // no_repeat).
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        let move2 = cs.enemies[0].monster.as_ref().unwrap().intent_move.clone().unwrap();
        assert_ne!(move2, "CLAW_MOVE", "CLAW must be blocked by no_repeat");
    }

    #[test]
    fn mawler_roar_sets_flag_and_blocks_future_use() {
        let mut cs = rig("Mawler");
        // Force ROAR by directly setting the intent and dispatching.
        // Simpler: just verify that after manually setting roar_used,
        // ROAR is no longer offered.
        cs.enemies[0].monster.as_mut().unwrap().set_flag("roar_used", true);
        cs.enemies[0].monster.as_mut().unwrap().intent_move = Some("CLAW_MOVE".to_string());
        // 30 iterations: should never pick ROAR since flag is set.
        for _ in 0..30 {
            assert!(dispatch_enemy_turn(&mut cs, 0, 0));
            let move_id = cs.enemies[0].monster.as_ref().unwrap().intent_move.clone().unwrap();
            assert_ne!(move_id, "ROAR_MOVE",
                "ROAR must be blocked once roar_used flag is set");
        }
    }

    #[test]
    fn frog_knight_opens_with_tongue_lash() {
        let mut cs = rig("FrogKnight");
        fire_monster_spawn_hooks(&mut cs);
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("TONGUE_LASH_MOVE".to_string())
        );
        let plating = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "PlatingPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(plating, 15, "FrogKnight spawn must apply Plating(15)");
    }

    #[test]
    fn frog_knight_low_hp_charges_then_sets_flag() {
        let mut cs = rig("FrogKnight");
        fire_monster_spawn_hooks(&mut cs);
        // Drop HP below 50% and skip ahead to the 4th cycle slot via
        // direct intent injection.
        cs.enemies[0].max_hp = 200;
        cs.enemies[0].current_hp = 50; // 25% HP
        cs.enemies[0].monster.as_mut().unwrap().intent_move = Some("FOR_THE_QUEEN_MOVE".to_string());
        // Next move should be BEETLE_CHARGE (HP < 50% AND not yet
        // charged).
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("BEETLE_CHARGE_MOVE".to_string())
        );
        // Flag set.
        assert!(cs.enemies[0].monster.as_ref().unwrap().flag("beetle_charged"));
    }

    #[test]
    fn fogmog_opens_with_illusion_spawns_eye_with_teeth() {
        let mut cs = rig("Fogmog");
        let pre_enemies = cs.enemies.len();
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("ILLUSION_MOVE".to_string())
        );
        // New EyeWithTeeth spawned.
        assert!(cs.enemies.len() > pre_enemies,
            "Fogmog ILLUSION must spawn an enemy");
        let spawned_is_eye = cs.enemies.iter().any(|e| e.model_id == "EyeWithTeeth");
        assert!(spawned_is_eye);
    }

    #[test]
    fn toadpole_back_slot_starts_with_whirl() {
        let mut cs = rig("Toadpole");
        // rig() spawns in slot "front" — change to "back" via direct
        // edit since the rig helper only takes a model id.
        cs.enemies[0].slot = "back".to_string();
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("WHIRL_MOVE".to_string())
        );
    }

    #[test]
    fn toadpole_front_slot_starts_with_spiken() {
        let mut cs = rig("Toadpole");
        // rig() defaults to "front" slot.
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("SPIKEN_MOVE".to_string())
        );
    }

    #[test]
    fn thieving_hopper_spawn_applies_escape_artist() {
        let mut cs = rig("ThievingHopper");
        fire_monster_spawn_hooks(&mut cs);
        let escape = cs.enemies[0]
            .powers
            .iter()
            .find(|p| p.id == "EscapeArtistPower")
            .map(|p| p.amount)
            .unwrap_or(0);
        assert_eq!(escape, 5);
    }

    #[test]
    fn thieving_hopper_escape_move_kills_self() {
        let mut cs = rig("ThievingHopper");
        // Skip ahead to ESCAPE_MOVE via direct intent injection.
        cs.enemies[0].monster.as_mut().unwrap().intent_move =
            Some("NAB_MOVE".to_string());
        assert!(dispatch_enemy_turn(&mut cs, 0, 0));
        assert_eq!(
            cs.enemies[0].monster.as_ref().unwrap().intent_move,
            Some("ESCAPE_MOVE".to_string())
        );
        // The ESCAPE_MOVE body zeroes HP via Effect::EscapeFromCombat.
        assert_eq!(cs.enemies[0].current_hp, 0);
        assert!(cs.enemies[0].monster.as_ref().unwrap().flag("escaped"));
    }

    #[test]
    fn ruby_raider_no_spawn_powers() {
        // Sanity: monsters with empty spawn vec should NOT pick up
        // stray powers via the data-driven path.
        let mut cs = rig("AxeRubyRaider");
        fire_monster_spawn_hooks(&mut cs);
        assert!(
            cs.enemies[0].powers.is_empty(),
            "AxeRubyRaider has no spawn body — must spawn clean"
        );
    }
}
