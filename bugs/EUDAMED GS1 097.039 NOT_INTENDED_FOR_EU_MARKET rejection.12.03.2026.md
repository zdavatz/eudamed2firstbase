# GS1 097.039: NOT_INTENDED_FOR_EU_MARKET rejected for MDR/IVDR devices

**Date:** 12.03.2026
**EUDAMED Rule:** BR-UDID-073 allows this status
**GS1 Rule(s) triggered:** 097.039 (216x)
**Affected items:** 111 base units + 105 package DIs = 216 rejections
**Note:** This is a GS1 validation rule issue, not an EUDAMED bug — but relevant for SANTE because it blocks CH-exclusive devices.

## Issue

GS1 GDSN validation rule 097.039 rejects MDR/IVDR devices with status `NOT_INTENDED_FOR_EU_MARKET`:

> 097.039: If regulatoryAct equals ('MDR' or 'IVDR') and eUMedicalDeviceStatusCode equals 'NOT_INTENDED_FOR_EU_MARKET', then eUMedicalDeviceStatusCode only [allows certain values]

This rule makes sense from an EU perspective — if a device is under MDR/IVDR, "not intended for EU market" seems contradictory. However, for **Switzerland** this is a critical use case: devices registered in EUDAMED under MDR but marketed exclusively in Switzerland (not EU) legitimately have this status.

## Affected manufacturers

| Manufacturer | SRN | Affected UDI-DIs |
|---|---|---|
| Ansell Healthcare Europe NV | BE-MF-000000691 | 65 |
| QA MED Solutions | DK-MF-000001649 | 45 |
| (Saudi manufacturer) | SA-MF-000047358 | 1 |

## CH relevance

These are exactly the **CH-exclusive** devices — products available in Switzerland but not placed on the EU market. They are registered in EUDAMED (as required by the MRA) but with status NOT_INTENDED_FOR_EU_MARKET. Swiss hospitals need these device records in firstbase.

## Relationship to BR-UDID-073 bug

Compounded by EUDAMED BR-UDID-073 status propagation bug: 110 of 111 devices have Package DIs with inconsistent `ON_MARKET` status while the Base Unit is `NOT_INTENDED`. See separate bug report.

## Action needed

1. **GS1:** Request relaxation of 097.039 for Swiss firstbase (CH is not EU — NOT_INTENDED_FOR_EU_MARKET is valid for CH market)
2. **EUDAMED/SANTE:** Fix BR-UDID-073 status propagation so Container Packages correctly inherit NOT_INTENDED status
