# CLAUDE.md

## Project Overview

EUDAMED DTX XML to GS1 firstbase JSON converter. Reads EUDAMED PullResponse XML from `xml/`, writes firstbase JSON to `json/`.

## Build & Run

```bash
cargo build
cargo run
```

No tests yet. Validate output by diffing `json/firstbase_27.02.2026.json` against `maik/CIN_7612345000435_07612345780313_097.json`.

## Architecture

- **eudamed.rs**: XML parsing using `roxmltree` DOM traversal (not serde). Switched from `quick-xml` serde due to element ordering issues. Uses `local_name()` to handle namespace prefixes transparently.
- **firstbase.rs**: Output JSON model with serde `Serialize`. Uses `#[serde(rename = ...)]` for GS1 field names and `skip_serializing_if` for optional fields.
- **transform.rs**: Core conversion logic. Builds packaging hierarchy by walking parent-child DI references. Sorts languages (en, fr, de, it), substances (WHO before ECHA), market infos (ORIGINAL_PLACED first).
- **mappings.rs**: All code translation tables as match statements. Derived from the UDID_CodeLists sheet of the GS1 UDI Connector Profile spreadsheet.
- **config.rs**: Loads `config.toml` for provider GLN, GPC codes, target market, sterilisation method, and endocrine substance identifier lookups.

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

- TradeItemSynchronisationDates: empty (meta-dates not in EUDAMED XML)
- DirectPartMarkingIdentifier: not generated (value not derivable from XML)
- Language ordering within multi-language arrays may differ from reference (reference is inconsistent)
