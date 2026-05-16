//! sts2-ui — sandbox UI for verifying card / relic / potion / enchantment
//! behavior interactively. Build a deck, pick any subset of the 120
//! monsters as enemies, fight them through real intent execution.
//!
//! Layout:
//!   - Setup phase: search + click to add cards/relics/potions to a
//!     deck. Click a deck card to attach an enchantment. Pick enemies
//!     from the monster registry — defaults to 2× BigDummy if nothing
//!     else is selected so card/relic testing against a punching bag
//!     still "just works."
//!   - Combat phase: top shows enemies + player, bottom shows hand,
//!     side panel lists draw/discard/exhaust piles and the combat log.
//!     Click an enemy first to target it; then click a card to play.
//!     On End Turn, each living enemy runs its AI intent via
//!     `monster_dispatch::dispatch_enemy_turn`.

use eframe::egui;
use std::collections::HashMap;

use sts2_sim::card::{self as cardmod, CardRarity, TargetType};
use sts2_sim::character;
use sts2_sim::combat::{
    CardInstance, CardPile, CombatSide, CombatState, Creature, CreatureKind,
    EnchantmentInstance, PileType, PlayResult, PlayerState,
};
use sts2_sim::enchantment as enchmod;
use sts2_sim::encounter::EncounterData;
use sts2_sim::monster as monstermod;
use sts2_sim::monster_ai;
use sts2_sim::monster_dispatch;
use sts2_sim::potion as potionmod;
use sts2_sim::relic as relicmod;

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("STS2 Combat Sandbox"),
        ..Default::default()
    };
    eframe::run_native(
        "sts2-ui",
        opts,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Box::new(App::default())
        }),
    )
}

// ----------------------------------------------------------------------
// Deck-builder state
// ----------------------------------------------------------------------

#[derive(Clone, Debug)]
struct DeckEntry {
    card_id: String,
    upgrade: i32,
    /// Optional enchantment id + amount.
    enchantment: Option<(String, i32)>,
}

/// Slot labels used when seating successive enemies. Most BySlot
/// patterns key on "first"/"second"/"third"/"fourth" or "front"/"back";
/// arbitrary unique strings work for the rest.
const DEFAULT_SLOTS: &[&str] = &["first", "second", "third", "fourth"];

fn default_dummies() -> Vec<EnemySlot> {
    vec![
        EnemySlot { monster_id: "BigDummy".into(), slot_label: "first".into() },
        EnemySlot { monster_id: "BigDummy".into(), slot_label: "second".into() },
    ]
}

/// One enemy slot in the encounter the user is building. Slot label
/// matters for `BySlot` AI patterns (Inklet, Wriggler, Decimillipede)
/// — for everything else any unique label works.
#[derive(Clone, Debug)]
struct EnemySlot {
    monster_id: String,
    slot_label: String,
}

#[derive(Default)]
struct Builder {
    character_id: String, // "Ironclad" / "Silent" / "Defect" / etc.
    max_hp: i32,
    starting_hp: i32,
    deck: Vec<DeckEntry>,
    relics: Vec<String>,
    /// Indexed potion belt with fixed 3 slots.
    potions: Vec<String>,
    /// Enemies to seat at combat start. Defaults to 2× BigDummy so the
    /// "punching bag for testing cards" use case still works without
    /// clicking through the picker.
    enemies: Vec<EnemySlot>,
    card_filter: String,
    relic_filter: String,
    potion_filter: String,
    enchantment_filter: String,
    monster_filter: String,
    /// Which deck card is selected for enchantment-editing (if any).
    selected_deck_idx: Option<usize>,
}

impl Builder {
    /// Empty-slate starter. No starting deck, no relics, no potions —
    /// every card / relic / potion that ends up in the loadout is
    /// explicitly added through the UI. Use the buttons in the side
    /// panels to add items, the X button to remove them.
    fn new_empty(character_id: &str) -> Self {
        let cd = character::by_id(character_id);
        let max_hp = cd.and_then(|c| c.starting_hp).unwrap_or(80);
        Self {
            character_id: character_id.to_string(),
            max_hp,
            starting_hp: max_hp,
            deck: Vec::new(),
            relics: Vec::new(),
            potions: Vec::new(),
            enemies: default_dummies(),
            ..Default::default()
        }
    }

    /// Populate the deck and relics from the character's starter set.
    /// Wired to the "Load starter deck" button so users who want a
    /// realistic combat can opt in.
    fn load_starter(&mut self) {
        if let Some(cd) = character::by_id(&self.character_id) {
            self.deck = cd
                .starting_deck
                .iter()
                .map(|id| DeckEntry { card_id: id.clone(), upgrade: 0, enchantment: None })
                .collect();
            self.relics = cd.starting_relics.clone();
            self.max_hp = cd.starting_hp.unwrap_or(80);
            self.starting_hp = self.max_hp;
        }
    }
}

// ----------------------------------------------------------------------
// Combat-phase state
// ----------------------------------------------------------------------

struct ActiveCombat {
    cs: CombatState,
    /// Selected enemy index for targeted-attack plays.
    target_enemy: usize,
    /// Per-tick combat log.
    log: Vec<String>,
    /// Side-channel HP snapshot per enemy at last tick so we can
    /// summarize what changed when displaying the log. Length matches
    /// the enemy roster at combat start.
    last_enemy_hp: Vec<i32>,
    last_player_hp: i32,
    last_player_block: i32,
    /// While a `pending_choice` is open, this holds the agent's
    /// in-progress pick set (indices into the choice's pile). Cleared
    /// on resolve. RL training would pass this directly to
    /// resolve_pending_choice; the UI manages it via the choice overlay.
    pending_picks: Vec<usize>,
    /// Stable RNG counter for enemy-turn dispatch. Bumped each enemy
    /// turn so weighted-random patterns don't lock onto the same roll.
    enemy_turn_counter: u32,
}

// ----------------------------------------------------------------------
// App
// ----------------------------------------------------------------------

enum Phase {
    Setup(Builder),
    Combat(Builder, ActiveCombat),
}

struct App {
    phase: Phase,
}

impl Default for App {
    fn default() -> Self {
        Self { phase: Phase::Setup(Builder::new_empty("Ironclad")) }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut transition: Option<Phase> = None;
        match &mut self.phase {
            Phase::Setup(b) => {
                if let Some(next) = render_setup(ctx, b) {
                    transition = Some(next);
                }
            }
            Phase::Combat(b, ac) => {
                if let Some(next) = render_combat(ctx, b, ac) {
                    transition = Some(next);
                }
            }
        }
        if let Some(n) = transition {
            self.phase = n;
        }
    }
}

// ----------------------------------------------------------------------
// Setup phase
// ----------------------------------------------------------------------

fn render_setup(ctx: &egui::Context, b: &mut Builder) -> Option<Phase> {
    let mut start = false;
    egui::TopBottomPanel::top("setup_header").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Deck Builder");
            ui.separator();
            ui.label("Character:");
            for ch in ["Ironclad", "Silent", "Defect", "Regent", "Necrobinder"] {
                if ui.selectable_label(b.character_id == ch, ch).clicked() {
                    // Character switch only updates id + max HP; deck /
                    // relics are NOT auto-replaced. The user is in
                    // control of what lands in the loadout.
                    if let Some(cd) = character::by_id(ch) {
                        b.character_id = ch.to_string();
                        b.max_hp = cd.starting_hp.unwrap_or(80);
                        b.starting_hp = b.max_hp;
                    }
                }
            }
            ui.separator();
            if ui.button("⟲ Load starter deck").on_hover_text(
                "Replace the current loadout with the character's starting \
                deck + starter relics + 80/80 HP. Use this if you want a \
                realistic combat as a baseline.").clicked()
            {
                b.load_starter();
            }
            if ui.button("⌫ Clear all").on_hover_text(
                "Empty the deck, relics, and potions.").clicked()
            {
                b.deck.clear();
                b.relics.clear();
                b.potions.clear();
            }
            ui.separator();
            ui.label("HP:");
            ui.add(egui::DragValue::new(&mut b.starting_hp).clamp_range(1..=999));
            ui.label("/");
            ui.add(egui::DragValue::new(&mut b.max_hp).clamp_range(1..=999));
            if b.starting_hp > b.max_hp { b.starting_hp = b.max_hp; }
            ui.separator();
            let enemy_summary = if b.enemies.is_empty() {
                "no enemies".to_string()
            } else if b.enemies.len() == 1 {
                b.enemies[0].monster_id.clone()
            } else {
                format!("{} enemies", b.enemies.len())
            };
            let start_label = format!("▶ Start Combat (vs {})", enemy_summary);
            let enabled = !b.enemies.is_empty();
            if ui.add_enabled(enabled,
                egui::Button::new(egui::RichText::new(start_label).strong())
            ).on_hover_text(if enabled {
                "Begin combat with the picked enemies."
            } else {
                "Pick at least one enemy in the right panel."
            }).clicked() {
                start = true;
            }
        });
    });

    egui::SidePanel::left("setup_left").min_width(330.0).show(ctx, |ui| {
        ui.heading("Cards");
        ui.horizontal(|ui| {
            ui.label("filter:");
            ui.text_edit_singleline(&mut b.card_filter);
        });
        let filter = b.card_filter.to_ascii_lowercase();
        let character = b.character_id.clone();
        egui::ScrollArea::vertical()
            .id_source("cards_scroll")
            .max_height(ui.available_height() - 8.0)
            .show(ui, |ui| {
                for c in cardmod::ALL_CARDS.iter() {
                    if !filter.is_empty() && !c.id.to_ascii_lowercase().contains(&filter) {
                        continue;
                    }
                    // Hide non-playable categories by default (Status/Curse
                    // can be added by typing them).
                    let category_ok = c.pool == character
                        || c.pool == "Colorless"
                        || !filter.is_empty();
                    if !category_ok { continue; }
                    let label = format!("[{}] {} (cost {})", short_rarity(c.rarity), c.id, c.energy_cost);
                    if ui.button(label).clicked() {
                        b.deck.push(DeckEntry {
                            card_id: c.id.clone(), upgrade: 0, enchantment: None,
                        });
                    }
                }
            });
    });

    egui::SidePanel::right("setup_right").min_width(330.0).show(ctx, |ui| {
        ui.heading("Relics");
        ui.horizontal(|ui| {
            ui.label("filter:");
            ui.text_edit_singleline(&mut b.relic_filter);
        });
        let filter = b.relic_filter.to_ascii_lowercase();
        egui::ScrollArea::vertical()
            .id_source("relics_scroll")
            .max_height(200.0)
            .show(ui, |ui| {
                for r in relicmod::ALL_RELICS.iter() {
                    if !filter.is_empty() && !r.id.to_ascii_lowercase().contains(&filter) {
                        continue;
                    }
                    if ui.button(format!("[{:?}] {}", r.rarity, r.id)).clicked() {
                        b.relics.push(r.id.clone());
                    }
                }
            });
        ui.separator();
        ui.heading("Potions");
        ui.horizontal(|ui| {
            ui.label("filter:");
            ui.text_edit_singleline(&mut b.potion_filter);
        });
        let pf = b.potion_filter.to_ascii_lowercase();
        egui::ScrollArea::vertical()
            .id_source("potions_scroll")
            .max_height(160.0)
            .show(ui, |ui| {
                for p in potionmod::ALL_POTIONS.iter() {
                    if p.id == "DeprecatedPotion" { continue; }
                    if !pf.is_empty() && !p.id.to_ascii_lowercase().contains(&pf) {
                        continue;
                    }
                    let disabled = b.potions.len() >= 3;
                    let resp = ui.add_enabled(!disabled,
                        egui::Button::new(format!("[{:?}] {}", p.rarity, p.id)));
                    if resp.clicked() {
                        b.potions.push(p.id.clone());
                    }
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.heading(format!("Enemies ({})", b.enemies.len()));
            if ui.small_button("Reset to 2× BigDummy").on_hover_text(
                "Restore the original sandbox dummies."
            ).clicked() {
                b.enemies = default_dummies();
            }
            if ui.small_button("Clear").clicked() {
                b.enemies.clear();
            }
        });
        // Selected enemies: show + allow remove + edit slot label.
        let mut remove_enemy: Option<usize> = None;
        for i in 0..b.enemies.len() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("👹 {}", b.enemies[i].monster_id))
                    .color(egui::Color32::from_rgb(255, 140, 120)));
                ui.label("slot:");
                ui.add(egui::TextEdit::singleline(&mut b.enemies[i].slot_label)
                    .desired_width(72.0));
                if ui.small_button(
                    egui::RichText::new("✕").color(egui::Color32::from_rgb(255, 110, 110))
                ).clicked() {
                    remove_enemy = Some(i);
                }
            });
        }
        if let Some(i) = remove_enemy { b.enemies.remove(i); }
        ui.horizontal(|ui| {
            ui.label("filter:");
            ui.text_edit_singleline(&mut b.monster_filter);
        });
        let mf = b.monster_filter.to_ascii_lowercase();
        egui::ScrollArea::vertical()
            .id_source("monsters_scroll")
            .max_height(220.0)
            .show(ui, |ui| {
                for md in monstermod::ALL_MONSTERS.iter() {
                    if !mf.is_empty() && !md.id.to_ascii_lowercase().contains(&mf) {
                        continue;
                    }
                    // Filter abstract base — no instances to seat.
                    if md.id == "DecimillipedeSegment" { continue; }
                    let has_ai = monster_ai::ai_for(&md.id).is_some();
                    let hp = md.max_hp_base.unwrap_or(0);
                    let label = format!(
                        "{}{} (HP {})",
                        if has_ai { "✓ " } else { "? " },
                        md.id,
                        hp,
                    );
                    if ui.button(label)
                        .on_hover_text(if has_ai {
                            "Has AI dispatch — will run intents during enemy turn."
                        } else {
                            "No AI dispatch registered — will sit idle."
                        })
                        .clicked()
                    {
                        let next_slot = DEFAULT_SLOTS
                            .get(b.enemies.len())
                            .copied()
                            .unwrap_or("extra")
                            .to_string();
                        b.enemies.push(EnemySlot {
                            monster_id: md.id.clone(),
                            slot_label: next_slot,
                        });
                    }
                }
            });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading(format!("Deck ({} cards) · Relics ({}) · Potions ({}/3)",
            b.deck.len(), b.relics.len(), b.potions.len()));
        ui.label("Click a deck card to attach an enchantment. Use ✕ to remove.");
        ui.separator();
        // Relics + potions row. Item label shows the id; the ✕ button
        // next to it removes the entry from the loadout.
        ui.horizontal_wrapped(|ui| {
            let mut remove_relic: Option<usize> = None;
            for i in 0..b.relics.len() {
                ui.label(egui::RichText::new(format!("◆ {}", &b.relics[i]))
                    .color(egui::Color32::from_rgb(255, 200, 80)));
                if ui.small_button(
                    egui::RichText::new("✕").color(egui::Color32::from_rgb(255, 110, 110))
                ).clicked() {
                    remove_relic = Some(i);
                }
            }
            if let Some(i) = remove_relic { b.relics.remove(i); }
        });
        ui.horizontal_wrapped(|ui| {
            let mut remove_potion: Option<usize> = None;
            for i in 0..b.potions.len() {
                ui.label(egui::RichText::new(format!("🧪 {}", &b.potions[i]))
                    .color(egui::Color32::from_rgb(120, 200, 255)));
                if ui.small_button(
                    egui::RichText::new("✕").color(egui::Color32::from_rgb(255, 110, 110))
                ).clicked() {
                    remove_potion = Some(i);
                }
            }
            if let Some(i) = remove_potion { b.potions.remove(i); }
        });
        ui.separator();
        let avail = ui.available_height() - 30.0;
        egui::ScrollArea::vertical()
            .id_source("deck_scroll")
            .max_height(avail)
            .show(ui, |ui| {
                let mut to_delete: Option<usize> = None;
                for (i, e) in b.deck.iter_mut().enumerate() {
                    let row = ui.horizontal(|ui| {
                        // Card row.
                        let label = format!("{:>3}.  {}{}",
                            i, e.card_id,
                            if e.upgrade > 0 { "+" } else { "" });
                        let card_resp = ui.add(egui::Button::new(label).min_size([300.0, 0.0].into()));
                        if card_resp.clicked() {
                            b.selected_deck_idx = Some(i);
                        }
                        // Upgrade toggle.
                        let max_up = cardmod::by_id(&e.card_id)
                            .map(|d| d.max_upgrade_level).unwrap_or(0);
                        let can_upgrade = max_up > 0;
                        if ui.add_enabled(can_upgrade,
                            egui::Button::new(if e.upgrade > 0 { "−" } else { "+" }))
                            .clicked()
                        {
                            e.upgrade = if e.upgrade > 0 { 0 } else { max_up };
                        }
                        // Enchantment summary.
                        if let Some((eid, amt)) = &e.enchantment {
                            ui.label(egui::RichText::new(format!("✦ {} ({})", eid, amt))
                                .color(egui::Color32::from_rgb(200, 120, 255)));
                        } else {
                            ui.label("(no enchantment)");
                        }
                        if ui.button(
                            egui::RichText::new("✕").color(egui::Color32::from_rgb(255, 110, 110))
                        ).clicked() {
                            to_delete = Some(i);
                        }
                    });
                    let _ = row;
                }
                if let Some(i) = to_delete {
                    b.deck.remove(i);
                    if b.selected_deck_idx == Some(i) {
                        b.selected_deck_idx = None;
                    }
                }
            });
    });

    // Enchantment picker window when a deck card is selected.
    if let Some(idx) = b.selected_deck_idx {
        let card_id = b.deck.get(idx).map(|e| e.card_id.clone()).unwrap_or_default();
        let mut open = true;
        let mut commit: Option<Option<(String, i32)>> = None;
        let mut staged_amount: i32 = b.deck.get(idx)
            .and_then(|e| e.enchantment.as_ref().map(|(_, a)| *a)).unwrap_or(1);
        egui::Window::new(format!("Attach enchantment → {}", card_id))
            .open(&mut open)
            .resizable(true)
            .default_size([400.0, 500.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("filter:");
                    ui.text_edit_singleline(&mut b.enchantment_filter);
                });
                ui.horizontal(|ui| {
                    ui.label("amount:");
                    ui.add(egui::DragValue::new(&mut staged_amount).clamp_range(0..=10));
                });
                if ui.button(egui::RichText::new("Remove enchantment")
                    .color(egui::Color32::from_rgb(255, 100, 100))).clicked()
                {
                    commit = Some(None);
                }
                ui.separator();
                let ef = b.enchantment_filter.to_ascii_lowercase();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for e in enchmod::ALL_ENCHANTMENTS.iter() {
                        if e.id == "DeprecatedEnchantment" { continue; }
                        if !ef.is_empty() && !e.id.to_ascii_lowercase().contains(&ef) {
                            continue;
                        }
                        if ui.button(&e.id).clicked() {
                            commit = Some(Some((e.id.clone(), staged_amount)));
                        }
                    }
                });
            });
        if !open {
            b.selected_deck_idx = None;
        }
        if let Some(c) = commit {
            if let Some(entry) = b.deck.get_mut(idx) {
                entry.enchantment = c;
            }
            b.selected_deck_idx = None;
        } else if let Some(entry) = b.deck.get_mut(idx) {
            // Persist amount edits even if no enchantment button was clicked.
            if let Some((_, amt)) = entry.enchantment.as_mut() {
                *amt = staged_amount;
            }
        }
    }

    if start {
        return Some(start_combat(b.clone_for_combat()));
    }
    None
}

impl Builder {
    fn clone_for_combat(&self) -> Builder {
        // Avoid Clone derive on Builder since we only need a snapshot.
        Builder {
            character_id: self.character_id.clone(),
            max_hp: self.max_hp,
            starting_hp: self.starting_hp,
            deck: self.deck.clone(),
            relics: self.relics.clone(),
            potions: self.potions.clone(),
            enemies: self.enemies.clone(),
            card_filter: String::new(),
            relic_filter: String::new(),
            potion_filter: String::new(),
            enchantment_filter: String::new(),
            monster_filter: String::new(),
            selected_deck_idx: None,
        }
    }
}

fn short_rarity(r: CardRarity) -> &'static str {
    match r {
        CardRarity::Basic => "B",
        CardRarity::Common => "C",
        CardRarity::Uncommon => "U",
        CardRarity::Rare => "R",
        CardRarity::Status => "S",
        CardRarity::Curse => "X",
        CardRarity::Ancient => "A",
        CardRarity::Event => "E",
        CardRarity::Token => "T",
        CardRarity::Quest => "Q",
        CardRarity::None => "-",
    }
}

// ----------------------------------------------------------------------
// Combat phase
// ----------------------------------------------------------------------

fn start_combat(b: Builder) -> Phase {
    // Build deck instances with enchantments + upgrades.
    let deck: Vec<CardInstance> = b
        .deck
        .iter()
        .filter_map(|e| {
            let data = cardmod::by_id(&e.card_id)?;
            let mut inst = CardInstance::from_card(data, e.upgrade);
            if let Some((eid, amt)) = &e.enchantment {
                inst.enchantment = Some(EnchantmentInstance {
                    id: eid.clone(),
                    amount: *amt,
                    consumed_this_combat: false,
                    state: Default::default(),
                });
            }
            Some(inst)
        })
        .collect();

    // Synthesize an encounter with 2 BigDummy.
    let fake_enc = EncounterData {
        id: "sandbox/two_dummies".to_string(),
        room_type: None,
        is_weak: false,
        slots: Vec::new(),
        canonical_monsters: Vec::new(),
        possible_monsters: Vec::new(),
        tags: Vec::new(),
        acts: Vec::new(),
    };
    let mut cs = CombatState::start(&fake_enc, Vec::new(), Vec::new());

    // Player.
    let cd = character::by_id(&b.character_id).expect("character");
    let creature = Creature {
        kind: CreatureKind::Player,
        model_id: cd.id.clone(),
        slot: String::new(),
        current_hp: b.starting_hp,
        max_hp: b.max_hp,
        block: 0,
        powers: Vec::new(),
        afflictions: Vec::new(),
        player: Some(PlayerState {
            draw: CardPile::with_cards(PileType::Draw, deck),
            hand: CardPile::new(PileType::Hand),
            discard: CardPile::new(PileType::Discard),
            exhaust: CardPile::new(PileType::Exhaust),
            play_pile: Vec::new(),
            energy: 0,
            turn_energy: 3,
            relics: b.relics.clone(),
            pending_gold: 0,
            pending_stars: 0,
            orb_queue: Vec::new(),
            orb_slots: if cd.id == "Defect" { 3 } else { 0 },
            pending_forge: 0,
            osty: None,
            relic_counters: HashMap::new(),
            hand_draw_round1_delta: 0,
        }),
        monster: None,
    };
    cs.allies.push(creature);

    // Seat the user-picked enemies. Falls back to 2× BigDummy if the
    // user somehow cleared the list — the Start button guards against
    // this, but defensive fallback keeps the UI from ever entering
    // combat without an opponent.
    if b.enemies.is_empty() {
        for (slot, slot_label) in DEFAULT_SLOTS.iter().take(2).enumerate() {
            cs.enemies.push(Creature::from_monster_spawn("BigDummy", slot_label));
            let _ = slot;
        }
    } else {
        for e in &b.enemies {
            cs.enemies.push(Creature::from_monster_spawn(&e.monster_id, &e.slot_label));
        }
    }
    // Fire `AfterAddedToRoom` spawn payloads (HardenedShellPower for
    // SkulkingColony, AsleepPower for LagavulinMatriarch, etc.). This
    // was skipped in the dummies-only build because BigDummy has no
    // spawn body — now that real monsters can be seated, it's
    // required for their AI to start in the correct state.
    monster_dispatch::fire_monster_spawn_hooks(&mut cs);

    // Set potions on belt for use during combat.
    if let Some(ps) = cs.allies[0].player.as_mut() {
        // store potion ids in pseudo run-log entries? our PlayerState
        // doesn't carry potions directly; we'll route through the
        // combat-side use_potion helper which looks up by id, so we
        // need a parallel "potion belt" we can render. Stash on the
        // ActiveCombat below.
        let _ = ps;
    }

    // RL/UI mode: any PlayerInteractive / AwaitPlayerChoice surface
    // pauses combat and surfaces a `pending_choice` instead of
    // auto-picking the bottom-N. The UI renders the prompt and the
    // user confirms picks. Without this flip, Acrobatics's "discard
    // 1" auto-discarded the bottom card with no UI feedback.
    cs.auto_resolve_choices = false;
    // Initial: shuffle innate to hand + draw 5. Turn-start hand draw
    // bypasses NoDrawPower (per C# ShouldDraw(fromHandDraw=true)).
    cs.move_innate_cards_to_hand(0);
    let mut rng = sts2_sim::rng::Rng::new(0xC0FFEE, 0);
    cs.draw_cards_initial(0, 5, &mut rng);
    // begin_turn for energy refresh + AfterPlayerTurnStart hooks.
    cs.begin_turn(CombatSide::Player);

    let snapshot_enemy: Vec<i32> =
        cs.enemies.iter().map(|c| c.current_hp).collect();
    let snapshot_hp = cs.allies[0].current_hp;
    let snapshot_block = cs.allies[0].block;
    let enemy_summary = cs
        .enemies
        .iter()
        .map(|c| c.model_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let active = ActiveCombat {
        cs,
        target_enemy: 0,
        log: vec![
            "Combat started. Player turn 1.".to_string(),
            format!("Enemies: {}", enemy_summary),
        ],
        last_enemy_hp: snapshot_enemy,
        last_player_hp: snapshot_hp,
        last_player_block: snapshot_block,
        pending_picks: Vec::new(),
        enemy_turn_counter: 0,
    };
    Phase::Combat(b, active)
}

fn render_combat(
    ctx: &egui::Context,
    b: &mut Builder,
    ac: &mut ActiveCombat,
) -> Option<Phase> {
    let mut reset = false;
    let mut back_to_setup = false;
    egui::TopBottomPanel::top("combat_header").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading(format!("Combat · Round {} · {:?} turn",
                ac.cs.round_number, ac.cs.current_side));
            ui.separator();
            if ui.button("End Turn").clicked() {
                tick_end_turn(ac);
            }
            if ui.button("Reset Combat").clicked() {
                reset = true;
            }
            if ui.button("← Back to Setup").clicked() {
                back_to_setup = true;
            }
        });
    });

    // Bottom: hand (separate to avoid eating space).
    egui::TopBottomPanel::bottom("hand_panel")
        .resizable(true)
        .min_height(200.0)
        .show(ctx, |ui| {
            render_hand(ui, ac);
        });

    // Side: piles + log.
    egui::SidePanel::right("right_panel").min_width(360.0).show(ctx, |ui| {
        render_piles(ui, ac);
        ui.separator();
        render_log(ui, ac);
    });

    // Center: enemies + player.
    egui::CentralPanel::default().show(ctx, |ui| {
        render_enemies(ui, ac);
        ui.separator();
        render_player(ui, ac, b);
    });

    // Modal-style choice overlay. When a card with a player-choice
    // selector plays (Acrobatics discard / Brand exhaust / etc.),
    // the simulator stages `pending_choice` and pauses. The UI
    // blocks other inputs until resolved.
    if ac.cs.pending_choice.is_some() {
        render_choice_overlay(ctx, ac);
    }

    if back_to_setup {
        return Some(Phase::Setup(b.clone_for_combat()));
    }
    if reset {
        return Some(start_combat(b.clone_for_combat()));
    }
    None
}

fn render_enemies(ui: &mut egui::Ui, ac: &mut ActiveCombat) {
    // Clamp target if the enemy roster shrank (corpses stay seated
    // but we still defend against out-of-bounds access).
    if ac.target_enemy >= ac.cs.enemies.len() && !ac.cs.enemies.is_empty() {
        ac.target_enemy = 0;
    }
    ui.horizontal(|ui| {
        ui.heading("Enemies");
        ui.label(format!("(target: enemy {})", ac.target_enemy));
    });
    let target = ac.target_enemy;
    ui.horizontal_wrapped(|ui| {
        for (i, e) in ac.cs.enemies.iter().enumerate() {
            let selected = i == target;
            let frame = egui::Frame::group(ui.style())
                .stroke(if selected {
                    egui::Stroke::new(2.5, egui::Color32::YELLOW)
                } else {
                    egui::Stroke::new(1.0, egui::Color32::DARK_GRAY)
                });
            frame.show(ui, |ui| {
                ui.set_min_width(260.0);
                ui.vertical(|ui| {
                    let hp_pct = (e.current_hp as f32 / e.max_hp.max(1) as f32).clamp(0.0, 1.0);
                    ui.label(egui::RichText::new(format!("{} #{}", e.model_id, i)).strong());
                    ui.label(format!("HP: {} / {}", e.current_hp, e.max_hp));
                    let (rect, _) = ui.allocate_exact_size(
                        [240.0, 12.0].into(),
                        egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(50, 30, 30));
                    let mut bar = rect;
                    bar.set_width(rect.width() * hp_pct);
                    painter.rect_filled(bar, 2.0,
                        egui::Color32::from_rgb(200, 60, 60));
                    if e.block > 0 {
                        ui.label(format!("🛡 Block: {}", e.block));
                    }
                    if !e.powers.is_empty() {
                        ui.label(format!("Powers: {}", e.powers.iter()
                            .map(|p| format!("{}({})", p.id, p.amount))
                            .collect::<Vec<_>>().join(", ")));
                    }
                    // Show what was last played AND a preview of the
                    // next move the AI would pick if we ended the turn
                    // right now. Preview just calls pick_next_move with
                    // a transient RNG — it's purely informational, the
                    // actual roll on EndTurn uses the combat RNG.
                    if let Some(m) = e.monster.as_ref() {
                        if let Some(intent) = m.intent_move.as_ref() {
                            ui.label(format!("Last move: {}", intent));
                        } else {
                            ui.label("Last move: —");
                        }
                    }
                    if let Some(ai) = monster_ai::ai_for(&e.model_id) {
                        let last = e.monster.as_ref()
                            .and_then(|m| m.intent_move.as_deref());
                        let slot = e.slot.clone();
                        let mut preview_rng = sts2_sim::rng::Rng::new(
                            0xC0FFEE_u32.wrapping_add(i as u32),
                            ac.cs.round_number,
                        );
                        let preview = monster_ai::pick_next_move(
                            &ai.pattern,
                            &ac.cs,
                            i,
                            last,
                            &slot,
                            &mut preview_rng,
                        );
                        if let Some(p) = preview {
                            ui.label(egui::RichText::new(format!("Next: {}", p))
                                .color(egui::Color32::from_rgb(255, 220, 120)));
                        }
                    }
                    if ui.button(if selected { "✓ Targeted" } else { "Target" }).clicked() {
                        ac.target_enemy = i;
                    }
                });
            });
        }
    });
}

fn render_player(ui: &mut egui::Ui, ac: &mut ActiveCombat, b: &Builder) {
    let player = &ac.cs.allies[0];
    let ps = player.player.as_ref();
    ui.heading(format!("{} (player)", player.model_id));
    ui.horizontal(|ui| {
        ui.label(format!("HP: {} / {}", player.current_hp, player.max_hp));
        if player.block > 0 {
            ui.label(egui::RichText::new(format!("🛡 {}", player.block))
                .color(egui::Color32::LIGHT_BLUE));
        }
        if let Some(p) = ps {
            ui.label(egui::RichText::new(format!("⚡ {}/{}", p.energy, p.turn_energy))
                .color(egui::Color32::YELLOW));
        }
    });
    if !player.powers.is_empty() {
        ui.label(format!("Powers: {}", player.powers.iter()
            .map(|p| format!("{}({})", p.id, p.amount))
            .collect::<Vec<_>>().join(", ")));
    }
    if let Some(p) = ps {
        if !p.relics.is_empty() {
            ui.label(format!("Relics: {}", p.relics.join(", ")));
        }
        if p.orb_slots > 0 || !p.orb_queue.is_empty() {
            ui.label(format!("Orbs: {} (slots {})",
                p.orb_queue.iter().map(|o| o.id.as_str()).collect::<Vec<_>>().join(", "),
                p.orb_slots));
        }
        if p.pending_forge > 0 {
            ui.label(format!("Pending Forge: {}", p.pending_forge));
        }
    }
    ui.separator();
    // Potion belt (combat-side use_potion).
    if !b.potions.is_empty() {
        ui.label("Potion belt:");
        ui.horizontal_wrapped(|ui| {
            for (slot, pid) in b.potions.iter().enumerate() {
                let resp = ui.button(format!("[{}] 🧪 {}", slot, pid));
                if resp.clicked() {
                    let target = Some((CombatSide::Enemy, ac.target_enemy));
                    let ok = ac.cs.use_potion(0, pid, target);
                    ac.log.push(format!("Use potion {} → {}",
                        pid, if ok { "ok" } else { "rejected" }));
                    summarize_diff(ac);
                }
            }
        });
    }
}

fn render_hand(ui: &mut egui::Ui, ac: &mut ActiveCombat) {
    let n = ac.cs.allies[0].player.as_ref().map(|p| p.hand.cards.len()).unwrap_or(0);
    let pending = ac.cs.pending_choice.is_some();
    ui.heading(format!("Hand ({} cards){}",
        n, if pending { " — waiting on choice" } else { "" }));
    let mut to_play: Option<usize> = None;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            for i in 0..n {
                let (label, color, can_play) = {
                    let card = &ac.cs.allies[0].player.as_ref().unwrap().hand.cards[i];
                    let data = cardmod::by_id(&card.id);
                    let cost = card.effective_energy_cost();
                    let energy = ac.cs.allies[0].player.as_ref().unwrap().energy;
                    let ench_marker = card.enchantment.as_ref()
                        .map(|e| format!(" ✦{}({})", e.id, e.amount))
                        .unwrap_or_default();
                    let label = format!(
                        "[{cost}] {}{}{}",
                        card.id,
                        if card.upgrade_level > 0 { "+" } else { "" },
                        ench_marker);
                    let color = match data.map(|d| d.card_type) {
                        Some(cardmod::CardType::Attack) => egui::Color32::from_rgb(220, 110, 110),
                        Some(cardmod::CardType::Skill)  => egui::Color32::from_rgb(110, 180, 220),
                        Some(cardmod::CardType::Power)  => egui::Color32::from_rgb(180, 220, 110),
                        Some(cardmod::CardType::Status) => egui::Color32::from_rgb(100, 100, 100),
                        Some(cardmod::CardType::Curse)  => egui::Color32::from_rgb(80, 30, 80),
                        _ => egui::Color32::GRAY,
                    };
                    let unplayable = data.map(|d| d.keywords.iter().any(|k| k == "Unplayable")).unwrap_or(false);
                    let can_play = energy >= cost && !unplayable && !pending;
                    (label, color, can_play)
                };
                let btn = egui::Button::new(egui::RichText::new(label).color(color))
                    .min_size([170.0, 110.0].into());
                let resp = ui.add_enabled(can_play, btn);
                if resp.clicked() { to_play = Some(i); }
            }
        });
    });
    if let Some(i) = to_play {
        let card_id = ac.cs.allies[0].player.as_ref().unwrap().hand.cards[i].id.clone();
        let data = cardmod::by_id(&card_id);
        let target = match data.map(|d| d.target_type) {
            Some(TargetType::AnyEnemy)
            | Some(TargetType::RandomEnemy) => Some((CombatSide::Enemy, ac.target_enemy)),
            Some(TargetType::AnyAlly) => Some((CombatSide::Player, 0)),
            _ => None,
        };
        let res = ac.cs.play_card(0, i, target);
        match res {
            PlayResult::Ok => ac.log.push(format!("Played {} → ok", card_id)),
            PlayResult::Unhandled => ac.log.push(format!("Played {} → unhandled", card_id)),
            other => ac.log.push(format!("Played {} → {:?}", card_id, other)),
        }
        summarize_diff(ac);
    }
}

fn render_piles(ui: &mut egui::Ui, ac: &mut ActiveCombat) {
    ui.heading("Piles");
    let player = ac.cs.allies[0].player.as_ref();
    let Some(p) = player else { return };
    egui::CollapsingHeader::new(format!("Draw ({})", p.draw.cards.len()))
        .default_open(false)
        .show(ui, |ui| {
            pile_listing(ui, &p.draw.cards);
        });
    egui::CollapsingHeader::new(format!("Discard ({})", p.discard.cards.len()))
        .default_open(true)
        .show(ui, |ui| {
            pile_listing(ui, &p.discard.cards);
        });
    egui::CollapsingHeader::new(format!("Exhaust ({})", p.exhaust.cards.len()))
        .default_open(false)
        .show(ui, |ui| {
            pile_listing(ui, &p.exhaust.cards);
        });
}

fn pile_listing(ui: &mut egui::Ui, cards: &[CardInstance]) {
    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
        for c in cards {
            let ench = c.enchantment.as_ref()
                .map(|e| format!(" ✦{}", e.id))
                .unwrap_or_default();
            ui.label(format!("{}{}{}",
                c.id,
                if c.upgrade_level > 0 { "+" } else { "" },
                ench));
        }
    });
}

fn render_log(ui: &mut egui::Ui, ac: &mut ActiveCombat) {
    ui.heading("Log");
    egui::ScrollArea::vertical()
        .id_source("log_scroll")
        .stick_to_bottom(true)
        .max_height(ui.available_height())
        .show(ui, |ui| {
            for line in &ac.log {
                ui.label(line);
            }
        });
}

fn render_choice_overlay(ctx: &egui::Context, ac: &mut ActiveCombat) {
    let mut confirm = false;
    let mut cancel = false;
    let (pile, n_min, n_max, source, action_label) = {
        let pc = ac.cs.pending_choice.as_ref().unwrap();
        let action_label = match &pc.action {
            sts2_sim::combat::ChoiceAction::Discard => "Discard".to_string(),
            sts2_sim::combat::ChoiceAction::Exhaust => "Exhaust".to_string(),
            sts2_sim::combat::ChoiceAction::Move { .. } => "Move".to_string(),
            sts2_sim::combat::ChoiceAction::Upgrade => "Upgrade".to_string(),
            sts2_sim::combat::ChoiceAction::SetCost { cost, .. } => format!("Set cost to {}", cost),
            sts2_sim::combat::ChoiceAction::IncrementCounter { key, delta } => {
                format!("Bump {} by {}", key, delta)
            }
        };
        (pc.pile, pc.n_min, pc.n_max, pc.source_card_id.clone(), action_label)
    };
    egui::Window::new(format!("{} — pick from {:?} (up to {})", action_label, pile, n_max))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(540.0)
        .show(ctx, |ui| {
            ui.label(format!(
                "Source: {} · pick {}–{} card(s)",
                source, n_min, n_max));
            ui.separator();
            // Pile candidates with click-to-toggle selection.
            let cards: Vec<(String, i32)> = {
                let player = ac.cs.allies[0].player.as_ref().unwrap();
                let pile_ref = match pile {
                    PileType::Hand => &player.hand,
                    PileType::Draw => &player.draw,
                    PileType::Discard => &player.discard,
                    PileType::Exhaust => &player.exhaust,
                    _ => return,
                };
                pile_ref.cards.iter()
                    .map(|c| (c.id.clone(), c.upgrade_level))
                    .collect()
            };
            ui.horizontal_wrapped(|ui| {
                for (i, (id, up)) in cards.iter().enumerate() {
                    let picked = ac.pending_picks.contains(&i);
                    let label = format!("{}{}", id, if *up > 0 { "+" } else { "" });
                    let bg = if picked {
                        egui::Color32::from_rgb(80, 140, 80)
                    } else {
                        egui::Color32::from_rgb(50, 50, 50)
                    };
                    let resp = ui.add(egui::Button::new(
                        egui::RichText::new(label).color(egui::Color32::WHITE))
                        .fill(bg)
                        .min_size([150.0, 40.0].into()));
                    if resp.clicked() {
                        if picked {
                            ac.pending_picks.retain(|&x| x != i);
                        } else if (ac.pending_picks.len() as i32) < n_max {
                            ac.pending_picks.push(i);
                        }
                    }
                }
            });
            ui.separator();
            let count = ac.pending_picks.len() as i32;
            let count_ok = count >= n_min && count <= n_max;
            ui.label(format!("Selected: {} (need {}–{})", count, n_min, n_max));
            ui.horizontal(|ui| {
                let confirm_resp = ui.add_enabled(count_ok,
                    egui::Button::new(egui::RichText::new("✓ Confirm")
                        .color(egui::Color32::from_rgb(140, 220, 140))));
                if confirm_resp.clicked() { confirm = true; }
                if n_min == 0 {
                    let skip_resp = ui.add(egui::Button::new("Skip (0)"));
                    if skip_resp.clicked() {
                        ac.pending_picks.clear();
                        confirm = true;
                    }
                }
                let cancel_resp = ui.add(egui::Button::new(
                    egui::RichText::new("✕ Cancel")
                        .color(egui::Color32::from_rgb(220, 140, 140))));
                if cancel_resp.clicked() { cancel = true; }
            });
        });
    if confirm {
        let picks = std::mem::take(&mut ac.pending_picks);
        match sts2_sim::effects::resolve_pending_choice(&mut ac.cs, &picks) {
            Ok(()) => {
                ac.log.push(format!("Choice resolved: {} pick(s)", picks.len()));
                summarize_diff(ac);
            }
            Err(e) => {
                ac.log.push(format!("Choice error: {}", e));
                // Restore picks so user can retry.
                ac.pending_picks = picks;
            }
        }
    }
    if cancel {
        // Clear the pending choice; abandons the half-resolved card.
        ac.cs.pending_choice = None;
        ac.pending_picks.clear();
        ac.log.push("Choice cancelled.".to_string());
    }
}

fn tick_end_turn(ac: &mut ActiveCombat) {
    // Full turn cycle so all the tick_* hooks fire on the right
    // boundaries. The C# turn loop is:
    //   end_turn(Player)  →  fires BeforeTurnEnd / hand-flush / etc.
    //   begin_turn(Enemy) →  flips current_side + enemy intents
    //   <each enemy runs its AI move via dispatch_enemy_turn>
    //   end_turn(Enemy)   →  fires tick_duration_debuffs (Vuln/Weak/Frail
    //                        on BOTH sides), tick_plating, tick_slumber,
    //                        tick_asleep, AfterEnemyTurnEnd hooks
    //   begin_turn(Player)→  refreshes energy, ticks Poison/DemonForm,
    //                        fires AfterPlayerTurnStart hooks
    ac.cs.end_turn();
    ac.cs.begin_turn(CombatSide::Enemy);
    // Run each living enemy's AI intent. Picks the next move from the
    // pattern, executes its effect body, writes intent_move back.
    let enemy_count = ac.cs.enemies.len();
    for i in 0..enemy_count {
        if ac.cs.enemies.get(i).map(|e| e.current_hp <= 0).unwrap_or(true) {
            continue;
        }
        let dispatched = monster_dispatch::dispatch_enemy_turn(&mut ac.cs, i, 0);
        if !dispatched {
            // No AI entry for this monster (shouldn't happen post-batch-5
            // for any concrete MonsterModel, but log it loudly so it's
            // visible if it ever recurs).
            let id = ac.cs.enemies.get(i).map(|e| e.model_id.clone()).unwrap_or_default();
            ac.log.push(format!("  enemy[{}] {} has no AI dispatch — skipped", i, id));
        }
        ac.enemy_turn_counter = ac.enemy_turn_counter.wrapping_add(1);
    }
    ac.cs.end_turn();
    ac.cs.begin_turn(CombatSide::Player);
    let mut rng = sts2_sim::rng::Rng::new(0xC0FFEE, ac.cs.round_number);
    ac.cs.draw_cards_initial(0, 5, &mut rng);
    ac.log.push(format!("End turn → round {}", ac.cs.round_number));
    summarize_diff(ac);
}

fn summarize_diff(ac: &mut ActiveCombat) {
    let player = &ac.cs.allies[0];
    if player.current_hp != ac.last_player_hp {
        let d = player.current_hp - ac.last_player_hp;
        ac.log.push(format!("  player HP {:+} → {}", d, player.current_hp));
        ac.last_player_hp = player.current_hp;
    }
    if player.block != ac.last_player_block {
        let d = player.block - ac.last_player_block;
        ac.log.push(format!("  player Block {:+} → {}", d, player.block));
        ac.last_player_block = player.block;
    }
    // Resize the snapshot if the enemy roster grew (summons) or
    // shrank (only theoretically — corpses stay seated).
    if ac.last_enemy_hp.len() != ac.cs.enemies.len() {
        ac.last_enemy_hp.resize(ac.cs.enemies.len(), 0);
    }
    for i in 0..ac.cs.enemies.len() {
        let cur = ac.cs.enemies[i].current_hp;
        if cur != ac.last_enemy_hp[i] {
            let d = cur - ac.last_enemy_hp[i];
            ac.log.push(format!("  enemy[{}] HP {:+} → {}", i, d, cur));
            ac.last_enemy_hp[i] = cur;
        }
    }
}
