# EUDAMED Data Quality: Null MDR mandatory boolean fields in Basic UDI-DI

**Date:** 12.03.2026
**EUDAMED Rule:** MDR Art. 29, Annex VI Part A Section 6 — mandatory device characteristics
**GS1 Rule(s) triggered:** 097.010, 097.011
**Affected items:** ~2% of MDR Basic UDI-DI records (69 of 3,504 in sample)
**SANTE Ticket:** (to be filed)

## Issue

Basic UDI-DI records for MDR devices have null values for mandatory boolean fields: `active`, `implantable`, `measuringFunction`. These fields are required by MDR Annex VI Part A Section 6 for all device registrations.

## Expected vs Actual

| Field | Expected (MDR mandatory) | Actual |
|---|---|---|
| active | true / false | null |
| implantable | true / false | null |
| measuringFunction | true / false | null |

All three fields are consistently null together — suggesting these records were incompletely migrated or registered before mandatory field enforcement.

## Scale

| Sample | Total MDR devices | Null booleans | Percentage |
|---|---|---|---|
| First 5,000 BUDI cache files | 3,504 | 69 | 2.0% |

Extrapolating to the full EUDAMED database (~1.3M records), this could affect ~26,000 devices.

## Sample UUIDs

| UUID | API URL |
|---|---|
| 0000764a-fd6d-413c-8ede-e9964b1a5c87 | `https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/0000764a-fd6d-413c-8ede-e9964b1a5c87?languageIso2Code=en` |
| 000e147b-a260-4a09-ab95-0243d7bfa6b1 | `https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/000e147b-a260-4a09-ab95-0243d7bfa6b1?languageIso2Code=en` |
| 00110ac3-61e5-46e4-b346-1d8a0f4070f8 | `https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/00110ac3-61e5-46e4-b346-1d8a0f4070f8?languageIso2Code=en` |
| 186e7274-9ea1-4494-941d-8dbc0b665146 | `https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/186e7274-9ea1-4494-941d-8dbc0b665146?languageIso2Code=en` |
| 186f94db-1b1e-4f20-a1a6-c586ef4ccc5e | `https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/186f94db-1b1e-4f20-a1a6-c586ef4ccc5e?languageIso2Code=en` |

## Impact on Swiss firstbase

Our converter defaults null booleans to `false`, which allows most devices to pass GS1 validation. However, this is semantically incorrect — a null `implantable` is not the same as `implantable=false`. If a device is actually implantable but recorded as null in EUDAMED, Swiss hospitals receiving the data would not see the implantable flag.

## Workaround

Current: default null booleans to `false` (permits import but may be inaccurate).
Correct: EUDAMED should enforce non-null values for MDR mandatory fields.
