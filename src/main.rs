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
const BOSS_BLUE: egui::Color32 = egui::Color32::from_rgb(47, 139, 252);
const BOSS_BG: egui::Color32 = egui::Color32::from_rgb(12, 20, 45);

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    update_interval_secs: u64,
    server_port: u16,
    ngrok_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { update_interval_secs: 30, server_port: 8080, ngrok_url: String::new() }
    }
}

impl Config {
    fn path() -> std::path::PathBuf {
        let mut p = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        p.push("tbh-monitor");
        let _ = std::fs::create_dir_all(&p);
        p.push("config.json");
        p
    }
    fn load() -> Self {
        let p = Self::path();
        std::fs::read_to_string(&p).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), s);
        }
    }
}

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
    qr_counter: u64,
    server_running: bool,
    server_port: u16,
    server_handle: Option<thread::JoinHandle<()>>,
    active_tab: Tab,
    item_names: std::collections::HashMap<String, String>,
    sort_by: SortBy,
    filter_category: ItemCategoryFilter,
    search_query: String,
    ngrok_process: Option<std::process::Child>,
    ngrok_status: String,
    icon_map: std::collections::HashMap<String, String>,
    icon_textures: std::collections::HashMap<String, egui::TextureHandle>,
    icon_download_thread: Option<thread::JoinHandle<Vec<(String, Vec<u8>)>>>,
    icon_loaded: bool,
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
            qr_counter: 0,
            server_running: false,
            server_port: 8080,
            server_handle: None,
            active_tab: Tab::Dashboard,
            item_names: std::collections::HashMap::new(),
            sort_by: SortBy::Name,
            filter_category: ItemCategoryFilter::All,
            search_query: String::new(),
            ngrok_process: None,
            ngrok_status: String::new(),
            icon_map: std::collections::HashMap::new(),
            icon_textures: std::collections::HashMap::new(),
            icon_download_thread: None,
            icon_loaded: false,
        }
    }
}

impl TbMonitorApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load();
        let mut app = Self {
            update_interval: Duration::from_secs(cfg.update_interval_secs),
            server_port: cfg.server_port,
            ngrok_url: cfg.ngrok_url,
            ..Self::default()
        };
        app.load_item_names();
        app.load_save();
        app.start_icon_download();
        app
    }
    
    fn load_item_names(&mut self) {
        let names_json = include_str!("..\\data\\names_en.json");
        if let Ok(names) = serde_json::from_str::<std::collections::HashMap<String, String>>(names_json) {
            self.item_names = names;
        }
        let icon_json = include_str!("..\\data\\icon_map.json");
        if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, String>>(icon_json) {
            self.icon_map = map;
        }
    }
    
    fn get_item_name(&self, key: i64) -> String {
        let key_str = key.to_string();
        
        // 1. Try exact match first (works for materials, grade-0 equipment)
        if let Some(name) = self.item_names.get(&key_str) {
            return name.clone();
        }
        
        // 2. Equipment key: category(2) + grade(1) + item_sub_id(3)
        //    base_key = category * 10000 + (item_sub_id / 10)
        let category = key / 10000;
        let raw_id = key % 1000;
        
        if category >= 30 && category < 90 && raw_id >= 10 {
            let item_index = raw_id / 10;
            let base_key = category * 10000 + item_index;
            if let Some(name) = self.item_names.get(&base_key.to_string()) {
                return name.clone();
            }
        }
        
        // 3. Stage boxes
        if category == 91 { return "Normal Chest".to_string(); }
        if category == 92 { return "Boss Chest".to_string(); }
        
        // 4. Descriptive fallback
        let grade = Self::item_grade(key);
        let grade_str = Self::grade_name(grade);
        let type_str = Self::item_type(key);
        if !grade_str.is_empty() && type_str != "Item" {
            format!("{} {}", grade_str, type_str)
        } else if type_str != "Item" {
            format!("{} (Grade {})", type_str, grade)
        } else {
            format!("Item #{}", key)
        }
    }
    
    fn item_grade(key: i64) -> i64 {
        let s = key.to_string();
        if s.len() == 6 {
            s.chars().nth(2).unwrap_or('0').to_digit(10).unwrap_or(0) as i64
        } else {
            0
        }
    }
    
    fn item_type(key: i64) -> &'static str {
        match key / 10000 {
            11=>"Gem", 12=>"Material", 13=>"Scroll", 14=>"Ingot", 16=>"Coin", 19=>"Soulstone",
            30=>"Sword", 31=>"Bow", 32=>"Staff", 33=>"Scepter", 34=>"Crossbow", 35=>"Axe",
            40=>"Shield", 41=>"Arrow", 42=>"Orb", 43=>"Tome", 44=>"Bolt", 45=>"Hatchet",
            50=>"Helmet", 51=>"Armor", 52=>"Gloves", 53=>"Boots",
            60=>"Amulet", 61=>"Earring", 62=>"Ring", 63=>"Bracer",
            91=>"Stage Box", 92=>"Boss Box",
            _=>"Item",
        }
    }
    
    fn grade_name(grade: i64) -> &'static str {
        match grade {
            0=>"Common", 1=>"Uncommon", 2=>"Rare", 3=>"Legendary", 4=>"Immortal",
            5=>"Arcana", 6=>"Beyond", 7=>"Celestial", 8=>"Divine", 9=>"Cosmic",
            _=>"",
        }
    }
    
    fn grade_color(grade: i64) -> egui::Color32 {
        match grade {
            0=>egui::Color32::from_rgb(228,228,228),  // Common: #e4e4e4
            1=>egui::Color32::from_rgb(84,252,12),    // Uncommon: #54fc0c
            2=>egui::Color32::from_rgb(47,139,252),   // Rare: #2f8bfc
            3=>egui::Color32::from_rgb(252,156,12),   // Legendary: #fc9c0c
            4=>egui::Color32::from_rgb(252,36,36),    // Immortal: #fc2424
            5=>egui::Color32::from_rgb(180,12,252),   // Arcana: #b40cfc
            6=>egui::Color32::from_rgb(252,36,108),   // Beyond: #fc246c
            7=>egui::Color32::from_rgb(108,204,228),  // Celestial: #6ccce4
            8=>egui::Color32::from_rgb(252,228,84),   // Divine: #fce454
            9=>egui::Color32::from_rgb(252,252,252),  // Cosmic: #fcfcfc
            _=>egui::Color32::GRAY,
        }
    }
    
    fn item_grade_bg(grade: i64) -> egui::Color32 {
        match grade {
            0=>egui::Color32::from_rgb(30,30,32),   // Common: dark gray
            1=>egui::Color32::from_rgb(15,35,15),   // Uncommon: dark green
            2=>egui::Color32::from_rgb(12,20,45),   // Rare: dark blue
            3=>egui::Color32::from_rgb(42,28,8),    // Legendary: dark orange
            4=>egui::Color32::from_rgb(45,10,10),   // Immortal: dark red
            5=>egui::Color32::from_rgb(32,10,48),   // Arcana: dark purple
            6=>egui::Color32::from_rgb(48,10,22),   // Beyond: dark pink
            7=>egui::Color32::from_rgb(15,35,42),   // Celestial: dark cyan
            8=>egui::Color32::from_rgb(45,40,12),   // Divine: dark gold
            9=>egui::Color32::from_rgb(42,42,48),  // Cosmic: dark white
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
            self.qr_counter += 1;
            let tex_name = format!("qr_{}", self.qr_counter);
            self.qr_texture = Some(ctx.load_texture(&tex_name, color_image, egui::TextureOptions::NEAREST));
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
    
    fn ngrok_is_installed() -> bool {
        std::process::Command::new("ngrok").arg("version").output().map_or(false, |o| o.status.success())
    }
    
    fn fetch_ngrok_url() -> Option<String> {
        // Use curl.exe to query ngrok's local API
        let out = std::process::Command::new("curl.exe")
            .args(["-s", "http://127.0.0.1:4040/api/tunnels"])
            .output().ok()?;
        if !out.status.success() { return None; }
        let body = String::from_utf8_lossy(&out.stdout);
        // Parse JSON to find https tunnel URL
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(tunnels) = val.get("tunnels").and_then(|t| t.as_array()) {
                for t in tunnels {
                    if let Some(url) = t.get("public_url").and_then(|u| u.as_str()) {
                        if url.starts_with("https://") {
                            return Some(url.to_string());
                        }
                    }
                }
            }
        }
        None
    }
    
    fn start_ngrok(&mut self) {
        if !Self::ngrok_is_installed() {
            self.ngrok_status = "ngrok tidak terinstall — download dari https://ngrok.com/download".to_string();
            return;
        }
        self.stop_ngrok();
        let port = self.server_port.to_string();
        match std::process::Command::new("ngrok")
            .args(["http", &port, "--log=stdout"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                self.ngrok_process = Some(child);
                self.ngrok_status = "ngrok running, menunggu URL...".to_string();
            }
            Err(e) => {
                self.ngrok_status = format!("Gagal start ngrok: {}", e);
            }
        }
    }
    
    fn stop_ngrok(&mut self) {
        if let Some(mut child) = self.ngrok_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn base_key(key: i64) -> String {
        let category = key / 10000;
        let raw_id = key % 1000;
        if category >= 11 && category < 90 && raw_id >= 10 {
            let item_index = raw_id / 10;
            (category * 10000 + item_index).to_string()
        } else {
            key.to_string()
        }
    }

    fn get_icon_filename(&self, key: i64) -> Option<String> {
        let bk = Self::base_key(key);
        self.icon_map.get(&bk).cloned()
    }

    fn start_icon_download(&mut self) {
        if self.icon_download_thread.is_some() { return; }
        // Collect all unique icon filenames from current save
        let needed = self.collect_needed_icons();
        if needed.is_empty() { self.icon_loaded = true; return; }
        let needed_arc = Arc::new(needed);
        let _status = self.status.clone();
        self.status = format!("Downloading {} icons...", needed_arc.len());
        let handle = thread::spawn(move || {
            let mut results = Vec::new();
            let base_url = "https://raw.githubusercontent.com/andrenogrib/tbh_saveeditor/main/data/icons";
            let dest_dir = dirs::config_dir().map(|mut p| { p.push("tbh-monitor"); p.push("icons"); p }).unwrap_or_else(|| std::path::PathBuf::from("icons"));
            let _ = std::fs::create_dir_all(&dest_dir);
            for fname in needed_arc.iter() {
                let dest = dest_dir.join(fname);
                if dest.exists() {
                    match std::fs::read(&dest) {
                        Ok(bytes) => { results.push((fname.clone(), bytes)); continue; }
                        Err(_) => {}
                    }
                }
                // Download from GitHub
                let url = format!("{}/{}", base_url, fname);
                if let Ok(output) = std::process::Command::new("curl.exe")
                    .args(["-s", &url])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .output()
                {
                    if output.status.success() {
                        let bytes = output.stdout;
                        let _ = std::fs::write(&dest, &bytes);
                        results.push((fname.clone(), bytes));
                    }
                }
            }
            results
        });
        self.icon_download_thread = Some(handle);
    }

    fn collect_needed_icons(&self) -> Vec<String> {
        let mut needed: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(ref player) = self.player_data {
            // From items
            if let Some(items) = player.other.get("itemSaveDatas").and_then(|i| i.as_array()) {
                for item in items {
                    if let Some(key) = item.get("ItemKey").and_then(|k| k.as_i64()) {
                        if let Some(fname) = self.get_icon_filename(key) {
                            needed.insert(fname);
                        }
                    }
                }
            }
            // From hero equipment
            if let Some(heroes) = player.other.get("heroSaveDatas").and_then(|h| h.as_array()) {
                if let Some(items) = player.other.get("itemSaveDatas").and_then(|i| i.as_array()) {
                    for hero in heroes {
                        if let Some(ids) = hero.get("equippedItemIds").and_then(|e| e.as_array()) {
                            for id_val in ids {
                                if let Some(uid) = id_val.as_i64() {
                                    if uid == 0 { continue; }
                                    for item in items {
                                        if item.get("UniqueId").and_then(|u| u.as_i64()) == Some(uid) {
                                            if let Some(key) = item.get("ItemKey").and_then(|k| k.as_i64()) {
                                                if let Some(fname) = self.get_icon_filename(key) {
                                                    needed.insert(fname);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        needed.into_iter().collect()
    }

    fn process_icon_downloads(&mut self, ctx: &egui::Context) {
        if let Some(handle) = self.icon_download_thread.take() {
            if handle.is_finished() {
                match handle.join() {
                    Ok(results) => {
                        for (fname, bytes) in results {
                            self.load_icon_texture(ctx, &fname, &bytes);
                        }
                        self.icon_loaded = true;
                        self.status = "Loaded (with icons)".to_string();
                        ctx.request_repaint();
                    }
                    Err(_) => {
                        self.icon_loaded = true;
                        self.status = "Loaded (icons failed)".to_string();
                    }
                }
            } else {
                self.icon_download_thread = Some(handle);
            }
        } else if self.needs_icon_download() {
            self.start_icon_download();
        }
    }

    fn needs_icon_download(&self) -> bool {
        !self.icon_loaded && self.icon_download_thread.is_none() && self.player_data.is_some()
    }

    fn load_icon_texture(&mut self, ctx: &egui::Context, fname: &str, bytes: &[u8]) {
        if let Ok(img) = image::load_from_memory(bytes) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let pixels = rgba.into_raw();
            let color_img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
            let tex_name = format!("item_icon_{}", fname);
            let tex = ctx.load_texture(&tex_name, color_img, egui::TextureOptions::NEAREST);
            self.icon_textures.insert(fname.to_string(), tex);
        }
    }

    fn get_icon_texture(&self, key: i64) -> Option<&egui::TextureHandle> {
        let fname = self.get_icon_filename(key)?;
        self.icon_textures.get(&fname)
    }
}

impl TbMonitorApp {
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
        let grade = Self::item_grade(key).max(0).min(9);
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

    fn rune_name(key: i64) -> &'static str {
        match key {
            1 => "Rune of War",
            10 => "Rune of Wealth",
            11 | 12 | 13 | 14 | 15 | 16 => "Rune of Expansion",
            20 => "Rune of Growth",
            21 | 24 => "Rune of Command",
            22 | 23 => "Rune of Expansion",
            25 => "Rune of Wealth",
            26 => "Rune of Growth",
            27 => "Rune of Awakening",
            101 | 103 | 105 | 107 | 109 | 111 | 113 | 115 | 117 | 119 | 121 | 123 | 125 | 127 => "Rune of Exploration",
            102 | 104 | 106 | 108 | 110 | 112 | 114 | 116 | 118 | 120 | 122 | 124 | 126 | 128 => "Rune of Conquest",
            201 | 202 | 203 | 204 | 205 | 206 | 207 | 208 | 209 | 210 | 211 | 212 | 213 | 214 | 215 => "Rune of Wealth",
            301 | 302 | 303 | 304 | 305 | 306 | 307 | 308 | 309 | 310 | 311 | 312 | 313 | 314 | 315 => "Rune of Growth",
            401 | 403 | 407 | 410 | 412 => "Rune of the Shield",
            402 | 404 | 406 | 4082 | 4101 => "Rune of the Gale",
            405 | 408 | 411 | 413 | 4031 | 4081 => "Rune of War",
            409 | 414 | 4061 => "Rune of Frenzy",
            1021 => "Rune of Opening",
            1031 => "Rune of Containment",
            1051 | 1054 | 1801 | 1802 | 1803 | 1804 | 1805 => "Rune of Expansion",
            1052 => "Rune of Containment",
            1053 => "Rune of Exploration",
            1055 => "Rune of Opening",
            1056 | 1281 | 12821 | 11004 => "Rune of Infinity",
            1061 | 1101 | 1161 | 11002 => "Rune of Containment",
            1071 | 11003 => "Rune of the Vault",
            1171 => "Rune of Brevity",
            2031 | 3122 | 2132 | 3032 => "Rune of Alchemy",
            2032 | 3031 | 3121 | 2131 => "Rune of Forging",
            2071 | 2091 | 2111 | 2151 | 2152 => "Rune of Wealth",
            3061 | 3091 | 3151 | 3152 => "Rune of Growth",
            11001 => "Rune of Repose",
            110011 => "Rune of Hoarding",
            110012 | 180301 | 180501 => "Rune of Training",
            13001 | 16001 | 160011 => "Rune of Storage",
            13002 | 15001 | 1902001 => "Rune of the Mainspring",
            130021 | 150011 | 190301 | 190302 | 190401 | 190501 | 190502 | 1905011 | 1905021 | 19020011 => "Rune of Lubrication",
            15002 | 180201 | 180401 | 180601 => "Rune of Hoarding",
            _ => "Unknown Rune",
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
        if items.is_empty() { return; }
        use taffy::prelude::*;
        let avail_w = ui.available_width();
        let pad = spacing;
        let content_w = (avail_w - pad * 2.0).max(0.0);
        let cols = (((content_w + spacing) / (min_card_w + spacing)).floor() as usize)
            .clamp(1, max_cols.max(1));
        let card_w = ((content_w - spacing * (cols as f32 - 1.0)) / cols as f32).max(min_card_w);

        let mut taffy = TaffyTree::<()>::new();
        let mut child_nodes = Vec::new();

        for chunk in items.chunks(cols) {
            let row_h = chunk.iter().map(|it| height_fn(it)).fold(default_h, f32::max);
            for _item in chunk {
                let child_style = Style {
                    size: Size {
                        width: Dimension::Length(card_w),
                        height: Dimension::Length(row_h),
                    },
                    ..Default::default()
                };
                let child = taffy.new_leaf(child_style).unwrap();
                child_nodes.push(child);
            }
        }

        let container_style = Style {
            display: Display::Grid,
            grid_template_columns: (0..cols)
                .map(|_| TrackSizingFunction::from_length(card_w))
                .collect(),
            gap: Size {
                width: LengthPercentage::Length(spacing),
                height: LengthPercentage::Length(spacing),
            },
            padding: Rect {
                left: LengthPercentage::Length(pad),
                right: LengthPercentage::Length(pad),
                top: LengthPercentage::Length(pad),
                bottom: LengthPercentage::Length(pad),
            },
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        };

        let container = taffy.new_with_children(container_style, &child_nodes).unwrap();
        taffy.compute_layout(
            container,
            Size {
                width: AvailableSpace::Definite(avail_w),
                height: AvailableSpace::MaxContent,
            },
        ).unwrap();

        let container_layout = taffy.layout(container).unwrap();
        let total_grid_h = container_layout.size.height;

        ui.allocate_ui(egui::vec2(avail_w, total_grid_h), |ui| {
            for (idx, &child) in child_nodes.iter().enumerate() {
                let layout = taffy.layout(child).unwrap();
                let x = layout.location.x;
                let y = layout.location.y;
                let w = layout.size.width;
                let h = layout.size.height;

                let rect = egui::Rect::from_min_size(
                    ui.min_rect().min + egui::vec2(x, y),
                    egui::vec2(w, h),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                    render_item(ui, &items[idx], w, h);
                });
            }
        });
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
        grid_pad: f32,
        mut render_item: impl FnMut(&mut egui::Ui, &T, f32, f32),
    ) {
        if items.is_empty() { return; }
        use taffy::prelude::*;
        let avail_w = ui.available_width();
        let content_w = (avail_w - grid_pad * 2.0).max(0.0);
        let cols = (((content_w + spacing) / (card_w + spacing)).floor() as usize)
            .clamp(1, max_cols.max(1));

        let mut taffy = TaffyTree::<()>::new();
        let mut child_nodes = Vec::new();
        for _item in items {
            let child_style = Style {
                size: Size {
                    width: Dimension::Length(card_w),
                    height: Dimension::Length(card_h),
                },
                ..Default::default()
            };
            let child = taffy.new_leaf(child_style).unwrap();
            child_nodes.push(child);
        }

        let container_style = Style {
            display: Display::Grid,
            grid_template_columns: (0..cols)
                .map(|_| TrackSizingFunction::from_length(card_w))
                .collect(),
            gap: Size {
                width: LengthPercentage::Length(spacing),
                height: LengthPercentage::Length(spacing),
            },
            padding: Rect {
                left: LengthPercentage::Length(grid_pad),
                right: LengthPercentage::Length(grid_pad),
                top: LengthPercentage::Length(grid_pad),
                bottom: LengthPercentage::Length(grid_pad),
            },
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        };

        let container = taffy.new_with_children(container_style, &child_nodes).unwrap();
        taffy.compute_layout(
            container,
            Size {
                width: AvailableSpace::Definite(avail_w),
                height: AvailableSpace::MaxContent,
            },
        ).unwrap();

        let container_layout = taffy.layout(container).unwrap();
        let total_grid_h = container_layout.size.height;

        ui.allocate_ui(egui::vec2(avail_w, total_grid_h), |ui| {
            for (idx, &child) in child_nodes.iter().enumerate() {
                let layout = taffy.layout(child).unwrap();
                let x = layout.location.x;
                let y = layout.location.y;
                let w = layout.size.width;
                let h = layout.size.height;

                let rect = egui::Rect::from_min_size(
                    ui.min_rect().min + egui::vec2(x, y),
                    egui::vec2(w, h),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                    render_item(ui, &items[idx], w, h);
                });
            }
        });
    }

    /// Like `grid_stretch`, but forces every card to be a perfect square
    /// (height == width) and stretches columns so the row always fills the
    /// full available width — no leftover margin on either side, so no
    /// dead space before a scrollbar. Best for icon-grid content
    /// (inventory items, chests) where uniform square tiles look best.
    ///
    /// `max_card_w` caps how large a card can stretch to — without it, a
    /// wide window with few columns would stretch cards (and therefore
    /// their forced-equal height) far past what the content needs, leaving
    /// a dead vertical gap under the content in every row.
    fn grid_square<T>(
        ui: &mut egui::Ui,
        items: &[T],
        spacing: f32,
        min_card_w: f32,
        max_card_w: f32,
        max_cols: usize,
        mut render_item: impl FnMut(&mut egui::Ui, &T, f32, f32),
    ) {
        if items.is_empty() { return; }
        use taffy::prelude::*;
        let avail_w = ui.available_width();
        // Smallest column count that keeps cards from exceeding max_card_w...
        let cols_for_max_w = (((avail_w + spacing) / (max_card_w + spacing)).ceil() as usize).max(1);
        // ...but never more columns than would shrink cards below min_card_w.
        let cols_for_min_w = (((avail_w + spacing) / (min_card_w + spacing)).floor() as usize).max(1);
        let cols = cols_for_max_w.min(cols_for_min_w).clamp(1, max_cols.max(1));
        let card_w = ((avail_w - spacing * (cols as f32 - 1.0)) / cols as f32).max(min_card_w);
        let card_h = card_w;

        let mut taffy = TaffyTree::<()>::new();
        let mut child_nodes = Vec::new();
        for _item in items {
            let child_style = Style {
                size: Size {
                    width: Dimension::Length(card_w),
                    height: Dimension::Length(card_h),
                },
                ..Default::default()
            };
            child_nodes.push(taffy.new_leaf(child_style).unwrap());
        }

        let container_style = Style {
            display: Display::Grid,
            grid_template_columns: (0..cols)
                .map(|_| TrackSizingFunction::from_length(card_w))
                .collect(),
            gap: Size {
                width: LengthPercentage::Length(spacing),
                height: LengthPercentage::Length(spacing),
            },
            ..Default::default()
        };

        let container = taffy.new_with_children(container_style, &child_nodes).unwrap();
        taffy.compute_layout(
            container,
            Size {
                width: AvailableSpace::Definite(avail_w),
                height: AvailableSpace::MaxContent,
            },
        ).unwrap();

        let container_layout = taffy.layout(container).unwrap();
        let total_grid_h = container_layout.size.height;

        ui.allocate_ui(egui::vec2(avail_w, total_grid_h), |ui| {
            for (idx, &child) in child_nodes.iter().enumerate() {
                let layout = taffy.layout(child).unwrap();
                let x = layout.location.x;
                let y = layout.location.y;
                let w = layout.size.width;
                let h = layout.size.height;

                let rect = egui::Rect::from_min_size(
                    ui.min_rect().min + egui::vec2(x, y),
                    egui::vec2(w, h),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                    render_item(ui, &items[idx], w, h);
                });
            }
        });
    }

    fn grid_masonry<T>(
        ui: &mut egui::Ui,
        items: &[T],
        heights: &[f32],
        spacing: f32,
        min_card_w: f32,
        max_cols: usize,
        mut render_item: impl FnMut(&mut egui::Ui, &T, f32, f32),
    ) {
        if items.is_empty() { return; }
        use taffy::prelude::*;
        let avail_w = ui.available_width();
        // padding on both sides
        let pad = spacing;
        let content_w = (avail_w - pad * 2.0).max(0.0);
        let cols = (((content_w + spacing) / (min_card_w + spacing)).floor() as usize)
            .clamp(1, max_cols.max(1));
        let card_w = ((content_w - spacing * (cols as f32 - 1.0)) / cols as f32).max(min_card_w);

        // Distribute items to columns (shortest column first)
        let mut col_heights: Vec<f32> = vec![0.0; cols];
        let mut col_items: Vec<Vec<(usize, f32)>> = vec![Vec::new(); cols];
        for (i, h) in heights.iter().enumerate() {
            let shortest = col_heights.iter().enumerate().min_by_key(|(_, ch)| (*ch * 1000.0) as i64).unwrap().0;
            if !col_items[shortest].is_empty() {
                col_heights[shortest] += spacing;
            }
            col_items[shortest].push((i, *h));
            col_heights[shortest] += h;
        }

        // Build taffy tree: one column per grid column
        let mut taffy = TaffyTree::<()>::new();
        let mut col_nodes = Vec::new();
        for col in &col_items {
            let mut child_nodes = Vec::new();
            for &(_, h) in col {
                let child_style = Style {
                    size: Size {
                        width: Dimension::Length(card_w),
                        height: Dimension::Length(h),
                    },
                    ..Default::default()
                };
                child_nodes.push(taffy.new_leaf(child_style).unwrap());
            }
            let col_style = Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                gap: Size {
                    width: LengthPercentage::Length(0.0),
                    height: LengthPercentage::Length(spacing),
                },
                ..Default::default()
            };
            let col_node = if child_nodes.is_empty() {
                taffy.new_leaf(Style { size: Size { width: Dimension::Length(card_w), height: Dimension::Length(0.0) }, ..Default::default() }).unwrap()
            } else {
                taffy.new_with_children(col_style, &child_nodes).unwrap()
            };
            col_nodes.push(col_node);
        }

        let container_style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            gap: Size {
                width: LengthPercentage::Length(spacing),
                height: LengthPercentage::Length(0.0),
            },
            padding: Rect {
                left: LengthPercentage::Length(pad),
                right: LengthPercentage::Length(pad),
                top: LengthPercentage::Length(pad),
                bottom: LengthPercentage::Length(pad),
            },
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        };
        let container = taffy.new_with_children(container_style, &col_nodes).unwrap();
        taffy.compute_layout(
            container,
            Size {
                width: AvailableSpace::Definite(avail_w),
                height: AvailableSpace::MaxContent,
            },
        ).unwrap();

        let total_h = taffy.layout(container).unwrap().size.height;

        ui.allocate_ui(egui::vec2(avail_w, total_h), |ui| {
            // Render each column
            for (col_idx, col) in col_items.iter().enumerate() {
                let col_layout = taffy.layout(col_nodes[col_idx]).unwrap();
                let col_x = col_layout.location.x;
                let col_y = col_layout.location.y;

                ui.allocate_new_ui(
                    egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                        ui.min_rect().min + egui::vec2(col_x, col_y),
                        egui::vec2(card_w, col_layout.size.height),
                    )),
                    |ui| {
                        for &(item_idx, h) in col {
                            let child_layout = taffy.layout(taffy.children(col_nodes[col_idx]).unwrap()[col.iter().position(|&(i, _)| i == item_idx).unwrap()]).unwrap();
                            let child_y = child_layout.location.y;
                            ui.allocate_new_ui(
                                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                                    ui.min_rect().min + egui::vec2(0.0, child_y),
                                    egui::vec2(card_w, h),
                                )),
                                |ui| {
                                    render_item(ui, &items[item_idx], card_w, h);
                                },
                            );
                        }
                    },
                );
            }
        });
    }

    fn stat_card(ui: &mut egui::Ui, width: f32, title: &str, value_text: egui::RichText) {
        ui.allocate_ui_with_layout(
            egui::vec2(width, 70.0),
            egui::Layout::top_down_justified(egui::Align::Min),
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
            // Re-download icons if save changed
            if self.player_data.is_some() && self.icon_loaded {
                self.icon_loaded = false;
                self.start_icon_download();
            }
        }

        // Process icon downloads
        self.process_icon_downloads(ctx);
        
        // Auto-poll ngrok URL if ngrok is running
        if self.ngrok_process.is_some() && self.ngrok_url.is_empty() {
            if let Some(url) = Self::fetch_ngrok_url() {
                self.ngrok_url = url.clone();
                self.ngrok_status = format!("ngrok: {}", url);
                self.generate_qr(ctx);
            } else {
                // Check if ngrok process died (auth error etc.)
                if let Some(ref mut child) = self.ngrok_process {
                    if child.try_wait().ok().flatten().is_some() {
                        self.ngrok_process = None;
                        self.ngrok_status = "ngrok gagal — cek terminal atau login dengan: ngrok config add-authtoken <token>".to_string();
                    }
                }
            }
        }
        
        ctx.set_visuals(egui::Visuals {
            dark_mode: true,
            override_text_color: Some(TEXT_PRIMARY),
            panel_fill: BG_DARK,
            window_fill: CARD_BG,
            ..Default::default()
        });
        
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::NONE.fill(BG_DARK).inner_margin(egui::Margin::symmetric(16, 8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new("TBH INDEX").color(ACCENT).size(20.0).strong()));
                    ui.add(egui::Label::new(egui::RichText::new("Taskbar Hero Stash Tracker").color(TEXT_MUTED).size(12.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("Settings").color(TEXT_SECONDARY).size(12.0)).fill(CARD_BG).stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))).clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        ui.label(egui::RichText::new(&self.status).color(TEXT_MUTED).size(11.0));
                    });
                });
            });
        
        egui::TopBottomPanel::top("nav")
            .frame(egui::Frame::NONE.fill(BG_DARK).inner_margin(egui::Margin::symmetric(16, 4)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [(Tab::Dashboard, "Dashboard"), (Tab::Heroes, "Heroes"), (Tab::Inventory, "Inventory"), (Tab::Runes, "Runes")] {
                        let selected = self.active_tab == tab;
                        let text = egui::RichText::new(label).color(if selected { ACCENT } else { TEXT_SECONDARY }).size(13.0);
                        if ui.selectable_label(selected, text).clicked() { self.active_tab = tab; }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(egui::RichText::new("Reload (sec)").color(TEXT_MUTED).size(11.0)));
                        let mut secs = self.update_interval.as_secs() as f32;
                        ui.add(egui::Slider::new(&mut secs, 5.0..=300.0).text(""));
                        if self.update_interval.as_secs() != secs as u64 {
                            self.update_interval = Duration::from_secs_f32(secs);
                            Config { update_interval_secs: secs as u64, server_port: self.server_port, ngrok_url: self.ngrok_url.clone() }.save();
                        }
                    });
                });
            });
        
        if self.show_settings {
            egui::Window::new("Settings")
                .collapsible(false).resizable(false)
                .frame(egui::Frame::NONE.fill(CARD_BG).corner_radius(12.0).stroke(egui::Stroke::new(1.0_f32, CARD_BORDER)).inner_margin(20.0))
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("Update Interval (sec)").color(TEXT_SECONDARY).size(12.0));
                    let mut secs = self.update_interval.as_secs() as u32;
                    if ui.add(egui::DragValue::new(&mut secs).speed(1).range(5..=300)).changed() {
                        self.update_interval = Duration::from_secs(secs as u64);
                        Config { update_interval_secs: secs as u64, server_port: self.server_port, ngrok_url: self.ngrok_url.clone() }.save();
                    }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Server Port").color(TEXT_SECONDARY).size(12.0));
                    let mut port = self.server_port;
                    if ui.add(egui::DragValue::new(&mut port).speed(1).range(1024..=65535)).changed() {
                        self.server_port = port;
                        Config { update_interval_secs: self.update_interval.as_secs(), server_port: port, ngrok_url: self.ngrok_url.clone() }.save();
                    }
                    if !self.server_running { if ui.button("Start Server").clicked() { self.start_server(); } }
                    else { ui.label(egui::RichText::new("Server running!").color(GREEN).size(12.0)); }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Ngrok URL").color(TEXT_SECONDARY).size(12.0));
                    if ui.text_edit_singleline(&mut self.ngrok_url).changed() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        Config { update_interval_secs: self.update_interval.as_secs(), server_port: self.server_port, ngrok_url: self.ngrok_url.clone() }.save();
                        self.qr_texture = None;
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Start Ngrok (1-click)").clicked() {
                            self.start_ngrok();
                        }
                        if self.ngrok_process.is_some() {
                            if ui.button("Stop Ngrok").clicked() {
                                self.stop_ngrok();
                                self.ngrok_url.clear();
                                self.ngrok_status.clear();
                                self.qr_texture = None;
                            }
                        }
                    });
                    if !self.ngrok_status.is_empty() {
                        ui.label(egui::RichText::new(&self.ngrok_status).color(TEXT_SECONDARY).size(11.0));
                    }
                    if !self.ngrok_url.is_empty() {
                        if self.qr_texture.is_none() {
                            self.generate_qr(ctx);
                        }
                        if let Some(tex) = &self.qr_texture {
                            ui.add_space(8.0);
                            ui.add(egui::widgets::Image::new(tex).max_width(160.0).max_height(160.0));
                            ui.label(egui::RichText::new(&self.ngrok_url).color(TEXT_SECONDARY).size(9.0));
                        }
                    }
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() { self.show_settings = false; }
                });
        }
        
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG_DARK).inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| {
                if let Some(player) = self.player_data.clone() {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(16.0);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(16, 0))
                            .show(ui, |ui| {
                                match self.active_tab {
                                    Tab::Dashboard => self.render_dashboard(ui, &player),
                                    Tab::Heroes => self.render_heroes(ui, &player),
                                    Tab::Inventory => self.render_inventory(ui, &player),
                                    Tab::Runes => self.render_runes(ui, &player),
                                }
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
                3,
                80.0,
                |_| 80.0,
                |ui, hero, hero_card_w, hero_card_h| {
                    let key = hero.get("heroKey").and_then(|k| k.as_i64()).unwrap_or(0);
                    let level = hero.get("HeroLevel").and_then(|l| l.as_i64()).unwrap_or(0);
                    let exp = hero.get("HeroExp").and_then(|e| e.as_f64()).unwrap_or(0.0) as i64;
                    let unlocked = hero.get("IsUnLock").and_then(|u| u.as_bool()).unwrap_or(false);

                    let response = ui.allocate_ui_with_layout(
                        egui::vec2(hero_card_w, hero_card_h),
                        egui::Layout::top_down_justified(egui::Align::Min),
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
                let height = if unlocked { if has_gear { 120.0 } else { 110.0 } } else { 60.0 };
                HeroCard { value: hero, key, level, exp, unlocked, ability_points, allocated, equipped_ids, has_gear, height }
            }).collect();

            let mut hero_cards = hero_cards;
            hero_cards.sort_by(|a, b| {
                let a_cnt = a.equipped_ids.iter().filter(|&&id| id != 0).count();
                let b_cnt = b.equipped_ids.iter().filter(|&&id| id != 0).count();
                b_cnt.cmp(&a_cnt)
            });

            let hero_heights: Vec<f32> = hero_cards.iter().map(|hc| hc.height).collect();

            Self::grid_masonry(
                ui,
                &hero_cards,
                &hero_heights,
                spacing,
                280.0,
                3,
                |ui, hc, card_w, card_h| {
                        let _response = ui.allocate_ui_with_layout(
                            egui::vec2(card_w, card_h),
                            egui::Layout::top_down_justified(egui::Align::Min),
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

                                                let e_card_w = 22.0;
                                                let e_margin = 1.0;
                                                let e_outer = e_card_w + e_margin * 2.0;
                                                let e_card_h = 26.0;

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
                                                        e_card_h,
                                                        10,
                                                        1.0,
                                                        |ui, (slot_idx, item), _slot_w, _slot_h| {
                                                                let item_key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                                                                let grade = Self::item_grade(item_key).max(0).min(9);
                                                                let bg = Self::item_grade_bg(grade);
                                                                let border_color = Self::grade_color(grade);
                                                                let name = self.get_item_name(item_key);
                                                                let chaotic = item.get("IsChaotic").and_then(|c| c.as_bool()).unwrap_or(false);
                                                                let enchants = item.get("EnchantCount").and_then(|e| e.as_array())
                                                                    .map(|a| a.iter().filter_map(|v| v.as_i64()).sum::<i64>()).unwrap_or(0);
                                                                let icon_texture = self.get_icon_texture(item_key);

                                                                let (slot_rect, resp) = ui.allocate_exact_size(
                                                                    egui::vec2(e_outer, e_card_h),
                                                                    egui::Sense::hover(),
                                                                );
                                                                let is_slot_hovered = resp.hovered();
                                                                let slot_bg = if is_slot_hovered {
                                                                    egui::Color32::from_rgb(bg.r().saturating_add(25), bg.g().saturating_add(25), bg.b().saturating_add(30))
                                                                } else {
                                                                    bg
                                                                };
                                                                let sp = ui.painter();
                                                                sp.rect_filled(slot_rect, 3.0, slot_bg);
                                                                sp.rect_stroke(slot_rect, 3.0, egui::Stroke::new(1.0_f32, border_color), egui::StrokeKind::Outside);

                                                                // Icon fills the slot
                                                                if let Some(tex) = icon_texture {
                                                                    let ir = slot_rect.shrink(2.0);
                                                                    sp.image(
                                                                        tex.id(),
                                                                        ir,
                                                                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                                                        egui::Color32::WHITE,
                                                                    );
                                                                }

                                                                // Grade/enchant tiny indicator at bottom
                                                                if chaotic {
                                                                    sp.circle_filled(slot_rect.left_bottom() + egui::vec2(3.0, -3.0), 2.0, YELLOW);
                                                                }

                                                                 let tip_name = name.clone();
                                                                let tip_border = border_color;
                                                                let tip_slot = SLOT_NAMES[*slot_idx];
                                                                let grade_name = Self::grade_name(grade);
                                                                let tip_type = Self::item_type(item_key);
                                                                let item_ref = item;
                                                                let tip_uid = item.get("UniqueId").and_then(|u| u.as_i64()).unwrap_or(0);
                                                                let tip_level = item.get("Level").and_then(|l| l.as_i64())
                                                                    .or_else(|| item.get("ItemLevel").and_then(|l| l.as_i64()))
                                                                    .unwrap_or(0);
                                                                let tip_qty = item.get("Quantity").and_then(|q| q.as_i64()).unwrap_or(1);
                                                                let tip_insc = item.get("InscriptionAppliedTotalCount").and_then(|i| i.as_i64()).unwrap_or(0);
                                                                let tip_engr = item.get("EngravingAppliedTotalCount").and_then(|e| e.as_i64()).unwrap_or(0);
                                                                let tip_deco = item.get("DecorationAppliedTotalCount").and_then(|d| d.as_i64()).unwrap_or(0);
                                                                resp.on_hover_ui(move |ui| {
                                                                    ui.set_min_width(220.0);
                                                                    ui.label(egui::RichText::new(tip_slot).color(TEXT_MUTED).size(10.0));
                                                                    ui.label(egui::RichText::new(&tip_name).color(tip_border).size(14.0).strong());
                                                                    ui.label(egui::RichText::new(format!("Grade: {}", grade_name)).color(tip_border).size(11.0));
                                                                    ui.label(egui::RichText::new(format!("Type: {}", tip_type)).color(TEXT_SECONDARY).size(11.0));
                                                                    if tip_level > 0 {
                                                                        ui.label(egui::RichText::new(format!("Level: {}", tip_level)).color(TEXT_SECONDARY).size(11.0));
                                                                    }
                                                                    ui.label(egui::RichText::new(format!("UID: {}", tip_uid)).color(TEXT_MUTED).size(10.0));
                                                                    if tip_qty > 1 {
                                                                        ui.label(egui::RichText::new(format!("Quantity: {}", tip_qty)).color(YELLOW).size(11.0).strong());
                                                                    }
                                                                    if chaotic {
                                                                        ui.add_space(2.0);
                                                                        ui.label(egui::RichText::new("CHAOTIC").color(YELLOW).size(10.0).strong());
                                                                    }
                                                                    if enchants > 0 {
                                                                        ui.label(egui::RichText::new(format!("Enchants: +{}", enchants)).color(ACCENT).size(10.0));
                                                                    }
                                                                    let stats = Self::format_item_stats(item_ref);
                                                                    if !stats.is_empty() {
                                                                        ui.add_space(4.0);
                                                                        ui.separator();
                                                                        ui.add_space(4.0);
                                                                        ui.label(egui::RichText::new("Stats & Buffs:").color(TEXT_MUTED).size(10.0));
                                                                        for stat in stats {
                                                                            ui.label(egui::RichText::new(format!("  • {}", stat)).color(GREEN).size(10.0));
                                                                        }
                                                                    }
                                                                    // Socket / Inscription / Engraving / Decoration
                                                                    let socket_data = item_ref.get("SocketData").and_then(|s| s.as_array());
                                                                    let socket_count = socket_data.map(|a| a.len()).unwrap_or(0);
                                                                    let has_extra = tip_insc > 0 || tip_engr > 0 || tip_deco > 0 || socket_count > 0;
                                                                    if has_extra {
                                                                        ui.add_space(4.0);
                                                                        ui.separator();
                                                                        ui.add_space(2.0);
                                                                        if socket_count > 0 {
                                                                            let filled = socket_data.map(|a| a.iter().filter(|s| !s.is_null() && s.as_object().map_or(false, |o| !o.is_empty())).count()).unwrap_or(0);
                                                                            ui.label(egui::RichText::new(format!("  Sockets: {}/{}", filled, socket_count)).color(TEXT_SECONDARY).size(10.0));
                                                                        }
                                                                        if tip_insc > 0 {
                                                                            ui.label(egui::RichText::new(format!("  Inscriptions: {}", tip_insc)).color(TEXT_SECONDARY).size(10.0));
                                                                        }
                                                                        if tip_engr > 0 {
                                                                            ui.label(egui::RichText::new(format!("  Engravings: {}", tip_engr)).color(TEXT_SECONDARY).size(10.0));
                                                                        }
                                                                        if tip_deco > 0 {
                                                                            ui.label(egui::RichText::new(format!("  Decorations: {}", tip_deco)).color(TEXT_SECONDARY).size(10.0));
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
            // Separate chests from regular items
            let chests: Vec<_> = items.iter().filter(|item| {
                let key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                let cat = key / 10000;
                cat == 91 || cat == 92
            }).collect();
            
            let regular: Vec<_> = items.iter().filter(|item| {
                let key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                let cat = key / 10000;
                cat != 91 && cat != 92
            }).collect();
            
            // ---- Chests Section ----
            if !chests.is_empty() {
                ui.label(egui::RichText::new(format!("Chests ({})", chests.len())).color(TEXT_MUTED).size(13.0).strong());
                ui.add_space(6.0);
                let chest_spacing = COMPACT_GRID_SPACING;
                let c_card_w = 44.0;
                let c_margin = 2.0;
                let c_outer = c_card_w + c_margin * 2.0;
                Self::grid_square(
                    ui,
                    &chests,
                    chest_spacing,
                    c_outer,
                    c_outer + 4.0,
                    usize::MAX,
                    |ui, item, card_w, card_h| {
                        let key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                        let name = self.get_item_name(key);
                        let cat = key / 10000;
                        let chest_label = if cat == 91 { "Stage" } else { "Boss" };
                        let is_boss = cat == 92;
                        let border_color = if is_boss { BOSS_BLUE } else { TEXT_MUTED };
                        let bg = if is_boss { BOSS_BG } else { CARD_BG };
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(card_w, card_h), egui::Sense::hover());
                        let hovered = response.hovered();
                        let card_bg = if hovered {
                            egui::Color32::from_rgb(bg.r().saturating_add(25), bg.g().saturating_add(25), bg.b().saturating_add(30))
                        } else { bg };
                        let painter = ui.painter();
                        painter.rect_filled(rect, 4.0, card_bg);
                        painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0_f32, border_color), egui::StrokeKind::Outside);
                        let label_color = if is_boss { BOSS_BLUE } else { TEXT_SECONDARY };
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            chest_label,
                            egui::FontId::proportional(7.0),
                            label_color,
                        );
                        response.on_hover_ui(move |ui| {
                            ui.set_min_width(200.0);
                            ui.label(egui::RichText::new(&name).color(border_color).size(14.0).strong());
                            ui.label(egui::RichText::new(format!("Type: {}", chest_label)).color(TEXT_SECONDARY).size(11.0));
                            ui.label(egui::RichText::new(format!("ID: {}", key)).color(TEXT_MUTED).size(10.0));
                            let qty = item.get("Quantity").and_then(|q| q.as_i64()).unwrap_or(1);
                            if qty > 1 {
                                ui.label(egui::RichText::new(format!("Quantity: {}", qty)).color(YELLOW).size(11.0).strong());
                            }
                        });
                    },
                );
                ui.add_space(16.0);
            }
            
            // ---- Regular Items ----
            let mut sorted: Vec<_> = regular.iter().collect();
            
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
            ui.add_space(4.0);
            
            let card_w = 44.0;
            let spacing = COMPACT_GRID_SPACING;

            Self::grid_square(
                ui,
                &sorted,
                spacing,
                card_w,
                card_w + 4.0,
                usize::MAX,
                |ui, item, card_w, card_h| {
                        let key = item.get("ItemKey").and_then(|k| k.as_i64()).unwrap_or(0);
                        let is_chaotic = item.get("IsChaotic").and_then(|c| c.as_bool()).unwrap_or(false);
                        let enchants = item.get("EnchantCount")
                            .and_then(|c| c.as_array())
                            .map(|a| a.iter().filter_map(|v| v.as_i64()).sum::<i64>())
                            .unwrap_or(0);
                        let level = item.get("Level").and_then(|l| l.as_i64())
                            .or_else(|| item.get("ItemLevel").and_then(|l| l.as_i64()))
                            .unwrap_or(0);

                        let name = self.get_item_name(key);
                        let grade = Self::item_grade(key).max(0).min(9);
                        let bg = Self::item_grade_bg(grade);
                        let border_color = Self::grade_color(grade);
                        let grade_name = Self::grade_name(grade);
                        let icon_texture = self.get_icon_texture(key);

                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(card_w, card_h),
                            egui::Sense::hover(),
                        );
                        let is_item_hovered = response.hovered();
                        let card_bg = if is_item_hovered {
                            egui::Color32::from_rgb(bg.r().saturating_add(25), bg.g().saturating_add(25), bg.b().saturating_add(30))
                        } else {
                            bg
                        };
                        let painter = ui.painter();

                        // Background
                        painter.rect_filled(rect, 4.0, card_bg);
                        // Border
                        painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0_f32, border_color), egui::StrokeKind::Outside);

                        // Icon — fills the entire card with tiny margin
                        if let Some(tex) = icon_texture {
                            let icon_rect = rect.shrink(2.0);
                            painter.image(
                                tex.id(),
                                icon_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        }

                        // Chaotic badge
                        if is_chaotic {
                            painter.circle_filled(rect.left_top() + egui::vec2(4.0, 4.0), 2.5, YELLOW);
                        }

                        // Level badge
                        if level > 0 {
                            let badge_w = 5.0 + (level.to_string().len() as f32) * 4.0;
                            let badge_h = 10.0;
                            let badge_rect = egui::Rect::from_min_size(
                                rect.right_top() + egui::vec2(-badge_w - 1.0, 1.0),
                                egui::vec2(badge_w, badge_h),
                            );
                            painter.rect_filled(badge_rect, 2.0, egui::Color32::from_rgba_unmultiplied(8, 8, 12, 225));
                            painter.text(
                                badge_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                level.to_string(),
                                egui::FontId::proportional(7.0),
                                egui::Color32::from_rgb(235, 235, 235),
                            );
                        }
                        
                        let tooltip_name = name.clone();
                        let tooltip_type = Self::item_type(key);
                        let tooltip_border = border_color;
                        let tooltip_bg = bg;
                        let tooltip_icon = icon_texture;
                        let item_ref = item;
                        let tooltip_uid = item.get("UniqueId").and_then(|u| u.as_i64()).unwrap_or(0);
                        let tooltip_qty = item.get("Quantity").and_then(|q| q.as_i64()).unwrap_or(1);
                        let tooltip_level = level;
                        response.on_hover_ui(move |ui| {
                            ui.set_max_width(280.0);
                            ui.vertical(|ui| {
                                // ---- Header: icon + name + rarity pill + level ----
                                ui.horizontal(|ui| {
                                    let icon_box = egui::vec2(56.0, 56.0);
                                    let (icon_rect, _) = ui.allocate_exact_size(icon_box, egui::Sense::hover());
                                    ui.painter().rect_filled(icon_rect, 6.0, tooltip_bg);
                                    ui.painter().rect_stroke(icon_rect, 6.0, egui::Stroke::new(1.5_f32, tooltip_border), egui::StrokeKind::Outside);
                                    if let Some(tex) = tooltip_icon {
                                        let inner = icon_rect.shrink(5.0);
                                        ui.put(inner, egui::widgets::Image::from_texture(tex));
                                    }

                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(&tooltip_name).color(tooltip_border).size(15.0).strong());
                                        ui.horizontal(|ui| {
                                            egui::Frame::NONE
                                                .fill(tooltip_bg)
                                                .corner_radius(4.0)
                                                .stroke(egui::Stroke::new(1.0_f32, tooltip_border))
                                                .inner_margin(egui::Margin::symmetric(6, 1))
                                                .show(ui, |ui| {
                                                    ui.label(egui::RichText::new(grade_name).color(tooltip_border).size(10.0).strong());
                                                });
                                            if tooltip_level > 0 {
                                                ui.label(egui::RichText::new(format!("Lv.{}", tooltip_level)).color(TEXT_SECONDARY).size(12.0));
                                            }
                                        });
                                    });
                                });

                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(6.0);

                                // ---- Meta info row: Type, UID, Qty ----
                                ui.horizontal(|ui| {
                                    egui::Frame::NONE
                                        .fill(CARD_BG)
                                        .corner_radius(5.0)
                                        .stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))
                                        .inner_margin(egui::Margin::symmetric(8, 4))
                                        .show(ui, |ui| {
                                            ui.label(egui::RichText::new(tooltip_type).color(TEXT_SECONDARY).size(11.0));
                                        });
                                    ui.add_space(6.0);
                                    egui::Frame::NONE
                                        .fill(CARD_BG)
                                        .corner_radius(5.0)
                                        .stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))
                                        .inner_margin(egui::Margin::symmetric(8, 4))
                                        .show(ui, |ui| {
                                            ui.label(egui::RichText::new(format!("UID: {}", tooltip_uid)).color(TEXT_MUTED).size(10.0));
                                        });
                                    if tooltip_qty > 1 {
                                        ui.add_space(6.0);
                                        egui::Frame::NONE
                                            .fill(CARD_BG)
                                            .corner_radius(5.0)
                                            .stroke(egui::Stroke::new(1.0_f32, CARD_BORDER))
                                            .inner_margin(egui::Margin::symmetric(8, 4))
                                            .show(ui, |ui| {
                                                ui.label(egui::RichText::new(format!("Qty: {}", tooltip_qty)).color(YELLOW).size(10.0).strong());
                                            });
                                    }
                                });
                                ui.add_space(8.0);

                                // ---- Stats: split "Label: Value" into two-column rows ----
                                let stats = Self::format_item_stats(item_ref);
                                for stat in &stats {
                                    if let Some((label, value)) = stat.split_once(": ") {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(label).color(ACCENT).size(12.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new(value).color(TEXT_PRIMARY).size(12.0).strong());
                                            });
                                        });
                                    } else {
                                        ui.label(egui::RichText::new(stat).color(TEXT_SECONDARY).size(12.0));
                                    }
                                }

                                if is_chaotic {
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new("CHAOTIC").color(YELLOW).size(12.0).strong());
                                }
                                if enchants > 0 {
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Enchant Level").color(ACCENT).size(12.0));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new(format!("+{}", enchants)).color(TEXT_PRIMARY).size(12.0).strong());
                                        });
                                    });
                                }

                                // ---- Additional meta: Socket, Inscription, Engraving, Decoration ----
                                let insc = item_ref.get("InscriptionAppliedTotalCount").and_then(|i| i.as_i64()).unwrap_or(0);
                                let engr = item_ref.get("EngravingAppliedTotalCount").and_then(|e| e.as_i64()).unwrap_or(0);
                                let deco = item_ref.get("DecorationAppliedTotalCount").and_then(|d| d.as_i64()).unwrap_or(0);
                                let socket_data = item_ref.get("SocketData").and_then(|s| s.as_array());
                                let socket_count = socket_data.map(|a| a.len()).unwrap_or(0);

                                let has_extra = insc > 0 || engr > 0 || deco > 0 || socket_count > 0;
                                if has_extra {
                                    ui.add_space(4.0);
                                    ui.separator();
                                    ui.add_space(4.0);
                                    if socket_count > 0 {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Sockets").color(TEXT_MUTED).size(11.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new(format!("{}/{} filled", socket_data.map(|a| a.iter().filter(|s| !s.is_null() && s.as_object().map_or(false, |o| !o.is_empty())).count()).unwrap_or(0), socket_count)).color(TEXT_PRIMARY).size(11.0));
                                            });
                                        });
                                    }
                                    if insc > 0 {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Inscriptions").color(TEXT_MUTED).size(11.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new(format!("{}", insc)).color(TEXT_PRIMARY).size(11.0));
                                            });
                                        });
                                    }
                                    if engr > 0 {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Engravings").color(TEXT_MUTED).size(11.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new(format!("{}", engr)).color(TEXT_PRIMARY).size(11.0));
                                            });
                                        });
                                    }
                                    if deco > 0 {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Decorations").color(TEXT_MUTED).size(11.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new(format!("{}", deco)).color(TEXT_PRIMARY).size(11.0));
                                            });
                                        });
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
                                 egui::Layout::top_down_justified(egui::Align::Min),
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

            ui.add_space(20.0);
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
                            egui::Layout::top_down_justified(egui::Align::Min),
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
                                        ui.label(egui::RichText::new(Self::rune_name(key)).color(TEXT_PRIMARY).size(11.0).strong());
                                        ui.add_space(2.0);
                                        if is_unlocked {
                                            ui.label(egui::RichText::new(format!("Lv. {}", level)).color(egui::Color32::from_rgb(168, 85, 247)).size(10.0).strong());
                                        } else {
                                            ui.label(egui::RichText::new("Locked").color(TEXT_MUTED).size(10.0));
                                        }
                                    });
                            },
                        ).response;

                        resp.on_hover_ui(move |ui| {
                            ui.set_min_width(180.0);
                            ui.label(egui::RichText::new(Self::rune_name(key)).color(TEXT_PRIMARY).size(13.0).strong());
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("Level: {}", level)).color(if is_unlocked { egui::Color32::from_rgb(168, 85, 247) } else { TEXT_MUTED }).size(11.0));
                            ui.label(egui::RichText::new(format!("Status: {}", if is_unlocked { "Active" } else { "Locked" })).color(if is_unlocked { GREEN } else { TEXT_MUTED }).size(11.0));
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
