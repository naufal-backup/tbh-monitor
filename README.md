# TBH Monitor

**TBH Monitor** (TaskbarHero Background Monitor) adalah desktop app untuk membaca dan menampilkan save file game **TaskbarHero** secara real-time — gold, heroes, inventory, rune, dan pet — lengkap dengan opsi untuk mengekspos data tersebut lewat REST API + QR code agar bisa diakses dari perangkat lain (misalnya HP).

Dibuat dengan Rust menggunakan [egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui) untuk UI desktop-nya.

## Fitur

- **Dashboard** — ringkasan gold, jumlah hero, item, progres rune, dan daftar hero singkat.
- **Heroes** — detail tiap hero: level, EXP, ability points, skill yang di-equip, dan gear yang terpasang per slot.
- **Inventory** — daftar seluruh item dengan pencarian, filter kategori (senjata, armor, jewelry, material, dll), dan sorting (nama, tipe, grade, ID).
- **Runes** — progres rune tree beserta status tiap node, dan daftar pet/companion beserta buff pasifnya.
- **Local API server** (opsional) — jalankan server lokal (Actix Web) yang mengekspos data save file lewat endpoint REST, plus generate QR code untuk akses cepat dari HP (misalnya lewat ngrok).
- Auto-refresh data save file secara berkala.

## Instalasi & Menjalankan

Butuh [Rust toolchain](https://rustup.rs/) (edisi 2021 ke atas).

```bash
git clone https://github.com/naufal-backup/tbh-monitor.git
cd tbh-monitor
cargo run --release
```

Build binary release (Windows) akan tersimpan di `target/release/tbh-monitor.exe`.

### Lokasi save file

Secara default, app membaca save file TaskbarHero dari:

```
%USERPROFILE%\AppData\LocalLow\TesseractStudio\TaskbarHero\SaveFile_Live.es3
```

## API Endpoint

Saat local server dijalankan dari dalam app (tab Settings), endpoint berikut tersedia di `http://localhost:<port>`:

| Endpoint         | Deskripsi                              |
|-------------------|-----------------------------------------|
| `GET /api/data`      | Seluruh save data mentah               |
| `GET /api/player`    | Data player (hero, item, rune, dll)    |
| `GET /api/inventory` | Daftar item inventory saja             |

## Tech Stack

- [egui](https://crates.io/crates/egui) / [eframe](https://crates.io/crates/eframe) — UI desktop
- [actix-web](https://crates.io/crates/actix-web) — local REST API server
- [aes](https://crates.io/crates/aes) / [cbc](https://crates.io/crates/cbc) / [pbkdf2](https://crates.io/crates/pbkdf2) — dekripsi save file format ES3
- [qrcode](https://crates.io/crates/qrcode) — generate QR code untuk akses API dari HP
- [serde](https://crates.io/crates/serde) / [serde_json](https://crates.io/crates/serde_json) — parsing data save file

## Struktur Project

```
src/
├── main.rs    # UI utama (egui) — dashboard, heroes, inventory, runes
├── es3.rs     # Loader & dekripsi save file format .es3
└── server.rs  # Local REST API server (actix-web)
data/
└── names_en.json  # Mapping ID item ke nama item (bahasa Inggris)
```

## Disclaimer

Project ini adalah tool pihak ketiga yang tidak berafiliasi dengan pengembang TaskbarHero. Gunakan dengan risiko sendiri — selalu backup save file sebelum digunakan.
