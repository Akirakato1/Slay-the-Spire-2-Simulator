//! sts2-ui — sandbox UI for verifying card / relic / potion / enchantment
//! behavior interactively. Build a deck, fight 2× BigDummy (9999 HP each).
//!
//! Layout:
//!   - Setup phase: search + click to add cards/relics/potions to a
//!     deck. Click a deck card to attach an enchantment.
//!   - Combat phase: top shows enemies + player, bottom shows hand,
//!     side panel lists draw/discard/exhaust piles and the combat log.
//!     Click an enemy first to target it; then click a card to play.

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

#[derive(Default)]
struct Builder {
    character_id: String, // "Ironclad" / "Silent" / "Defect" / etc.
    max_hp: i32,
    starting_hp: i32,
    deck: Vec<DeckEntry>,
    relics: Vec<String>,
    /// Indexed potion belt with fixed 3 slots.
    potions: Vec<String>,
    card_filter: String,
    relic_filter: String,
    potion_filter: String,
    enchantment_filter: String,
    /// Which deck card is selected for enchantment-editing (if any).
    selected_deck_idx: Option<usize>,
}

impl Builder {
    fn new_ironclad() -> Self {
        let cd = character::by_id("Ironclad").expect("Ironclad");
        let deck: Vec<DeckEntry> = cd
            .starting_deck
            .iter()
            .map(|id| DeckEntry { card_id: id.clone(), upgrade: 0, enchantment: None })
            .collect();
        Self {
            character_id: "Ironclad".to_string(),
            max_hp: 80,
            starting_hp: 80,
            deck,
            relics: cd.starting_relics.clone(),
            potions: Vec::new(),
            ..Default::default()
        }
    }
}

// ----------------------------------------------------------------------
// Combat-phase state
// ----------------------------------------------------------------------

struct ActiveCombat {
    cs: CombatState,
    /// Selected enemy index (0 or 1) for targeted-attack plays.
    target_enemy: usize,
    /// Per-tick combat log.
    log: Vec<String>,
    /// Side-channel HP snapshot at last tick so we can summarize what
    /// changed when displaying the log.
    last_enemy_hp: [i32; 2],
    last_player_hp: i32,
    last_player_block: i32,
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
        Self { phase: Phase::Setup(Builder::new_ironclad()) }
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
                    if let Some(cd) = character::by_id(ch) {
                        b.character_id = ch.to_string();
                        b.deck = cd.starting_deck.iter().map(|id| DeckEntry {
                            card_id: id.clone(), upgrade: 0, enchantment: None,
                        }).collect();
                        b.relics = cd.starting_relics.clone();
                        b.max_hp = cd.starting_hp.unwrap_or(80);
                        b.starting_hp = b.max_hp;
                    }
                }
            }
            ui.separator();
            ui.label("HP:");
            ui.add(egui::DragValue::new(&mut b.starting_hp).clamp_range(1..=999));
            ui.label("/");
            ui.add(egui::DragValue::new(&mut b.max_hp).clamp_range(1..=999));
            if b.starting_hp > b.max_hp { b.starting_hp = b.max_hp; }
            ui.separator();
            if ui.button(egui::RichText::new("▶ Start Combat (vs 2× BigDummy)").strong()).clicked() {
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
            .max_height(200.0)
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
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading(format!("Deck ({} cards) · Relics ({}) · Potions ({}/3)",
            b.deck.len(), b.relics.len(), b.potions.len()));
        ui.label("Click a deck card to attach an enchantment. Click a relic/potion to remove it.");
        ui.separator();
        // Relics + potions row.
        ui.horizontal_wrapped(|ui| {
            for i in (0..b.relics.len()).rev() {
                let resp = ui.add(egui::Button::new(
                    egui::RichText::new(format!("◆ {}", &b.relics[i]))
                        .color(egui::Color32::from_rgb(255, 200, 80))));
                if resp.clicked() {
                    b.relics.remove(i);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for i in (0..b.potions.len()).rev() {
                let resp = ui.add(egui::Button::new(
                    egui::RichText::new(format!("🧪 {}", &b.potions[i]))
                        .color(egui::Color32::from_rgb(120, 200, 255))));
                if resp.clicked() {
                    b.potions.remove(i);
                }
            }
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
                        if ui.button("✕").clicked() {
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
            card_filter: String::new(),
            relic_filter: String::new(),
            potion_filter: String::new(),
            enchantment_filter: String::new(),
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
        slots: Vec::new(),
        canonical_monsters: Vec::new(),
        possible_monsters: Vec::new(),
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

    // 2 BigDummies.
    cs.enemies.push(Creature::from_monster_spawn("BigDummy", "first"));
    cs.enemies.push(Creature::from_monster_spawn("BigDummy", "second"));

    // Set potions on belt for use during combat.
    if let Some(ps) = cs.allies[0].player.as_mut() {
        // store potion ids in pseudo run-log entries? our PlayerState
        // doesn't carry potions directly; we'll route through the
        // combat-side use_potion helper which looks up by id, so we
        // need a parallel "potion belt" we can render. Stash on the
        // ActiveCombat below.
        let _ = ps;
    }

    // Initial: shuffle innate to hand + draw 5.
    cs.move_innate_cards_to_hand(0);
    let mut rng = sts2_sim::rng::Rng::new(0xC0FFEE, 0);
    cs.draw_cards(0, 5, &mut rng);
    // begin_turn for energy refresh + AfterPlayerTurnStart hooks.
    cs.begin_turn(CombatSide::Player);

    let snapshot_enemy = [cs.enemies[0].current_hp, cs.enemies[1].current_hp];
    let snapshot_hp = cs.allies[0].current_hp;
    let snapshot_block = cs.allies[0].block;
    let active = ActiveCombat {
        cs,
        target_enemy: 0,
        log: vec!["Combat started. Player turn 1.".to_string()],
        last_enemy_hp: snapshot_enemy,
        last_player_hp: snapshot_hp,
        last_player_block: snapshot_block,
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

    if back_to_setup {
        return Some(Phase::Setup(b.clone_for_combat()));
    }
    if reset {
        return Some(start_combat(b.clone_for_combat()));
    }
    None
}

fn render_enemies(ui: &mut egui::Ui, ac: &mut ActiveCombat) {
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
                    if let Some(m) = e.monster.as_ref() {
                        if let Some(intent) = m.intent_move.as_ref() {
                            ui.label(format!("Intent: {}", intent));
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
    ui.heading(format!("Hand ({} cards)", n));
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
                    let can_play = energy >= cost && !unplayable;
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

fn tick_end_turn(ac: &mut ActiveCombat) {
    // Full turn cycle so all the tick_* hooks fire on the right
    // boundaries. The C# turn loop is:
    //   end_turn(Player)  →  fires BeforeTurnEnd / hand-flush / etc.
    //   begin_turn(Enemy) →  flips current_side + enemy intents
    //   end_turn(Enemy)   →  fires tick_duration_debuffs (Vuln/Weak/Frail
    //                        on BOTH sides), tick_plating, tick_slumber,
    //                        tick_asleep, AfterEnemyTurnEnd hooks
    //   begin_turn(Player)→  refreshes energy, ticks Poison/DemonForm,
    //                        fires AfterPlayerTurnStart hooks
    // Skipping the enemy half (as the previous UI did) means Vulnerable
    // / Weak / Frail / Plating never tick down.
    ac.cs.end_turn();
    ac.cs.begin_turn(CombatSide::Enemy);
    // No intent execution: BigDummy is the punching bag with no
    // intent_move. If we later introduce a non-dummy enemy, this is
    // where monster_dispatch::execute_intent(...) would go.
    ac.cs.end_turn();
    ac.cs.begin_turn(CombatSide::Player);
    let mut rng = sts2_sim::rng::Rng::new(0xC0FFEE, ac.cs.round_number);
    ac.cs.draw_cards(0, 5, &mut rng);
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
    for i in 0..2.min(ac.cs.enemies.len()) {
        let cur = ac.cs.enemies[i].current_hp;
        if cur != ac.last_enemy_hp[i] {
            let d = cur - ac.last_enemy_hp[i];
            ac.log.push(format!("  enemy[{}] HP {:+} → {}", i, d, cur));
            ac.last_enemy_hp[i] = cur;
        }
    }
}
