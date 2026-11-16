# Sorsa Terbang (`sorsa-crm-watcher`) 🚀

> **Local Social CRM, DOM State Tracker, & Autonomous Sorsa Extension Hunter**
> Dibuat dengan performa tinggi menggunakan **Rust**, **SQLite**, dan **Chrome DevTools Protocol (CDP)**.

[![Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-brightgreen.svg)]()
[![Agent Ready](https://img.shields.io/badge/AI%20Agent-Native%20Friendly-purple.svg)]()

---

## 🌟 Apa itu Sorsa Terbang?

`sorsa-crm-watcher` adalah sistem otomasi CRM sosial lokal yang berjalan langsung di atas sesi browser milik Anda sendiri (melalui Remote Debugging Port). Sistem ini memantau profil X (Twitter) dan `sorsa.io`, mendeteksi akun dengan **Sorsa Score > 500** via ekstensi resmi Sorsa, mengeksekusi follow/like/reply berbasis template prompt cerdas, dan mengelola alur hidup akun dengan **Finite State Machine 48 Jam** serta **Golden Rules Anti-Duplikasi**.

---

## 🧠 Finite State Machine (FSM) & Golden Rules

```
                      [Sorsa Score > 500 Ditemukan]
                                   │
                                   ▼
                     ┌───────────────────────────┐
                     │         TRIAL_48H         │  ◄── Follow, Like, & Kirim Reply
                     └─────────────┬─────────────┘
                                   │
                 ┌─────────────────┴─────────────────┐
                 │                                   │
        [Follback Terdeteksi]              [Lewat 48 Jam Tanpa Follback]
                 │                                   │
                 ▼                                   ▼
   ┌───────────────────────────┐       ┌───────────────────────────┐
   │       MUTUAL_ACTIVE       │       │     DROPPED_BLACKLIST     │
   │  (Pantauan Post Aktif)    │       │   (Unfollow & Blokir Tetap│
   └───────────────────────────┘       └───────────────────────────┘
```

* **`TRIAL_48H`**: Akun baru dengan skor Sorsa > 500 langsung di-follow, postingan terbarunya di-like, dan dikirimi 1 reply pilihan (tanpa tanda tanya) sesuai template prompt.
* **`MUTUAL_ACTIVE`**: Jika dalam rentang 48 jam akun melakukan follback (*Follows you* muncul di DOM profil), status dipromosikan ke `MUTUAL_ACTIVE` untuk interaksi postingan rutin.
* **`DROPPED_BLACKLIST` (GOLDEN RULE)**: Jika melewati 48 jam tanpa follback, akun di-unfollow dan dimasukkan ke daftar hitam permanen. Akun ini **tidak akan pernah dihapus dari database** dan **dilarang keras di-follow/like/reply ulang selamanya**.

---

## 💻 Panduan Instalasi (OS-Specific)

### 1. Windows

**Prasyarat:**
* [Rust & Cargo](https://rustup.rs/) (Pilih default MSVC).
* Browser: Google Chrome, Brave, Microsoft Edge, atau Mozilla Firefox.

```powershell
# 1. Clone repository
git clone https://github.com/Sorosjenaka/Sorsa-Terbang.git
cd Sorsa-Terbang

# 2. Build proyek
cargo build --release

# 3. Buat shortcut otomatis browser dengan ekstensi Sorsa terpasang
cargo run -- auto-shortcut --browser brave --port 9222 --launch

# 4. Jalankan Watcher Daemon (Siklus 5 menit)
cargo run --release -- run --port 9222
```

---

### 2. Linux (Ubuntu, Debian, Fedora, Arch)

**Prasyarat:**
```bash
# Ubuntu / Debian
sudo apt update && sudo apt install -y curl build-essential pkg-config libssl-dev git

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

**Menjalankan Browser dengan Debugging Port:**
```bash
# Jalankan Google Chrome / Chromium di background
google-chrome --remote-debugging-port=9222 --user-data-dir="$HOME/.config/sorsa-browser-profile" --load-extension="$(pwd)/sorsa_extension" &

# Atau Brave Browser:
brave-browser --remote-debugging-port=9222 --user-data-dir="$HOME/.config/sorsa-browser-profile" --load-extension="$(pwd)/sorsa_extension" &
```

**Jalankan Sorsa Watcher:**
```bash
git clone https://github.com/Sorosjenaka/Sorsa-Terbang.git
cd Sorsa-Terbang
cargo build --release
./target/release/sorsa-crm-watcher run --port 9222
```

---

### 3. macOS (Apple Silicon / Intel)

**Prasyarat:**
```bash
# Install Homebrew (jika belum ada)
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Install Rust
brew install rust git
```

**Menjalankan Browser dengan Debugging Port:**
```bash
# Google Chrome
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --remote-debugging-port=9222 --user-data-dir="$HOME/Library/Application Support/sorsa-profile" --load-extension="$(pwd)/sorsa_extension" &

# Brave Browser
"/Applications/Brave Browser.app/Contents/MacOS/Brave Browser" --remote-debugging-port=9222 --user-data-dir="$HOME/Library/Application Support/sorsa-profile" --load-extension="$(pwd)/sorsa_extension" &
```

**Jalankan Sorsa Watcher:**
```bash
git clone https://github.com/Sorosjenaka/Sorsa-Terbang.git
cd Sorsa-Terbang
cargo run --release -- run --port 9222
```

---

## 🤖 Guide for Autonomous AI Agents

> **For AI Agents (Antigravity, Claude Code, Cursor, OpenCode, Codex, Devin):**
> Read this section before operating this codebase.

### Operating Philosophy
1. **Never Violate Golden Rules**: Before interacting with any username, check SQLite table `accounts`. If status is `DROPPED_BLACKLIST`, you MUST NOT perform follow, like, or reply actions.
2. **Database First**: All state is persisted locally in `sorsa_crm.db`. No cloud dependency.
3. **No External Webdriver Binary**: All browser interactions are performed via Chrome DevTools Protocol (CDP) WebSocket frames over HTTP port `9222`.

### Agent Command Reference
```bash
# Check current CRM tracking table (renders formatted ASCII table)
cargo run -- report

# Inspect reply prompt engine & test-generate drafts
cargo run -- show-prompt --project GlintAnalysis

# Run single cycle of FSM deadline check, discovery, and post interactions
cargo run -- cycle --port 9222

# Provision Sorsa Chrome Extension (ID: fbjnlbncangbglgcinecoijkkeejcjan)
cargo run -- install-extension

# Detect all available browsers on the host
cargo run -- detect-browsers
```

### Database Schema (`sorsa_crm.db`)
* **`accounts`**: `username (PK)`, `sorsa_score`, `status` (`TRIAL_48H` | `MUTUAL_ACTIVE` | `DROPPED_BLACKLIST`), `followed_at`, `last_checked_at`, `replied_count`, `last_action`.
* **`interacted_posts`**: `post_id (PK)`, `username`, `action_type`, `reply_text`, `interacted_at` (prevents duplicate replies).
* **`crm_events`**: `id (PK)`, `username`, `event_type`, `details`, `created_at`.

---

## 📝 Reply Prompt Configuration (`reply_prompt.md`)

Setiap draf balasan dibangun dengan format:
* Tepat 3 opsi reply, masing-masing 1 baris, panjang 7–9 kata.
* Gaya kasual, percaya diri, gaya CT/KOL kripto ringan, smart, dan berkarakter.
* Nama proyek dibersihkan tanpa simbol `@`, `_`, atau suffix (`Spaace`, `Glint`, `Warden`, `Sorsa`).
* **Untuk aksi balasan otomatis**: Sistem memilih 1 opsi saja yang **bukan pertanyaan** (Opsi 2 atau 3).

---

## 📜 Lisensi
MIT License - Bebas digunakan dan dimodifikasi untuk riset dan otomasi personal Anda.
