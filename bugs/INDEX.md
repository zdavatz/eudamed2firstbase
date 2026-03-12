# EUDAMED Data Inconsistency Reports

Reports for SANTE ticket submission. Each documents an EUDAMED data quality issue or business rule violation that causes GS1 firstbase import failures.

## Reports

| # | Date | Category | Title | GS1 Errors | Items | SANTE Ticket |
|---|---|---|---|---|---|---|
| 1 | 12.03.2026 | BR-UDID-073 | [Status propagation to container packages](EUDAMED%20BR-UDID-073%20Bug%20Report.12.03.2026.md) | 097.039 (216x), 097.040 (40x), 910.004 (40x) | 296 | — |
| 2 | 12.03.2026 | Data Quality | [ON_MARKET without country information](EUDAMED%20Data%20Quality%20ON_MARKET%20without%20countries.12.03.2026.md) | 097.020, 097.010, 097.011, G541 (92 total) | 18 | — |
| 3 | 12.03.2026 | Data Quality | [Null MDR mandatory boolean fields](EUDAMED%20Data%20Quality%20null%20MDR%20booleans.12.03.2026.md) | 097.010, 097.011 | ~2% of MDR devices | — |
| 4 | 12.03.2026 | Data Quality | [MDR Class III missing certificate](EUDAMED%20Data%20Quality%20MDR%20Class%20III%20missing%20certificate.12.03.2026.md) | 097.101 (6x) | 3+ | — |
| 5 | 12.03.2026 | GS1 Rule | [NOT_INTENDED_FOR_EU_MARKET rejected for CH-exclusive devices](EUDAMED%20GS1%20097.039%20NOT_INTENDED_FOR_EU_MARKET%20rejection.12.03.2026.md) | 097.039 (216x) | 111 | — |

## Error summary (from log_11.03.2026.log)

| GS1 Error | Count | Root Cause | Report # |
|---|---|---|---|
| 097.039 | 216 | BR-UDID-073 + GS1 rule too strict for CH | 1, 5 |
| 097.040 | 40 | BR-UDID-073 status not propagated | 1 |
| 910.004 | 40 | Follow-on: children can't be discontinued | 1 |
| 097.020 | 18 | ON_MARKET with no countries | 2 |
| 097.010 | 18 | Missing MDR mandatory fields | 2, 3 |
| 097.011 | 18 | Missing MDR mandatory fields | 2, 3 |
| G541 | 18 | Invalid code values (empty defaults) | 2 |
| 097.054 | 10 | Missing AR SRN for non-EU manufacturer | (investigated, mostly correct) |
| 097.108 | 6 | MDD + wrong IVDD risk class | 2 (cascade) |
| 097.101 | 6 | MDR Class III without certificate | 4 |
| SYS25 | 2 | Timestamp conflict on re-upload | (operational, not EUDAMED bug) |
| **Total** | **392** | | |

## Error summary (from re-push 12.03.2026, 2485 items)

| GS1 Error | Count | Change | Root Cause | Report # |
|---|---|---|---|---|
| 097.021 | ~216 | new | NOT_INTENDED + country data (BR-UDID-073 cascade) | 1 |
| 097.039 | ~108 | -108 | NOT_INTENDED + MDR/IVDR (CH-exclusive) | 1, 5 |
| 097.040 | 4 | -36 | NO_LONGER status propagation | 1 |
| 910.004 | 4 | -36 | Children can't be discontinued | 1 |
| 097.020 | 0 | **-18** | **Fixed by fallback workaround** | 2 |
| 097.010 | 4 | -14 | Null MDR mandatory fields | 2, 3 |
| 097.011 | 4 | -14 | Null MDR mandatory fields | 2, 3 |
| G541 | 4 | -14 | Invalid code values | 2 |
| 097.054 | 2 | -8 | Missing AR SRN | — |
| 097.101 | 0 | **-6** | **Potentially resolved** | 4 |
| G485 | 2 | new | discontinuedDateTime on re-upload | (operational) |
| **Total** | **~348** | **-44** | | |
