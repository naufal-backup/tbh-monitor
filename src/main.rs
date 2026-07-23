#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod es3;
mod server;

use eframe::egui;
use qrcode::QrCode;
use image::Luma;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use std::thread;

const BG_DARK: egui::Color32 = egui::Color32::from_rgb(15, 15, 21);
const CARD_BG: egui::Color32 = egui::Color32::from_rgb(24, 25, 34);
const CARD_BORDER: egui::Color32 = egui::Color32::from_rgb(40, 42, 58);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(99, 102, 241);
const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(229, 231, 235);
const TEXT_SECONDARY: egui::Color32 = egui::Color32::from_rgb(156, 163, 175);
const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(107, 114, 128);
const GREEN: egui::Color32 = egui::Color32::from_rgb(34, 197, 94);
const YELLOW: egui::Color32 = egui::Color32::from_rgb(234, 179, 8);

// Shared grid spacing tiers so every page lines up the same way.
// GRID_SPACING: content cards (hero/rune/pet cards, stat cards).
// COMPACT_GRID_SPACING: small icon-like slots (inventory items, equipped gear).
const GRID_SPACING: f32 = 12.0;
const COMPACT_GRID_SPACING: f32 = 8.0;

#[derive(Clone, PartialEq)]
enum SortBy {
    Name,
    Type,
    Grade,
    Key,
}

#[derive(Clone, PartialEq)]
enum ItemCategoryFilter {
    All,
    Weapon,
    Offhand,
    Armor,
    Jewelry,
    Material,
}

struct TbMonitorApp {
    save_data: Option<es3::SaveData>,
    player_data: Option<es3::PlayerData>,
    last_update: Instant,
    update_interval: Duration,
    status: String,
    show_settings: bool,
    ngrok_url: String,
    qr_texture: Option<egui::TextureHandle>,
    server_running: bool,
    server_port: u16,
    server_handle: Option<thread::JoinHandle<()>>,
    active_tab: Tab,
    item_names: std::collections::HashMap<String, String>,
    sort_by: SortBy,
    filter_category: ItemCategoryFilter,
    search_query: String,
}

#[derive(PartialEq)]
enum Tab {
    Dashboard,
    Heroes,
    Inventory,
    Runes,
}

impl Default for TbMonitorApp {
    fn default() -> Self {
        Self {
            save_data: None,
            player_data: None,
            last_update: Instant::now(),
            update_interval: Duration::from_secs(30),
            status: "Loading...".to_string(),
            show_settings: false,
            ngrok_url: String::new(),
            qr_texture: None,
            server_running: false,
            server_port: 8080,
            server_handle: None,
            active_tab: Tab::Dashboard,
            item_names: std::collections::HashMap::new(),
            sort_by: SortBy::Name,
            filter_category: ItemCategoryFilter::All,
            search_query: String::new(),
        }
    }
}

impl TbMonitorApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();
        app.load_item_names();
        app.load_save();
        app
    }
    
    fn load_item_names(&mut self) {
        let names_json = include_str!("..\\data\\names_en.json");
        if let Ok(names) = serde_json::from_str::<std::collections::HashMap<String, String>>(names_json) {
            self.item_names = names;
        }
    }
    
    fn get_item_name(&self, key: i64) -> String {
        let key_str = key.to_string();
        
        // 1. Try exact match first
        if let Some(name) = self.item_names.get(&key_str) {
            return name.clone();
        }
        
        // 2. Equipment key matching
        let category = key / 10000;
        let raw_id = key % 1000;
        let grade = Self::item_grade(key);
        
        if category >= 30 {
            let item_index = if raw_id > 0 { (raw_id - 1) % 20 + 1 } else { 1 };
            let base_key = category * 10000 + item_index;
            if let Some(name) = self.item_names.get(&base_key.to_string()) {
                return name.clone();
            }
            
            let base_key_direct = category * 10000 + (key % 100);
            if let Some(name) = self.item_names.get(&base_key_direct.to_string()) {
                return name.clone();
            }

            let base_key_mod = category * 10000 + (key % 1000);
            if let Some(name) = self.item_names.get(&base_key_mod.to_string()) {
                return name.clone();
            }
        } else {
            let modelo = (key % 10000 / 10) % 100;
            let base = (key / 10000) * 10000 + modelo;
            if let Some(name) = self.item_names.get(&base.to_string()) {
                return name.clone();
            }
        }
        
        // 3. Descriptive fallback (never show raw ID)
        let grade_str = Self::grade_name(grade);
        let type_str = Self::item_type(key);
        if !grade_str.is_empty() && type_str != "Item" {
            format!("{} {}", grade_str, type_str)
        } else if type_str != "Item" {
            format!("{} (Grade {})", type_str, grade)
        } else {
            format!("Equipment (T{})", grade)
        }
    }
    
    fn item_grade(key: i64) -> i64 {
        let s = key.to_string();
        if s.len() == 6 && key >= 300000 {
            s.chars().nth(2).unwrap_or('0').to_digit(10).unwrap_or(0) as i64
        } else {
            1
        }
    }
    
    fn item_type(key: i64) -> &'static str {
        match key / 10000 {
            11=>"Gem", 12=>"Material", 13=>"Scroll", 14=>"Ingot", 16=>"Coin", 19=>"Soulstone",
            30=>"Sword", 31=>"Bow", 32=>"Staff", 33=>"Scepter", 34=>"Crossbow", 35=>"Axe",
            40=>"Shield", 41=>"Arrow", 42=>"Orb", 43=>"Tome", 44=>"Bolt", 45=>"Hatchet",
            50=>"Helmet", 51=>"Armor", 52=>"Gloves", 53=>"Boots",
            60=>"Amulet", 61=>"Earring", 62=>"Ring", 63=>"Bracer",
            _=>"Item",
        }
    }
    
    fn grade_name(grade: i64) -> &'static str {
        match grade {
            1=>"Common", 2=>"Uncommon", 3=>"Rare", 4=>"Legendary", 5=>"Immortal",
            6=>"Arcana", 7=>"Beyond", 8=>"Celestial", 9=>"Divine", 10=>"Cosmic",
            _=>"",
        }
    }
    
    fn grade_color(grade: i64) -> egui::Color32 {
        match grade {
            1=>egui::Color32::from_rgb(228,228,228),  // Common: #e4e4e4
            2=>egui::Color32::from_rgb(84,252,12),    // Uncommon: #54fc0c
            3=>egui::Color32::from_rgb(47,139,252),   // Rare: #2f8bfc
            4=>egui::Color32::from_rgb(252,156,12),   // Legendary: #fc9c0c
            5=>egui::Color32::from_rgb(252,36,36),    // Immortal: #fc2424
            6=>egui::Color32::from_rgb(180,12,252),   // Arcana: #b40cfc
            7=>egui::Color32::from_rgb(252,36,108),   // Beyond: #fc246c
            8=>egui::Color32::from_rgb(108,204,228),  // Celestial: #6ccce4
            9=>egui::Color32::from_rgb(252,228,84),   // Divine: #fce454
            10=>egui::Color32::from_rgb(252,252,252), // Cosmic: #fcfcfc
            _=>egui::Color32::GRAY,
        }
    }
    
    fn item_grade_bg(grade: i64) -> egui::Color32 {
        match grade {
            1=>egui::Color32::from_rgb(30,30,32),   // Common: dark gray
            2=>egui::Color32::from_rgb(15,35,15),   // Uncommon: dark green
            3=>egui::Color32::from_rgb(12,20,45),   // Rare: dark blue
            4=>egui::Color32::from_rgb(42,28,8),    // Legendary: dark orange
            5=>egui::Color32::from_rgb(45,10,10),   // Immortal: dark red
            6=>egui::Color32::from_rgb(32,10,48),   // Arcana: dark purple
            7=>egui::Color32::from_rgb(48,10,22),   // Beyond: dark pink
            8=>egui::Color32::from_rgb(15,35,42),   // Celestial: dark cyan
            9=>egui::Color32::from_rgb(45,40,12),   // Divine: dark gold
            10=>egui::Color32::from_rgb(42,42,48),  // Cosmic: dark white
            _=>CARD_BG,
        }
    }
    
    fn load_save(&mut self) {
        let save_path = es3::get_default_save_path();
        if save_path.exists() {
            match es3::load_save_file(&save_path) {
                Ok(data) => {
                    self.player_data = data.parse_player().ok();
                    self.save_data = Some(data);
                    self.status = "Loaded".to_string();
                }
                Err(e) => { self.status = format!("Error: {}", e); }
            }
        } else {
            self.status = "Save file not found".to_string();
        }
    }
    
    fn generate_qr(&mut self, ctx: &egui::Context) {
        if self.ngrok_url.is_empty() { return; }
        let api_url = format!("{}/api/data", self.ngrok_url.trim_end_matches('/'));
        if let Ok(code) = QrCode::new(api_url.as_bytes()) {
            let img = code.render::<Luma<u8>>().build();
            let pixels: Vec<u8> = img.pixels().map(|p| p[0]).collect();
            let size = img.width() as usize;
            let color_image = egui::ColorImage::from_gray([size, size], &pixels);
            self.qr_texture = Some(ctx.load_texture("qr_code", color_image, egui::TextureOptions::NEAREST));
        }
    }
    
    fn start_server(&mut self) {
        if self.server_running { return; }
        let save_data = match &self.save_data {
            Some(data) => Arc::new(RwLock::new(Some(serde_json::to_value(data).unwrap()))),
            None => Arc::new(RwLock::new(None)),
        };
        let save_path = es3::get_default_save_path();
        let port = self.server_port;
        self.server_handle = Some(thread::spawn(move || {
            server::start_server(save_data, save_path, port);
        }));
        self.server_running = true;
        self.status = format!("Server running on port {}", port);
    }
    
    fn fmt_num(n: i64) -> String {
        if n >= 1_000_000_000 { format!("{:.1}B", n as f64 / 1e9) }
        else if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1e6) }
        else if n >= 1_000 { format!("{:.1}K", n as f64 / 1e3) }
        else { format!("{}", n) }
    }
    
    fn hero_name(key: i64) -> &'static str {
        match key { 101=>"Knight", 201=>"Ranger", 301=>"Sorcerer", 401=>"Priest", 501=>"Hunter", 601=>"Slayer", _=>"Unknown" }
    }

    fn stat_name(stat_type: i64) -> &'static str {
        match stat_type {
            1 => "Attack Power",
            2 => "Defense",
            3 => "Max HP",
            4 => "Crit Rate",
            5 => "Crit Damage",
            6 => "Attack Speed",
            7 => "Move Speed",
            8 => "HP Regen",
            9 => "Life Steal",
            10 => "Cooldown Reduction",
            11 => "Elemental Damage",
            12 => "Drop Rate",
            13 => "Gold Gain",
            _ => "Bonus Stat",
        }
    }

    fn format_item_stats(item: &serde_json::Value) -> Vec<String> {
        let mut stats = Vec::new();
        let key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
        let grade = Self::item_grade(key).max(1);
        let category = key / 10000;

        // Base Equipment Stats
        match category {
            30..=35 => {
                // Main Hand Weapons
                let base_atk = grade * 18 + (key % 100).min(20);
                stats.push(format!("Base Attack Power: +{}", base_atk));
            }
            40..=45 => {
                // Offhand items
                let base_def = grade * 10;
                stats.push(format!("Base Defense: +{}", base_def));
            }
            50..=53 => {
                // Armor pieces
                let base_def = grade * 8;
                let base_hp = grade * 45;
                stats.push(format!("Base Defense: +{}", base_def));
                stats.push(format!("Base Max HP: +{}", base_hp));
            }
            60..=63 => {
                // Accessories
                let stat_bonus = grade * 3;
                stats.push(format!("All Stats Bonus: +{}%", stat_bonus));
            }
            11 => { stats.push("Type: Socketing Gem".to_string()); }
            12 => { stats.push("Type: Crafting Material".to_string()); }
            13 => { stats.push("Type: Inscription Scroll".to_string()); }
            14 => { stats.push("Type: Refined Metal Ingot".to_string()); }
            16 => { stats.push("Type: Anniversary Currency Coin".to_string()); }
            19 => { stats.push("Type: Hero Soulstone".to_string()); }
            _ => {}
        }

        // Enchantment Data
        if let Some(enchants) = item.get("EnchantData").and_then(|e| e.as_array()) {
            for enc in enchants {
                let stat_type = enc.get("StatType").and_then(|s| s.as_i64()).unwrap_or(0);
                let val = enc.get("Value").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let tier = enc.get("Tier").and_then(|t| t.as_i64()).unwrap_or(0);
                if stat_type > 0 {
                    let name = Self::stat_name(stat_type);
                    if val != 0.0 {
                        if val.fract() == 0.0 {
                            stats.push(format!("Enchant {}: +{}", name, val as i64));
                        } else {
                            stats.push(format!("Enchant {}: +{:.1}%", name, val));
                        }
                    } else if tier > 0 {
                        stats.push(format!("Enchant {}: Tier {}", name, tier));
                    } else {
                        stats.push(format!("Enchant {}", name));
                    }
                }
            }
        }

        // Enchant Level sum
        if let Some(ench_arr) = item.get("EnchantCount").and_then(|e| e.as_array()) {
            let total_ench: i64 = ench_arr.iter().filter_map(|v| v.as_i64()).sum();
            if total_ench > 0 {
                stats.push(format!("Enchant Level: +{}", total_ench));
            }
        }

        // Chaotic Status
        if item.get("IsChaotic").and_then(|c| c.as_bool()).unwrap_or(false) {
            stats.push("Chaotic Modifier: Active (+Bonus / Risk)".to_string());
        }

        let insc = item.get("InscriptionAppliedTotalCount").and_then(|i| i.as_i64()).unwrap_or(0);
        if insc > 0 {
            stats.push(format!("Applied Inscriptions: {}", insc));
        }
        let engr = item.get("EngravingAppliedTotalCount").and_then(|e| e.as_i64()).unwrap_or(0);
        if engr > 0 {
            stats.push(format!("Applied Engravings: {}", engr));
        }
        let deco = item.get("DecorationAppliedTotalCount").and_then(|d| d.as_i64()).unwrap_or(0);
        if deco > 0 {
            stats.push(format!("Applied Decorations: {}", deco));
        }

        stats
    }

    fn pet_info(key: i64) -> (&'static str, &'static str) {
        match key {
            1001 => ("Bat", "+5% Item Drop Rate, +3% Move Speed"),
            1002 => ("Watcher", "+10% EXP Gain, +5% Sight Range"),
            1003 => ("Burning Skeleton", "+8% Fire Damage, +5% Attack Power"),
            1004 => ("Blue Golem", "+10% Max HP, +5% Armor"),
            1005 => ("Dark Spirit", "+5% Skill Cooldown, +8% Dark Damage"),
            2001 => ("Sword (DLC)", "+10% All Attack Damage"),
            2002 => ("Butterfly (DLC)", "+10% Move Speed, +5% Item Drop Rate"),
            2003 => ("Dragon (DLC)", "+15% Gold, +15% EXP, +10% Chest Drop Rate"),
            _ => ("Companion Pet", "Passive Companion Buff"),
        }
    }
    
    fn card(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
        let is_hovered = ui.rect_contains_pointer(ui.max_rect());
        let fill = if is_hovered { egui::Color32::from_rgb(30, 32, 45) } else { CARD_BG };
        let stroke = if is_hovered { egui::Stroke::new(1.0_f32, ACCENT) } else { egui::Stroke::new(1.0_f32, CARD_BORDER) };
        egui::Frame::NONE
            .fill(fill)
            .corner_radius(10.0)
            .stroke(stroke)
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(title).color(if is_hovered { TEXT_PRIMARY } else { TEXT_SECONDARY }).size(11.0).strong());
                    ui.add_space(6.0);
                    add_contents(ui);
                });
            });
    }

    /// Responsive grid where each column stretches to fill the row.
    /// Used for content cards (heroes, runes, pets, stats): the same
    /// column-count formula and centering logic runs on every page, so
    /// grids never end up lopsided or using a different breakpoint scheme.
    ///
    /// `min_card_w` is the smallest a card is allowed to shrink to;
    /// `max_cols` caps how many columns can appear even on very wide windows;
    /// `height_fn` lets rows size themselves to their tallest card (pass
    /// `|_| default_h` for uniform-height grids).
    fn grid_stretch<T>(
        ui: &mut egui::Ui,
        items: &[T],
        spacing: f32,
        min_card_w: f32,
        max_cols: usize,
        default_h: f32,
        height_fn: impl Fn(&T) -> f32,
        mut render_item: impl FnMut(&mut egui::Ui, &T, f32, f32),
    ) {
        let avail_w = ui.available_width();
        let cols = (((avail_w + spacing) / (min_card_w + spacing)).floor() as usize)
            .clamp(1, max_cols.max(1));
        let card_w = ((avail_w - spacing * (cols as f32 - 1.0)) / cols as f32).max(min_card_w);
        let row_w = (cols as f32 * card_w) + ((cols as f32 - 1.0) * spacing);
        let side_margin = ((avail_w - row_w) / 2.0).max(0.0);

        for chunk in items.chunks(cols) {
            let row_h = chunk.iter().map(|it| height_fn(it)).fold(default_h, f32::max);
            ui.horizontal(|ui| {
                ui.add_space(side_margin);
                ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
                for item in chunk {
                    render_item(ui, item, card_w, row_h);
                }
            });
            ui.add_space(spacing);
        }
    }

    /// Responsive grid where each card keeps a fixed size (icon/slot-like
    /// content) and extra row width is distributed as centered margins,
    /// instead of stretching cards awkwardly wide.
    fn grid_fixed<T>(
        ui: &mut egui::Ui,
        items: &[T],
        spacing: f32,
        card_w: f32,
        card_h: f32,
        max_cols: usize,
        mut render_item: impl FnMut(&mut egui::Ui, &T, f32, f32),
    ) {
        let avail_w = ui.available_width();
        let cols = (((avail_w + spacing) / (card_w + spacing)).floor() as usize)
            .clamp(1, max_cols.max(1));
        let row_w = (cols as f32 * card_w) + ((cols as f32 - 1.0) * spacing);
        let side_margin = ((avail_w - row_w) / 2.0).max(0.0);

        for chunk in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.add_space(side_margin);
                ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
                for item in chunk {
                    render_item(ui, item, card_w, card_h);
                }
            });
            ui.add_space(spacing);
        }
    }

    fn stat_card(ui: &mut egui::Ui, width: f32, title: &str, value_text: egui::RichText) {
        ui.allocate_ui_with_layout(
            egui::vec2(width, 70.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let is_hovered = ui.rect_contains_pointer(ui.max_rect());
                let fill = if is_hovered { egui::Color32::from_rgb(32, 35, 48) } else { CARD_BG };
                let stroke = if is_hovered { egui::Stroke::new(1.0_f32, ACCENT) } else { egui::Stroke::new(1.0_f32, CARD_BORDER) };
                egui::Frame::NONE
                    .fill(fill)
                    .corner_radius(10.0)
                    .stroke(stroke)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        ui.label(egui::RichText::new(title).color(if is_hovered { TEXT_PRIMARY } else { TEXT_SECONDARY }).size(11.0).strong());
                        ui.add_space(4.0);
                        ui.label(value_text);
                    });
            },
        );
    }
}

impl eframe::App for TbMonitorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_update.elapsed() >= self.update_interval {
            self.load_save();
            self.last_update = Instant::now();
        }
        
        ctx.set_visuals(egui::Visuals {
            dark_mode: true,
            override_text_color: Some(TEXT_PRIMARY),
            panel_fill: BG_DARK,
            window_fill: CARD_BG,
            ..Default::default()
        });
        
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.add(egui::Label::new(egui::RichText::new("TBH INDEX").color(ACCENT).size(20.0).strong()));
                ui.add(egui::Label::new(egui::RichText::new("Taskbar Hero Stash Tracker").color(TEXT_MUTED).size(12.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    if ui.add(egui::Button::new(egui::RichText::new("Settings").color(TEXT_SECONDARY).size(12.0)).fill(CARD_BG).stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))).clicked() {
                        self.show_settings = !self.show_settings;
                    }
                    ui.label(egui::RichText::new(&self.status).color(TEXT_MUTED).size(11.0));
                });
            });
            ui.add_space(8.0);
        });
        
        egui::TopBottomPanel::top("nav").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                for (tab, label) in [(Tab::Dashboard, "Dashboard"), (Tab::Heroes, "Heroes"), (Tab::Inventory, "Inventory"), (Tab::Runes, "Runes")] {
                    let selected = self.active_tab == tab;
                    let text = egui::RichText::new(label).color(if selected { ACCENT } else { TEXT_SECONDARY }).size(13.0);
                    if ui.selectable_label(selected, text).clicked() { self.active_tab = tab; }
                }
            });
            ui.add_space(4.0);
        });
        
        if self.show_settings {
            egui::Window::new("Settings")
                .collapsible(false).resizable(false)
                .frame(egui::Frame::NONE.fill(CARD_BG).corner_radius(12.0).stroke(egui::Stroke::new(1.0_f32, CARD_BORDER)).inner_margin(20.0))
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("Update Interval (sec)").color(TEXT_SECONDARY).size(12.0));
                    let mut secs = self.update_interval.as_secs() as u32;
                    if ui.add(egui::DragValue::new(&mut secs).speed(1).range(5..=300)).changed() { self.update_interval = Duration::from_secs(secs as u64); }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Server Port").color(TEXT_SECONDARY).size(12.0));
                    let mut port = self.server_port;
                    if ui.add(egui::DragValue::new(&mut port).speed(1).range(1024..=65535)).changed() { self.server_port = port; }
                    if !self.server_running { if ui.button("Start Server").clicked() { self.start_server(); } }
                    else { ui.label(egui::RichText::new("Server running!").color(GREEN).size(12.0)); }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Ngrok URL").color(TEXT_SECONDARY).size(12.0));
                    ui.text_edit_singleline(&mut self.ngrok_url);
                    if ui.button("Generate QR").clicked() { self.generate_qr(ctx); }
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() { self.show_settings = false; }
                });
        }
        
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(player) = self.player_data.clone() {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.vertical(|ui| {
                            match self.active_tab {
                                Tab::Dashboard => self.render_dashboard(ui, &player),
                                Tab::Heroes => self.render_heroes(ui, &player),
                                Tab::Inventory => self.render_inventory(ui, &player),
                                Tab::Runes => self.render_runes(ui, &player),
                            }
                        });
                    });
                    ui.add_space(16.0);
                });
            } else {
                ui.centered_and_justified(|ui| { ui.label(egui::RichText::new("No save data loaded").color(TEXT_MUTED)); });
            }
        });
    }
}

impl TbMonitorApp {
    fn render_dashboard(&self, ui: &mut egui::Ui, player: &es3::PlayerData) {
        let gold = player.other.get("currenySaveDatas").and_then(|c| c.as_array())
            .and_then(|a| a.first()).and_then(|c| c.get("Quantity")).and_then(|q| q.as_i64()).unwrap_or(0);
        let hero_count = player.other.get("heroSaveDatas").and_then(|h| h.as_array()).map(|a| a.len()).unwrap_or(0);
        let item_count = player.other.get("itemSaveDatas").and_then(|i| i.as_array()).map(|a| a.len()).unwrap_or(0);
        let rune_count = player.other.get("RuneSaveData").and_then(|r| r.as_array())
            .map(|a| a.iter().filter(|r| r.get("Level").and_then(|l| l.as_i64()).unwrap_or(0) > 0).count()).unwrap_or(0);
        let rune_total = player.other.get("RuneSaveData").and_then(|r| r.as_array()).map(|a| a.len()).unwrap_or(0);
        
        ui.label(egui::RichText::new("OVERVIEW").color(TEXT_SECONDARY).size(14.0).strong());
        ui.add_space(10.0);
        
        let spacing = GRID_SPACING;
        let stats: [(&str, String, egui::Color32); 4] = [
            ("GOLD", Self::fmt_num(gold), YELLOW),
            ("HEROES", format!("{}", hero_count), ACCENT),
            ("ITEMS", format!("{}", item_count), GREEN),
            ("RUNES", format!("{}/{}", rune_count, rune_total), egui::Color32::from_rgb(168, 85, 247)),
        ];
        Self::grid_stretch(
            ui,
            &stats,
            spacing,
            150.0,
            4,
            70.0,
            |_| 70.0,
            |ui, (title, value, color), card_w, _row_h| {
                Self::stat_card(ui, card_w, title, egui::RichText::new(value).color(*color).size(24.0).strong());
            },
        );

        ui.add_space(20.0);
        ui.label(egui::RichText::new("HEROES SUMMARY").color(TEXT_SECONDARY).size(14.0).strong());
        ui.add_space(10.0);

        if let Some(heroes) = player.other.get("heroSaveDatas").and_then(|h| h.as_array()) {
            let heroes: Vec<&serde_json::Value> = heroes.iter().collect();
            Self::grid_stretch(
                ui,
                &heroes,
                spacing,
                200.0,
                4,
                80.0,
                |_| 80.0,
                |ui, hero, hero_card_w, hero_card_h| {
                    let key = hero.get("heroKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    let level = hero.get("HeroLevel").and_then(|l| l.as_i64()).unwrap_or(0);
                    let exp = hero.get("HeroExp").and_then(|e| e.as_f64()).unwrap_or(0.0) as i64;
                    let unlocked = hero.get("IsUnLock").and_then(|u| u.as_bool()).unwrap_or(false);

                    let response = ui.allocate_ui_with_layout(
                        egui::vec2(hero_card_w, hero_card_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::Frame::NONE
                                .fill(CARD_BG)
                                .corner_radius(10.0)
                                .stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))
                                .inner_margin(egui::Margin::same(12))
                                .show(ui, |ui| {
                                    ui.set_width(hero_card_w - 24.0);
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(Self::hero_name(key)).color(TEXT_PRIMARY).size(14.0).strong());
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if unlocked {
                                                ui.label(egui::RichText::new("UNLOCKED").color(GREEN).size(9.0).strong());
                                            } else {
                                                ui.label(egui::RichText::new("LOCKED").color(TEXT_MUTED).size(9.0));
                                            }
                                        });
                                    });
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new(format!("Lv.{}", level)).color(ACCENT).size(20.0).strong());
                                    ui.label(egui::RichText::new(format!("EXP: {}", Self::fmt_num(exp))).color(TEXT_SECONDARY).size(10.0));
                                });
                        },
                    );

                    response.response.on_hover_ui(|ui| {
                        ui.set_min_width(200.0);
                        ui.label(egui::RichText::new(Self::hero_name(key)).color(TEXT_PRIMARY).size(14.0).strong());
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(format!("Level: {}", level)).color(TEXT_SECONDARY).size(12.0));
                        ui.label(egui::RichText::new(format!("EXP: {}", Self::fmt_num(exp))).color(TEXT_SECONDARY).size(12.0));
                        let ability_points = hero.get("AbilityPoint").and_then(|a| a.as_i64()).unwrap_or(0);
                        let allocated = hero.get("AllocatedHeroAbilityPoint").and_then(|a| a.as_i64()).unwrap_or(0);
                        ui.label(egui::RichText::new(format!("Skill Points: {} (Allocated: {})", ability_points, allocated)).color(TEXT_SECONDARY).size(12.0));
                    });
                },
            );
        }
    }
    
    fn render_heroes(&self, ui: &mut egui::Ui, player: &es3::PlayerData) {
        ui.label(egui::RichText::new("HEROES DETAILS").color(TEXT_SECONDARY).size(14.0).strong());
        ui.add_space(12.0);

        let mut item_map: std::collections::HashMap<i64, &serde_json::Value> = std::collections::HashMap::new();
        if let Some(items) = player.other.get("itemSaveDatas").and_then(|i| i.as_array()) {
            for item in items {
                let uid = item.get("UniqueId").and_then(|u| u.as_i64()).unwrap_or(0);
                if uid != 0 {
                    item_map.insert(uid, item);
                }
            }
        }

        static SLOT_NAMES: [&str; 10] = ["Main Hand", "Off Hand", "Head", "Body", "Hands", "Feet", "Neck", "Left Ear", "Right Ear", "Finger"];

        if let Some(heroes) = player.other.get("heroSaveDatas").and_then(|h| h.as_array()) {
            let spacing = GRID_SPACING;

            // Pre-compute data for each hero with height
            struct HeroCard<'a> {
                value: &'a serde_json::Value,
                key: i64,
                level: i64,
                exp: f64,
                unlocked: bool,
                ability_points: i64,
                allocated: i64,
                equipped_ids: Vec<i64>,
                has_gear: bool,
                height: f32,
            }

            let hero_cards: Vec<HeroCard> = heroes.iter().map(|hero| {
                let key = hero.get("heroKey").and_then(|k| k.as_i64()).unwrap_or(0);
                let level = hero.get("HeroLevel").and_then(|l| l.as_i64()).unwrap_or(0);
                let exp = hero.get("HeroExp").and_then(|e| e.as_f64()).unwrap_or(0.0);
                let unlocked = hero.get("IsUnLock").and_then(|u| u.as_bool()).unwrap_or(false);
                let ability_points = hero.get("AbilityPoint").and_then(|a| a.as_i64()).unwrap_or(0);
                let allocated = hero.get("AllocatedHeroAbilityPoint").and_then(|a| a.as_i64()).unwrap_or(0);
                let equipped_ids: Vec<i64> = hero.get("equippedItemIds").and_then(|e| e.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
                    .unwrap_or_default();
                let has_gear = unlocked && equipped_ids.iter().any(|&id| id != 0);
                let height = if unlocked { if has_gear { 270.0 } else { 110.0 } } else { 60.0 };
                HeroCard { value: hero, key, level, exp, unlocked, ability_points, allocated, equipped_ids, has_gear, height }
            }).collect();

            Self::grid_stretch(
                ui,
                &hero_cards,
                spacing,
                340.0,
                2,
                60.0,
                |hc| hc.height,
                |ui, hc, card_w, row_height| {
                        let _response = ui.allocate_ui_with_layout(
                            egui::vec2(card_w, row_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::Frame::NONE
                                    .fill(CARD_BG)
                                    .corner_radius(10.0)
                                    .stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))
                                    .inner_margin(egui::Margin::same(12))
                                    .show(ui, |ui| {
                                        ui.set_width(card_w - 24.0);
                                        let header_resp = ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(Self::hero_name(hc.key)).color(TEXT_PRIMARY).size(15.0).strong());
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new(format!("Lv.{}", hc.level)).color(ACCENT).size(13.0).strong());
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if hc.unlocked {
                                                    ui.label(egui::RichText::new("UNLOCKED").color(GREEN).size(9.0).strong());
                                                } else {
                                                    ui.label(egui::RichText::new("LOCKED").color(TEXT_MUTED).size(9.0));
                                                }
                                            });
                                        }).response;

                                        let hero_name_str = Self::hero_name(hc.key);
                                        let hc_unlocked = hc.unlocked;
                                        let hc_level = hc.level;
                                        let hc_exp = hc.exp;
                                        let hc_ap = hc.ability_points;
                                        let hc_alloc = hc.allocated;
                                        let hc_val = hc.value;

                                        header_resp.on_hover_ui(|ui| {
                                            ui.set_min_width(220.0);
                                            ui.label(egui::RichText::new(hero_name_str).color(TEXT_PRIMARY).size(15.0).strong());
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new(format!("Status: {}", if hc_unlocked { "Unlocked" } else { "Locked" })).color(if hc_unlocked { GREEN } else { TEXT_MUTED }).size(12.0));
                                            ui.label(egui::RichText::new(format!("Level: {} | EXP: {}", hc_level, Self::fmt_num(hc_exp as i64))).color(TEXT_SECONDARY).size(12.0));
                                            ui.label(egui::RichText::new(format!("Ability Points: {} ({} allocated)", hc_ap, hc_alloc)).color(TEXT_SECONDARY).size(12.0));

                                            if let Some(skills) = hc_val.get("equippedSKillKey").and_then(|s| s.as_array()) {
                                                let valid_skills: Vec<&serde_json::Value> = skills.iter().filter(|s| s.as_i64().unwrap_or(-1) > 0).collect();
                                                if !valid_skills.is_empty() {
                                                    ui.add_space(4.0);
                                                    ui.label(egui::RichText::new("Equipped Skills:").color(TEXT_MUTED).size(11.0));
                                                    for skill in &valid_skills {
                                                        ui.label(egui::RichText::new(format!("  Skill {}", skill)).color(ACCENT).size(11.0));
                                                    }
                                                }
                                            }
                                        });

                                        if hc.unlocked {
                                            ui.add_space(4.0);
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new(format!("EXP: {}", Self::fmt_num(hc.exp as i64))).color(TEXT_SECONDARY).size(10.0));
                                                ui.add_space(6.0);
                                                ui.label(egui::RichText::new(format!("SP: {}/{}", hc.allocated, hc.ability_points + hc.allocated)).color(TEXT_SECONDARY).size(10.0));
                                            });
                                            if let Some(skills) = hc.value.get("equippedSKillKey").and_then(|s| s.as_array()) {
                                                let valid_skills: Vec<&serde_json::Value> = skills.iter().filter(|s| s.as_i64().unwrap_or(-1) > 0).collect();
                                                if !valid_skills.is_empty() {
                                                    ui.add_space(2.0);
                                                    ui.horizontal(|ui| {
                                                        ui.label(egui::RichText::new("Skills:").color(TEXT_MUTED).size(9.0));
                                                        for skill in &valid_skills {
                                                            ui.label(egui::RichText::new(format!("[{}]", skill)).color(ACCENT).size(9.0));
                                                        }
                                                    });
                                                }
                                            }
                                            if hc.has_gear {
                                                ui.add_space(4.0);
                                                ui.horizontal(|ui| {
                                                    ui.label(egui::RichText::new("Equipment:").color(TEXT_MUTED).size(9.0));
                                                    let cnt = hc.equipped_ids.iter().filter(|&&id| id != 0).count();
                                                    ui.label(egui::RichText::new(format!("{} slotted", cnt)).color(TEXT_SECONDARY).size(9.0));
                                                });
                                                ui.add_space(2.0);

                                                let e_card_w = 52.0;
                                                let e_margin = 3.0;
                                                let e_outer = e_card_w + e_margin * 2.0;

                                                let display_items: Vec<(usize, &serde_json::Value)> = hc.equipped_ids.iter().enumerate()
                                                    .filter(|(_, uid)| **uid != 0)
                                                    .filter_map(|(i, uid)| item_map.get(uid).map(|&item| (i, item)))
                                                    .collect();

                                                if !display_items.is_empty() {
                                                    Self::grid_fixed(
                                                        ui,
                                                        &display_items,
                                                        COMPACT_GRID_SPACING,
                                                        e_outer,
                                                        46.0,
                                                        5,
                                                        |ui, (slot_idx, item), _slot_w, _slot_h| {
                                                                let item_key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                                                                let grade = Self::item_grade(item_key).max(1).min(10);
                                                                let bg = Self::item_grade_bg(grade);
                                                                let border_color = Self::grade_color(grade);
                                                                let name = self.get_item_name(item_key);
                                                                let short: String = name.chars().take(10).collect();
                                                                let chaotic = item.get("IsChaotic").and_then(|c| c.as_bool()).unwrap_or(false);
                                                                let enchants = item.get("EnchantCount").and_then(|e| e.as_array())
                                                                    .map(|a| a.iter().filter_map(|v| v.as_i64()).sum::<i64>()).unwrap_or(0);

                                                                let resp = ui.allocate_ui_with_layout(
                                                                    egui::vec2(e_outer, 46.0),
                                                                    egui::Layout::top_down(egui::Align::Center),
                                                                    |ui| {
                                                                        let is_slot_hovered = ui.rect_contains_pointer(ui.max_rect());
                                                                        let slot_stroke = egui::Stroke::new(1.0_f32, border_color);
                                                                        let slot_bg = if is_slot_hovered {
                                                                            egui::Color32::from_rgb(bg.r().saturating_add(25), bg.g().saturating_add(25), bg.b().saturating_add(30))
                                                                        } else {
                                                                            bg
                                                                        };
                                                                        egui::Frame::NONE
                                                                            .fill(slot_bg)
                                                                            .corner_radius(4.0)
                                                                            .stroke(slot_stroke)
                                                                            .inner_margin(egui::Margin::same(e_margin as i8))
                                                                            .show(ui, |ui| {
                                                                                ui.set_width(e_card_w);
                                                                                ui.vertical_centered(|ui| {
                                                                                    ui.add(egui::Label::new(egui::RichText::new(SLOT_NAMES[*slot_idx]).color(TEXT_MUTED).size(7.0)));
                                                                                    ui.add_space(1.0);
                                                                                    ui.label(egui::RichText::new(&short).color(TEXT_PRIMARY).size(7.0).strong());
                                                                                    if chaotic {
                                                                                        ui.label(egui::RichText::new("C").color(YELLOW).size(6.0).strong());
                                                                                    } else if enchants > 0 {
                                                                                        ui.label(egui::RichText::new(format!("+{}", enchants)).color(ACCENT).size(6.0));
                                                                                    }
                                                                                });
                                                                            });
                                                                    },
                                                                ).response;

                                                                 let tip_name = name.clone();
                                                                let tip_border = border_color;
                                                                let tip_slot = SLOT_NAMES[*slot_idx];
                                                                let grade_name = Self::grade_name(grade);
                                                                let tip_type = Self::item_type(item_key);
                                                                let item_ref = item;
                                                                resp.on_hover_ui(move |ui| {
                                                                    ui.set_min_width(200.0);
                                                                    ui.label(egui::RichText::new(tip_slot).color(TEXT_MUTED).size(10.0));
                                                                    ui.label(egui::RichText::new(&tip_name).color(tip_border).size(14.0).strong());
                                                                    ui.label(egui::RichText::new(format!("Grade: {}", grade_name)).color(tip_border).size(11.0));
                                                                    ui.label(egui::RichText::new(format!("Type: {}", tip_type)).color(TEXT_SECONDARY).size(11.0));
                                                                    if chaotic {
                                                                        ui.label(egui::RichText::new("CHAOTIC").color(YELLOW).size(10.0).strong());
                                                                    }
                                                                    if enchants > 0 {
                                                                        ui.label(egui::RichText::new(format!("Enchants: {}", enchants)).color(ACCENT).size(10.0));
                                                                    }
                                                                    let stats = Self::format_item_stats(item_ref);
                                                                    if !stats.is_empty() {
                                                                        ui.add_space(4.0);
                                                                        ui.label(egui::RichText::new("Stats & Buffs:").color(TEXT_MUTED).size(10.0));
                                                                        for stat in stats {
                                                                            ui.label(egui::RichText::new(format!("  • {}", stat)).color(GREEN).size(10.0));
                                                                        }
                                                                    }
                                                });
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    });
                            },
                        );
                },
            );
        }
    }
    
    fn render_inventory(&mut self, ui: &mut egui::Ui, player: &es3::PlayerData) {
        ui.label(egui::RichText::new("INVENTORY").color(TEXT_SECONDARY).size(14.0).strong());
        ui.add_space(12.0);
        
        // Sort and Search controls
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Search:").color(TEXT_SECONDARY).size(12.0));
            ui.add(egui::TextEdit::singleline(&mut self.search_query).desired_width(160.0).hint_text("Filter items..."));
            
            ui.add_space(12.0);

            ui.label(egui::RichText::new("Category:").color(TEXT_SECONDARY).size(12.0));
            egui::ComboBox::from_id_salt("filter_combo")
                .selected_text(match self.filter_category {
                    ItemCategoryFilter::All => "All Categories",
                    ItemCategoryFilter::Weapon => "Weapons",
                    ItemCategoryFilter::Offhand => "Offhands",
                    ItemCategoryFilter::Armor => "Armor",
                    ItemCategoryFilter::Jewelry => "Jewelry",
                    ItemCategoryFilter::Material => "Gems & Materials",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.filter_category, ItemCategoryFilter::All, "All Categories");
                    ui.selectable_value(&mut self.filter_category, ItemCategoryFilter::Weapon, "Weapons");
                    ui.selectable_value(&mut self.filter_category, ItemCategoryFilter::Offhand, "Offhands");
                    ui.selectable_value(&mut self.filter_category, ItemCategoryFilter::Armor, "Armor");
                    ui.selectable_value(&mut self.filter_category, ItemCategoryFilter::Jewelry, "Jewelry");
                    ui.selectable_value(&mut self.filter_category, ItemCategoryFilter::Material, "Gems & Materials");
                });
            
            ui.add_space(12.0);
            
            ui.label(egui::RichText::new("Sort by:").color(TEXT_SECONDARY).size(12.0));
            egui::ComboBox::from_id_salt("sort_combo")
                .selected_text(match self.sort_by {
                    SortBy::Name => "Name",
                    SortBy::Type => "Type",
                    SortBy::Grade => "Grade",
                    SortBy::Key => "ID",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.sort_by, SortBy::Name, "Name");
                    ui.selectable_value(&mut self.sort_by, SortBy::Type, "Type");
                    ui.selectable_value(&mut self.sort_by, SortBy::Grade, "Grade");
                    ui.selectable_value(&mut self.sort_by, SortBy::Key, "ID");
                });
        });
        
        ui.add_space(12.0);
        
        if let Some(items) = player.other.get("itemSaveDatas").and_then(|i| i.as_array()) {
            let mut sorted: Vec<_> = items.iter().collect();
            
            // Filter by Category and Search
            let query = self.search_query.to_lowercase();
            sorted.retain(|item| {
                let key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                let cat = key / 10000;

                // Category Filter
                let cat_match = match self.filter_category {
                    ItemCategoryFilter::All => true,
                    ItemCategoryFilter::Weapon => (30..=35).contains(&cat),
                    ItemCategoryFilter::Offhand => (40..=45).contains(&cat),
                    ItemCategoryFilter::Armor => (50..=53).contains(&cat),
                    ItemCategoryFilter::Jewelry => (60..=63).contains(&cat),
                    ItemCategoryFilter::Material => matches!(cat, 11 | 12 | 13 | 14 | 16 | 19),
                };

                if !cat_match {
                    return false;
                }

                // Search Filter
                if query.is_empty() { return true; }
                let name = self.get_item_name(key);
                let itype = Self::item_type(key);
                name.to_lowercase().contains(&query) || itype.to_lowercase().contains(&query)
            });
            
            // Sort
            match self.sort_by {
                SortBy::Name => sorted.sort_by(|a, b| {
                    let ka = a.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    let kb = b.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    self.get_item_name(ka).cmp(&self.get_item_name(kb))
                }),
                SortBy::Type => sorted.sort_by(|a, b| {
                    let ka = a.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    let kb = b.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    Self::item_type(ka).cmp(Self::item_type(kb))
                }),
                SortBy::Grade => sorted.sort_by(|a, b| {
                    let ka = a.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    let kb = b.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    let ga = Self::item_grade(ka);
                    let gb = Self::item_grade(kb);
                    gb.cmp(&ga).then_with(|| self.get_item_name(ka).cmp(&self.get_item_name(kb)))
                }),
                SortBy::Key => sorted.sort_by(|a, b| {
                    let ka = a.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    let kb = b.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    ka.cmp(&kb)
                }),
            }
            
            ui.label(egui::RichText::new(&format!("{} items", sorted.len())).color(TEXT_MUTED).size(11.0));
            ui.add_space(8.0);
            
            let card_content_w = 100.0;
            let margin = 8.0;
            let card_outer_w = card_content_w + (margin * 2.0); // 116.0
            let spacing = COMPACT_GRID_SPACING;

            Self::grid_fixed(
                ui,
                &sorted,
                spacing,
                card_outer_w,
                72.0,
                usize::MAX,
                |ui, item, _card_w, _card_h| {
                        let key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                        let is_chaotic = item.get("IsChaotic").and_then(|c| c.as_bool()).unwrap_or(false);
                        let enchants = item.get("EnchantCount").and_then(|c| c.as_i64()).unwrap_or(0);
                        
                        let name = self.get_item_name(key);
                        let short_name: String = name.chars().take(15).collect();
                        let grade = Self::item_grade(key).max(1).min(10);
                        let bg = Self::item_grade_bg(grade);
                        let border_color = Self::grade_color(grade);
                        let grade_name = Self::grade_name(grade);
                        
                        let response = ui.allocate_ui_with_layout(
                            egui::vec2(card_outer_w, 72.0),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                let is_item_hovered = ui.rect_contains_pointer(ui.max_rect());
                                let card_stroke = egui::Stroke::new(1.5_f32, border_color);
                                let card_bg = if is_item_hovered {
                                    egui::Color32::from_rgb(bg.r().saturating_add(25), bg.g().saturating_add(25), bg.b().saturating_add(30))
                                } else {
                                    bg
                                };
                                egui::Frame::NONE
                                    .fill(card_bg)
                                    .corner_radius(6.0)
                                    .stroke(card_stroke)
                                    .inner_margin(egui::Margin::same(margin as i8))
                                    .show(ui, |ui| {
                                        ui.set_width(card_content_w);
                                        ui.vertical_centered(|ui| {
                                            ui.add(egui::Label::new(egui::RichText::new(Self::item_type(key)).color(TEXT_SECONDARY).size(9.0)));
                                            ui.add_space(2.0);
                                            ui.label(egui::RichText::new(&short_name).color(TEXT_PRIMARY).size(10.0).strong());
                                            ui.label(egui::RichText::new(grade_name).color(border_color).size(8.0));
                                            if is_chaotic {
                                                ui.label(egui::RichText::new("CHAOTIC").color(YELLOW).size(8.0).strong());
                                            }
                                            if enchants > 0 {
                                                ui.label(egui::RichText::new(format!("+{} ench", enchants)).color(ACCENT).size(8.0));
                                            }
                                        });
                                    });
                            },
                        ).response;
                        
                        let tooltip_name = name.clone();
                        let tooltip_type = Self::item_type(key);
                        let tooltip_border = border_color;
                        let item_ref = item;
                        response.on_hover_ui(move |ui| {
                            ui.set_min_width(220.0);
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&tooltip_name).color(tooltip_border).size(14.0).strong());
                                ui.label(egui::RichText::new(format!("Grade: {}", grade_name)).color(tooltip_border).size(11.0));
                                ui.label(egui::RichText::new(format!("Type: {}", tooltip_type)).color(TEXT_SECONDARY).size(11.0));
                                ui.label(egui::RichText::new(format!("ID: {}", key)).color(TEXT_MUTED).size(10.0));
                                
                                if is_chaotic {
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new("CHAOTIC").color(YELLOW).size(11.0).strong());
                                }
                                
                                if enchants > 0 {
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new(format!("Enchants: {}", enchants)).color(ACCENT).size(11.0));
                                }

                                let stats = Self::format_item_stats(item_ref);
                                if !stats.is_empty() {
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new("Stats & Buffs:").color(TEXT_MUTED).size(10.0));
                                    for stat in stats {
                                        ui.label(egui::RichText::new(format!("  • {}", stat)).color(GREEN).size(10.0));
                                    }
                                }
                            });
                        });
                },
            );
        }
    }
    
    fn render_runes(&self, ui: &mut egui::Ui, player: &es3::PlayerData) {
        ui.label(egui::RichText::new("RUNE PROGRESS").color(TEXT_SECONDARY).size(14.0).strong());
        ui.add_space(12.0);
        if let Some(runes) = player.other.get("RuneSaveData").and_then(|r| r.as_array()) {
            let total = runes.len();
            let unlocked = runes.iter().filter(|r| r.get("Level").and_then(|l| l.as_i64()).unwrap_or(0) > 0).count();
            let progress = if total > 0 { unlocked as f32 / total as f32 } else { 0.0 };
            
            Self::card(ui, "RUNE TREE OVERVIEW", |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{}/{} nodes unlocked", unlocked, total)).color(TEXT_PRIMARY).size(16.0).strong());
                    ui.label(egui::RichText::new(format!("({:.1}%)", progress * 100.0)).color(TEXT_SECONDARY).size(12.0));
                });
                ui.add_space(8.0);
                ui.add(egui::ProgressBar::new(progress).fill(egui::Color32::from_rgb(168, 85, 247)).animate(true));
            });

            ui.add_space(16.0);
            ui.label(egui::RichText::new("RUNE NODES").color(TEXT_SECONDARY).size(14.0).strong());
            ui.add_space(10.0);

            let spacing = GRID_SPACING;

            Self::grid_stretch(
                ui,
                runes,
                spacing,
                110.0,
                6,
                65.0,
                |_| 65.0,
                |ui, rune, card_w, card_h| {
                        let key = rune.get("RuneKey").and_then(|k| k.as_i64()).unwrap_or(0);
                        let level = rune.get("Level").and_then(|l| l.as_i64()).unwrap_or(0);
                        let is_unlocked = level > 0;

                        let resp = ui.allocate_ui_with_layout(
                            egui::vec2(card_w, card_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let is_rune_hovered = ui.rect_contains_pointer(ui.max_rect());
                                let rune_stroke = if is_unlocked {
                                    egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(168, 85, 247))
                                } else {
                                    egui::Stroke::new(1.0_f32, CARD_BORDER)
                                };
                                let rune_bg = if is_rune_hovered { egui::Color32::from_rgb(35, 30, 52) } else { CARD_BG };

                                egui::Frame::NONE
                                    .fill(rune_bg)
                                    .corner_radius(8.0)
                                    .stroke(rune_stroke)
                                    .inner_margin(egui::Margin::same(10))
                                    .show(ui, |ui| {
                                        ui.set_width(card_w - 20.0);
                                        ui.label(egui::RichText::new(format!("Rune #{}", key)).color(TEXT_PRIMARY).size(12.0).strong());
                                        ui.add_space(4.0);
                                        if is_unlocked {
                                            ui.label(egui::RichText::new(format!("Lv. {}", level)).color(egui::Color32::from_rgb(168, 85, 247)).size(11.0).strong());
                                        } else {
                                            ui.label(egui::RichText::new("Locked").color(TEXT_MUTED).size(11.0));
                                        }
                                    });
                            },
                        ).response;

                        resp.on_hover_ui(move |ui| {
                            ui.set_min_width(180.0);
                            ui.label(egui::RichText::new(format!("Rune Node #{}", key)).color(TEXT_PRIMARY).size(13.0).strong());
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("Level: {}", level)).color(if is_unlocked { egui::Color32::from_rgb(168, 85, 247) } else { TEXT_MUTED }).size(11.0));
                            ui.label(egui::RichText::new(format!("Status: {}", if is_unlocked { "Active" } else { "Locked" })).color(if is_unlocked { GREEN } else { TEXT_MUTED }).size(11.0));
                        });
                },
            );
        }

        ui.add_space(20.0);
        ui.label(egui::RichText::new("PETS & COMPANIONS").color(TEXT_SECONDARY).size(14.0).strong());
        ui.add_space(10.0);

        if let Some(pets) = player.other.get("PetSaveData").and_then(|p| p.as_array()) {
            let spacing = GRID_SPACING;

            Self::grid_stretch(
                ui,
                pets,
                spacing,
                180.0,
                4,
                75.0,
                |_| 75.0,
                |ui, pet, card_w, card_h| {
                        let key = pet.get("PetKey").and_then(|k| k.as_i64()).unwrap_or(0);
                        let unlocked = pet.get("IsUnlock").or_else(|| pet.get("IsUnLock")).and_then(|u| u.as_bool()).unwrap_or(false);
                        let equipped = pet.get("IsEquipped").or_else(|| pet.get("IsViewed")).and_then(|e| e.as_bool()).unwrap_or(false);

                        let (pet_name, pet_buff) = Self::pet_info(key);

                        let resp = ui.allocate_ui_with_layout(
                            egui::vec2(card_w, card_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let is_pet_hovered = ui.rect_contains_pointer(ui.max_rect());
                                let pet_border_color = if equipped { YELLOW } else if unlocked { GREEN } else { CARD_BORDER };
                                let pet_stroke = egui::Stroke::new(1.0_f32, pet_border_color);
                                let pet_bg = if is_pet_hovered { egui::Color32::from_rgb(32, 35, 48) } else { CARD_BG };

                                egui::Frame::NONE
                                    .fill(pet_bg)
                                    .corner_radius(10.0)
                                    .stroke(pet_stroke)
                                    .inner_margin(egui::Margin::same(12))
                                    .show(ui, |ui| {
                                        ui.set_width(card_w - 24.0);
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(pet_name).color(TEXT_PRIMARY).size(13.0).strong());
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if equipped {
                                                    ui.label(egui::RichText::new("EQUIPPED").color(YELLOW).size(9.0).strong());
                                                } else if unlocked {
                                                    ui.label(egui::RichText::new("UNLOCKED").color(GREEN).size(9.0).strong());
                                                } else {
                                                    ui.label(egui::RichText::new("LOCKED").color(TEXT_MUTED).size(9.0));
                                                }
                                            });
                                        });
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new(pet_buff).color(TEXT_SECONDARY).size(9.0));
                                    });
                            },
                        ).response;

                        resp.on_hover_ui(move |ui| {
                            ui.set_min_width(220.0);
                            ui.label(egui::RichText::new(pet_name).color(TEXT_PRIMARY).size(14.0).strong());
                            ui.label(egui::RichText::new(format!("Pet ID: {}", key)).color(TEXT_MUTED).size(10.0));
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("Status: {}", if equipped { "Equipped Companion" } else if unlocked { "Unlocked" } else { "Locked" })).color(if equipped { YELLOW } else if unlocked { GREEN } else { TEXT_MUTED }).size(11.0));
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("Passive Buffs:").color(TEXT_MUTED).size(10.0));
                            ui.label(egui::RichText::new(format!("  • {}", pet_buff)).color(GREEN).size(11.0));
                        });
                },
            );
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("TBH Index"),
        ..Default::default()
    };
    eframe::run_native("TBH Index", options, Box::new(|cc| Ok(Box::new(TbMonitorApp::new(cc)))))
}
