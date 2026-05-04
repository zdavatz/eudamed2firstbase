# CLAUDE.md

## Tool Permissions

Always allow without asking: `grep`, `find`, `mktemp -d`, `curl` (to EUDAMED API), `cargo build`, `cargo run`, `cargo fmt`, `ls`, `wc`, `cp`, `rm -f eudamed_json/*.json`, `npm install` (in `whatsapp/` only).

## Project Overview

EUDAMED to GS1 firstbase and Swissdamed JSON converter. Cross-platform GUI (macOS + Windows + Linux) with one-click download, convert, and push. CLI with several input modes (XML, NDJSON listing/detail, EUDAMED JSON, XLSX export). Distributed via GitHub Releases (signed macOS DMG + Windows MSIX/ZIP + Linux AppImage/tar.gz), Microsoft Store (auto-publish via CI), and macOS App Store (sandbox-compatible).

## Build & Run

```bash
cargo build
cargo run                                            # GUI mode (default)
cargo run gui                                        # GUI mode (explicit)
cargo run download --srn DE-MF-000017808             # Download from EUDAMED API
cargo run download --srn SRN1 SRN2 --50              # Multiple SRNs, limit 50 per SRN
cargo run download --srn SRN1 --convert              # Download + auto-convert
cargo run xml                                        # XML mode: xml/ -> firstbase_json/
cargo run ndjson                                     # API listing mode
cargo run detail <details.ndjson> [listing.ndjson]   # API detail mode
cargo run firstbase                                  # eudamed_json/detail/ -> firstbase_json/
cargo run swissdamed                                 # eudamed_json/ -> swissdamed_json/
cargo run xlsx <details.ndjson>                      # detail NDJSON -> xlsx/<stem>.xlsx
cargo run count SRN1 SRN2                            # Count devices per SRN (parallel)
cargo run count --file srns.txt                      # Count from text file
cargo run count --xlsx file.xlsx [col]               # Count from XLSX, writes GTIN_Count back
cargo run check srns.txt [--threads N]               # Check + download + convert + push
cargo run status                                     # Live snapshot of ingest + push state
cargo run regenerate                                 # Rewrite all firstbase_json/ from eudamed_json/detail/
cargo run repush-srn DE-MF-000005190 [SRN2 ...]      # Restore from processed/, push
cargo run repush-srn --file srns.txt
cargo run repush-srn --reconvert DE-MF-000005190     # Reconvert + push (after converter change)
cargo run mailto file.csv --to a@b.ch --from x@y.ch  # Gmail attachment
cargo run whatsapp file.html --group 120363...@g.us  # WhatsApp send
cargo run whatsapp --list-groups
cargo run whatsapp --list-contacts [filter]
cargo run whatsapp --pair                            # QR pairing
cargo run scan [dir]                                 # Parallel GTIN scan
./download.sh --10                                   # Download + convert 10 products
```

No tests yet. Validate output by diffing `firstbase_json/firstbase_28.02.2026.json` against `maik/CIN_7612345000435_07612345780313_097.json`.

### Schema Validation

```bash
python3 firstbase_validation.py              # validate against GS1 Swagger schemas
python3 firstbase_validation.py --verbose
python3 firstbase_validation.py --dump-schema MedicalDeviceInformation
```

Validates against Product API (recipient, `test-productapi-firstbase.gs1.ch`) and Catalogue Item API (sender, `test-webapi-firstbase.gs1.ch:5443`). Caches in `.swagger_cache_product.json` / `.swagger_cache_catalogue.json`.

### Formatting

Always run `cargo fmt` after working with the codebase.

## Architecture

- **download.rs**: Shared download module (GUI + CLI). `DownloadProgress` trait abstracts progress reporting. `run_download()` does parallel listing downloads (50 threads default) → `listing_cache` SQLite table → on-the-fly version classification against `udi_versions` (cached prepared stmt, end-of-SRN counters `[N↑ new, N↑ udi, N↑ budi, N same]`) → pre-download version check skips unchanged devices → parallel detail/basic download with retry → version indexing into `udi_versions`. EUDAMED API silently caps `pageSize` to 20.
- **docs/version-tracking.{html,pdf}**: User-facing white paper on Basic UDI-DI / UDI-DI version tracking. Self-contained HTML, A4 print rules. PDF rendered via headless Chrome: `chrome --headless --disable-gpu --no-pdf-header-footer --print-to-pdf=docs/X.pdf file://$PWD/docs/X.html`. Update HTML when `version_db.rs` or `download.rs` algorithm changes.
- **docs/gui-modes.{html,pdf}**: User-facing guide for the 6 GUI buttons (Mode 0–5). Per mode: Pipeline, when to use, when not to use, pitfalls. Plus FAQ.
- **docs/legacy-global-model.{html,pdf}**: Q&A snapshot on legacy `globalModelDescription` + FLD-UDID-22.
- **whatsapp.rs** + **whatsapp/**: WhatsApp sending via Baileys (`@whiskeysockets/baileys` v7) — Node script `send.mjs` auto-detects MIME (images via `sendMessage({image})`, everything else via `sendMessage({document})`). **Requires Node.js ≥ 22**; `whatsapp.rs` searches `/opt/homebrew/bin/node`, `/usr/local/bin/node`, then latest `~/.nvm/versions/node/*/bin/node`. Session in `whatsapp/auth/` (gitignored). Pairing QR rendered native in GUI via `qrcode` crate (`__QR__:<data>` sentinel from Node). `normalize_jid()` accepts plain `+41 79 …` numbers. Baileys is unofficial protocol — CLI/dev only, not in App Store / MS Store builds.
- **mail.rs**: Gmail API send via Google Service Account (.p12 + domain-wide delegation). Credentials in `config.toml` `[gmail]`. JWT via `jsonwebtoken`, multipart MIME, base64 attachment. Auto-detects content type. Non-ASCII subjects RFC 2047 encoded. OpenSSL via absolute path (no PATH hijacking).
- **gui.rs**: Cross-platform GUI (egui/eframe). One-click pipeline: download → convert → push. Full Firstbase API push in Rust: token (3x retry) → `Live/CreateMany` (100-item batches, 429 retry) → poll `RequestStatus/Get` → token refresh → `AddMany` → poll. Settings auto-saved to `settings.json`. Env vars: `FIRSTBASE_EMAIL`, `FIRSTBASE_PASSWORD`, `SWISSDAMED_CLIENT_ID`, `SWISSDAMED_CLIENT_SECRET`, `SWISSDAMED_BASE_URL`. Data dir: `~/eudamed2firstbase/` (Windows: `%USERPROFILE%`, macOS Sandbox: `~/Library/Containers/.../Data/eudamed2firstbase/`). Six pipeline modes — button labels carry the mode number: `0: DL+Push <target>`, `1: Convert & Push (all)`, `2: Convert & Push SRNs`, `3: Repush failed`, `4: Repush SRN`, `5: Reconvert + Repush SRN`. Mode 5 reconverts from `eudamed_json/detail/` then falls back to `processed/` for missing UUIDs (logs a WARNING line). Only ACCEPTED files move to `processed/`; rejected stay in `firstbase_json/`. GTIN dedup prefers MDR over MDD (Issue #8).
- **Convert-skip-without-output fallback**: both gui.rs convert and the `firstbase`/`eudamed_json` subcommand guard `detect_changes() → has_any_change()==false` with a disk-check. If the output is in neither `firstbase_json/<uuid>.json` nor `firstbase_json/processed/<uuid>.json`, the converter falls through to actual conversion. Fixes a latent bug where the download pipeline would index `udi_versions` *before* convert ran, causing convert to see a hash match and silently skip every freshly-downloaded device.
- **build.rs**: Windows icon embedding via `winresource`.
- **bundle_macos.sh**: macOS `.app` bundle: `--universal` (arm64+x86_64), `--sign`, `--dmg`, `--notarize`.
- **entitlements.plist**: Hardened Runtime (Developer ID, JIT + network).
- **entitlements-appstore.plist**: Mac App Store (sandbox + JIT + network + file access + `application-identifier 4B37356EGR.com.ywesee.eudamed2firstbase`).
- **windows/AppxManifest.xml**: MSIX manifest (Store ID 9P889JD1XWS2, Publisher ywesee GmbH).
- **.github/workflows/release.yml**: CI/CD on tag push (`v*`): macOS universal binary + signed DMG + App Store .pkg upload (iTMSTransporter, fallback altool); Windows exe + ZIP + MSIX + Microsoft Store submission via REST API. Patched winit (no `_CGSSetWindowBackgroundBlurRadius` for App Store). Post-commit polling: `/submissions/{id}/status` every 30s (max 10min) until accepted state. Secrets: `MACOS_*`, `APPLE_*`, `MSSTORE_*`.
- **eudamed.rs**: XML parsing via `roxmltree` (DOM, not serde — element ordering issues with quick-xml).
- **api_json.rs**: EUDAMED listing NDJSON (serde, flat `ApiDevice`).
- **api_detail.rs**: EUDAMED detail NDJSON (serde). Rich `ApiDeviceDetail` (clinical sizes, substances, market info, certificates, secondary DI, direct marking, unit of use, linked devices). `BasicUdiDiData` for Basic UDI-DI (MDR booleans, multiComponent, riskClass, manufacturer/AR, basicUdi code, legislation). `regulatory_act()` extracts MDR/IVDR/MDD/AIMDD/IVDD from legislation field — more accurate than risk-class inference.
- **firstbase.rs**: Output JSON model with serde. `DraftItemDocument` wraps `{"DraftItem": {"TradeItem": ..., "Identifier": "Draft_<uuid>"}}` (Identifier inside DraftItem, required by Catalogue Item API).
- **transform.rs**: XML → firstbase. Builds packaging hierarchy via parent-child DI references.
- **transform_api.rs**: API listing → firstbase. Simple flat mapping.
- **transform_detail.rs**: API detail → firstbase. Richest output: clinical, market, IFU, substances (Chemical), product designer (EPD), secondary DI, direct marking, unit of use DI, related devices, regulatory module (MDR/IVDR+EU), ORIGINAL_PLACED vs ADDITIONAL_MARKET split. Package levels inherit `eu_status` and `discontinuedDateTime` from base unit. SPP detection via `multiComponent.criterion=="SPP"` (FLD-UDID-261), gated on `is_mdr` (SPP is MDR-only). ContactType: `is_system_or_pack && is_mdr` ⇒ EPP (097.016), else EMA (097.049 forbids SPP fields under EMA). EAR contact added for non-EU manufacturers when AR exists (097.054). Trade Name emitted twice in description module: full text + truncated 40-char DescriptionShort. Packaging hierarchy from `containedItem` (recursive). Non-GS1 primary DIs (HIBC/IFA) moved to `AdditionalTradeItemIdentification`. `globalModelNumber` ← Basic UDI-DI code; `globalModelDescription` ← Basic UDI-DI `deviceName` (097.025). `deviceModel` → MODEL_NUMBER, `reference` → MANUFACTURER_PART_NUMBER (both truncated to 80 chars per GDSN limit). Legacy devices strip MDR-only fields (097.095). NOT_INTENDED_FOR_EU_MARKET skips sales module entirely (097.021). Certificates from `deviceCertificateInfoListForDisplay` (manufacturer-provided + NB-provided distinguished by `nbProvidedCertificate`). 7 of 10 CertificateLink fields mapped; 3 (FLD-UDID-350/357/361) have no GDSN pendant.
- **eudamed_json.rs**: EUDAMED device-level JSON (serde). `EudamedDevice` with inline manufacturer/AR, basicUdi, riskClass.
- **transform_eudamed_json.rs**: EUDAMED device-level → firstbase. Includes full manufacturer/AR contact info. No GTIN (device-level records).
- The `eudamed_json` mode auto-detects file type: UDI-DI level (has `primaryDi` object) → `transform_detail`; device-level (Basic UDI-DI, `primaryDi` null) → `transform_eudamed_json`. Cache miss fetches Basic UDI-DI on demand from EUDAMED API.
- **xlsx_export.rs**: Detail NDJSON → XLSX. Flattens `ApiDeviceDetail` into columns plus certificate columns from BUDI cache (multiple certs newline-separated). Uses `rust_xlsxwriter`.
- **Push logs split per environment**: `firstbase_env` column on `push_log` and `push_session`; `api_base` on `push_session`. HTML logs in `log/firstbase_test/` or `log/firstbase_prod/`. Banner: red "PRODUCTION — LIVE DATA" or blue "TEST ENVIRONMENT". GUI has separate WhatsApp buttons per env.
- **version_db.rs**: SQLite (`db/version_tracking.db`, WAL mode). Tables: `udi_versions` (per-section version numbers per UUID + SHA256 hash of full Detail JSON for fast-path change detection), `listing_cache` (per-SRN listing snapshot with device_status + version_number), `push_log` (per-UUID ACCEPTED/REJECTED), `push_session` (per-push summary), `push_error` (per-error with attribute). `detect_changes()` returns a `ChangeSet` with per-section booleans (NEW, MFR+CERT, STATUS+MARKET, etc.). HTML logs generated from DB.
- **mappings.rs**: Code translation tables. Derived from UDID_CodeLists sheet of `maik/GS1_UDI_Connector_Profile_Overview_Apr_2026_V1.1_notForPublicSharing.xlsx`. Includes: issuing agency → type code (GS1/HIBC/ICCBBA/IFA, EUDAMED-assigned → IFA), CMR type, full ISO 3166-1 country alpha-2 → GS1 numeric (250 entries; `XI` Northern Ireland kept as `"XI"`, `GB` aliased to `826`; both filtered from market sales by `is_valid_gdsn_market_country`). Risk class refdata + `risk_class_system_code` (76 for MDR/IVDR Regulation, 85 for MDD/AIMDD/IVDD Directive). `multi_component_to_gs1` for non-SPP path (default DEVICE), `spp_type_to_gs1` for SPP path (only PROCEDURE_PACK/SYSTEM allowed) — disjoint code lists, must not share a function. `mu_code_to_characteristic_code` (MU137..MU176 → `ClinicalSizeCharacteristicsCode`, 35 codes; when Some, emit as characteristic and skip MeasurementValue; when None, treat as unit via `measurement_unit_to_gs1`).
- **config.rs**: Loads `config.toml` (provider GLN, publish GLN, GPC codes, target market, Gmail credentials, endocrine substance lookups). `config.sample.toml` is template; `config.toml` is gitignored. Embedded `DEFAULT_CONFIG` fallback.
- **download.sh**: Unified download + convert script. Usage: `./download.sh --N` or `./download.sh --srn <SRN> [SRN2 ...] [--N]`. EUDAMED API uses 0-based pagination.
- **`regenerate` subcommand**: rayon-parallel rewrite of every `eudamed_json/detail/*.json` → `firstbase_json/<uuid>.json` with DraftItem envelope. Ignores `udi_versions` by design.
- **`repush-srn` subcommand**: CLI mirror of GUI Mode 4. SRN args or `--file srns.txt`. Queries `listing_cache` for UUIDs, restores matching files from `processed/` to `firstbase_json/`, pushes via `gui::push_to_firstbase()`. `--reconvert` flag (mirror of Mode 5) re-runs `transform_detail` first, then restores from processed/ for any remaining gaps.
- **`reconvert_uuids_from_detail()` helper**: rayon-parallel re-conversion of `eudamed_json/detail/<uuid>.json` → `firstbase_json/<uuid>.json`. Optional `uuids_filter` for subset rewrites. Used by `regenerate`, `repush-srn --reconvert`, GUI Mode 5.
- **`status` subcommand**: read-only snapshot of ingest + push state. Safe alongside running `check` (DB in WAL mode).
- **scan.rs**: Fast parallel GTIN scanner for firstbase JSON. String search (no JSON parse). Outputs `filepath\tGTIN`.
- **firstbase_validation.py**: Schema validation against cached GS1 Swagger spec. Cache in `.swagger_cache.json`. Note: `IsBrandBankPublication` exists only in Product API, not in Catalogue Item API.
- **swissdamed.rs**: Swissdamed M2M API output model. Uppercase language codes (DE/EN/FR/IT/ANY), `textValue` field name, separate endpoints per legislation. OpenAPI spec: `https://playground.swissdamed.ch/v3/api-docs/udi-m2m-v1`. Playground note: UDI-1063 applies — DI-Codes registered by one CHRN block other users.
- **push_to_swissdamed.sh**: Pushes pre-built JSON from `swissdamed_json/`. Auto-routes per legislation. OAuth2 via Azure CIAM. Skips already ACCEPTED UUIDs.
- **push_to_firstbase.sh**: Pushes firstbase JSON to GS1 Catalogue Item API. PublishToGln required first arg. All devices (incl. legacy MDD/AIMDD/IVDD since 097.096 downgrade on 2026-03-10) via `Live/CreateMany` + `AddMany`. Keeps `CatalogueItemChildItemLink` nested (G472 fix). Filters non-numeric GTINs (HIBC/IFA). Auto-throttles (1s/8s). Retries 429. Token refreshed before AddMany. HTML log to `log/firstbase_<env>/MM.HH_DD.MM.YYYY.log.html`.

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

- **#1 BR-UDID-073**: Status not propagated from Base Unit to Container Packages — fixed in v1.0.49 by propagating `eu_status` + `discontinuedDateTime` to all package levels.
- **#2 ON_MARKET without countries**: 7 devices have ON_MARKET status but null marketInfoLink + null placedOnTheMarket. Workaround: 097.020 fallback to manufacturer country (EU/EEA) or DE.
- **#3 null MDR booleans**: ~2% of MDR Basic UDI-DI records have null active/implantable/measuringFunction. Default to false.
- **#4 MDR Class III missing certificate**: Closed (resolved 2026-03-12).
- **#5 097.039 NOT_INTENDED_FOR_EU_MARKET**: Closed (warning since 2026-03-25).
- **#6 1:n Mapping Gaps**: 17 EUDAMED fields with fallback logic.
- **#7 GDSN mandatory gaps**: Packaging hierarchy (PALLET not derivable) and issuingEntityCode (parsed but not mapped).
- **#8 GTIN deduplication**: Same GTIN under MDR and MDD; GDSN allows only one. Push deduper prefers MDR (has `GlobalModelNumber`).
- **#9 097.041 MDR Class IIB implantable without certificate**: 332x. EUDAMED data quality.
- **#10 Updateable rules (097.029 / 097.036 / G485)**: Reopened 2026-05-04 — G485 actively blocks `discontinuedDateTime` re-pushes for NO_LONGER devices (the field becomes protected after first ACCEPTED). Short-term mitigation shipped in v1.0.53: Mode 4/5 + `repush-srn` skip NO_LONGER + already-ACCEPTED in this env (`version_db::filter_skip_no_longer_accepted`). Long-term `DocumentCommand: "CORRECT"` work tracked in #40.
- **#11 097.078 StorageHandling language**: Closed (fixed 2026-03-26).
- **#12 097.054 Non-EU manufacturers missing AR SRN**: 150x. No fallback placeholder.
- **#13 097.083 medicinalProduct=true without substance data**: 6x.
- **#40 `DocumentCommand: "CORRECT"` support**: Push protected fields (097.029/097.036/G485) without rejection. Plan: classify per UUID via `push_log` ACCEPTED+env, split into to_add/to_correct lists, run two `CreateMany` rounds, AddMany joint at the end. Blocking question: full TradeItem payload vs diff payload (CORRECT semantics — needs GS1 confirmation before full implementation).

## Screenshots

`screenshots/macos/` (2560×1600 Retina, App Store) and `screenshots/windows/` (Microsoft Store, auto-uploaded via CI). Icon at `assets/icon_256x256.png`.

## Reference Files (in maik/)

- `EUDAMED_APP-DTX-000084634.xml` — Input reference
- `CIN_7612345000435_07612345780313_097.json` — Output reference
- `GS1_UDI_Connector_Profile_Overview_*.xlsx` — Authoritative mapping spec (UDID_CodeLists sheet drives `mappings.rs`)

## Known Gaps vs Reference

- TradeItemSynchronisationDates: `lastChangeDateTime` = current UTC (avoids SYS25 + G572); `effectiveDateTime` = EUDAMED `version_date`; `publicationDateTime` = current UTC; `discontinuedDateTime` = today+1 when NO_LONGER.
- DirectPartMarkingIdentifier: from `directMarkingDi` in EUDAMED JSON (not derivable from XML).
- Language ordering may differ from reference (reference is inconsistent).
- ADDITIONAL market country ordering may differ from reference.
- CatalogueItem Identifier: random v4 UUIDs.
- TradeItemUnitDescriptorCode: UDI-DI → BASE_UNIT_OR_EACH; Package → CASE or PACK_OR_INNER_PACK. No PALLET. Package DIs inherit EMA/EAR contacts (SRN only) from base unit so CH-REPs can filter by SRN.
- ManufacturerDeclaredReusabilityTypeCode: SINGLE_USE / LIMITED_REUSABLE / REUSABLE. REUSABLE_SAME_PATIENT not derivable.

## GS1 firstbase Catalogue Item API

- **Endpoints**: Test `https://test-webapi-firstbase.gs1.ch:5443`; Production `https://webapi-firstbase.gs1.ch`.
- **Swagger UI**: `<base>/helpPages/catalogueItemApi/index`.
- **GUI environment switch**: `FirstbaseEnv` enum (Test/Production), defaults to Test. Production needs separate credentials and a production-valid Publish-To-GLN.
- **Auth**: `POST /Account/Token` with `{"UserEmail":..., "Password":..., "Gln":...}` → JWT (~48h).
- **Password reset**: "Platform Auth (UAT) password reset for API" link (M2M Quick Guide PDF page 10), NOT the Web-UI SSO reset.
- **Workflow**: `Live/CreateMany` (batches of 100, `DocumentCommand: "Add"`, no `DataRecipient`) → poll `RequestStatus/Get` until Done (up to 6min, 15s intervals) → `AddMany` to publish to recipient GLN → poll until Done. Both async — must poll before proceeding. Token refreshed before AddMany.
- **PublishToGln**: first CLI argument to `push_to_firstbase.sh` (e.g. `7612345000527` for GS1 Switzerland UDI Data Dump).
- **Basic UDI-DI cache** in `eudamed_json/basic/`, keyed by UDI-DI UUID. Provides MDR booleans, riskClass, regulatory act, manufacturer/AR SRN, basicUdi code. Falls back to false defaults on miss. Populated via `GET /devices/basicUdiData/udiDiData/{uuid}`.

## EUDAMED Public API

- Base: `https://ec.europa.eu/tools/eudamed/api/devices/udiDiData`
- Listing: `GET ?page=N&pageSize=300&iso2Code=en&languageIso2Code=en` (capped to 20 server-side, 0-based pagination).
- Listing with SRN: `GET ?...&srn=<SRN>` — Swiss SRNs (`CH-MF-*`/`CH-AR-*`) not in EUDAMED.
- Detail: `GET /{uuid}?languageIso2Code=en`.
- Basic UDI-DI: `GET /basicUdiData/udiDiData/{uuid}?languageIso2Code=en`.
- Detail lacks manufacturer/AR SRN, risk class, MDR booleans → merged from BUDI cache and/or listing data.
- Serde structs use `#[allow(dead_code)]` (parsing-only fields).
