# eudamed2firstbase

Rust GUI + CLI tool that converts EUDAMED medical device data into GS1 firstbase and Swissdamed JSON format. Cross-platform GUI (macOS + Windows + Linux) for one-click download, convert, and push. Distributed via GitHub Releases (macOS DMG, Windows MSIX/ZIP, Linux AppImage/tar.gz), Microsoft Store (auto-publish with DE/EN listings + screenshots), and macOS App Store (sandbox-compatible, iTMSTransporter upload). Also supports CLI with six input modes: GUI (default), download, check, XML, NDJSON, EUDAMED JSON, and XLSX export.

## GUI (recommended)

```bash
cargo run                                     # launch GUI (default when no arguments)
cargo run gui                                 # also launches GUI
./bundle_macos.sh                             # build macOS .app bundle
```

The GUI provides:
- SRN input (multiple SRNs, one per line or space/comma-separated)
- Limit per SRN option
- Push target selector: **GS1 firstbase** or **Swissdamed** (radio buttons)
- Target-specific credentials (collapsible):
  - Firstbase: email, password, provider GLN, publish-to GLN
  - Swissdamed: client ID, client secret, API base URL
- Dry run mode (download & convert only, no push)
- Resizable splitter between settings panel and log panel
- One-click pipeline: Download → Convert → Push (full Firstbase API push in Rust: CreateMany + polling + AddMany, no shell script needed)
- Pre-download version check (compares listing `versionNumber` vs version DB, skips unchanged devices) → Download details (50 parallel, 3 retries) → Download Basic UDI-DI (50 parallel, 3 retries) → Convert to target format (rayon-parallel per-core) → Push with token retry, 429 backoff, and RequestStatus polling
- Download log written to `eudamed_json/log/download.log` (same format as `download.sh`)
- Live progress reporting: listing pages ("page 2/54 — 40 devices so far (of 1074 total)"), per-SRN version classification ("SRN DE-MF-...: 509 devices [12↑ new, 0↑ udi, 0↑ budi, 497 same]"), detail/basic download counters ("detail 10/1074 downloaded"), conversion summary
- Live scrollable log output with file save paths
- Worker thread panic protection: panics in the background pipeline are caught and displayed in the log (not silently lost)
- Persistent settings across restarts (`settings.json`)
- Auto-saved logs to `logs/`
- All data stored in `~/eudamed2firstbase/` (Windows: `%USERPROFILE%\eudamed2firstbase\`)
- WhatsApp integration (Baileys): pair this device via native in-GUI QR modal, send any push-log HTML as a document to a group/user JID, session persists across restarts
- **Environment-segregated push logs** (since v1.0.39): every push is tagged Test or Production in the DB (`push_log.firstbase_env`, `push_session.firstbase_env`), HTML logs land under `log/firstbase_test/` or `log/firstbase_prod/` (never mixed), and each report has a full-width coloured banner — red for PRODUCTION, blue for TEST — showing the API base URL so the environment cannot be missed. Separate "Send latest Prod log" and "Send latest Test log" WhatsApp buttons.
- **"Repush SRN" button** (since v1.0.41): takes the SRN list from the SRN input, looks the UUIDs up in `listing_cache`, restores any matching `<uuid>.json` from `firstbase_json/processed/` back into `firstbase_json/`, then pushes. Bypasses the `udi_versions` unchanged-skip — the right tool when you want to re-send a specific manufacturer's devices to Firstbase. Mirrored as the CLI subcommand `cargo run repush-srn <SRN> [SRN2 …]`.
- **"Reconvert + Repush SRN" button** (since v1.0.44): same as Repush SRN, but first re-runs the converter for the SRN's UUIDs from `eudamed_json/detail/` and writes fresh `firstbase_json/<uuid>.json`. Use this whenever the converter has gained new GS1 fields (e.g. v1.0.43 added `DescriptionShort`) and you want them live in Firstbase without waiting for upstream EUDAMED data to change. CLI equivalent: `cargo run repush-srn --reconvert <SRN>`.
- **Version line on every pipeline start** (since v1.0.41): the first log line of each run prints `eudamed2firstbase v<version> — mode: <name>` (GUI modes 0–4, CLI subcommand name) so remote users can confirm which binary is running.
- **`DescriptionShort` mapped from EUDAMED Trade Name** (since v1.0.43): the Trade Name (FLD-UDID-176) is now emitted twice in the `TradeItemDescriptionInformation` block — once into `TradeItemDescription` (TC ID 3318, full text) and once into `DescriptionShort` (TC ID 3297, truncated to 40 characters with char-aware UTF-8 cut). `DescriptionShort` is the field Firstbase renders in the item-list overview, so devices are now identifiable in the Catalogue browser without opening each item. All four converter paths (XML, API listing, API detail, EUDAMED JSON) emit both fields with matching language codes.
- **`ContactTypeCode` derived from SRN role** (since v1.0.45, [#30](https://github.com/zdavatz/eudamed2firstbase/issues/30)): `EMA` vs `EPP` is now bound to the SRN prefix (`-MF-` → `EMA`, `-PR-` → `EPP`) instead of the device's `multiComponent.criterion`. Fixes ≈84 records where a `-MF-` Manufacturer had registered an SPP device in EUDAMED — the converter previously emitted `EPP` + `-MF-` SRN, a 097.026 mismatch. Affected SRNs include `FR-MF-000026140`, `CZ-MF-000040871`, `CH-MF-000014141`. Run **Reconvert + Repush SRN** for these once on v1.0.45+ to overwrite the existing records in Firstbase.
- **SPP detection bound to `multiComponent.criterion`** (since v1.0.46, [#31](https://github.com/zdavatz/eudamed2firstbase/issues/31)): real System-or-Procedure-Pack (MDR Art. 22(1)/(3)) is now distinguished from MDR devices that just have a multi-component shape (MDR Art. 22(4), *"Procedure pack which is a device in itself"*, FLD-UDID-12). Discriminator is `criterion` (`SPP` vs `STANDARD`), not the `code` suffix — the previous code-suffix match wrongly classified ~600 MF-Actor devices (across 18 SRNs incl. `US-MF-000004112`, `NL-MF-000017126`, `CN-MF-000019124`, `CZ-MF-000040871`, …) as SPP. They now emit `multiComponentDeviceTypeCode=PROCEDURE_PACK` instead of `systemOrProcedurePackTypeCode`, with full healthcare-module booleans and EU sales markets. **Workflow to refresh existing records:** run **Download → Convert → Push** (Mode 0) for the affected SRNs — Mode 5 alone is not enough when `eudamed_json/detail/<uuid>.json` is missing on disk (the GUI now warns when that happens).
- **Mode 5 warns when reconvert misses UUIDs** (since v1.0.46): "Reconvert + Repush SRN" logs how many UUIDs lack `eudamed_json/detail/<uuid>.json` and therefore fall back to the pre-fix `firstbase_json/processed/` content. If you see this warning, run **Download** first so reconvert can pick up the freshest converter logic.
- **Mode numbers in button labels** (since v1.0.47): every action button is now prefixed with its `pipeline_mode` integer (`0: DL+Push …`, `1: Convert & Push (all)`, `2: Convert & Push SRNs`, `3: Repush failed`, `4: Repush SRN`, `5: Reconvert + Repush SRN`) so the mode being executed matches what shows up in the log line `eudamed2firstbase v<version> — mode: <name>`.
- **`ContactTypeCode` driven by SPP+MDR, not by SRN prefix** (since v1.0.48, [#33](https://github.com/zdavatz/eudamed2firstbase/issues/33)): replaces v1.0.45's SRN-prefix heuristic with the actual GS1 rule predicates. `EMA` vs `EPP` is now derived from `is_system_or_pack && is_mdr` — directly from the three rules **097.016** (SPP+MDR ⇒ EPP+SRN), **097.049** (EMA + any reg ⇒ no `systemOrProcedurePackTypeCode`), **097.056** (EPP ⇒ MDR+EU). `is_system_or_pack` itself is now also gated on `is_mdr` (criterion=SPP under IVDR/legacy is treated as data-quality issue and reverts to non-SPP behavior). Implication: an MF-actor that registered an SPP-MDR device in EUDAMED now correctly gets `EPP` + their MF-SRN; PR-actor with a non-SPP device gets `EMA`. Fixes Maik's 2026-05-03 push of 18 MF-SRNs which returned **1208× 097.049**, **928× 097.016**, **70× 097.056** across the same ~302 devices — all addressed in one shot. Run **Reconvert + Repush SRN** for those SRNs on v1.0.48+ to overwrite the existing rejected records.

Environment variables override saved credentials: `FIRSTBASE_EMAIL`, `FIRSTBASE_PASSWORD`, `SWISSDAMED_CLIENT_ID`, `SWISSDAMED_CLIENT_SECRET`, `SWISSDAMED_BASE_URL`.

## Release / Distribution

Releases are built via GitHub Actions on tag push (`v*`):

**macOS** (notarized DMG, universal arm64+x86_64):
```bash
./bundle_macos.sh                    # Local: native release, no signing
./bundle_macos.sh --universal        # Local: universal binary (arm64 + x86_64)
./bundle_macos.sh --sign             # Local: universal + code sign
./bundle_macos.sh --dmg              # Local: universal + sign + DMG
./bundle_macos.sh --notarize         # Local: universal + sign + DMG + notarize
```

**Windows** (portable ZIP + MSIX for Windows Store):
- Built on `windows-latest` runner in GitHub Actions
- MSIX package created with `makeappx.exe` from `windows/AppxManifest.xml`
- Portable ZIP also available for direct distribution

**Creating a release:**
```bash
git tag v0.1.0
git push origin v0.1.0
```

**Sandbox support:** The GUI detects macOS App Sandbox and redirects all file I/O to the container directory (`~/Library/Containers/com.ywesee.eudamed2firstbase/Data/`). Non-sandboxed builds use the current working directory.

**Required GitHub secrets for signing/notarization/store:**
- macOS App Store signing: `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD`, `MACOS_INSTALLER_CERTIFICATE`, `MACOS_INSTALLER_CERTIFICATE_PASSWORD`
- macOS DMG signing: `MACOS_DEVELOPER_ID_CERTIFICATE`, `MACOS_DEVELOPER_ID_CERTIFICATE_PASSWORD`
- macOS App Store upload: `MACOS_PROVISIONING_PROFILE`, `APPLE_API_KEY_P8`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`, `APPLE_TEAM_ID`
- macOS DMG notarization: `APPLE_API_KEY_P8`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`
- Microsoft Store: `MSSTORE_TENANT_ID`, `MSSTORE_CLIENT_ID`, `MSSTORE_CLIENT_SECRET` + variable `MSSTORE_ENABLED=true`
- App IDs: macOS App Store (Apple ID: 6761303902), Microsoft Store (9P889JD1XWS2, yweseeGmbH.Eudamed2Firstbase)

**Note:** First Microsoft Store submission must be done manually via Partner Center (upload MSIX, fill screenshots/age ratings, submit for certification). Subsequent updates are automated via the CI pipeline (bilingual DE/EN listing, screenshots from `screenshots/windows/`, dynamic release notes). Patched winit removes `_CGSSetWindowBackgroundBlurRadius` private API for Apple App Store compliance.

**Post-commit status polling (since v1.0.42):** `POST /commit` to the Partner Center API returns `CommitStarted` synchronously, but Microsoft can silently roll the submission back to Draft if the async package validation fails — producing a green CI run and a stale Store listing. The `publish-microsoft-store` job now polls `/submissions/{id}/status` every 30s (up to 10 min total) after commit. Transitions into `PreProcessing`, `Certification`, `Release`, `Published`, `PendingPublication`, or `Publishing` are treated as accepted (the package made it into Microsoft's certification queue); `CommitFailed`, `Canceled`, `PreProcessingFailed`, `CertificationFailed`, `PublishFailed`, or `ReleaseFailed` dump `statusDetails.errors`, `warnings`, and `certificationReports` to the job log and fail the CI. Diagnosed after v1.0.41 appeared green in CI but showed up in Partner Center as a Draft with the previously published 1.0.39 MSIX still attached. Observed behaviour on the first v1.0.42 run: eight polls of `CommitStarted` (Microsoft's internal preprocessing window), then poll 9 flipped to `Certification` and the job exited clean — total publish step 10m44s.

## Quick Start: Download & Convert from EUDAMED API (CLI)

```bash
cargo run download --srn IN-MF-000014457              # all products for a manufacturer SRN
cargo run download --srn IN-MF-000014457 --50          # first 50 products for a specific SRN
cargo run download --srn SRN1 SRN2 SRN3                # multiple SRNs
cargo run download --srn SRN1 SRN2 --50                # multiple SRNs, limit 50 per SRN
cargo run download --srn SRN1 --convert                # download + auto-convert to firstbase JSON
./download.sh --srn IN-MF-000014457                    # legacy bash script (same functionality)

# Count devices per SRN (parallel EUDAMED API queries)
cargo run count DE-MF-000006701 US-MF-000021065        # count for specific SRNs (TSV output)
cargo run count --file srns.txt                        # count from text file
cargo run count --xlsx file.xlsx                       # count from XLSX col D, writes GTIN_Count column back
cargo run count --xlsx file.xlsx 6                     # custom column number

# Check SRNs for updates, download changed, convert, push to Firstbase
cargo run check /tmp/srn_update                        # check SRNs from file (one per line)
cargo run check /tmp/srn_update --threads 50           # with parallel threads

# Live snapshot of ingest + push state (safe alongside a running `check`)
cargo run status                                       # counts from listing_cache, udi_versions, firstbase_json, push_log

# Force re-convert every local detail file → firstbase_json (rayon parallel, ignores version tracking)
cargo run regenerate                                   # all eudamed_json/detail/*.json → firstbase_json/

# Repush devices for specific SRN(s): restore matching files from processed/ back to firstbase_json/, then push
cargo run repush-srn DE-MF-000005190                       # by SRN argument(s)
cargo run repush-srn DE-MF-000005190 CH-MF-000012345       # multiple SRNs
cargo run repush-srn --file srns.txt                       # SRN list from file (one per line)
cargo run repush-srn --reconvert DE-MF-000005190           # Reconvert + Repush: re-run transform_detail for the SRN's UUIDs (picks up new GS1 fields like DescriptionShort), then push

# Send file as email attachment via Gmail API (service account)
cargo run mailto /tmp/report.csv --to "a@gs1.ch, b@gs1.ch" --from sender@ywesee.com --subject "Report"
cargo run mailto file.xlsx --to recipient@example.com --from sender@example.com --p12 /path/to/key.p12

# Send file (PDF/HTML/image/…) via WhatsApp (Baileys)
cargo run whatsapp --pair                                            # first run: scan QR in terminal
cargo run whatsapp --list-groups                                     # list joined groups with JIDs
cargo run whatsapp --list-contacts [filter]                          # list 1:1 contacts known to this session
cargo run whatsapp log/15.30_17.04.2026.log.html --group 120363…@g.us --caption "Push log"
```

## WhatsApp

Push logs (and any other file — PDF, HTML, image, XLSX) can be sent to WhatsApp groups or users via [Baileys](https://github.com/WhiskeySockets/Baileys) (unofficial WhatsApp Web protocol).

**Setup (once per machine):**

```bash
cd whatsapp && npm install
```

Requires **Node.js ≥ 22** (Baileys v7 segfaults on older versions). The Rust binary locates Node via Homebrew, `/usr/local/bin`, `~/.nvm/versions/node/*/bin`, or `C:\Program Files\nodejs\node.exe`.

**First-run pairing — either route works:**

- **GUI** (no terminal needed): launch the app, expand the **WhatsApp** section, click **Pair / Link Device**. A modal with a native QR code opens — scan it in WhatsApp → Settings → Linked Devices → Link a Device. The modal closes automatically once paired.
- **CLI**: `cargo run whatsapp --pair` — QR is printed in the terminal.

After pairing, the session persists in `whatsapp/auth/` (gitignored). Subsequent sends are one-shot and non-interactive.

**Sending from the GUI:** enter a recipient in the **Phone / Group** field — either a plain phone number like `+41 79 236 45 44` (spaces, `+`, dashes, parens, dots all accepted — normalised on send) or a group JID like `120363…@g.us`. The GUI echoes the normalised value to the log so you can verify it. Click **Send latest Prod log** (red) or **Send latest Test log** (blue) to ship the newest HTML report as a document. The entered value is persisted in `settings.json`.

**Sending from the CLI:** `cargo run whatsapp <file> --group <jid> [--caption <text>]`. The script auto-detects MIME by extension (PDF, HTML, JSON, XLSX → `sendMessage({document})`; PNG/JPG → `sendMessage({image})`).

**Finding contact JIDs:** groups are listed via `cargo run whatsapp --list-groups`. For 1:1 contacts, run `cargo run whatsapp --list-contacts [filter]` — Baileys only sees contacts the phone has actively pushed via `messaging-history.set` or that have messaged you during this session, so a contact you've only sent (and never received from) may show as `(unknown)`. Easiest workaround: open the chat on your phone, tap the contact name, copy the number, and format as `<digits-with-country-code>@s.whatsapp.net` (no `+`, no spaces).

**Not in packaged builds:** the Node subprocess + Baileys can't be shipped in App Store / MS Store builds, so WhatsApp is a developer/server-side feature. The GitHub Release and local `cargo run` work normally.

## Documentation

| Document | Source | PDF |
|---|---|---|
| Update monitoring for Basic UDI-DI &amp; UDI-DI entries | [`docs/version-tracking.html`](docs/version-tracking.html) | [`docs/version-tracking.pdf`](docs/version-tracking.pdf) |
| Legacy MDD/AIMDD/IVDD `globalModelDescription` &amp; FLD-UDID-22 — status quo + Umstellungsplan, sobald GS1-Rule 097.116 auf MDR/IVDR-only verfeinert ist | [`docs/legacy-global-model.html`](docs/legacy-global-model.html) | [`docs/legacy-global-model.pdf`](docs/legacy-global-model.pdf) |
| GUI-Modi 0–5 — Anleitung: was jeder Knopf tut, wann er der richtige ist, Stolperfallen + FAQ ([Issue #32](https://github.com/zdavatz/eudamed2firstbase/issues/32)) | [`docs/gui-modes.html`](docs/gui-modes.html) | [`docs/gui-modes.pdf`](docs/gui-modes.pdf) |

Regenerate the PDF after editing the HTML:

```bash
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --headless --disable-gpu --no-pdf-header-footer \
  --print-to-pdf=docs/version-tracking.pdf \
  file://$PWD/docs/version-tracking.html
```

The download script handles the full pipeline: listing download (with optional SRN filtering), UUID extraction, parallel detail download to `eudamed_json/` as individual JSON files (with resume support), Basic UDI-DI download (for MDR mandatory fields), and firstbase JSON conversion via `cargo run firstbase`.

The `--srn` option uses server-side filtering via the API's `srn=` parameter, which matches manufacturer SRN (`manufacturerSrn`) and authorised representative SRN (`authorisedRepresentativeSrn`). **Note:** Swiss SRNs (`CH-MF-*`, `CH-AR-*`) are not registered in EUDAMED — use the actual EU/EEA manufacturer SRNs (e.g. `DE-MF-*`, `BE-MF-*`) instead. Multiple SRNs can be specified after `--srn` and their results are combined. Listing data is stored in temp files (only used for UUID extraction) — device details are saved directly as `eudamed_json/<uuid>.json`.

## Manual Usage

### Mode 1: XML (DTX PullResponse)

1. Place EUDAMED XML files in the `xml/` directory
2. Run: `cargo run`
3. Output: `firstbase_json/firstbase_dd.mm.yyyy.json`
4. Successfully processed XML files move to `xml/processed/`

### Mode 2: EUDAMED JSON (individual device files) — primary mode

1. Place EUDAMED JSON files in the `eudamed_json/` directory
2. Run: `cargo run firstbase` or `cargo run firstbase <directory>`
3. Output: one firstbase JSON file per input file in `firstbase_json/`
4. EUDAMED files stay in `eudamed_json/detail/` and `eudamed_json/basic/` — version DB tracks what's been processed
5. Auto-detects file type:
   - **UDI-DI level** (has `primaryDi`): full conversion with GTIN, trade name, clinical sizes, market info (ORIGINAL_PLACED/ADDITIONAL split), storage, warnings, substances (CMR/endocrine/medicinal → ChemicalRegulationModule), product designer (EPD contact with address/email/phone), secondary DI, direct marking, unit of use, related devices (REPLACED/REPLACED_BY), regulatory module (MDR/IVDR+EU), packaging hierarchy from `containedItem` (nested CatalogueItemChildItemLink with PACK_OR_INNER_PACK/CASE descriptors, EMA/EAR contacts on package DIs). Merges Basic UDI-DI data from cache for MDR mandatory fields (active, implantable, measuringFunction, multiComponent, tissue, manufacturer/AR SRN, risk class). On cache miss, fetches Basic UDI-DI on demand from EUDAMED API.
   - **Device level** (Basic UDI-DI, no `primaryDi`): manufacturer/AR contact info, risk class, device flags — no GTIN

### Mode 3: API Listing (NDJSON, legacy)

1. Place listing NDJSON files in a directory
2. Run: `cargo run ndjson` or `cargo run ndjson <directory>`
3. Output: `firstbase_json/firstbase_eudamed_*_dd.mm.yyyy.json`

### Mode 4: API Detail (NDJSON with listing merge, legacy)

1. Run: `cargo run detail <details.ndjson> [listing.ndjson]`
2. The optional listing file provides manufacturer SRN, authorised rep SRN, and risk class
3. Output: batch file `firstbase_json/firstbase_eudamed_*_details_dd.mm.yyyy.json` plus individual `firstbase_json/<uuid>.json` per device

### Mode 5: XLSX Export

1. Run: `cargo run xlsx <details.ndjson>`
2. Output: `xlsx/<input_stem>.xlsx`
3. Flattens detail NDJSON into a spreadsheet with columns: UUID, Primary DI, Issuing Agency, Trade Name, Reference, Device Status, Sterile, Single Use, Latex, Reprocessed, Base Quantity, Direct Marking, Clinical Sizes, Markets, Additional Info URL, Version Date, plus certificate columns from Basic UDI-DI cache: Cert Type, Cert Number, Cert Revision, Cert Expiry, Cert Start, Cert Issue Date, Cert NB Name, Cert NB Number, Cert NB Provided (MFR/NB), Cert Status (issued/supplemented/amended). Multiple certificates per device are newline-separated within cells.

## Configuration

Copy `config.sample.toml` to `config.toml` and fill in your values. `config.toml` is gitignored so secrets never end up in the repository.

```toml
[provider]
gln         = "7612345000480"
party_name  = "EUDAMED Public Download Importing"
publish_gln = "7612345000527"          # Default recipient GLN for pushes

[target_market]
country_code = "097"

[gpc]
segment_code = "51000000"
class_code = "51150100"
family_code = "51150000"
category_code = "10005844"
category_name = "Medical Devices"

[gmail]
p12_key       = "/path/to/your-service-account.p12"
service_email = "your-service-account@your-project.iam.gserviceaccount.com"

[endocrine_substances.Estradiol]
ec_number = "200-023-8"
cas_number = "50-28-2"
```

The `[gmail]` section is only needed for `cargo run mailto`. All other fields have embedded defaults that work without a `config.toml` file.

## Project Structure

```
src/
  main.rs                    # Entry point: GUI (no args) or CLI routing for download/xml/ndjson/detail/eudamed_json modes
  download.rs                # Shared EUDAMED download module (listings, version check, parallel fetch, retry)
  gui.rs                     # Cross-platform GUI (egui/eframe): SRN input, credentials, download+convert pipeline
  config.rs                  # config.toml parsing
  eudamed.rs                 # EUDAMED XML parsing (roxmltree DOM)
  api_json.rs                # EUDAMED API listing NDJSON parsing (serde)
  api_detail.rs              # EUDAMED API detail + Basic UDI-DI parsing (serde, substances, MDR booleans)
  eudamed_json.rs            # EUDAMED JSON file parsing (serde, individual device files)
  firstbase.rs               # GS1 firstbase JSON output model (serde)
  transform.rs               # XML -> firstbase conversion logic
  transform_api.rs           # API listing -> firstbase conversion logic
  transform_detail.rs        # API detail -> firstbase conversion (substances, EPD contact, sales split, related devices)
  transform_eudamed_json.rs  # EUDAMED JSON -> firstbase conversion (1:1 file mapping)
  mappings.rs                # Code mapping tables (country, risk class, clinical sizes, units, issuing agency, CMR, multiComponent)
  xlsx_export.rs             # NDJSON detail -> XLSX spreadsheet export
  version_db.rs              # SQLite version tracking DB (per-section change detection)
  scan.rs                    # Fast parallel GTIN scanner for push_to_firstbase.sh (rayon, string search)
  swissdamed.rs              # Swissdamed M2M API mapper (EUDAMED JSON → Swissdamed JSON, ~1:1)

build.rs                   # Windows icon embedding (winresource)
bundle_macos.sh            # macOS .app bundle (--universal, --sign, --dmg, --notarize)
entitlements.plist         # Hardened Runtime entitlements (Developer ID distribution)
entitlements-appstore.plist # Mac App Store entitlements (sandboxed)
assets/                    # App icons: icon.icns (macOS), icon.ico (Windows), icon_256x256.png (GUI)
windows/                   # AppxManifest.xml + Store tile assets for MSIX/Microsoft Store
.github/workflows/         # CI/CD: build + sign + publish to GitHub/MS Store/App Store on tag push
download.sh                # Unified download + convert script (listing + detail + Basic UDI-DI + convert)
download_10k.sh            # Legacy: download 10k listings
download_details.sh        # Legacy: download details from UUID list
firstbase_validation.py    # Schema validation against GS1 Product API Swagger spec
push_to_firstbase.sh       # Push firstbase JSON to GS1 Catalogue Item API (Live+Publish for all devices)
push_to_swissdamed.sh      # Push swissdamed JSON to Swissdamed M2M API (OAuth2 Azure CIAM, per-legislation endpoints)
swissdamed_json/           # Swissdamed M2M JSON output (526K+ devices: MDR/MDD/SPP/IVDR/IVDD/AIMDD)
winit-patched/             # Patched winit 0.30.13 (removed private macOS API for App Store compliance)
log/                       # API push logs (MM.HH_DD.MM.YYYY.log.html with GTIN-mapped errors + raw API JSON)
```

## What it does

- Wraps output in DraftItem envelope: `{"DraftItem": {"TradeItem": ..., "Identifier": "Draft_<uuid>"}}`
- Writes both batch JSON and individual per-UUID files in detail mode
- Parses EUDAMED PullResponse XML with full namespace handling
- Reconstructs packaging hierarchy (base unit -> intermediate -> outermost package)
- Translates all EUDAMED codes to GS1 equivalents:
  - Country codes (alpha-2 -> numeric)
  - Risk class (CLASS_IIA -> EU_CLASS_IIA, etc.)
  - Device status (ON_THE_MARKET -> ON_MARKET, etc.)
  - Production identifiers (SERIALISATION_NUMBER -> SERIAL_NUMBER, etc.)
  - Clinical size types (~65 CST codes)
  - Measurement units (~136 MU codes)
  - Storage handling conditions
  - Substance types (CMR, endocrine, medicinal, human product)
  - Sterilisation: sterile=true → UNSPECIFIED, false → NOT_STERILISED; sterilization-before-use=true → UNSPECIFIED, false → NO_STERILISATION_REQUIRED
  - Issuing agency (GS1/HIBC/ICCBBA/IFA)
- Only GS1 identifiers (GTIN/GMN) are written to the `Gtin` field; non-GS1 primary DIs (HIBC, IFA/PPN) are placed in `AdditionalTradeItemIdentification` with the appropriate type code
- Maps substances to ChemicalRegulationInformation (WHO for medicinal/human, ECHA for CMR/endocrine)
- Extracts contact information (manufacturer, authorised representative, product designer)
- Generates market info with country-specific sales conditions
- Maps Notified Body certificates (CertificateLink) to `CertificationInformationModule` — see [EUDAMED UDI Registration Process](#eudamed-udi-registration-process) below

## EUDAMED UDI Registration Process

Per the [EUDAMED UDI Registration Process](https://health.ec.europa.eu/document/download/c3231845-228e-437a-8d77-510ecc3a548b_de?filename=md_eudamed-udi-registration-process_en.pdf), high-risk class devices follow a two-phase registration:

1. **Manufacturer registers** the device: Basic UDI-DI → UDI-DI information → **Certificate information** (DeviceCertificateInfo, FLD-UDID-60..64). Certificate information is required for MDR Class III and Class IIb, and IVDR Class D, C, and B with Self-testing or Near patient testing. After submission, the device is "SUBMITTED" but **not yet publicly available**.

2. **Notified Body confirms** the device data by registering the relevant product certificate (**CertificateLink**, FLD-UDID-344..361). Only after NB confirmation does the device become **REGISTERED** and publicly available in EUDAMED (MDR Art 29(3), IVDR Art 26(2)).

Both certificate types are stored in `deviceCertificateInfoListForDisplay` in the Basic UDI-DI record, distinguished by the `nbProvidedCertificate` flag:

| Source | Entity | EUDAMED Fields | GS1 CertificationStandard examples |
|---|---|---|---|
| Manufacturer | DeviceCertificateInfo | FLD-UDID-60..64 | MDR_TECHNICAL_DOCUMENTATION, MDR_TYPE_EXAMINATION |
| Notified Body | CertificateLink | FLD-UDID-344..361 | MDR_QUALITY_MANAGEMENT_SYSTEM, MDR_QUALITY_ASSURANCE |

CertificateLink field mapping status (7 of 10 mapped):

| FLD-UDID | Field | GS1 Mapping | Status |
|---|---|---|---|
| 360 | Certificate Type | CertificationStandard | ✅ mapped |
| 344 | Certificate Number | CertificationValue | ✅ mapped |
| 345 | Revision Number | CertificationIdentification | ✅ mapped |
| 346 | Issue Date | (fallback for StartingValidityDate) | ✅ mapped |
| 347 | Starting Validity Date | CertificationEffectiveStartDateTime | ✅ mapped |
| 348 | Expiry Date | CertificationEffectiveEndDateTime | ✅ mapped |
| 349 | Notified Body | EU_NOTIFIED_BODY_NUMBER | ✅ mapped |
| 350 | Certificate Status | — | ❌ no GDSN pendant |
| 357 | Decision Date | — | ❌ no GDSN pendant |
| 361 | Starting Decision Applicability Date | — | ❌ no GDSN pendant |

The 3 unmapped fields (Certificate Status, Decision Date, Starting Decision Applicability Date) are deserialized from EUDAMED but have no corresponding GDSN attribute. Possible options: AvpList (GS1 extension mechanism), XLSX export column, or not needed. Needs clarification with GS1.

For hospital customers receiving the EUDAMED data dump via GS1 firstbase, the CertificateLink data provides proof that the Notified Body has confirmed the device — essential for high-risk device procurement decisions.

**Multi-certificate emission.** When EUDAMED holds multiple certificates of different `CertificationStandard` for the same device (typical MDR pattern: `MDR_QUALITY_MANAGEMENT_SYSTEM` + `MDR_TECHNICAL_DOCUMENTATION`), each is emitted as its own element in the `CertificationInformation` array — the GDSN schema requires this because `CertificationStandard` is a single-string field per object. End-to-end verified on 2026-04-27 by re-pushing all 306 devices for SRN `IT-MF-000029499` to GS1 firstbase TEST: 306 accepted, 0 rejected, both standards visible in Firstbase. Across our reference set, 344 of 788 devices carry ≥2 `CertificationInformation` entries.

## EUDAMED Public API

The download script uses the EUDAMED public API at `https://ec.europa.eu/tools/eudamed/api/devices/udiDiData`:

- **Listing endpoint**: `GET ?page=N&pageSize=300` — basic device info (GTIN, risk class, manufacturer)
- **Listing with SRN filter**: `GET ?page=N&pageSize=300&srn=<SRN>` — server-side filtering by manufacturer or authorised rep SRN
- **Detail endpoint**: `GET /{uuid}?languageIso2Code=en` — full device data (clinical sizes, substances, market info, warnings)

- **Basic UDI-DI endpoint**: `GET /basicUdiData/udiDiData/{uuid}?languageIso2Code=en` — Basic UDI-DI record for a UDI-DI UUID

The detail endpoint provides richer data but lacks manufacturer/AR SRN, risk class, and MDR mandatory boolean fields (active, implantable, measuringFunction, multiComponent, tissue). These are merged from the Basic UDI-DI cache (`eudamed_json/basic/`) and/or listing data.

## Validation

### Offline: Swagger Schema Validation

Validates generated firstbase JSON against two GS1 Swagger schemas:

- **Product API** (recipient): 978 definitions, 189 TradeItem properties — `test-productapi-firstbase.gs1.ch`
- **Catalogue Item API** (sender): 1043 definitions, 188 TradeItem properties — `test-webapi-firstbase.gs1.ch:5443`

If you're directly working inside the Swiss firstbase ecosystem (web UI + API for Swiss suppliers, importers, hospitals/pharmacies), you will most often use the [Product API](https://test-productapi-firstbase.gs1.ch/helpPages/productapi/index). If you're doing classic GDSN data synchronisation (send/receive product data with international partners or in GDSN format), you will usually interact with the [Catalogue Item API](https://test-webapi-firstbase.gs1.ch:5443/helpPages/catalogueItemApi/index).

```bash
python3 firstbase_validation.py                          # validate all files in firstbase_json/
python3 firstbase_validation.py file.json                # validate specific file(s)
python3 firstbase_validation.py --verbose                # show per-file pass/fail detail
python3 firstbase_validation.py --dump-schema TradeItem  # inspect a schema definition
python3 firstbase_validation.py --refresh                # re-download Swagger spec
```

Checks field names, data types, enum values, and nested module structures recursively, including packaging hierarchy children.

You can drill into any nested type the same way, e.g.:

```bash
python3 firstbase_validation.py --dump-schema MedicalDeviceInformation
python3 firstbase_validation.py --dump-schema HealthcareItemInformation
python3 firstbase_validation.py --dump-schema SalesInformation
```

### Online: Catalogue Item API Validation

You can upload generated JSON directly to the GS1 firstbase Catalogue Item API for server-side validation. This catches issues that the offline Swagger check misses (e.g. GTIN check digits, code list membership).

**Environments:**

- **Test**: `https://test-webapi-firstbase.gs1.ch:5443` — default, safe for validation
- **Production**: `https://webapi-firstbase.gs1.ch` — real data. The GUI has an Environment radio (Test/Production) in the firstbase credentials panel; selecting Production shows a red warning. Production requires separate credentials and a production-valid `Publish To GLN`.

#### 1. Get an Access Token

The API uses token-based authentication via the GS1 Platform Auth SSO.

**First-time setup — password reset:**

1. Open the [M2M Quick Guide PDF](maik/5329.pdf) (page 10) in a PDF viewer
2. Click the **"Platform Auth (UAT) password reset for API"** hyperlink — this is a different link than the Web-UI SSO reset
3. Enter your email and set a password
4. Use this password for API token requests

**Important:** The Web-UI password reset link (`uat-sso.tradeconnectors.org/ResetPassword/ChangePassword?...redirectAfterResetPasswordUrl=https://test-firstbase.gs1.ch/`) resets the **Media API / Web-UI** password, not the REST API password. You must use the "Platform Auth (UAT) password reset for API" link from the PDF.

**Request a token:**

```bash
curl -s -X POST 'https://test-webapi-firstbase.gs1.ch:5443/Account/Token' \
  -H 'Content-Type: application/json' \
  -d '{"UserEmail":"you@example.com","Password":"your-api-password","Gln":"7612345000480"}'
```

This returns a JWT bearer token (valid ~48h).

#### 2. Create a Draft

```bash
TOKEN="<your-token>"
curl -s -X POST 'https://test-webapi-firstbase.gs1.ch:5443/CatalogueItem/Draft/CreateOne' \
  -H 'Content-Type: application/json' \
  -H "Authorization: bearer $TOKEN" \
  -d @firstbase_json/<uuid>.json
```

The response contains `ResponseStatusCode: "ACCEPTED"` on success, or `AttributeException` / `GS1Error` details on validation failure.

#### 3. Publish to a Recipient

After creating drafts, publish them to a recipient GLN (e.g. `7612345000527` for GS1 Switzerland UDI Data Dump):

```bash
curl -s -X POST 'https://test-webapi-firstbase.gs1.ch:5443/CatalogueItemPublication/AddMany' \
  -H 'Content-Type: application/json' \
  -H "Authorization: bearer $TOKEN" \
  -d '{
    "Items": [{
      "Identifier": "Draft_<uuid>",
      "DataSource": "7612345000480",
      "Gtin": "06944233413739",
      "TargetMarket": "097",
      "PublishToGln": ["7612345000527"]
    }]
  }'
```

You can publish multiple items in a single request by adding more objects to the `Items` array. The response returns a `RequestIdentifier` on success.

#### 4. Bulk Workflow: push_to_firstbase.sh

The `push_to_firstbase.sh` script handles the full workflow:

- **All devices** (MDR/IVDR/MDD/AIMDD/IVDD) → `Live/CreateMany` (batches of 100) → poll `RequestStatus/Get` until Done → `AddMany` (publish to recipient) → poll `RequestStatus/Get` until Done

`Live/CreateMany` creates/updates items in the supplier account (7612345000480). `AddMany` publishes them to the recipient GLN (e.g. 7612345000527). Both are async — the script polls `RequestStatus/Get` after each step until Done (up to 6 minutes, 15s intervals). Two HTML logs are written per push: one for CreateMany, one for AddMany.

Since 2026-03-10, GS1 rule 097.096 was downgraded from error to warning — legacy devices (MDD/AIMDD/IVDD) can now be published too. Includes automatic throttling (1s for ≤60 files, 8s for larger batches), HTTP 429 retry with `retry-after` backoff.

```bash
./push_to_firstbase.sh 7612345000527                    # push all UUID files in firstbase_json/
./push_to_firstbase.sh -v 7612345000527                 # verbose: show curl connection details
./push_to_firstbase.sh 7612345000527 --dir /path/to/dir # push files from a custom directory
./push_to_firstbase.sh 7612345000527 --dry-run          # show what would be pushed, no API calls
./push_to_firstbase.sh --status <reqid>                  # query status of a previous request
```

The first positional argument is the recipient GLN (PublishToGln) — the GLN of the data pool or company to publish to (e.g. `7612345000527` for GS1 Switzerland UDI Data Dump, `7612345000350` for SuperAdmin Company CH).

Environment variables for credentials:

```bash
export FIRSTBASE_EMAIL="you@example.com"
export FIRSTBASE_PASSWORD="your-api-password"
export FIRSTBASE_GLN="7612345000480"
./push_to_firstbase.sh 7612345000527
```

All devices are created as live products via `Live/CreateMany` (batches of 100, `DocumentCommand: "Add"`). The script polls `RequestStatus/Get` until async processing is Done (up to 6 minutes), refreshes the auth token, then publishes to the specified recipient GLN via `AddMany` and polls until Done. Both steps retry HTTP 429 with `retryAfter` backoff. Per-UUID ACCEPTED/REJECTED results are logged to `push_log`, `push_session`, and `push_error` tables in `db/version_tracking.db`. Successfully sent files are moved to `firstbase_json/processed/`; rejected files stay in `firstbase_json/` for retry via "Repush failed" button. GTIN deduplication prefers MDR/IVDR over MDD/legacy when same GTIN exists in multiple files. Files without a valid numeric GTIN (HIBC/IFA devices) are automatically skipped to prevent whole-batch rejection.

**Credentials:** `FIRSTBASE_EMAIL` and `FIRSTBASE_PASSWORD` must be set as environment variables (in `~/.bashrc`). The script will abort if they are not set.

**Packaging hierarchy handling:** Files with `CatalogueItemChildItemLink` (packaging hierarchy) are sent with children nested inline — the GS1 API requires parent and child items in the same document structure. Flattening children into separate `Items` array entries causes G472 ("corresponding item record must be populated inside the same CIN document"). Both parent and child GTINs are published via `AddMany`.

**Important:** Do NOT pass `DataRecipient` in `Live/CreateMany` — it causes 910.031 "not allowed to create private version". `AddMany` only works on live products — it will fail with 910.033 on draft-only items.

#### Validation Error Fixes Applied

After initial submission of 100 devices (1341 errors, 15 patterns), the following fixes were applied:

| Error | Count | Fix |
|---|---|---|
| G572 lastChangeDateTime in future | 88x | lastChangeDateTime uses current UTC time (avoids SYS25 on re-uploads and G572 future-date rejection from timezone mismatch); effectiveDateTime uses `version_date` from EUDAMED; discontinuedDateTime=today+1 for NO_LONGER_ON_THE_MARKET |
| G641 device self-replacement | 10x | Skip referenced trade items where linked DI = own DI |
| 097.011 missing MDR boolean fields | 648x | Use real values from Basic UDI-DI cache; fall back to false |
| 097.010 missing multiComponent/tissue | 264x | Use real multiComponent from Basic UDI-DI; fall back to `DEVICE` |
| 097.025 missing globalModelNumber | 176x | Use primary DI code as fallback; globalModelDescription uses `deviceName` (FLD-UDID-22) from Basic UDI-DI |
| 097.025 missing globalModelDescription en | — | Treat `allLanguagesApplicable` as English; fallback to `primaryDi.code` (not tradeName) |
| 097.025 MODEL_NUMBER from deviceModel | — | `deviceModel` (FLD-UDID-20) from Basic UDI-DI mapped to `additionalTradeItemIdentification` with typeCode `MODEL_NUMBER` for all devices (not just legacy) |
| 097.013 uDIProductionIdentifierTypeCode | — | From `udiPiType` (mandatory under MDR/IVDR, never null). Legacy devices stripped per 097.095. BATCH_NUMBER fallback removed |
| G541 invalid country code 826 (UK/NI) | — | Skip GB/XI from market sales conditions post-Brexit; XI will become valid with GDSN March/May 2026 release |
| 097.072 missing additionalDescription | 60x | Resolved by defaulting multiComponentDeviceTypeCode to DEVICE |
| 097.020 ON_MARKET needs ORIGINAL_PLACED | 25x | Use `placedOnTheMarket` country when `marketInfoLink` is null; enforce exactly one ORIGINAL_PLACED country. Final fallback: manufacturer country (if EU/EEA) or DE — Member State info is OOS for swissdamed |
| 097.074 storage description missing (BR-UDID-028) | 9x | Fix `extract_descriptions` to handle `language: null` (default to "en"). SHC codes requiring description per BR-UDID-028: SHC06/07/08/09/10/13/21/22/23/25/45 — fallback to code as placeholder only when EUDAMED provides no text |
| 097.005 invalid risk class | 5x | Set MDR vs IVDR regulatory act based on risk class |
| 097.022 Class I implantable conflict | 36x | Data quality issue in EUDAMED (not fixable) |
| 097.009 EMA contact with SRN required | 16x | Already generated from Basic UDI-DI cache (99.2% coverage); remaining files lack cache entries |
| 097.003 missing risk class system 76 | — | Always emit classification system 76; fallback to EU_CLASS_I |
| 097.005 risk class system/code mapping | — | System 76 (MDR/IVDR): EU_CLASS_A/B/C/D for IVDR; System 85 (IVDD/AIMDD): IVDD_GENERAL, IVDD_DEVICES_SELF_TESTING, IVDD_ANNEX_II_LIST_A/B, AIMDD |
| 097.015 implantable IIB exempt field | — | Add `IsDeviceExemptFromImplantObligations` (default false) for implantable + EU_CLASS_IIB |
| 097.009 missing EMA contact with SRN | — | Always emit EMA contact with manufacturer SRN; fallback `XX-MF-000000000` when no Basic UDI-DI data available |
| 097.026 missing Actor contactTypeCode | — | EMA always emitted (was sometimes missing when no Basic UDI-DI cache) |
| 097.054 non-EU needs EAR contact | — | Add EAR contact for non-EU manufacturers only when AR exists in EUDAMED (no fallback). EEA-only countries (IS, LI, NO) treated as non-EU per EUDAMED validation |
| 097.046 IVDR boolean fields missing | — | Add 7 IVDR fields (reagent, instrument, self-testing, etc.) default false |
| 097.047 IVDR isNewDevice missing | — | Default `IsNewDevice` to false for IVDR devices |
| 097.080 CMR/endocrine missing description | — | Always include `regulatedChemicalDescription` with `languageCode: "en"` for CMR/endocrine substances |
| 097.081 endocrine missing description | — | ENDOCRINE_SUBSTANCE always gets description even when CAS/EC identifiers present |
| 097.101 MDR/IVDR certificates | — | Parse `deviceCertificateInfoListForDisplay` from Basic UDI-DI; maps both DeviceCertificateInfo (manufacturer, FLD-UDID-60..64) and CertificateLink (NB-provided, FLD-UDID-344..361) certificate types: MDR/IVDR technical-documentation, type-examination, quality-management-system, quality-assurance; `certificateRevision` → `CertificationIdentification`; `issueDate` fallback for `startingValidityDate` |
| 097.070 DEVICE_SIZE_TEXT_SPECIFY description | — | Add `ClinicalSizeDescription` with text value when `ClinicalSizeTypeCode` is `DEVICE_SIZE_TEXT_SPECIFY` (BR-UDID-722) |
| 097.002 legacy risk class system 85 | — | MDD/AIMDD/IVDD devices use classification system 85 (not 76) per BR-DTX-UDID-002 |
| 097.025 legacy MODEL_NUMBER | — | Legacy devices (no globalModelInformation) get `MODEL_NUMBER` in additionalTradeItemIdentification as Basic UDI-DI reference |
| 097.095 legacy device forbidden fields | — | Strip globalModelNumber, directPartMarkingIdentifier, udidDeviceCount, uDIProductionIdentifierTypeCode, annexXVIIntendedPurposeTypeCode, CMR/endocrine substances for MDD/AIMDD/IVDD devices (BR-DTX-UDID-089) |
| 097.105 MDD certificate required | — | Map MDD legacy certificates (ii-4→MDD_II_4, ii-excluding-4→MDD_II_EX_4, iii→MDD_III, iv→MDD_IV, v→MDD_V, vi→MDD_VI); warn when missing |
| 097.118 GS1 direct marking 14 digits | — | Skip GS1 direct marking DI if not exactly 14 digits (BR-UDID-003) |
| 097.096 legacy device publication | — | Since 2026-03-10 downgraded from error to warning — legacy devices now publishable via Live/CreateMany + AddMany |
| 097.091 SOFTWARE_IDENTIFICATION needs SOFTWARE | — | Add `SpecialDeviceTypeCode: SOFTWARE` when production identifiers include `SOFTWARE_IDENTIFICATION` (BR-DTX-UDI-104) |
| 097.101 MDR Class III certificate required | — | Warning emitted for MDR EU_CLASS_III devices missing MDR_TECHNICAL_DOCUMENTATION or MDR_TYPE_EXAMINATION certificate |
| 097.006 missing MANUFACTURER_PART_NUMBER | — | Always emit `MANUFACTURER_PART_NUMBER` in additionalTradeItemIdentification; falls back to primary DI code when device reference is empty |
| 097.087 secondary DI type code | — | Secondary DI uses correct type code from issuing agency (HIBC/IFA/ICCBBA/GS1) instead of hardcoded GTIN_14 (BR-UDID-020) |
| SCHEMA additionalTradeItemIdentification too long | 14x | Truncate `deviceModel` (MODEL_NUMBER) and `reference` (MANUFACTURER_PART_NUMBER) to 80 characters — GDSN max length for additionalTradeItemIdentificationValue |
| 097.042 certification org identifier type | — | Notified body number (e.g. "0197") in `AdditionalCertificationOrganisationIdentifier` with type `EU_NOTIFIED_BODY_NUMBER` (was `SRN`) |
| 097.105 MDD CertificationValue required | — | `CertificationValue` set to `certificateNumber` from EUDAMED (was missing) |
| G541 DIRECTION_OF_VIEW | 1x | CST63 coming with GDSN May release |

#### UDID → GDSN Mapping Decisions

| EUDAMED field | GDSN field | Mapping |
|---|---|---|
| singleUse=true, numberOfReuses=0 | ManufacturerDeclaredReusabilityTypeCode | SINGLE_USE |
| singleUse=false, numberOfReuses>0 | ManufacturerDeclaredReusabilityTypeCode | LIMITED_REUSABLE + MaximumUsageCycleNumber |
| singleUse=false, no numberOfReuses | ManufacturerDeclaredReusabilityTypeCode | REUSABLE |
| — (not derivable) | ManufacturerDeclaredReusabilityTypeCode | REUSABLE_SAME_PATIENT — cannot be derived from EUDAMED |
| UDI-DI | TradeItemUnitDescriptorCode | BASE_UNIT_OR_EACH |
| Package DI (inner) | TradeItemUnitDescriptorCode | PACK_OR_INNER_PACK |
| Package DI (outer) | TradeItemUnitDescriptorCode | CASE |
| — (not derivable) | TradeItemUnitDescriptorCode | PALLET — not used, cannot be derived from EUDAMED |
| highest level unit | IsTradeItemADespatchUnit | true (BASE_UNIT_OR_EACH when no packaging, CASE for outermost) |
| all units | IsTradeItemAnOrderableUnit | true |
| BASE_UNIT_OR_EACH | IsTradeItemABaseUnit | true |
| (current UTC time) | lastChangeDateTime | Current UTC time at conversion (avoids SYS25 on re-uploads and G572 future-date rejection) |
| versionDate | effectiveDateTime | EUDAMED last update date |
| status=NO_LONGER_ON_THE_MARKET | discontinuedDateTime | today + 1 day |
| languageCode=ANY (allLanguagesApplicable) | languageCode | "en" (single entry, no additional languages) |
| unitOfUse (FLD-UDDI-135) | TradeItemInformation.TradeItemComponents.ComponentInformation | ComponentNumber=1, ComponentIdentification=GTIN with issuing agency, ComponentQuantity=baseQuantity |

## Version Tracking

The `eudamed_json` mode uses a SQLite database (`db/version_tracking.db`) to track per-section version numbers for each UDI-DI. EUDAMED versions each section independently — a manufacturer address change increments `manufacturer.versionNumber` without touching the UDI-DI root version.

Version numbers are indexed into `udi_versions` at two points:
- **On download**: newly downloaded detail files are automatically indexed (parallel parse + batch DB insert)
- **On conversion**: per-section version comparison determines what changed

On each converter run:
1. Computes SHA256 of the Detail API JSON (fast path: if hash unchanged → skip)
2. If hash differs, compares per-section version numbers to identify what changed
3. Logs a change summary: `NEW`, `MFR+CERT`, `STATUS+MARKET`, etc.
4. Updates the DB after successful conversion

**Skip-safety fallback (since v1.0.41):** the download step indexes `udi_versions` *before* convert runs (so repeat runs of `download --srn X` can skip unchanged devices without re-converting). On the very first download of a new SRN that caused step 1 to say "unchanged" even though the converter had never actually produced output, leaving `firstbase_json/` empty and therefore nothing to push. The converter now verifies that either `firstbase_json/<uuid>.json` or `firstbase_json/processed/<uuid>.json` exists before trusting an "unchanged" verdict; if neither is present, it falls through to actual conversion so the output is produced. Fixes both the full GUI pipeline (`gui.rs`) and the `firstbase`/`eudamed_json` subcommand (`main.rs`).

Tracked sections per UDI-DI (UUID):

| Section | Source | Version fields |
|---|---|---|
| UDI-DI root | Detail API `/{uuid}` | `versionNumber`, `versionDate` |
| Basic UDI-DI | BUDI API | `versionNumber`, `versionDate` |
| Manufacturer | BUDI → `manufacturer` | `versionNumber`, `lastUpdateDate` |
| Authorised Rep | BUDI → `authorisedRepresentative` | `versionNumber`, `lastUpdateDate` |
| Certificates | BUDI → `deviceCertificateInfoList[*]` | `[versionNumber, ...]` |
| Package | Detail → `containedItem` | `versionNumber`, `versionDate` |
| MarketInfo | Detail → `marketInfoLink` | `versionNumber`, `versionDate` |
| DeviceStatus | Detail → `deviceStatus` | status code, `statusDate` |
| ProductDesigner | Detail → `productDesigner` | `versionNumber`, `versionDate` |

```bash
# Inspect the version DB
sqlite3 db/version_tracking.db "SELECT uuid, gtin, udi_version, mfr_version, device_status FROM udi_versions LIMIT 10"

# Query push history for a UUID
sqlite3 db/version_tracking.db "SELECT pushed_at, status, error_code, error_msg FROM push_log WHERE uuid='<uuid>' ORDER BY pushed_at DESC"

# Summary of last push
sqlite3 db/version_tracking.db "SELECT status, COUNT(*) FROM push_log WHERE request_id='<req_id>' GROUP BY status"
```

## Known EUDAMED Bugs

Bug reports are tracked as [GitHub Issues](https://github.com/zdavatz/eudamed2firstbase/issues):

| # | Category | Title | GS1 Errors | Status |
|---|---|---|---|---|
| [#1](https://github.com/zdavatz/eudamed2firstbase/issues/1) | BR-UDID-073 | Status propagation to container packages | 097.039, 097.040, 910.004 | Open |
| [#2](https://github.com/zdavatz/eudamed2firstbase/issues/2) | Data Quality | ON_MARKET without country information | 097.020, 097.010, 097.011, G541 | 097.020 fixed (fallback) |
| [#3](https://github.com/zdavatz/eudamed2firstbase/issues/3) | Data Quality | Null MDR mandatory boolean fields | 097.010, 097.011 | Open (reduced) |
| [#4](https://github.com/zdavatz/eudamed2firstbase/issues/4) | Data Quality | MDR Class III missing certificate | 097.101 | Closed (resolved 12.03.2026) |
| [#5](https://github.com/zdavatz/eudamed2firstbase/issues/5) | GS1 Rule | NOT_INTENDED_FOR_EU_MARKET rejected for non-EU market devices | 097.039 | Closed (warning since 25.03.2026) |
| [#6](https://github.com/zdavatz/eudamed2firstbase/issues/6) | Mapping | 1:n Mapping Gaps: EUDAMED → GS1 fallback resolvers | — | Open (17 gaps documented) |
| [#7](https://github.com/zdavatz/eudamed2firstbase/issues/7) | Mapping | GDSN mandatory gaps: packaging hierarchy & issuingEntityCode | — | Open (2 gaps, 6 implemented) |
| [#8](https://github.com/zdavatz/eudamed2firstbase/issues/8) | Data | GTIN deduplication: MDR/IVDR priority over MDD/AIMDD/IVDD | v1.0.28 | Fixed (dedup + MDD files moved to processed/) |
| [#9](https://github.com/zdavatz/eudamed2firstbase/issues/9) | Data Quality | MDR Class IIB implantable without certificate | 097.041 | Open (332x, EUDAMED) |
| [#10](https://github.com/zdavatz/eudamed2firstbase/issues/10) | GS1 Rule | Updateable rules block field changes after first sync | 097.029, 097.036 | Open (GS1 disabling soon) |
| [#11](https://github.com/zdavatz/eudamed2firstbase/issues/11) | Mapping | Language mismatch in StorageHandling fallback | 097.078 | Closed (fixed 26.03.2026) |
| [#12](https://github.com/zdavatz/eudamed2firstbase/issues/12) | Data Quality | Non-EU manufacturers missing Authorised Representative SRN | 097.054 | Open (150x, EUDAMED) |
| [#13](https://github.com/zdavatz/eudamed2firstbase/issues/13) | Data Quality | medicinalProduct=true without regulated substance data | 097.083 | Open (6x, EUDAMED) |
| [#18](https://github.com/zdavatz/eudamed2firstbase/issues/18) | Mapping | Duplicate languageCode in tradeItemDescription | 097.078 | Fixed v1.0.28 (merge with " / ") |

Push 26.03.2026: 274 SRNs, 18,007 items → 7,009 ACCEPTED, 1,862 REJECTED. G541 mapping fixes deployed (SPP_PROCEDURE_PACK, COLOUR, BODY_WEIGHT_KG, MU999). G361 empty address fix deployed. GTIN batch filter added.

**Note on Target Market:** Pilot runs with TM=097 (Austria). The 097.xxx validation rules (097.038/039/040/020) must remain as errors — they prevent DRIFT before EUDAMED M2M errors are produced. The 756.xxx (Swiss) rules are not yet fully implemented. Only 097.040 has a Swiss equivalent (756.540). A TM=097→756 swap to bypass blocking rules is deferred.

## Screenshots

### macOS App Store (2560×1600 Retina)

Screenshots in `screenshots/macos/`:

| Screenshot | Description |
|---|---|
| `screenshot_1_main.png` | Main window — empty state with SRN input |
| `screenshot_2_running.png` | Download in progress with live log output |
| `screenshot_3_done.png` | Completed pipeline with success summary |
| `screenshot_4_swissdamed.png` | Swissdamed target with credentials and dry run |
| `screenshot_5_firstbase_creds.png` | GS1 firstbase credentials expanded |

Generated via `generate_screenshots.py` (requires Pillow).

### Windows Store (3840×2160 4K)

Screenshots in `screenshots/windows/` — light theme, Windows 11 title bar:

| Screenshot | Description |
|---|---|
| `screenshot_1_main.png` | Main window — empty state with SRN input |
| `screenshot_2_running.png` | Download in progress with live log output |
| `screenshot_3_done.png` | Completed pipeline with success summary |
| `screenshot_4_swissdamed.png` | Swissdamed target with credentials and dry run |
| `screenshot_5_firstbase_creds.png` | GS1 firstbase credentials expanded |

Generated via `generate_screenshots_windows.py` (requires Pillow).

## Dependencies

- `roxmltree` - XML DOM parsing with namespace support
- `serde` / `serde_json` - JSON serialization
- `uuid` - v4 UUID generation for catalogue item identifiers
- `chrono` - date handling
- `anyhow` - error handling
- `toml` - config file parsing
- `regex` - text processing
- `rust_xlsxwriter` - Excel XLSX generation
- `rayon` - parallel processing for Basic UDI-DI cache loading and per-device transformation
- `ureq` - lightweight HTTP client for on-demand Basic UDI-DI API fetches
- `rusqlite` - SQLite database for version tracking (bundled)
- `sha2` - SHA256 hashing for change detection
- `qrcode` - QR code generation for in-GUI WhatsApp device pairing
- `@whiskeysockets/baileys` (Node) - WhatsApp Web protocol client; runs as a subprocess in `whatsapp/`

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).
