# Pipeline Rilis & Auto-Update Maria

Desain GitHub Actions final: **CI otomatis untuk semua perubahan; publikasi
stabil lebih ketat daripada CI dan hanya dipicu oleh tag versi yang dibuat
user secara eksplisit.**

## 1. Tiga hasil yang dibedakan

| Hasil | Arti | Dipicu oleh |
|-------|------|-------------|
| **CI hijau** | Perubahan lolos pemeriksaan yang ditentukan (fmt/clippy/build/test) | Automatis — tiap push/PR (`ci.yml`) |
| **Patch diterima untuk rilis** | Lolos seluruh gerbang wajib + persetujuan sesuai aturan branch | Rulesets main (`ci.yml` + CODEOWNERS + required checks) |
| **Update diterbitkan** | Artefak dibangun, diverifikasi, dipublikasikan; baru sekarang landing page, `install.sh`, dan `maria update` boleh melihatnya sebagai update stabil | **Tag versi `vX.Y.Z` yang di-push user secara eksplisit** (`release.yml`) |

Prinsip desain: *CI boleh berjalan otomatis untuk semua perubahan. Publikasi
stabil harus lebih ketat daripada CI.*

## 2. Arsitektur

```
 Developer push / Pull Request
        │
        ▼
 ┌────────────────────────────────────────────────────────────┐
 │ Gerbang 1 — pemeriksaan patch  (otomatis, ci.yml + ruleset)│
 │   diff review · file sensitif (CODEOWNERS) · dependency   │
 │   secret scan · perubahan API/shared code                  │
 └────────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────────┐
 │ Gerbang 2 — validasi teknis (otomatis, ci.yml)             │
 │   Format · Clippy -D warnings · build · unit test ·        │
 │   regression per-area · integration · release build        │
 │   SEMUA job wajib lulus                                    │
 └────────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────────┐
 │ Gerbang 3 — persetujuan                                    │
 │   Review manusia + branch protection main (ruleset) +      │
 │   environment `release` (required reviewers, opsional)     │
 │   User membuat & push TAG:  git tag vX.Y.Z && git push     │
 └────────────────────────────────────────────────────────────┘
        │  (release.yml berjalan — CI commit harus hijau)
        ▼
 ┌────────────────────────────────────────────────────────────┐
 │ Release candidate → GitHub Release + manifest              │
 │   Build artefak · checksum · smoke test · validasi paket   │
 │   Satu sumber versi resmi utk SEMUA konsumen               │
 └───────────┬───────────────────────────────┬────────────────┘
             ▼                               ▼
   Landing page (manifest)        Maria CLI (maria update)
   versi terbaru + install.sh     deteksi → verifikasi → pasang
                                  → rollback (backup)
```

## 3. File workflow (1 file = 1 tanggung jawab)

### `ci.yml` — Gerbang teknis (AUTO, read-only)
- Jalan otomatis di tiap push/PR (`pull_request` + `push` ke main + dispatch).
- Job: `patch-check` (fmt/clippy `-D warnings`), `tests` (workspace), `area-detection` (dorny/paths-filter), regresi per-area (`parser-regression`, `simulator-regression`, `dependency-validation`, `installer-validation`, `documentation-check`), `release-build` (build + verifikasi binary + upload artifact).
- Filter area memakai nama crate aktual (`maria-parser`, `maria-core`, `maria-ast`; `maria-simulator`, `maria-elaboration`, `maria-ir`).
- Tidak menulis apa pun. Semua hijau = prasyarat rilis.

### `release.yml` — Publikasi stabil (TAG = PERINTAH USER)
Trigger:
```yaml
on:
  push:
    tags: ['v*']
  workflow_dispatch:   # opsional, input `tag`
```
Hanya berjalan saat user **secara eksplisit** membuat & push tag:
```bash
git tag v0.4.0 && git push origin v0.4.0
```

Alur job:
1. `validate` (environment `release`) — gerbang rilis:
   - tag `vX` harus sama dengan `version` di `Cargo.toml` (bump versi wajib);
   - CI hijau pada commit yang di-tag (check-runs `Maria CI` sukses, tidak ada check gagal);
   - pelaku punya permission `admin|write|maintain`;
   - (opsional) environment protection `release` di Settings → Environments → required reviewers → persetujuan manusia kedua.
2. `build` — `cargo build --release --bin maria --locked` pada SHA yang di-tag, `strip`, `sha256sum`, verify binary. Artefak: `maria` + `maria.sha256`.
3. `publish` — `gh release create vX` (idempoten: hapus release lama bertag sama dulu), `--generate-notes`, `--latest`, asset binary + checksum.
4. `sync-distribution` (environment `release`, token bot) — satu commit "release: sinkronkan distribusi vX":
   - blok `MARIA-LATEST-BEGIN/END` di `landing_page/src/content/docs/installation.mdx` (idempoten);
   - header `# Version:` + fallback versi di `install.sh`;
   - manifest `dist/latest.json` (root) → disalin ke `landing_page/public/version.json` (disajikan statis landing page).
5. `report` — ringkasan.

Commit sinkronisasi hanya menyentuh path distribusi; tidak memicu rilis ulang
(rilis hanya dipicu tag `v*`).

## 4. Klasifikasi risiko perubahan

| Perubahan | Gerbang tambahan |
|-----------|------------------|
| Dokumentasi saja | Markdown lint + link check |
| Lexer/parser | Parser regression + fuzz corpus (`maria-fuzz`) |
| Semantic analysis/type checking | Unit test + integration test |
| Elaboration/simulator | Regression RTL + differential test |
| Parallel evaluation | Stress test, determinisme, race/crash regression |
| Dependency / Cargo.lock | Audit dependency, build ulang, test |
| Installer/updater | Tes instalasi, checksum, rollback |
| Workflow/release | Review konfigurasi + validasi keamanan |

Jangan mengandalkan nama file saja: perubahan shared utility bisa memengaruhi
banyak crate. Karena itu **semua patch tetap menjalani baseline CI penuh**
(`tests` workspace), dan area terdampak mendapat pengujian tambahan.

## 5. CODEOWNERS & branch protection

- `CODEOWNERS` (sudah ada, lengkap): mengatur siapa wajib review per area.
  Tambahan: `/scripts/` untuk detector.
- Branch protection (dikonfigurasi di GitHub UI, **bukan file repo**):
  - Settings → Rules → Rulesets → ruleset untuk `main`;
  - Require a pull request before merging;
  - Require status checks: `Patch checks`, `Workspace tests`, `Release build` (nama harus sama dengan job yang muncul di Actions);
  - Require branches up to date;
  - Batasi bypass ruleset; admin berwenang tetap bisa bypass — konfigurasi menentukan.

## 6. Manifest update (`dist/latest.json`)

```json
{
  "version": "0.4.0",
  "tag": "v0.4.0",
  "published_at": "2026-09-20T12:00:00Z",
  "platforms": {
    "x86_64-unknown-linux-gnu": {
      "url": "https://github.com/Yoriyoi-drop/maria/releases/download/v0.4.0/maria",
      "sha256": "a1b2c3... (64 hex)"
    }
  }
}
```

- Dihasilkan **hanya** oleh `release.yml` setelah release resmi diterbitkan →
  konsumen tidak pernah melihat versi yang belum dirilis.
- Dibaca oleh: landing page (`/version.json` → badge versi di navbar),
  `install.sh` (via GitHub API releases/latest), `maria update` (manifest).
- Landing page TIDAK mengambil binary dari branch main — selalu dari release.

## 7. Auto-update di dalam Maria (implementasi: `crates/maria-tools/src/update.rs`)

Subcommand: `maria update` (alias `mupdate`).

| Perintah | Fungsi |
|----------|--------|
| `maria update check` | Baca manifest, bandingkan semver, lapor update tersedia (tanpa mengubah apa pun) |
| `maria update` | Pasang update terbaru (manifest) |
| `maria update --channel beta` | Pilih manifest `dist/latest-beta.json` |
| `maria update --version 0.4.0` | Pasang versi spesifik dari release v0.4.0 (fetch checksum asset langsung) |
| `maria update --rollback` | Kembalikan binary sebelumnya (backup `maria.bak`) |
| `maria update -y` | Lewati konfirmasi (untuk scripting/CI) |

Alur updater (fail-closed):
1. Baca versi lokal (`CARGO_PKG_VERSION`).
2. Ambil manifest stabil (curl; `file://`/path lokal = hook uji).
3. Bandingkan semver (implementasi mandiri di `parse_version` — tanpa dep baru).
4. Unduh artefak sesuai platform (deteksi `arch-os` ala install.sh).
5. Verifikasi SHA-256 (`sha2`) — tidak cocok → batalkan, jangan timpa.
6. Backup exe → `maria.bak` (buang backup lama), pasang atomik via `rename`.
7. Smoke test `--version` pada binary baru; gagal → rollback otomatis.
8. Sukses → lapor; `--rollback` kapan saja mengembalikan backup.

Keamanan: manifest hanya metadata (url + sha256) — **tidak pernah dieksekusi**.
Default eksplisit: tanpa `maria update`, tidak ada yang berubah.

Test: 8 unit test inline (`semver_compare_membandingkan_patch`,
`parse_version_beragam_format`, `sha256_dikenal`, `manifest_parse_valid_dan_invalid`,
`checksum_mismatch_ditolak`, `install_dan_rollback_atomik`, `fetch_text_file_url`,
`platform_key_pada_platform_ini`).

## 8. install.sh — verifikasi checksum

`install_binary()` sekarang mengunduh `maria.sha256` dari release yang sama,
membandingkan dengan `sha256sum` binary hasil unduh; mismatch → hapus file,
`exit 1` (fail-closed). Installer tetap memakai GitHub API `releases/latest`
untuk deteksi versi (satu sumber versi resmi).

## 9. Detector lokal (`scripts/maria-detector.py`)

- Hash file source + state di `.maria/auto-update/`; mendeteksi perubahan.
- Hanya menulis `update-payload.json` + mencetak langkah rilis (tag vX).
- **Tidak pernah** men-dispatch publish otomatis — menunggu perintah user.

## 10. Sekuritas & prasyarat GitHub

| Item | Nilai |
|------|-------|
| Environment `release` | opsional tapi dianjurkan: 1+ required reviewers |
| `RELEASE_BOT_TOKEN` (PAT) | wajib jika main diproteksi — dipakai `sync-distribution`; scope `repo`/fine-grained `Contents: write`; tanpa ini sinkron gagal keras (fail-closed, sengaja) |
| Pembatasan pelaku | permission `admin|write|maintain` via API di `validate` |
| Tag v* | buat hanya oleh yang berwenang; ruleset bisa membatasi `v*` tag creation |

## 11. Checklist implementasi

- [x] `ci.yml` — gerbang teknis + regresi per-area (filter memakai crate aktual)
- [x] CODEOWNERS + aturan review (`/scripts/` ditambahkan)
- [ ] Aktifkan branch protection untuk main (GitHub UI → Rulesets)
- [x] Klasifikasi area perubahan (section 4, ci.yml area-detection)
- [x] Regression + integration tests (ci.yml jobs)
- [x] `release.yml` — rilis via tag versi (gerbang ketat)
- [x] Manifest `dist/latest.json` setelah release valid
- [x] Landing page terhubung manifest (`VersionBadge` membaca `/version.json`)
- [x] `install.sh` terhubung release resmi + verifikasi checksum
- [x] Implementasi `maria update check` / `maria update` (+ `--channel`, `--version`, `--rollback`, `-y`)
- [x] Checksum + rollback (sha2 verify, backup `maria.bak`, smoke test)

## 12. Catatan

- Platform rilis saat ini: `x86_64-unknown-linux-gnu`. aarch64/macOS menyusul
  (perlu `cross`/runner macOS + entri platform di manifest).
- Implementasi `maria update` menyentuh binary (area CRITICAL): dilakukan
  manual, satu logical change, dengan test khusus (8 unit test lulus).