#!/usr/bin/env bash
# push_to_api.sh — Push firstbase JSON files to GS1 Catalogue Item API
# Uses Draft/CreateOne for each file, then publishes all via AddMany.
#
# Usage:
#   ./push_to_api.sh                    # push all UUID files in firstbase_json/
#   ./push_to_api.sh --dry-run          # show what would be pushed, no API calls
#   ./push_to_api.sh --status <reqid>   # query status of a previous request
#
# Environment:
#   FIRSTBASE_EMAIL    (default: zdavatz@ywesee.com)
#   FIRSTBASE_PASSWORD (default: from script)
#   FIRSTBASE_GLN      (default: 7612345000480)

set -euo pipefail

API_BASE="https://test-webapi-firstbase.gs1.ch:5443"
PUBLISH_GLN="4399902421386"  # firstbase UDI Connector
INPUT_DIR="firstbase_json"

EMAIL="${FIRSTBASE_EMAIL:-zdavatz@ywesee.com}"
PASSWORD="${FIRSTBASE_PASSWORD:-PrvggFj9Xj52DpU}"
GLN="${FIRSTBASE_GLN:-7612345000480}"

DRY_RUN=false
STATUS_MODE=false
REQUEST_ID=""

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)  DRY_RUN=true; shift ;;
        --status)   STATUS_MODE=true; REQUEST_ID="$2"; shift 2 ;;
        --dir)      INPUT_DIR="$2"; shift 2 ;;
        *)          echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# --- Helper: parse RequestStatus response ---
parse_status() {
    python3 -c "
import json, sys
data = json.load(sys.stdin)
status = data.get('Status', 'unknown')
print(f'Status: {status}')
errors = []
for item in data.get('Items', []):
    for resp in item.get('Gs1Response', []):
        for exc in resp.get('AttributeException', []):
            code = exc.get('ExceptionMessageCode', '')
            desc = exc.get('ExceptionMessageDesciption', '')[:120]
            errors.append(f'{code}: {desc}')
        for err in resp.get('Gs1Error', []):
            code = err.get('ErrorCode', '')
            desc = err.get('ErrorDescription', '')[:120]
            errors.append(f'{code}: {desc}')

print(f'Total errors: {len(errors)}')
if errors:
    from collections import Counter
    for pattern, count in Counter(errors).most_common(30):
        print(f'  {count:>4}x  {pattern}')
" 2>/dev/null || cat
}

# --- Status query mode ---
if $STATUS_MODE; then
    if [[ -z "$REQUEST_ID" ]]; then
        echo "Usage: $0 --status <RequestIdentifier>"
        exit 1
    fi

    echo "Getting token..."
    TOKEN=$(curl -s --max-time 60 -X POST "$API_BASE/Account/Token" \
        -H 'Content-Type: application/json' \
        -d "{\"UserEmail\":\"$EMAIL\",\"Password\":\"$PASSWORD\",\"Gln\":\"$GLN\"}" | tr -d '"')

    if [[ ${#TOKEN} -lt 20 ]]; then
        echo "ERROR: Failed to get token: $TOKEN"
        exit 1
    fi

    echo "Querying RequestStatus for $REQUEST_ID..."
    curl -s --max-time 120 -X POST "$API_BASE/RequestStatus/Get" \
        -H 'Content-Type: application/json' \
        -H "Authorization: bearer $TOKEN" \
        -d "{\"RequestIdentifier\":\"$REQUEST_ID\",\"IncludeGs1Response\":true}" | parse_status
    exit 0
fi

# --- Collect files ---
FILES=()
for f in "$INPUT_DIR"/*.json; do
    base=$(basename "$f")
    [[ "$base" == firstbase_* ]] && continue
    FILES+=("$f")
done

TOTAL=${#FILES[@]}
echo "Found $TOTAL individual JSON files in $INPUT_DIR/"

if [[ $TOTAL -eq 0 ]]; then
    echo "No files to push."
    exit 0
fi

# Throttle: Draft/CreateOne is limited to 1/sec, 60/min, 500/hour
# Small batches (<=60): 1s delay. Large batches: 8s delay to stay within 500/hour.
if [[ $TOTAL -le 60 ]]; then
    THROTTLE=1
else
    THROTTLE=8
fi
echo "Throttle: ${THROTTLE}s between requests (${TOTAL} files)"

if $DRY_RUN; then
    echo "[DRY RUN] Would push $TOTAL files with ${THROTTLE}s throttle"
    echo "First file: ${FILES[0]}"
    echo "Last file:  ${FILES[$((TOTAL-1))]}"
    exit 0
fi

# --- Get token ---
echo "Getting token..."
TOKEN=$(curl -s --max-time 60 -X POST "$API_BASE/Account/Token" \
    -H 'Content-Type: application/json' \
    -d "{\"UserEmail\":\"$EMAIL\",\"Password\":\"$PASSWORD\",\"Gln\":\"$GLN\"}" | tr -d '"')

if [[ ${#TOKEN} -lt 20 ]]; then
    echo "ERROR: Failed to get token: $TOKEN"
    exit 1
fi
echo "Token obtained (${#TOKEN} chars)"

# --- Step 1: Create drafts via Draft/CreateOne ---
ACCEPTED=0
FAILED=0
PUBLISH_ITEMS=()

for ((i=0; i<TOTAL; i++)); do
    FILE="${FILES[$i]}"
    BASE=$(basename "$FILE" .json)
    NUM=$((i+1))

    # Retry loop for 429 rate limiting
    for attempt in 1 2 3; do
        RESPONSE=$(curl -s -w "\n%{http_code}" --max-time 60 -X POST "$API_BASE/CatalogueItem/Draft/CreateOne" \
            -H 'Content-Type: application/json' \
            -H "Authorization: bearer $TOKEN" \
            -d @"$FILE" 2>&1)

        HTTP_CODE=$(echo "$RESPONSE" | tail -1)
        BODY=$(echo "$RESPONSE" | sed '$d')

        if [[ "$HTTP_CODE" == "429" ]]; then
            RETRY_AFTER=$(curl -sI --max-time 10 -X POST "$API_BASE/CatalogueItem/Draft/CreateOne" \
                -H 'Content-Type: application/json' \
                -H "Authorization: bearer $TOKEN" \
                -d '{}' 2>&1 | grep -i 'retry-after' | tr -d '\r' | awk '{print $2}')
            RETRY_AFTER=${RETRY_AFTER:-60}
            echo "  [$NUM/$TOTAL] 429 rate limited — waiting ${RETRY_AFTER}s (attempt $attempt/3)"
            sleep "$RETRY_AFTER"
            continue
        fi
        break
    done

    if [[ "$HTTP_CODE" == "429" ]]; then
        FAILED=$((FAILED+1))
        echo "  [$NUM/$TOTAL] FAIL: $BASE — 429 rate limited after 3 retries"
        continue
    fi

    # Check if response is JSON with RequestIdentifier
    REQ_ID=$(echo "$BODY" | python3 -c "import json,sys; print(json.load(sys.stdin).get('RequestIdentifier',''))" 2>/dev/null || echo "")

    if [[ -n "$REQ_ID" && "$REQ_ID" != "None" ]]; then
        # Extract Identifier, Gtin, TargetMarket for publish step
        ITEM_INFO=$(python3 -c "
import json
with open('$FILE') as f:
    doc = json.load(f)
di = doc['DraftItem']
ident = di.get('Identifier', '')
ti = di.get('TradeItem', {})
gtin = ti.get('Gtin', '')
tm = ti.get('TargetMarket', {}).get('TargetMarketCountryCode', {}).get('Value', '097')
print(f'{ident}|{gtin}|{tm}')
" 2>/dev/null || echo "")
        PUBLISH_ITEMS+=("$ITEM_INFO")
        ACCEPTED=$((ACCEPTED+1))
        echo "  [$NUM/$TOTAL] OK: $BASE"
    else
        FAILED=$((FAILED+1))
        echo "  [$NUM/$TOTAL] FAIL: $BASE — $(echo "$BODY" | head -c 200)"
    fi

    # Throttle between requests
    if [[ $i -lt $((TOTAL-1)) ]]; then
        sleep "$THROTTLE"
    fi
done

echo ""
echo "=== Draft Creation Summary ==="
echo "Total:    $TOTAL"
echo "Created:  $ACCEPTED"
echo "Failed:   $FAILED"

# --- Step 2: Publish all drafts via AddMany ---
if [[ $ACCEPTED -gt 0 ]]; then
    echo ""
    echo "=== Publishing $ACCEPTED drafts via AddMany ==="

    # Batch publish in groups of 100 (API limit)
    PUB_BATCH=100
    PUB_TOTAL=${#PUBLISH_ITEMS[@]}
    for ((pi=0; pi<PUB_TOTAL; pi+=PUB_BATCH)); do
        PUB_END=$((pi+PUB_BATCH))
        [[ $PUB_END -gt $PUB_TOTAL ]] && PUB_END=$PUB_TOTAL
        PUB_COUNT=$((PUB_END-pi))

        echo "  Publishing items $((pi+1))-$PUB_END of $PUB_TOTAL..."

        PUB_SLICE=("${PUBLISH_ITEMS[@]:$pi:$PUB_COUNT}")

        TMPFILE=$(mktemp)
        python3 -c "
import json, sys

gln = '$GLN'
publish_gln = '$PUBLISH_GLN'
items_raw = sys.argv[1:]
items = []
for raw in items_raw:
    parts = raw.split('|')
    if len(parts) != 3:
        continue
    ident, gtin, tm = parts
    items.append({
        'Identifier': ident,
        'DataSource': gln,
        'Gtin': gtin,
        'TargetMarket': tm,
        'PublishToGln': [publish_gln]
    })
payload = {'Items': items}
with open('$TMPFILE', 'w') as out:
    json.dump(payload, out)
" "${PUB_SLICE[@]}"

        RESPONSE=$(curl -s --max-time 180 -X POST "$API_BASE/CatalogueItemPublication/AddMany" \
            -H 'Content-Type: application/json' \
            -H "Authorization: bearer $TOKEN" \
            -d @"$TMPFILE" 2>&1)
        rm -f "$TMPFILE"

        echo "$RESPONSE" | python3 -c "
import json, sys
try:
    data = json.load(sys.stdin)
    print(json.dumps(data, indent=2)[:1000])
except:
    print(sys.stdin.read()[:500])
" 2>/dev/null || echo "$RESPONSE" | head -c 500

        # Throttle between publish batches
        sleep "$THROTTLE"
    done
fi
