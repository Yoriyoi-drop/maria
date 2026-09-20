# GitHub Actions Management — Maria

Desain penuh: [`doc/release-pipeline.md`](doc/release-pipeline.md).
Ringkasan eksekutif di bawah.

## Tiga hasil yang dibedakan

| Hasil | Dipicu oleh |
|-------|-------------|
| **CI hijau** | Automatis — tiap push/PR (`ci.yml`) |
| **Patch diterima untuk rilis** | Lolos seluruh gerbang wajib + persetujuan (ruleset main) |
| **Update diterbitkan** | **Tag versi yang di-push user eksplisit** (`release.yml`) |

Prinsip: CI boleh berjalan otomatis untuk semua perubahan. Publikasi stabil
harus lebih ketat daripada CI — dan tidak pernah mulai sebelum user
memerintahkan lewat `git tag vX.Y.Z && git push origin vX.Y.Z`.

## Arsitektur

```
push/PR ─▶ ci.yml (AUTO: fmt, clippy -D warnings, test workspace, regresi per-area, release build)
   └─ hijau ─▶ user push tag vX ─▶ release.yml
        ├─ seleksi: tag == version Cargo.toml · CI hijau · izin pelaku · (env release)
        ├─ build + checksum + smoke test
        ├─ GitHub Release (binary + maria.sha256)
        └─ sinkron: landing installation.mdx · install.sh · dist/latest.json
```

## Pemakaian

```bash
# 1. Bump versi di Cargo.toml, push → CI hijau (otomatis)

# 2. Rilis resmi — HANYA pada perintah user:
git tag v0.4.0 && git push origin v0.4.0

# 3. Konsumen:
curl -fsSL https://raw.githubusercontent.com/Yoriyoi-drop/maria/main/install.sh | bash
maria update check    # deteksi
maria update          # pasang (verifikasi SHA-256 + backup + smoke test)
maria update --rollback   # kembali ke binary sebelumnya
```

## File workflow

- **`ci.yml`** — Gerbang teknis. Auto di push/PR. Filter area memakai crate
  aktual (`maria-parser`, `maria-core`, `maria-ast`; `maria-simulator`,
  `maria-elaboration`, `maria-ir`). Read-only.
- **`release.yml`** — Publikasi stabil. Hanya `push tags: v*` (atau
  `workflow_dispatch`). Seleksi ketat → build → `gh release create` →
  sinkron landing/install.sh/manifest. `environment: release` untuk
  required reviewers opsional.

## Auto-update dalam binary

`maria update` (alias `mupdate`, implementasi `crates/maria-tools/src/update.rs`):

- `check` / pasang / `--channel` / `--version` / `--rollback` / `-y`
- alur: manifest → semver compare → unduh → SHA-256 verify (fail-closed) →
  backup `maria.bak` → `rename` atomik → smoke test → rollback otomatis
- 8 unit test inline, semua lulus.

## Sekuritas

- `RELEASE_BOT_TOKEN` (PAT) wajib jika main diproteksi untuk commit sinkron
  (`sync-distribution`). GITHUB_TOKEN fallback untuk main tak diproteksi.
- Gerbang: branch main, tag==versi, CI hijau, permission `admin|write|maintain`.
- Manifest hanya dibuat setelah release valid → konsumen tak pernah melihat
  versi belum rilis. `install.sh` + `maria update` verifikasi SHA-256.

## Catatan migrasi

`release-update.yml`, `trigger-management.yml` (rusak: logika terbalik,
referensi job tak ada, install.sh korup, push otomatis), serta draft-based
`release-prep.yml`/`release-publish.yml` dihapus — digantikan arsitektur
tag-gated di atas.