//! Rust-side parallel of the C# oracle host's combat scaffold.
//!
//! Provides a matching API surface (combat_new / combat_add_player /
//! combat_add_enemy / combat_force_card_to_hand / combat_play_card /
//! combat_dump) over the `sts2-sim` library. Together with the C#
//! host they form Phase 3 of the audit plan: drive both simulators
//! through identical inputs and diff the JSON dumps.
//!
//! The JSON schema emitted by `combat_dump` is byte-equivalent to
//! the C# host's `SerializeCombat` output. Id formats are converted
//! at the boundary (C# uses ModelId "CATEGORY.ENTRY"; Rust uses
//! PascalCase "Entry"). The conversion is reversible and uses the
//! same C# `StringHelper.Slugify` rule.

use serde_json::{json, Value};
use sts2_sim::card;
use sts2_sim::character;
use sts2_sim::combat::{
    CardInstance, CardPile, CombatSide, CombatState, Creature, CreatureKind,
    PileType, PlayResult, PlayerState,
};
use sts2_sim::encounter::EncounterData;

/// Wraps a CombatState with the side-channel state the C# host
/// tracks separately (master deck, potions, etc.) so we can serialize
/// a faithful parallel dump.
pub struct RustRig {
    pub combat: CombatState,
    /// Snapshot of the player's master deck taken at
    /// `combat_add_player`. Force-added cards don't append here
    /// (mirrors C# Player.Deck behavior — force_card_to_hand only
    /// modifies PCS.Hand, not Player.Deck).
    pub master_deck: Vec<(String, i32)>,
}

impl RustRig {
    pub fn new() -> Self {
        Self {
            combat: combat_new(),
            master_deck: Vec::new(),
        }
    }
    pub fn add_player(&mut self, character_modelid: &str, seed: u32) {
        combat_add_player(&mut self.combat, character_modelid, seed);
        // Snapshot starter deck.
        let char_rust = modelid_to_rust(character_modelid);
        let cd = character::by_id(&char_rust).unwrap();
        self.master_deck =
            cd.starting_deck.iter().map(|id| (id.clone(), 0)).collect();
    }
    pub fn add_enemy(&mut self, monster_modelid: &str) {
        combat_add_enemy(&mut self.combat, monster_modelid);
    }
    pub fn force_card_to_hand(&mut self, card_modelid: &str, upgrade: i32) {
        combat_force_card_to_hand(&mut self.combat, card_modelid, upgrade);
    }
    /// Grant a relic mid-combat. Mirrors the oracle host's
    /// `combat_grant_relic` RPC — pushes the relic id onto the player's
    /// `relics` list, then applies a curated subset of the run-state
    /// AfterObtained effects to the combat-level creature state.
    /// Oracle's `RelicCmd.Obtain` always fires `relic.AfterObtained()`;
    /// for relics that bump MaxHp / heal / change HP, that mutation is
    /// visible on the combat-frame `Creature` once the oracle dumps,
    /// so the rust mirror needs to apply the same delta to keep parity.
    pub fn grant_relic(&mut self, relic_modelid: &str) {
        let relic_rust = modelid_to_rust(relic_modelid);
        if let Some(ps) = self.combat.allies[0].player.as_mut() {
            if !ps.relics.contains(&relic_rust) {
                ps.relics.push(relic_rust.clone());
            }
        }
        // Apply the AfterObtained body to the combat-level creature.
        // run_state_effects returns the full hook→body table; we only
        // care about the AfterObtained body's combat-visible mutations.
        let Some(arms) = sts2_sim::effects::run_state_effects(&relic_rust) else {
            return;
        };
        for (hook, body) in arms {
            if !matches!(hook, sts2_sim::effects::RunStateHook::AfterObtained) {
                continue;
            }
            for eff in body {
                self.apply_after_obtained_effect(&eff);
            }
        }
    }

    /// Mirror a subset of run-state Effect variants onto the combat
    /// creature's HP/MaxHp. Other effects (gold, potion slots, run-state
    /// pile mutations) are out-of-band for the combat dump; ignored.
    fn apply_after_obtained_effect(&mut self, eff: &sts2_sim::effects::Effect) {
        use sts2_sim::effects::{AmountSpec, Effect};
        let resolve = |a: &AmountSpec| -> i32 {
            match a {
                AmountSpec::Fixed(n) => *n,
                _ => 0,
            }
        };
        let Some(c) = self.combat.allies.get_mut(0) else { return };
        match eff {
            Effect::GainRunStateMaxHp { amount } => {
                let amt = resolve(amount);
                c.max_hp += amt;
                c.current_hp += amt;
            }
            Effect::LoseRunStateMaxHp { amount } => {
                let amt = resolve(amount);
                c.max_hp = (c.max_hp - amt).max(1);
                if c.current_hp > c.max_hp {
                    c.current_hp = c.max_hp;
                }
            }
            Effect::LoseRunStateHp { amount } => {
                let amt = resolve(amount);
                c.current_hp = (c.current_hp - amt).max(0);
            }
            _ => {}
        }
    }
    /// Fire every player-relic's BeforeCombatStart hook. Caller invokes
    /// after granting any relics that should be present "from the start
    /// of combat" — Anchor (10 block), BloodVial (3 HP), etc.
    pub fn fire_before_combat_start(&mut self) {
        self.combat.fire_before_combat_start_hooks();
    }
    pub fn play_card(&mut self, hand_idx: usize, target_idx: Option<usize>) -> bool {
        combat_play_card(&mut self.combat, hand_idx, target_idx)
    }
    /// Play a card targeting an ally (Self/AnyAlly cards).
    pub fn play_card_ally(&mut self, hand_idx: usize, ally_idx: Option<usize>) -> bool {
        let target = ally_idx.map(|i| (CombatSide::Player, i));
        // Credit energy for the play.
        {
            let ps = self.combat.allies[0].player.as_ref().unwrap();
            let Some(c) = ps.hand.cards.get(hand_idx) else { return false };
            let cost = card::by_id(&c.id).map(|d| d.energy_cost).unwrap_or(1).max(0);
            let ps_mut = self.combat.allies[0].player.as_mut().unwrap();
            ps_mut.energy = ps_mut.energy.max(cost);
        }
        matches!(self.combat.play_card(0, hand_idx, target), PlayResult::Ok | PlayResult::Unhandled)
    }
    pub fn dump(&self) -> Value {
        combat_dump_with_master(&self.combat, &self.master_deck)
    }
}

impl Default for RustRig {
    fn default() -> Self { Self::new() }
}

/// Convert a C# ModelId form ("CATEGORY.ENTRY_WITH_UNDERSCORES") to
/// the Rust PascalCase id ("EntryWithUnderscores"). Strips the
/// "CATEGORY." prefix.
pub fn modelid_to_rust(modelid: &str) -> String {
    let entry = match modelid.split_once('.') {
        Some((_cat, e)) => e,
        None => modelid,
    };
    // SCREAMING_SNAKE → PascalCase: each underscore-separated chunk
    // gets its first char uppercased + rest lowercased.
    let mut out = String::with_capacity(entry.len());
    for chunk in entry.split('_') {
        if chunk.is_empty() {
            continue;
        }
        let mut chars = chunk.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
        }
        for c in chars {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// Inverse of `modelid_to_rust`. Prepends `prefix` (e.g., "CARD",
/// "MONSTER", "CHARACTER", "RELIC") + ".".
pub fn rust_to_modelid(pascal: &str, prefix: &str) -> String {
    let mut entry = String::with_capacity(pascal.len() + 4);
    for (i, c) in pascal.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            entry.push('_');
        }
        entry.push(c.to_ascii_uppercase());
    }
    format!("{prefix}.{entry}")
}

/// Build an empty combat state with no players or enemies. Matches
/// `combat_new` on the C# side. We use `CombatState::start` with an
/// empty encounter so allies / enemies start out as empty Vecs.
pub fn combat_new() -> CombatState {
    let fake_enc = EncounterData {
        id: "audit/empty".to_string(),
        room_type: None,
        slots: Vec::new(),
        canonical_monsters: Vec::new(),
        possible_monsters: Vec::new(),
    };
    CombatState::start(&fake_enc, Vec::new(), Vec::new())
}

/// Add the player. Mirrors C# `combat_add_player`. character_modelid
/// is the C# form ("CHARACTER.IRONCLAD"); we convert to "Ironclad"
/// for lookup. `seed` is unused for now (the Rust side doesn't shuffle
/// at population time — we keep deck in canonical order to match the
/// C# rig's post-UnstableShuffle output. Seed parity may be added
/// once we have a Rng harness on both sides.)
pub fn combat_add_player(cs: &mut CombatState, character_modelid: &str, _seed: u32) {
    let char_rust = modelid_to_rust(character_modelid);
    let cd = character::by_id(&char_rust)
        .unwrap_or_else(|| panic!("character {char_rust} not found"));
    let deck: Vec<CardInstance> = cd
        .starting_deck
        .iter()
        .map(|id| {
            let data = card::by_id(id).expect("starter card in registry");
            CardInstance::from_card(data, 0)
        })
        .collect();
    // Add ally creature directly. We bypass PlayerSetup since the
    // CombatState already exists.
    let creature = Creature {
        kind: CreatureKind::Player,
        model_id: cd.id.clone(),
        slot: String::new(),
        current_hp: cd.starting_hp.unwrap_or(80),
        max_hp: cd.starting_hp.unwrap_or(80),
        block: 0,
        powers: Vec::new(),
        afflictions: Vec::new(),
        player: Some(PlayerState {
            draw: CardPile::with_cards(PileType::Draw, deck),
            hand: CardPile::new(PileType::Hand),
            discard: CardPile::new(PileType::Discard),
            exhaust: CardPile::new(PileType::Exhaust),
            play_pile: Vec::new(),
            energy: 0,  // matches C# pre-turn-start (PlayerCombatState.Energy = 0)
            turn_energy: 3,  // Ironclad MaxEnergy
            relics: cd.starting_relics.clone(),
            pending_gold: 0,
            pending_stars: 0,
            orb_queue: Vec::new(),
            orb_slots: 3,
            pending_forge: 0,
            osty: None,
            relic_counters: std::collections::HashMap::new(),
        }),
        monster: None,
    };
    cs.allies.push(creature);
}

/// Add an enemy. `monster_modelid` is C# form ("MONSTER.BIG_DUMMY").
pub fn combat_add_enemy(cs: &mut CombatState, monster_modelid: &str) {
    let m_rust = modelid_to_rust(monster_modelid);
    cs.enemies.push(Creature::from_monster_spawn(&m_rust, ""));
}

/// Inject a card directly into hand. `card_modelid` is C# form.
pub fn combat_force_card_to_hand(
    cs: &mut CombatState,
    card_modelid: &str,
    upgrade_level: i32,
) {
    let card_rust = modelid_to_rust(card_modelid);
    let data = card::by_id(&card_rust)
        .unwrap_or_else(|| panic!("card {card_rust} not found"));
    let inst = CardInstance::from_card(data, upgrade_level);
    let ps = cs.allies[0].player.as_mut().expect("no player");
    ps.hand.cards.push(inst);
}

/// Play a card. Routes through the Rust sim's `play_card`. Target
/// index is into the enemies list.
pub fn combat_play_card(
    cs: &mut CombatState,
    hand_idx: usize,
    target_idx: Option<usize>,
) -> bool {
    let target = target_idx.map(|i| (CombatSide::Enemy, i));
    // For attack cards the player has energy 0 (matches C# pre-turn-
    // start). Force energy so the play succeeds — the C# host doesn't
    // check energy in its play path either. Use the card's energy_cost
    // value to credit the player.
    {
        let ps = cs.allies[0].player.as_ref().unwrap();
        let Some(card) = ps.hand.cards.get(hand_idx) else { return false };
        let cost = card::by_id(&card.id).map(|d| d.energy_cost).unwrap_or(1).max(0);
        let ps_mut = cs.allies[0].player.as_mut().unwrap();
        ps_mut.energy = ps_mut.energy.max(cost);
    }
    matches!(cs.play_card(0, hand_idx, target), PlayResult::Ok | PlayResult::Unhandled)
}

/// Serialize the combat state in the C#-compatible JSON schema.
/// Master deck is supplied separately (the Rust sim doesn't track
/// it on PlayerState — `RustRig` snapshots it at setup).
pub fn combat_dump_with_master(cs: &CombatState, master_deck: &[(String, i32)]) -> Value {
    let allies: Vec<Value> = cs
        .allies
        .iter()
        .map(|c| serialize_creature_with_master(c, master_deck))
        .collect();
    let enemies: Vec<Value> = cs.enemies.iter().map(|c| serialize_creature_with_master(c, &[])).collect();
    let side_int = match cs.current_side {
        CombatSide::None => 0,
        CombatSide::Player => 1,
        CombatSide::Enemy => 2,
    };
    json!({
        "round_number": cs.round_number,
        "current_side": side_int,
        "allies": allies,
        "enemies": enemies,
    })
}

/// Legacy form (no master deck) — kept for callers that don't use
/// `RustRig`. Emits an empty master_deck.
pub fn combat_dump(cs: &CombatState) -> Value {
    combat_dump_with_master(cs, &[])
}

fn serialize_creature_with_master(c: &Creature, master_deck: &[(String, i32)]) -> Value {
    let is_player = c.kind == CreatureKind::Player;
    // Sort powers by id so JSON-array order differences between Rust
    // and C# don't show up as diffs.
    let mut powers: Vec<Value> = c
        .powers
        .iter()
        .map(|p| json!({
            "id": rust_to_modelid(&p.id_trim_power(), "POWER"),
            "amount": p.amount,
        }))
        .collect();
    powers.sort_by(|a, b| {
        a["id"].as_str().unwrap_or("").cmp(b["id"].as_str().unwrap_or(""))
    });
    let mut obj = serde_json::Map::new();
    obj.insert("name".into(), Value::Null);
    obj.insert("current_hp".into(), Value::from(c.current_hp));
    obj.insert("max_hp".into(), Value::from(c.max_hp));
    obj.insert("block".into(), Value::from(c.block));
    obj.insert("is_player".into(), Value::from(is_player));
    obj.insert("powers".into(), Value::Array(powers));
    if is_player {
        if let Some(ps) = &c.player {
            obj.insert("player".into(), serialize_player(ps, master_deck));
        }
    }
    Value::Object(obj)
}

fn serialize_player(ps: &PlayerState, master_deck: &[(String, i32)]) -> Value {
    // Match serialize_pile's sort key so master_deck has the same
    // ordering convention.
    let mut master: Vec<Value> = master_deck
        .iter()
        .map(|(id, upg)| json!({
            "id": rust_to_modelid(id, "CARD"),
            "upgrade_level": upg,
        }))
        .collect();
    master.sort_by(|a, b| {
        let ka = (
            a["id"].as_str().unwrap_or(""),
            a["upgrade_level"].as_i64().unwrap_or(0),
        );
        let kb = (
            b["id"].as_str().unwrap_or(""),
            b["upgrade_level"].as_i64().unwrap_or(0),
        );
        ka.cmp(&kb)
    });
    json!({
        "max_energy_base": ps.turn_energy,
        "energy": ps.energy,
        "stars": ps.pending_stars,
        "hand": serialize_pile(&ps.hand),
        "draw": serialize_pile(&ps.draw),
        "discard": serialize_pile(&ps.discard),
        "exhaust": serialize_pile(&ps.exhaust),
        "play": Value::Array(Vec::new()),  // Rust has no Play pile (atomic in play_card)
        "master_deck": Value::Array(master),
        "relics": ps.relics.iter().map(|r| rust_to_modelid(r, "RELIC")).collect::<Vec<_>>(),
        "potions": Value::Array(vec![Value::Null, Value::Null, Value::Null]),  // 3 empty slots (matches C# default)
    })
}

fn serialize_pile(p: &CardPile) -> Value {
    // Sort by (id, upgrade_level) so parity diffs ignore within-pile
    // ordering (Rust internal Vec orientation differs from C#'s).
    let mut cards: Vec<Value> = p.cards.iter().map(serialize_card).collect();
    cards.sort_by(|a, b| {
        let ka = (
            a["id"].as_str().unwrap_or(""),
            a["upgrade_level"].as_i64().unwrap_or(0),
        );
        let kb = (
            b["id"].as_str().unwrap_or(""),
            b["upgrade_level"].as_i64().unwrap_or(0),
        );
        ka.cmp(&kb)
    });
    Value::Array(cards)
}

fn serialize_card(c: &CardInstance) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), Value::String(rust_to_modelid(&c.id, "CARD")));
    obj.insert("upgrade_level".into(), Value::from(c.upgrade_level));
    if let Some(e) = &c.enchantment {
        obj.insert(
            "enchantment".into(),
            json!({
                "id": rust_to_modelid(&e.id, "ENCHANTMENT"),
                "amount": e.amount,
            }),
        );
    }
    Value::Object(obj)
}


trait PowerIdTrim {
    fn id_trim_power(&self) -> String;
}

impl PowerIdTrim for sts2_sim::combat::PowerInstance {
    fn id_trim_power(&self) -> String {
        // Rust stores power ids as the full class name ("VulnerablePower")
        // and the C# ModelId is "POWER.VULNERABLE_POWER" (Slugify of the
        // full class name). Don't strip the suffix.
        self.id.clone()
    }
}
