# CLAUDE.md

## Project Overview

EUDAMED to GS1 firstbase JSON converter. Four input modes: DTX PullResponse XML, EUDAMED public API listing (NDJSON), API detail (NDJSON with listing merge), and EUDAMED JSON (individual device files).

## Build & Run

```bash
cargo build
cargo run                                            # XML mode: xml/ -> firstbase_json/
cargo run ndjson                                     # API listing mode: ndjson/ -> firstbase_json/
cargo run detail <details.ndjson> [listing.ndjson]   # API detail mode with optional listing merge
cargo run eudamed_json                               # EUDAMED JSON mode: eudamed_json/ -> firstbase_json/ (1:1)
./download.sh --10                                   # Download + convert 10 products from EUDAMED API
```

No tests yet. Validate output by diffing `firstbase_json/firstbase_28.02.2026.json` against `maik/CIN_7612345000435_07612345780313_097.json`.

## Architecture

- **eudamed.rs**: XML parsing using `roxmltree` DOM traversal (not serde). Switched from `quick-xml` serde due to element ordering issues. Uses `local_name()` to handle namespace prefixes transparently.
- **api_json.rs**: EUDAMED public API listing NDJSON parsing (serde). Flat `ApiDevice` struct.
- **api_detail.rs**: EUDAMED public API detail NDJSON parsing (serde). Rich `ApiDeviceDetail` struct with clinical sizes, substances, market info, warnings.
- **firstbase.rs**: Output JSON model with serde `Serialize`. Uses `#[serde(rename = ...)]` for GS1 field names and `skip_serializing_if` for optional fields.
- **transform.rs**: XML -> firstbase conversion. Builds packaging hierarchy by walking parent-child DI references. Sorts languages (en, fr, de, it), substances (WHO before ECHA), market infos (ORIGINAL_PLACED first).
- **transform_api.rs**: API listing -> firstbase conversion. Simpler mapping from flat listing data.
- **transform_detail.rs**: API detail -> firstbase conversion. Richest output with clinical data, market info, IFU URLs. Can merge listing data for manufacturer/AR SRN and risk class.
- **eudamed_json.rs**: EUDAMED JSON device-level file parsing (serde). `EudamedDevice` struct with inline manufacturer/AR objects, basicUdi, riskClass, device flags.
- **transform_eudamed_json.rs**: EUDAMED JSON device-level -> firstbase conversion. Includes full manufacturer/AR contact info with addresses, email, phone. No GTIN (device-level records).
- The `eudamed_json` mode auto-detects file type: UDI-DI level files (have `primaryDi` with GTIN, trade name, clinical data) use `api_detail`/`transform_detail`; device-level files (Basic UDI-DI) use `eudamed_json`/`transform_eudamed_json`.
- **mappings.rs**: All code translation tables as match statements. Derived from the UDID_CodeLists sheet of the GS1 UDI Connector Profile spreadsheet.
- **config.rs**: Loads `config.toml` for provider GLN, GPC codes, target market, sterilisation method, and endocrine substance identifier lookups.
- **download.sh**: Unified download + convert script. Usage: `./download.sh --N` or `./download.sh --srn <SRN> [--N]`. Downloads listing (with optional server-side SRN filtering via API `srn=` parameter), extracts UUIDs, fetches details in parallel (10 concurrent, with retry and resume), converts to firstbase JSON.

## Key Design Decisions

- `roxmltree` over `quick-xml` serde: EUDAMED XML has 30+ namespace prefixes and strict element ordering that broke serde deserialization.
- Flat domain structs with `Option<bool>` / `Option<String>` / `Vec<T>` instead of wrapper types.
- Packaging hierarchy reconstructed from flat package list by finding outermost package (not referenced as any child) and walking down.
- Endocrine substance EC/CAS identifiers come from config.toml lookup table since EUDAMED XML doesn't provide them.
- Sterilisation method is config-driven (EUDAMED only has boolean sterilization flag).

## Reference Files (in maik/)

- `EUDAMED_APP-DTX-000084634.xml` - Input reference
- `CIN_7612345000435_07612345780313_097.json` - Output reference
- `GS1_UDI_Connector_Profile_Overview_2026_02-27_zdavatz.xlsx` - Authoritative mapping specification

## Known Gaps vs Reference

- TradeItemSynchronisationDates: empty (meta-dates not in EUDAMED XML or API)
- DirectPartMarkingIdentifier: not generated (value not derivable from XML; API field `deviceMarking` is empty)
- Language ordering within multi-language arrays may differ from reference (reference is inconsistent)
- Sales conditions country ordering for ADDITIONAL markets may differ from reference (reference uses neither numeric nor XML order)
- CatalogueItem Identifier: generated as random v4 UUIDs (won't match reference's specific UUIDs)

## EUDAMED Public API

- Base URL: `https://ec.europa.eu/tools/eudamed/api/devices/udiDiData`
- Listing: `GET ?page=N&pageSize=300&iso2Code=en&languageIso2Code=en` — paginated, basic device data
- Listing with SRN: `GET ?page=N&pageSize=300&srn=<SRN>&iso2Code=en&languageIso2Code=en` — server-side filter by manufacturer or AR SRN
- Detail: `GET /{uuid}?languageIso2Code=en` — full device data per UUID
- Detail lacks manufacturer SRN, authorised rep SRN, and risk class → merged from listing data
- Serde deserialization structs use `#[allow(dead_code)]` since fields are needed for JSON parsing but not all read directly
