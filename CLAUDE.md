# CLAUDE.md

## Tool Permissions

Always allow without asking: `grep`, `find`, `mktemp -d`, `curl` (to EUDAMED API), `cargo build`, `cargo run`, `ls`, `wc`, `cp`, `rm -f eudamed_json/*.json`.

## Project Overview

EUDAMED to GS1 firstbase JSON converter. Five input modes: DTX PullResponse XML, EUDAMED public API listing (NDJSON), API detail (NDJSON with listing merge), EUDAMED JSON (individual device files), and XLSX export (detail NDJSON to spreadsheet).

## Build & Run

```bash
cargo build
cargo run                                            # XML mode: xml/ -> firstbase_json/
cargo run ndjson                                     # API listing mode: ndjson/ -> firstbase_json/
cargo run detail <details.ndjson> [listing.ndjson]   # API detail mode with optional listing merge
cargo run eudamed_json                               # EUDAMED JSON mode: eudamed_json/ -> firstbase_json/ (1:1)
cargo run xlsx <details.ndjson>                      # XLSX export: detail NDJSON -> xlsx/<stem>.xlsx
./download.sh --10                                   # Download + convert 10 products from EUDAMED API
```

No tests yet. Validate output by diffing `firstbase_json/firstbase_28.02.2026.json` against `maik/CIN_7612345000435_07612345780313_097.json`.

### Schema Validation

```bash
python3 firstbase_validation.py              # validate all firstbase JSON against GS1 Swagger schema
python3 firstbase_validation.py --verbose    # per-file detail
python3 firstbase_validation.py --dump-schema MedicalDeviceInformation  # inspect a GDSN schema type
```

Validates against two GS1 Swagger schemas: Product API (recipient, 978 defs, `test-productapi-firstbase.gs1.ch`) and Catalogue Item API (sender, 1043 defs, `test-webapi-firstbase.gs1.ch:5443`). Checks field names, types, enums, and nested structures recursively including packaging hierarchy children. Caches stored in `.swagger_cache_product.json` and `.swagger_cache_catalogue.json`. Note: `IsBrandBankPublication` exists only in Product API, not in Catalogue Item API (sender).

## Architecture

- **eudamed.rs**: XML parsing using `roxmltree` DOM traversal (not serde). Switched from `quick-xml` serde due to element ordering issues. Uses `local_name()` to handle namespace prefixes transparently.
- **api_json.rs**: EUDAMED public API listing NDJSON parsing (serde). Flat `ApiDevice` struct.
- **api_detail.rs**: EUDAMED public API detail NDJSON parsing (serde). Rich `ApiDeviceDetail` struct with clinical sizes, substances (CMR, endocrine, medicinal, human product), market info, warnings, product designer, secondary DI, direct marking, unit of use, linked devices. Also contains `BasicUdiDiData` struct for Basic UDI-DI records (MDR booleans, multiComponent, riskClass, manufacturer/AR, basicUdi code, legislation). `extract_lang_texts` handles `allLanguagesApplicable: true` with null language by defaulting to "en". `BasicUdiDiData::regulatory_act()` extracts legislation code (MDR/IVDR/MDD/AIMDD/IVDD) — more accurate than risk class inference for distinguishing MDR from MDD.
- **firstbase.rs**: Output JSON model with serde `Serialize`. Uses `#[serde(rename = ...)]` for GS1 field names and `skip_serializing_if` for optional fields. Top-level `DraftItemDocument` wraps `FirstbaseDocument` in `{"DraftItem": {"TradeItem": ..., "Identifier": "Draft_<uuid>"}}`. Identifier is inside DraftItem (required by Catalogue Item API CreateOne).
- **transform.rs**: XML -> firstbase conversion. Builds packaging hierarchy by walking parent-child DI references. Sorts languages (en, fr, de, it), substances (WHO before ECHA), market infos (ORIGINAL_PLACED first).
- **transform_api.rs**: API listing -> firstbase conversion. Simpler mapping from flat listing data.
- **transform_detail.rs**: API detail -> firstbase conversion. Richest output with clinical data, market info, IFU URLs, substances (ChemicalRegulationModule), product designer (EPD contact), secondary DI (as GTIN_14), direct marking, related devices (REPLACED/REPLACED_BY), regulatory module (MDR/IVDR+EU), ORIGINAL_PLACED vs ADDITIONAL_MARKET_AVAILABILITY sales split. Non-GS1 primary DIs (HIBC/IFA) moved to AdditionalTradeItemIdentification. Accepts optional `BasicUdiDiData` for real MDR mandatory fields (active, implantable, measuringFunction, multiComponent, tissue, manufacturer/AR SRN, risk class, basicUdi code). Falls back to false defaults when no Basic UDI-DI data available. Can also merge listing data for manufacturer/AR SRN and risk class (with dedup guards). GlobalModelDescription uses `deviceName` (FLD-UDID-22) from Basic UDI-DI with English fallback (097.025); `deviceModel` (FLD-UDID-20) mapped to MODEL_NUMBER in additionalTradeItemIdentification for all devices; UDIProductionIdentifierTypeCode defaults to BATCH_NUMBER when empty (097.013). ORIGINAL_PLACED uses `placedOnTheMarket` when `marketInfoLink` is null, enforces exactly one country (097.020). `extract_descriptions` handles `language: null` by defaulting to "en" (same as `extract_lang_texts`); storage handling description fallback (097.074/BR-UDID-028) limited to codes SHC06/07/08/09/10/13/21/22/23/25/45.
- **eudamed_json.rs**: EUDAMED JSON device-level file parsing (serde). `EudamedDevice` struct with inline manufacturer/AR objects, basicUdi, riskClass, device flags.
- **transform_eudamed_json.rs**: EUDAMED JSON device-level -> firstbase conversion. Includes full manufacturer/AR contact info with addresses, email, phone. No GTIN (device-level records).
- The `eudamed_json` mode auto-detects file type: UDI-DI level files (have `"primaryDi":{` object with GTIN, trade name, clinical data) use `api_detail`/`transform_detail`; device-level files (Basic UDI-DI, `primaryDi` null) use `eudamed_json`/`transform_eudamed_json`. On cache miss, fetches Basic UDI-DI on demand from EUDAMED API via `ureq` and caches the result.
- **xlsx_export.rs**: Detail NDJSON -> XLSX spreadsheet export. Flattens `ApiDeviceDetail` into columns (UUID, Primary DI, Issuing Agency, Trade Name, Reference, Device Status, booleans, markets, etc.). Uses `rust_xlsxwriter`.
- **mappings.rs**: All code translation tables as match statements. Derived from the UDID_CodeLists sheet of the GS1 UDI Connector Profile spreadsheet. Includes issuing agency to type code (GS1/HIBC/ICCBBA/IFA, plus EUDAMED-assigned → IFA), CMR type mapping, country alpha-2 to numeric (EU+EEA countries plus common non-EU manufacturer countries; GB/XI mapped to 826 for contacts but filtered from market sales via `is_valid_gdsn_market_country`), multi-component refdata codes (system/procedure-pack/spp-procedure-pack → GS1), risk class refdata codes (class-i through class-d, ivd-general, ivd-devices-self-testing, ivd-annex-ii-list-a/b, aimdd → GS1 + regulatory act MDR/IVDR/IVDD/AIMDD), and `risk_class_system_code` to select system 76 (MDR/IVDR Regulation) vs 85 (MDD/AIMDD/IVDD Directive).
- **config.rs**: Loads `config.toml` for provider GLN, GPC codes, target market, and endocrine substance identifier lookups.
- **download.sh**: Unified download + convert script. Usage: `./download.sh --N` or `./download.sh --srn <SRN> [SRN2 ...] [--N]`. Downloads listing to temp file (for UUID extraction only), fetches device details in parallel (10 concurrent, with retry and resume) saving individual JSON files directly to `eudamed_json/<uuid>.json`, downloads Basic UDI-DI data for MDR mandatory fields (cached in `/tmp/basic_udi_cache/`), converts via `cargo run eudamed_json`. Note: EUDAMED API uses 0-based pagination (page=0 is first page).
- **firstbase_validation.py**: Schema validation script. Downloads and caches the GS1 Product API Swagger spec (978 GDSN definitions) from `test-productapi-firstbase.gs1.ch`. Validates field names, data types, enum values, and nested structures recursively. Cache in `.swagger_cache.json`. Handles DraftItem wrapper, batch arrays, and direct TradeItem formats.
- **push_to_api.sh**: Pushes firstbase JSON files to GS1 Catalogue Item API via `Draft/CreateOne` (per file) then publishes via `AddMany` (batches of 100). Handles token acquisition, publish to GLN. Usage: `./push_to_api.sh`, `./push_to_api.sh --dir /path/to/dir`, or `./push_to_api.sh --status <reqid>`. Log output to `log/log_dd.mm.yyyy.log`.

## Key Design Decisions

- `roxmltree` over `quick-xml` serde: EUDAMED XML has 30+ namespace prefixes and strict element ordering that broke serde deserialization.
- Flat domain structs with `Option<bool>` / `Option<String>` / `Vec<T>` instead of wrapper types.
- Packaging hierarchy reconstructed from flat package list by finding outermost package (not referenced as any child) and walking down.
- Endocrine substance EC/CAS identifiers come from config.toml lookup table since EUDAMED XML doesn't provide them.
- Sterilisation uses UNSPECIFIED for true (actual method unknown from EUDAMED), NOT_STERILISED/NO_STERILISATION_REQUIRED for false. No config needed.
- Output wrapped in `DraftItem` envelope with `Identifier: "Draft_<uuid>"` inside DraftItem (not top-level) for Catalogue Item API CreateOne compatibility.
- Detail mode writes both a batch JSON file and individual `<uuid>.json` files.
- `TargetSector` is `["UDI_REGISTRY"]` only (no `HEALTHCARE`).
- Only GS1 identifiers go into `Gtin`; non-GS1 primary DIs (HIBC, IFA/PPN, EUDAMED-assigned) are placed in `AdditionalTradeItemIdentification`. GDSN requires a GS1 GTIN as primary identifier — devices with only HIBC/IFA DIs get an empty `Gtin` and cannot be submitted as GDSN drafts.
- `rayon` parallel processing for Basic UDI-DI cache loading (125K+ files) and per-device transformation (parse, transform, write individual JSON). ~5x speedup on multi-core machines.

## Reference Files (in maik/)

- `EUDAMED_APP-DTX-000084634.xml` - Input reference
- `CIN_7612345000435_07612345780313_097.json` - Output reference
- `GS1_UDI_Connector_Profile_Overview_2026_02-27_zdavatz.xlsx` - Authoritative mapping specification

## Known Gaps vs Reference

- TradeItemSynchronisationDates: lastChangeDateTime uses EUDAMED version_date; effective/publication use current time
- DirectPartMarkingIdentifier: generated from `directMarkingDi` in EUDAMED JSON; not derivable from XML
- Language ordering within multi-language arrays may differ from reference (reference is inconsistent)
- Sales conditions country ordering for ADDITIONAL markets may differ from reference (reference uses neither numeric nor XML order)
- CatalogueItem Identifier: generated as random v4 UUIDs (won't match reference's specific UUIDs)

## GS1 firstbase Catalogue Item API (Test)

- **Endpoint**: `https://test-webapi-firstbase.gs1.ch:5443`
- **Swagger UI**: `https://test-webapi-firstbase.gs1.ch:5443/helpPages/catalogueItemApi/index`
- **Auth**: `POST /Account/Token` with `{"UserEmail":"...","Password":"...","Gln":"7612345000480"}` → JWT bearer token (~48h validity)
- **Password reset**: Must use "Platform Auth (UAT) password reset for API" link from M2M Quick Guide PDF (page 10), NOT the Web-UI SSO reset link
- **Create draft**: `POST /CatalogueItem/Draft/CreateOne` — body is the DraftItem JSON file directly
- **Publish**: `POST /CatalogueItemPublication/AddMany` — Items array with Identifier, DataSource (GLN), Gtin, TargetMarket, PublishToGln array
- **PublishToGln**: `4399902421386` (GS1 UDI Connector recipient)
- **Workflow (preferred)**: Create drafts (`Draft/CreateOne` per file) → Publish all (`AddMany` with Items array, batches of 100) → Recipient sees data
- **Workflow (alternative)**: `Live/CreateMany` (batched, with `DocumentCommand: "Add"` and `PublishToGln`) → `RequestStatus/Get` (with `IncludeGs1Response: true`) for validation results. Note: `Live/CreateMany` currently returns HTTP 500 on the test server.
- **push_to_api.sh**: Automates the preferred workflow. Creates drafts one-by-one via `Draft/CreateOne`, then publishes in batches of 100 via `AddMany`. Auto-throttles: 1s delay for ≤60 files, 8s for larger batches (API limit: 1/sec, 60/min, 500/hour for write endpoints). Retries HTTP 429 with `retry-after` backoff (up to 3 attempts). Query status with `--status <reqid>`.
- **Basic UDI-DI merge**: MDR boolean fields (implantable, active, measuring, multiComponent, tissue, blood, etc.) use real values from Basic UDI-DI cache at `/tmp/basic_udi_cache/` (keyed by UDI-DI UUID). Also provides risk class, regulatory act (MDR/IVDR), manufacturer/AR SRN, and basicUdi code for globalModelNumber. Falls back to false defaults when cache miss. Cache populated via EUDAMED API: `GET /devices/basicUdiData/udiDiData/{udi-di-uuid}`.
- **Other validation fixes**: lastChangeDateTime uses EUDAMED version_date. Self-referencing devices skipped. First market country used as ORIGINAL_PLACED fallback. SHC code used as placeholder storage description. GlobalModelDescription uses `deviceName` (FLD-UDID-22) from Basic UDI-DI (097.025), fallback to first trade name or primary DI code. `deviceModel` (FLD-UDID-20) from Basic UDI-DI mapped to MODEL_NUMBER in additionalTradeItemIdentification for all devices; legacy devices fall back to basicUdi code. UDIProductionIdentifierTypeCode required for MDR/IVDR (097.013): defaults to BATCH_NUMBER when EUDAMED has no production identifiers. GB/XI (UK/Northern Ireland) skipped from market sales conditions post-Brexit (G541). EMA contact with SRN (097.009): generated from Basic UDI-DI cache manufacturer data (99.2% coverage); 16 files without cache entries lack EMA+SRN. Risk class system 76 always emitted (097.003), fallback EU_CLASS_I. Risk class uses system 76 for MDR/IVDR (EU_CLASS_A/B/C/D for IVDR) and system 85 for IVDD/AIMDD (IVDD_GENERAL, IVDD_DEVICES_SELF_TESTING, IVDD_ANNEX_II_LIST_A/B, AIMDD) per 097.005. IsDeviceExemptFromImplantObligations for implantable+IIB (097.015). EMA contact always emitted with manufacturer SRN; fallback XX-MF-000000000 when no Basic UDI-DI data (097.009/097.026). EAR contact added for non-EU manufacturers (097.054); fallback SRN=XX-AR-000000000; EEA-only countries (IS, LI, NO) treated as non-EU per EUDAMED validation. IVDR devices get 7 mandatory boolean fields defaulting to false (097.046) and IsNewDevice (097.047). CMR/endocrine substances always include regulatedChemicalDescription with languageCode "en" (097.080/097.081). MDR/IVDR certificates from `deviceCertificateInfoListForDisplay` mapped to CertificationInformationModule (097.101). DEVICE_SIZE_TEXT_SPECIFY requires ClinicalSizeDescription with text value (097.070). Legacy devices (MDD/AIMDD/IVDD) detected via `legislation` field from Basic UDI-DI: strip globalModelNumber, directPartMarkingIdentifier, udidDeviceCount, uDIProductionIdentifierTypeCode, annexXVIIntendedPurposeTypeCode, CMR/endocrine substances (097.095); add MODEL_NUMBER as Basic UDI-DI reference (097.025); use classification system 85 not 76 (097.002); map MDD certificates (097.105); warn that publication is blocked (097.096). GS1 direct marking DI validated as 14 digits (097.118). SOFTWARE_IDENTIFICATION production identifier triggers `SpecialDeviceTypeCode: SOFTWARE` (097.091). MDR EU_CLASS_III without MDR_TECHNICAL_DOCUMENTATION or MDR_TYPE_EXAMINATION certificate emits warning (097.101). MANUFACTURER_PART_NUMBER always emitted in additionalTradeItemIdentification; falls back to primary DI code (097.006). Secondary DI type code derived from issuing agency instead of hardcoded GTIN_14 (097.087).

## EUDAMED Public API

- Base URL: `https://ec.europa.eu/tools/eudamed/api/devices/udiDiData`
- Listing: `GET ?page=N&pageSize=300&iso2Code=en&languageIso2Code=en` — paginated, basic device data
- Listing with SRN: `GET ?page=N&pageSize=300&srn=<SRN>&iso2Code=en&languageIso2Code=en` — server-side filter by manufacturer or AR SRN
- Detail: `GET /{uuid}?languageIso2Code=en` — full device data per UUID
- Basic UDI-DI: `GET /basicUdiData/udiDiData/{uuid}?languageIso2Code=en` — Basic UDI-DI record for a UDI-DI UUID (MDR booleans, multiComponent, riskClass, manufacturer/AR SRN)
- Detail lacks manufacturer SRN, authorised rep SRN, risk class, and MDR boolean fields → merged from Basic UDI-DI cache and/or listing data
- Serde deserialization structs use `#[allow(dead_code)]` since fields are needed for JSON parsing but not all read directly
