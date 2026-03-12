# EUDAMED Data Inconsistency Report Template

## Format

Each report follows this structure:

```markdown
# EUDAMED [BR-UDID-xxx / Data Quality]: [Short Title]

**Date:** dd.mm.yyyy
**EUDAMED Rule:** BR-UDID-xxx (or "No rule — data quality")
**GS1 Rule(s) triggered:** 097.xxx, G5xx, etc.
**Affected items:** N base units + M package DIs = T total rejections
**SANTE Ticket:** (to be filed)

## Rule Text
> Exact quote from EUDAMED UDI Registration Process PDF or EUDAMED validation rules.

## Expected vs Actual

| | Expected | Actual |
|---|---|---|
| ... | ... | ... |

## Affected Manufacturers
| Manufacturer | SRN | Count |
|---|---|---|

## Sample UUIDs
| UUID | Issue | API URL |
|---|---|---|
| xxx | description | `https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/xxx?languageIso2Code=en` |

## Impact on Swiss firstbase
Why this matters for CH (e.g. CH-exclusive devices, missing from database, etc.)

## Workaround (if any)
What we could do on our side to mitigate, if anything.
```

## Naming Convention

`EUDAMED [category] [short-title].dd.mm.yyyy.md`

Categories:
- **BR-UDID-xxx** — violation of a specific EUDAMED business rule
- **Data Quality** — missing or inconsistent data without a specific rule violation
- **API** — API behavior inconsistency
