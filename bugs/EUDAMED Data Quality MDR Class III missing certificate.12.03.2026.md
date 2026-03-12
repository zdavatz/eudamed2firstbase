# EUDAMED Data Quality: MDR Class III devices without required certificates

**Date:** 12.03.2026
**EUDAMED Rule:** MDR Art. 29(3) — Notified Body confirmation required for high-risk devices
**GS1 Rule(s) triggered:** 097.101 (6x)
**Affected items:** At least 3 base units (6 errors including package DIs)
**SANTE Ticket:** (to be filed)

## Issue

MDR Class III devices are publicly available in EUDAMED without the required `MDR_TECHNICAL_DOCUMENTATION` or `MDR_TYPE_EXAMINATION` certificate in their Basic UDI-DI `deviceCertificateInfoListForDisplay`.

Per MDR Art. 29(3) and the EUDAMED UDI Registration Process, Class III devices require Notified Body confirmation (CertificateLink) before becoming publicly visible. The presence of these devices in the public API suggests either:
1. The certificate enforcement was not applied during migration
2. The certificates exist but are not exposed via the public API

## Expected vs Actual

| | Expected | Actual |
|---|---|---|
| MDR Class III certificate | MDR_TECHNICAL_DOCUMENTATION or MDR_TYPE_EXAMINATION present | No certificate or only QMS certificates |

## GS1 validation rule

> 097.101: For regulatoryAct equals 'MDR' and risk class equals 'EU_CLASS_III', then certificateStandard equals ('MDR_TECHNICAL_DOCUMENTATION' or 'MDR_TYPE_EXAMINATION') must be used.

## Impact on Swiss firstbase

These devices are rejected by GS1 097.101 validation because the certificate data is missing. Swiss hospitals cannot receive these Class III device records via GDSN.

## Workaround

None. Certificate data must come from EUDAMED. We cannot fabricate certificate numbers.
