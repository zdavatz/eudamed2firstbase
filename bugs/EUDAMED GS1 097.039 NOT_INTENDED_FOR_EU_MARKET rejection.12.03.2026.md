# GS1 097.039: NOT_INTENDED_FOR_EU_MARKET rejected for MDR/IVDR devices

**Date:** 12.03.2026
**EUDAMED Rule:** BR-UDID-073 allows this status
**GS1 Rule(s) triggered:** 097.039 (216x)
**Affected items:** 111 base units + 105 package DIs = 216 rejections
**Note:** This is a GS1 validation rule issue, not an EUDAMED bug — but relevant for SANTE because it blocks non-EU market devices (CH, UK, etc.).

## Issue

GS1 GDSN validation rule 097.039 rejects MDR/IVDR devices with status `NOT_INTENDED_FOR_EU_MARKET`:

> 097.039: If regulatoryAct equals ('MDR' or 'IVDR') and eUMedicalDeviceStatusCode equals 'NOT_INTENDED_FOR_EU_MARKET', then eUMedicalDeviceStatusCode only [allows certain values]

This rule makes sense from an EU perspective — if a device is under MDR/IVDR, "not intended for EU market" seems contradictory. However, for **non-EU markets** this is a legitimate use case: devices registered in EUDAMED under MDR but marketed outside the EU (e.g. Switzerland, UK, or other non-EU/EEA countries) legitimately have this status. EUDAMED only tracks EU/EEA markets — `NOT_INTENDED_FOR_EU_MARKET` does not indicate which non-EU market the device is intended for.

## Affected manufacturers

| Manufacturer | SRN | Affected UDI-DIs |
|---|---|---|
| Ansell Healthcare Europe NV | BE-MF-000000691 | 65 |
| QA MED Solutions | DK-MF-000001649 | 45 |
| (Saudi manufacturer) | SA-MF-000047358 | 1 |

## Non-EU market relevance

These are devices marketed outside the EU — potentially in Switzerland (CH), UK, or other non-EU/EEA countries. EUDAMED does not specify which non-EU market a device is intended for. They are registered in EUDAMED (e.g. as required by the CH-EU MRA for Swiss devices) but with status NOT_INTENDED_FOR_EU_MARKET. Swiss hospitals may need these device records in firstbase if the device is available in CH.

## Relationship to BR-UDID-073 bug

Compounded by EUDAMED BR-UDID-073 status propagation bug: 110 of 111 devices have Package DIs with inconsistent `ON_MARKET` status while the Base Unit is `NOT_INTENDED`. See separate bug report.

## Action needed

1. **GS1:** Request relaxation of 097.039 for Swiss firstbase (non-EU markets like CH need these devices — NOT_INTENDED_FOR_EU_MARKET is valid for devices marketed outside EU/EEA)
2. **EUDAMED/SANTE:** Fix BR-UDID-073 status propagation so Container Packages correctly inherit NOT_INTENDED status
