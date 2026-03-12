# EUDAMED BR-UDID-073 Bug Report

**Date:** 12.03.2026

**Rule:** BR-UDID-073 — Container Package Status Propagation

**Reference:** [EUDAMED UDI Registration Process](https://health.ec.europa.eu/document/download/c3231845-228e-437a-8d77-510ecc3a548b_de?filename=md_eudamed-udi-registration-process_en.pdf)

## Expected behavior (per BR-UDID-073 Rule 1)

> "If the status of the registered device is 'Not intended for the EU market' then its
> container packages status will be set to 'Not intended for the EU market' including all
> the children of the root element."

## Actual behavior

Container packages retain status `ON_THE_MARKET` while the registered device (Base Unit / UDI-DI) has status `NOT_INTENDED_FOR_EU_MARKET`.

## Evidence

110 of 111 affected UDI-DIs show inconsistent status propagation:

| Level | DI | Status |
|---|---|---|
| CASE (outermost package) | 05744001291047 | `on-the-market` |
| PACK_OR_INNER_PACK | 05744001291030 | `on-the-market` |
| BASE_UNIT_OR_EACH (UDI-DI) | 05744001291023 | `not-intended-for-eu-market` |

## Affected manufacturers

| Manufacturer | SRN | Affected UDI-DIs |
|---|---|---|
| Ansell Healthcare Europe NV | BE-MF-000000691 | 65 |
| QA MED Solutions | DK-MF-000001649 | 45 |
| (Saudi manufacturer) | SA-MF-000047358 | 1 |

## Impact

GS1 GDSN validation rule 097.039 rejects `NOT_INTENDED_FOR_EU_MARKET` for MDR/IVDR devices. Because EUDAMED does not propagate the status to container packages per BR-UDID-073, the packaging hierarchy contains inconsistent statuses — the base unit is rejected while its parent packages pass validation. This causes 216 item-level rejections when synchronising EUDAMED data via GDSN.

## Additional finding: BR-UDID-073 Rule 2 (NO_LONGER_PLACED_ON_THE_MARKET)

Same pattern observed for `NO_LONGER_PLACED_ON_THE_MARKET`: 23 base units with this status have container packages still set to `ON_THE_MARKET`, causing 40 rejections via GS1 rule 097.040 + 40 via 910.004 ("child item cannot be discontinued").

## Additional finding: 097.020 (ON_MARKET without countries)

7 UDI-DIs have status `ON_THE_MARKET` but both `marketInfoLink` and `placedOnTheMarket` are null — no country information at all. This violates the expectation that ON_MARKET devices have at least one market country.

## Sample UUIDs for verification

| UUID | Base Unit Status | Package Status |
|---|---|---|
| 0305c47a-a39c-4de2-a7ed-3fccff3ebd04 | NOT_INTENDED_FOR_EU_MARKET | ON_THE_MARKET |
| 186e7274-9ea1-4494-941d-8dbc0b665146 | ON_MARKET, no countries | — |

## EUDAMED API endpoints used

- Detail: `GET https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/{uuid}?languageIso2Code=en`
- Basic UDI-DI: `GET https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/{uuid}?languageIso2Code=en`
