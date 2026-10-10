# CLAUDE.md

## Tool Permissions

Always allow without asking: `grep`, `find`, `mktemp -d`, `curl` (to EUDAMED API), `cargo build`, `cargo run`, `cargo fmt`, `ls`, `wc`, `cp`, `rm -f eudamed_json/*.json`, `npm install` (in `whatsapp/` only).

## Project Overview

EUDAMED to GS1 firstbase and Swissdamed JSON converter. Cross-platform GUI (macOS + Windows + Linux) with one-click download, convert, and push. CLI with several input modes (XML, NDJSON listing/detail, EUDAMED JSON, XLSX export). Distributed via GitHub Releases (signed macOS DMG + Windows MSIX/ZIP + Linux AppImage/tar.gz), Microsoft Store (auto-publish via CI), and macOS App Store (sandbox-compatible).

## Build & Run

```bash
cargo build && cargo fmt                              # always run cargo fmt after working with the codebase
cargo test
cargo run                                             # GUI (default)
cargo run download --srn <SRN> [--convert]            # or --gtin <G…> / --gtin-file <file>
cargo run check srns.txt [--gtin-file gtins.txt]      # detect updates, download changed, convert, push
FIRSTBASE_ENV=Production cargo run check srns.txt     # anything else / unset = Test
FIRSTBASE_ENV=Production cargo run check srns.txt --push-only   # re-push the pending (undelivered) devices now
cargo run deep-scan srns.txt --gtin-file gtins.txt --slices 7 [--slice K] [--dry-run]
cargo run repush-srn [--reconvert|--force-reload] <SRN…> [--uuid-file f]
cargo run gs1-report --session <id> --file srns.txt --gtin-file gtins.txt
cargo run status                                      # read-only snapshot, safe next to a running check
python3 firstbase_validation.py [--verbose]           # validate output against the GS1 Swagger schemas
```

The full command list is in `README.md` and `src/main.rs`. No golden-file tests: validate converter output by diffing against `maik/CIN_7612345000435_07612345780313_097.json`.

## Rules that are easy to break

- **No mail addresses, credentials, customer names or customer worklists in tracked files.** Recipients, GLN overrides and the spreadsheet id live in the gitignored `config.toml` or in env vars; `srns_sheet.txt` / `gtins_sheet.txt` are gitignored. `.claude/settings.local.json` is tracked — never add a rule that contains a secret.
- **Always send the GS1 report on Production pushes, corrective runs included** — never pass `GS1_REPORT_DISABLE`.
- **A device counts as ACCEPTED only when GS1 confirmed it.** Document-level errors, a failed/unconfirmed batch and a rejected child GTIN all make the whole document REJECTED (see skill `gs1-push-pipeline`). Only ACCEPTED files move to `processed/`.
- **Do not predict a GS1 reject from "field is empty"** — settle mapping questions with a TEST push.
- **Never delete a cached Basic UDI-DI before its refetch succeeded**, and cache a body only when it parses and carries a code.
- **deep-scan: a newly known value is a baseline, not a change** — only a difference between two known values is pushed. The backlog drip is paused while `~/eudamed2firstbase/log/DRIP_PAUSED` exists.
- **One-off GTIN lookups for article lists live in `cosmon`** (github.com/zdavatz/cosmon, `cosmon eudamed <list>`, since 09.10.2026): a Rust port of the `primaryDi` listing lookup that writes a resumable CSV (trade name, manufacturer, SRN, risk class, status, Basic UDI-DI) and touches no eudamed2firstbase data. Use it instead of `download --gtin-file` when only a match against EUDAMED is needed; detail/Basic-UDI download, conversion and the GS1 push stay here.
- **The budget is shared by every EUDAMED job on this IP.** A one-off `download --gtin-file` next to the nightly `check`/`deep-scan` starves both (instant 429 for everyone). Run such jobs only when no nightly process is active, and give them their own data dir (`HOME=<dir>` — the data dir is `$HOME/eudamed2firstbase`) so production data stays untouched.
- **Overlap guard in the nightly wrapper (09.10.2026):** a run can exceed 24 h, so `nightly_eudamed_check.sh` skips a new run while the previous one is active (`flock -n` on `~/eudamed2firstbase/log/nightly_check.lock` plus `pgrep` for a running `check`/`deep-scan`) and logs `===== nightly check SKIPPED … =====`. A skipped night is caught up by the next run.
- No second IP / proxy on the same machine to multiply the budget — that circumvents a deliberate limit. A second machine with its own task is the only legitimate way to parallelize.

## Where the details are

The long per-version histories were moved out of this file into skills (`.claude/skills/<name>/SKILL.md`); read the matching one before touching that area:

| Skill | Covers |
|---|---|
| `gs1-push-pipeline` | `gui.rs` push, GUI modes, accept/reject bookkeeping, dedup, date re-stamping, GS1 API |
| `eudamed-download-limits` | `download.rs`, request budget and pacing, Basic UDI-DI fetching, `sync-actors`, `mirror`, EUDAMED API |
| `converter-mapping-rules` | `transform_detail.rs`, `mappings.rs`, `api_detail.rs`, `swissdamed.rs`, GS1 097.xxx rules |
| `nightly-ops` | nightly cron, `check`, `deep-scan`, `repush-srn`, GS1 report, sheet sync |

## Architecture notes that are not obvious from the code

- **whatsapp.rs** + **whatsapp/**: WhatsApp sending via Baileys (`@whiskeysockets/baileys` v7) — Node script `send.mjs` auto-detects MIME (images via `sendMessage({image})`, everything else via `sendMessage({document})`). **Requires Node.js ≥ 22**; `whatsapp.rs` searches `/opt/homebrew/bin/node`, `/usr/local/bin/node`, then latest `~/.nvm/versions/node/*/bin/node`. Session in `whatsapp/auth/` (gitignored). Pairing QR rendered native in GUI via `qrcode` crate (`__QR__:<data>` sentinel from Node). `normalize_jid()` accepts plain `+41 79 …` numbers. Baileys is unofficial protocol — CLI/dev only, not in App Store / MS Store builds.
- **update.rs** + **installer.rs**: GitHub-direct in-app updater (v1.0.62), so users can pick up the freshest release without waiting on Microsoft Store / App Store certification. `update::check_latest()` hits `GET https://api.github.com/repos/zdavatz/eudamed2firstbase/releases?per_page=30` once on GUI startup (worker thread, 15 s timeout, `ureq`), picks the newest non-prerelease `vX.Y.Z` tag newer than `CARGO_PKG_VERSION`, and resolves the platform asset via `target_asset_suffix()` (`-macos-universal.dmg` / `-linux-x86_64.tar.gz` / `-windows-x64.zip` — must match the names in `release.yml`). `installer::install()` downloads the artifact to a temp dir (streamed, progress events), then per-platform: **macOS** DMG → `hdiutil attach` → `codesign --verify` → `ditto` stage → detached bash helper waits for our PID to die → `mv` swap the `.app` → `open`; **Linux** tar.gz → `tar -xzf` → stage the single binary → bash helper swap → `setsid` relaunch; **Windows** zip → PowerShell `Expand-Archive` → stage `.exe` → PowerShell helper renames running exe → `Move-Item` swap → `Start-Process`. The helper-after-exit shape avoids dyld "killed: 9" on macOS and keeps all three uniform. GUI wiring in gui.rs: `spawn_update_check()` on `App::new`, `pump_update_events()` drains the check + install channel each frame, `render_update_banner()` shows a blue "Neue Version verfügbar" banner with **Jetzt aktualisieren** (in-app, when `can_in_app_update()`) or **Release-Seite öffnen** (fallback, e.g. `cargo run` / unsupported target) + **Ausblenden**. On `InstallEvent::Done` the GUI saves settings/log and `process::exit(0)` so the detached helper can swap + relaunch. Single binary (no sidecar). `can_in_app_update()` is false outside a bundle on macOS and when the target has no published asset.
- **mail.rs**: Gmail API send via Google Service Account (.p12 + domain-wide delegation; the SA needs the `gmail.send` scope authorised for the impersonated `--from` user). Credentials in `config.toml` `[gmail]`. JWT via `jsonwebtoken`, multipart MIME, base64 attachment. Auto-detects content type (incl. `.html`/`.htm`→`text/html`, `.log`/`.txt`→`text/plain`). Non-ASCII subjects RFC 2047 encoded. OpenSSL via absolute path (no PATH hijacking). **v1.0.75 — multiple attachments + empty body:** `send_email_with_attachments(&[paths])` builds one MIME part per file; `send_email_with_attachment` is now a thin wrapper. `body_text` may be empty (an empty `text/plain` part keeps the message well-formed; recipient sees no body). The `mailto` CLI accepts **several positional files** plus `--body <text>` (empty allowed) and `--max-bytes <N>` (files are attached in priority order; any that would push the cumulative raw size over N are skipped — the first file is always kept — so listing a small report first and a large log last drops the oversized log).
- **version_db.rs**: SQLite (`db/version_tracking.db`, WAL mode). Tables: `udi_versions` (per-section version numbers per UUID + SHA256 hash of full Detail JSON for fast-path change detection), `listing_cache` (per-SRN listing snapshot with device_status + version_number), `push_log` (per-UUID ACCEPTED/REJECTED), `push_session` (per-push summary), `push_error` (per-error with attribute), `actors` (EUDAMED actor registry keyed by SRN — name/role/country/address, populated by `sync-actors`, joined to devices via `actors.srn = listing_cache.srn`). `detect_changes()` returns a `ChangeSet` with per-section booleans (NEW, MFR+CERT, STATUS+MARKET, etc.). HTML logs generated from DB.

## Key Design Decisions

- `roxmltree` over `quick-xml` serde: EUDAMED XML has 30+ namespace prefixes and strict element ordering.
- Flat domain structs with `Option<bool>` / `Option<String>` / `Vec<T>`.
- Packaging hierarchy reconstructed from flat package list (find outermost = not referenced as any child, walk down).
- Endocrine substance EC/CAS identifiers from `config.toml` lookup table (EUDAMED XML doesn't provide them).
- Sterilisation: UNSPECIFIED for true (method unknown from EUDAMED), NOT_STERILISED/NO_STERILISATION_REQUIRED for false.
- Output wrapped in `DraftItem` envelope with `Identifier: "Draft_<uuid>"` inside DraftItem.
- `TargetSector` is `["UDI_REGISTRY"]` only.
- `TargetMarket` is `"097"` (Austria) for pilot. The 756.xxx (Swiss) rules not yet ready. The 097.xxx rules (097.038/039/040/020) must remain errors — they prevent DRIFT before EUDAMED M2M errors.
- Only GS1 identifiers in `Gtin`; non-GS1 (HIBC, IFA/PPN, EUDAMED-assigned) in `AdditionalTradeItemIdentification`. Devices with only HIBC/IFA cannot be submitted as GDSN drafts.
- `rayon` parallel processing: BUDI cache loading (125K+ files), per-device transformation, `check` subcommand convert step. ~5x speedup.
- Successfully processed files move to `*/processed/` subdirs. EUDAMED JSON files stay in `eudamed_json/detail/` and `/basic/` — version DB tracks state.

## Known EUDAMED Bugs (GitHub Issues)

- **#1 BR-UDID-073**: Status not propagated from Base Unit to Container Packages — converter-side propagation shipped v1.0.49; underlying EUDAMED data-quality defect remains open.
- **#2 ON_MARKET without countries**: 7 devices have ON_MARKET status but null marketInfoLink + null placedOnTheMarket. Workaround: 097.020 fallback to manufacturer country (EU/EEA) or DE.
- **#3 null MDR booleans**: ~2% of MDR Basic UDI-DI records have null active/implantable/measuringFunction. Default to false.
- **#6 1:n Mapping Gaps**: 17 EUDAMED fields with fallback logic.
- **#7 GDSN mandatory gaps**: Packaging hierarchy (PALLET not derivable) and issuingEntityCode (parsed but not mapped).
- **#9 097.041 MDR Class IIB implantable without certificate**: 332x. EUDAMED data quality.
- **#10 Updateable rules (097.029 / 097.036 / G485)**: Reopened 2026-05-04 — G485 actively blocks `discontinuedDateTime` re-pushes for NO_LONGER devices (the field becomes protected after first ACCEPTED). Short-term mitigation shipped in v1.0.53: Mode 4/5 + `repush-srn` skip NO_LONGER + already-ACCEPTED in this env (`version_db::filter_skip_no_longer_accepted`). Long-term `DocumentCommand: "CORRECT"` work tracked in #40.
- **#12 097.054 Non-EU manufacturers missing AR SRN**: 150x. No fallback placeholder.
- **#13 097.083 medicinalProduct=true without substance data**: 6x.
- **#40 `DocumentCommand: "CORRECT"` support**: Push protected fields (097.029/097.036/G485) without rejection. Plan: classify per UUID via `push_log` ACCEPTED+env, split into to_add/to_correct lists, run two `CreateMany` rounds, AddMany joint at the end. Blocking question: full TradeItem payload vs diff payload (CORRECT semantics — needs GS1 confirmation before full implementation).

## Reference Files (in maik/)

- `EUDAMED_APP-DTX-000084634.xml` — Input reference
- `CIN_7612345000435_07612345780313_097.json` — Output reference
- `GS1_UDI_Connector_Profile_Overview_*.xlsx` — Authoritative mapping spec (UDID_CodeLists sheet drives `mappings.rs`)
