# eudamed2firstbase

Rust CLI tool that converts EUDAMED medical device data into GS1 firstbase JSON format. Supports five input modes: DTX PullResponse XML, EUDAMED public API listing, EUDAMED public API detail (with listing merge), EUDAMED JSON (individual device files), and XLSX export.

## Quick Start: Download & Convert from EUDAMED API

```bash
./download.sh --10                            # download and convert first 10 products
./download.sh --100                           # download and convert first 100 products
./download.sh --srn IN-MF-000014457           # all products for a manufacturer SRN
./download.sh --srn IN-MF-000014457 --50      # first 50 products for a specific SRN
./download.sh --srn DE-AR-000006322           # all products for an authorised rep SRN
./download.sh --srn SRN1 SRN2 SRN3            # multiple SRNs combined into one file
./download.sh --srn SRN1 SRN2 --50            # multiple SRNs, limit 50 per SRN
```

The download script handles the full pipeline: listing download (with optional SRN filtering), UUID extraction, parallel detail download (with resume support), Basic UDI-DI download (for MDR mandatory fields), and firstbase JSON conversion.

The `--srn` option uses server-side filtering via the API's `srn=` parameter, which matches both manufacturer and authorised representative SRNs. Multiple SRNs can be specified after `--srn` and their results are combined into a single output file. Output is named `eudamed_<first-SRN>_+<N>srns.ndjson` for multi-SRN runs.

## Manual Usage

### Mode 1: XML (DTX PullResponse)

1. Place EUDAMED XML files in the `xml/` directory
2. Run: `cargo run`
3. Output: `firstbase_json/firstbase_dd.mm.yyyy.json`

### Mode 2: API Listing (NDJSON)

1. Place listing NDJSON files in the `ndjson/` directory
2. Run: `cargo run ndjson` or `cargo run ndjson <directory>`
3. Output: `firstbase_json/firstbase_eudamed_*_dd.mm.yyyy.json`

### Mode 3: API Detail (NDJSON with listing merge)

1. Run: `cargo run detail <details.ndjson> [listing.ndjson]`
2. The optional listing file provides manufacturer SRN, authorised rep SRN, and risk class
3. Output: batch file `firstbase_json/firstbase_eudamed_*_details_dd.mm.yyyy.json` plus individual `firstbase_json/<uuid>.json` per device

### Mode 5: XLSX Export

1. Run: `cargo run xlsx <details.ndjson>`
2. Output: `xlsx/<input_stem>.xlsx`
3. Flattens detail NDJSON into a spreadsheet with columns: UUID, Primary DI, Issuing Agency, Trade Name, Reference, Device Status, Sterile, Single Use, Latex, Reprocessed, Base Quantity, Direct Marking, Clinical Sizes, Markets, Additional Info URL, Version Date

### Mode 4: EUDAMED JSON (individual device files)

1. Place EUDAMED JSON files in the `eudamed_json/` directory
2. Run: `cargo run eudamed_json` or `cargo run eudamed_json <directory>`
3. Output: one firstbase JSON file per input file in `firstbase_json/`
4. Auto-detects file type:
   - **UDI-DI level** (has `primaryDi`): full conversion with GTIN, trade name, clinical sizes, market info (ORIGINAL_PLACED/ADDITIONAL split), storage, warnings, substances (CMR/endocrine/medicinal → ChemicalRegulationModule), product designer (EPD contact with address/email/phone), secondary DI, direct marking, unit of use, related devices (REPLACED/REPLACED_BY), regulatory module (MDR/IVDR+EU). Merges Basic UDI-DI data from cache for MDR mandatory fields (active, implantable, measuringFunction, multiComponent, tissue, manufacturer/AR SRN, risk class). On cache miss, fetches Basic UDI-DI on demand from EUDAMED API.
   - **Device level** (Basic UDI-DI, no `primaryDi`): manufacturer/AR contact info, risk class, device flags — no GTIN

## Configuration

`config.toml` provides values not available in the EUDAMED XML:

```toml
[provider]
gln = "7612345000480"
party_name = "EUDAMED Public Download Importing"

[target_market]
country_code = "097"

[gpc]
segment_code = "51000000"
class_code = "51150100"
family_code = "51150000"
category_code = "10005844"
category_name = "Medical Devices"

[endocrine_substances.Estradiol]
ec_number = "200-023-8"
cas_number = "50-28-2"
```

## Project Structure

```
src/
  main.rs                    # CLI entry point: routing for xml/ndjson/detail/eudamed_json modes
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

download.sh                # Unified download + convert script (listing + detail + Basic UDI-DI + convert)
download_10k.sh            # Legacy: download 10k listings
download_details.sh        # Legacy: download details from UUID list
firstbase_validation.py    # Schema validation against GS1 Product API Swagger spec
push_to_api.sh             # Push firstbase JSON to GS1 Catalogue Item API (Draft/CreateOne + AddMany publish)
log/                       # API push logs (log_dd.mm.yyyy.log)
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

## EUDAMED Public API

The download script uses the EUDAMED public API at `https://ec.europa.eu/tools/eudamed/api/devices/udiDiData`:

- **Listing endpoint**: `GET ?page=N&pageSize=300` — basic device info (GTIN, risk class, manufacturer)
- **Listing with SRN filter**: `GET ?page=N&pageSize=300&srn=<SRN>` — server-side filtering by manufacturer or authorised rep SRN
- **Detail endpoint**: `GET /{uuid}?languageIso2Code=en` — full device data (clinical sizes, substances, market info, warnings)

- **Basic UDI-DI endpoint**: `GET /basicUdiData/udiDiData/{uuid}?languageIso2Code=en` — Basic UDI-DI record for a UDI-DI UUID

The detail endpoint provides richer data but lacks manufacturer/AR SRN, risk class, and MDR mandatory boolean fields (active, implantable, measuringFunction, multiComponent, tissue). These are merged from the Basic UDI-DI cache (`/tmp/basic_udi_cache/`) and/or listing data.

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

After creating drafts, publish them to the firstbase UDI Connector (GLN `4399902421386`):

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
      "PublishToGln": ["4399902421386"]
    }]
  }'
```

You can publish multiple items in a single request by adding more objects to the `Items` array. The response returns a `RequestIdentifier` on success.

#### 4. Bulk Workflow: push_to_api.sh

The `push_to_api.sh` script handles the full workflow: token acquisition, draft creation via `Draft/CreateOne` (per file), and publish via `AddMany` (batches of 100). Includes automatic throttling (1s for ≤60 files, 8s for larger batches) and HTTP 429 retry with `retry-after` backoff.

```bash
./push_to_api.sh                    # push all UUID files in firstbase_json/
./push_to_api.sh --dir /path/to/dir # push files from a custom directory
./push_to_api.sh --dry-run          # show what would be pushed, no API calls
./push_to_api.sh --status <reqid>   # query status of a previous request
```

Environment variables for credentials:

```bash
export FIRSTBASE_EMAIL="you@example.com"
export FIRSTBASE_PASSWORD="your-api-password"
export FIRSTBASE_GLN="7612345000480"
./push_to_api.sh
```

The script creates each draft individually via `Draft/CreateOne`, then publishes all successful drafts to GLN `4399902421386` via `AddMany` in batches of 100. Files without a valid GS1 GTIN (HIBC/IFA devices) will fail at draft creation — this is expected.

**Note:** The `Live/CreateMany` endpoint currently returns HTTP 500 on the test server, so the script uses the `Draft/CreateOne` + `AddMany` workflow instead.

#### Validation Error Fixes Applied

After initial submission of 100 devices (1341 errors, 15 patterns), the following fixes were applied:

| Error | Count | Fix |
|---|---|---|
| G572 lastChangeDateTime in future | 88x | Use `version_date` from EUDAMED instead of `now()` |
| G641 device self-replacement | 10x | Skip referenced trade items where linked DI = own DI |
| 097.011 missing MDR boolean fields | 648x | Use real values from Basic UDI-DI cache; fall back to false |
| 097.010 missing multiComponent/tissue | 264x | Use real multiComponent from Basic UDI-DI; fall back to `DEVICE` |
| 097.025 missing globalModelNumber | 176x | Use primary DI code as fallback, trade name as description |
| 097.025 missing globalModelDescription en | — | Treat `allLanguagesApplicable` as English; fallback to first trade name or Basic UDI-DI device name |
| 097.013 missing uDIProductionIdentifierTypeCode | — | Default to `BATCH_NUMBER` when EUDAMED has no production identifiers (MDR/IVDR mandatory) |
| G541 invalid country code 826 (UK/NI) | — | Skip GB/XI from market sales conditions post-Brexit; XI will become valid with GDSN March/May 2026 release |
| 097.072 missing additionalDescription | 60x | Resolved by defaulting multiComponentDeviceTypeCode to DEVICE |
| 097.020 ON_MARKET needs ORIGINAL_PLACED | 25x | Use `placedOnTheMarket` country when `marketInfoLink` is null; enforce exactly one ORIGINAL_PLACED country |
| 097.074 storage description missing | 9x | Use SHC code as placeholder description |
| 097.005 invalid risk class | 5x | Set MDR vs IVDR regulatory act based on risk class |
| 097.022 Class I implantable conflict | 36x | Data quality issue in EUDAMED (not fixable) |
| 097.009 EMA contact with SRN required | 16x | Already generated from Basic UDI-DI cache (99.2% coverage); remaining files lack cache entries |
| 097.003 missing risk class system 76 | — | Always emit classification system 76; fallback to EU_CLASS_I |
| 097.005 invalid IVD risk class codes | — | Fix IVD mapping: CLASS_A/B/C/D → IVD_CLASS_A/B/C/D (not EU_CLASS_*) |
| 097.015 implantable IIB exempt field | — | Add `IsDeviceExemptFromImplantObligations` (default false) for implantable + EU_CLASS_IIB |
| 097.026 invalid Actor contactTypeCode | — | Remove EAR for EU manufacturers; only EMA/EPP allowed as Actor types |
| 097.054 non-EU needs EAR contact | — | Add EAR contact for non-EU manufacturers (detected from SRN prefix) |
| 097.046 IVDR boolean fields missing | — | Add 7 IVDR fields (reagent, instrument, self-testing, etc.) default false |
| 097.047 IVDR isNewDevice missing | — | Default `IsNewDevice` to false for IVDR devices |
| 097.080 CMR/endocrine missing description | — | Always include `regulatedChemicalDescription` with `languageCode: "en"` for CMR/endocrine substances |
| 097.081 endocrine missing description | — | ENDOCRINE_SUBSTANCE always gets description even when CAS/EC identifiers present |
| G541 DIRECTION_OF_VIEW | 1x | CST63 coming with GDSN May release |

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

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).
