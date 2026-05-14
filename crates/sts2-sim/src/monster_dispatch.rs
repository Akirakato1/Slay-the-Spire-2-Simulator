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

/// Fire spawn payloads (`AfterAddedToRoom`) for every enemy that has
/// one. Idempotent: only fires when called.
pub fn fire_monster_spawn_hooks(cs: &mut CombatState) {
    let n = cs.enemies.len();
    for i in 0..n {
        let id = cs.enemies[i].model_id.clone();
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
            "SkulkingColony" => skulking_colony_spawn(cs, i),
            "LouseProgenitor" => louse_progenitor_spawn(cs, i),
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
    let model_id = cs.enemies[enemy_idx].model_id.clone();
    let slot = cs.enemies[enemy_idx].slot.clone();
    let last_str = last_intent_str(cs, enemy_idx);
    let last_ref = last_str.as_deref();

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
        _ => {
            return false;
        }
    }
    true
}
