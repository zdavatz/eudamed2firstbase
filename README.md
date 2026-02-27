# eudamed2firstbase

Rust CLI tool that converts EUDAMED DTX PullResponse XML files into GS1 firstbase JSON format.

## Usage

1. Place EUDAMED XML files in the `xml/` directory
2. Configure `config.toml` with provider info, GPC codes, and sterilisation settings
3. Run the converter:

```bash
cargo run
```

Output is written to `json/firstbase_dd.mm.yyyy.json` (today's date).

## Configuration

`config.toml` provides values not available in the EUDAMED XML:

```toml
sterilisation_method = "OZONE"

[provider]
gln = "7612345000435"
party_name = "UDI manufacturer POC/MVP"

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
  main.rs        # CLI entry point: reads xml/, writes json/
  config.rs      # config.toml parsing
  eudamed.rs     # EUDAMED XML parsing (roxmltree DOM)
  firstbase.rs   # GS1 firstbase JSON output model (serde)
  transform.rs   # EUDAMED -> firstbase conversion logic
  mappings.rs    # Code mapping tables (country, risk class, clinical sizes, units, etc.)
```

## What it does

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
- Maps substances to ChemicalRegulationInformation (WHO for medicinal/human, ECHA for CMR/endocrine)
- Extracts contact information (manufacturer, authorised representative, product designer)
- Generates market info with country-specific sales conditions

## Dependencies

- `roxmltree` - XML DOM parsing with namespace support
- `serde` / `serde_json` - JSON serialization
- `chrono` - date handling
- `anyhow` - error handling
- `toml` - config file parsing
- `regex` - text processing
