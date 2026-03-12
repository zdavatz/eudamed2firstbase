# EUDAMED Data Quality: ON_MARKET without country information

**Date:** 12.03.2026
**EUDAMED Rule:** No specific rule — data quality issue
**GS1 Rule(s) triggered:** 097.020 (18x), 097.010 (18x), 097.011 (18x), G541 (18x), 097.054 (8x), 097.101 (6x), 097.108 (2x), SYS25 (2x)
**Affected items:** 7 base units + 11 package DIs = 18 total rejections (92 item-level errors)
**SANTE Ticket:** (to be filed)

## Issue

7 UDI-DI records have status `on-the-market` but both `marketInfoLink` and `placedOnTheMarket` are null — no country information at all. Additionally, their Basic UDI-DI records have null values for MDR mandatory boolean fields (`active`, `implantable`, `measuringFunction`).

## Expected vs Actual

| | Expected | Actual |
|---|---|---|
| marketInfoLink | At least one country for ON_MARKET | null |
| placedOnTheMarket | Fallback country | null |
| active (Basic UDI-DI) | true/false (MDR mandatory) | null |
| implantable (Basic UDI-DI) | true/false (MDR mandatory) | null |
| measuringFunction (Basic UDI-DI) | true/false (MDR mandatory) | null |

## Cascade effect

Because these devices lack both market countries AND MDR mandatory fields, a single device triggers **5 different GS1 validation errors** simultaneously:
- 097.020: ON_MARKET requires at least one country
- 097.010: multiComponentDeviceTypeCode required (falls back incorrectly without BUDI data)
- 097.011: MDR boolean fields required (active, implantable, measuringFunction all null)
- G541: Invalid code list value (empty regulatory act / risk class defaults)
- 097.054/097.101/097.108: Follow-on errors from missing regulatory context

## Affected devices

| UUID | GTIN | Manufacturer | SRN | Risk Class |
|---|---|---|---|---|
| 186e7274-9ea1-4494-941d-8dbc0b665146 | 03661379254684 | HEMODIA SAS | FR-PR-000007189 | class-iia |
| 186f94db-1b1e-4f20-a1a6-c586ef4ccc5e | 08710685098101 | Medica Europe B.V. | NL-PR-000000117 | class-iii |
| 18a660f4-e90f-4324-9c84-0ac8ebb9cb21 | 00850025688055 | Takeda Pharmaceuticals U.S.A., Inc. | US-PR-000007896 | class-iia |
| 18ab191a-215f-48c6-b8b0-6d0d8bfc2967 | 06974686055231 | Yiwu Ori-Power Medtech Co., Ltd. | CN-PR-000039035 | class-i |
| 18b136a1-665b-4010-b1ef-d91bde24c1e9 | 06951454742159 | GAUKE Healthcare Co., Ltd | CN-PR-000038433 | class-i |
| 18bd7774-639c-4bf3-b329-340891273dab | 08710685134380 | Medica Europe B.V. | NL-PR-000000117 | class-iib |
| 18bdd0fd-b20c-4e6c-a65e-cfc96f3d7cca | 07045430043831 | Laerdal Medical AS | NO-PR-000002650 | class-i |

## Broader issue: null MDR booleans

In a sample of 3,504 MDR Basic UDI-DI records, 69 (2.0%) have null values for `active`, `implantable`, and `measuringFunction` — fields that are mandatory per MDR. These are not limited to the 7 devices above.

## Sample API URLs for verification

- `https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/186e7274-9ea1-4494-941d-8dbc0b665146?languageIso2Code=en`
- `https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/186e7274-9ea1-4494-941d-8dbc0b665146?languageIso2Code=en`

## Impact on Swiss firstbase

These 7 devices plus their 11 package DIs are rejected entirely from GS1 firstbase. The missing data prevents any workaround on our side — we cannot infer market countries or MDR boolean values.

## Workaround

None possible. Market country and MDR booleans must be provided by EUDAMED.
