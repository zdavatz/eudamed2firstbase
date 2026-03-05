#!/usr/bin/env bash
# push_to_api.sh — Push firstbase JSON files to GS1 Catalogue Item API
# Step 1: Live/CreateMany (batches of 100) — creates live products
# Step 2: AddMany — publishes to recipient GLN
#
# Usage:
#   ./push_to_api.sh                    # push all UUID files in firstbase_json/
#   ./push_to_api.sh --dir /path/to/dir # push files from a custom directory
#   ./push_to_api.sh --dry-run          # show what would be pushed, no API calls
#   ./push_to_api.sh --status <reqid>   # query status of a previous request
#
# Environment:
#   FIRSTBASE_EMAIL    (default: zdavatz@ywesee.com)
#   FIRSTBASE_PASSWORD (default: from script)
#   FIRSTBASE_GLN      (default: 7612345000480)

set -euo pipefail

API_BASE="https://test-webapi-firstbase.gs1.ch:5443"
PUBLISH_GLN="7612345000350"  # SuperAdmin Company CH
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

# --- Helper: parse RequestStatus response (with full GS1 error details) ---
parse_status() {
    python3 -c "
import json, sys
data = json.load(sys.stdin)
status = data.get('Status', 'unknown')
print(f'Status: {status}')
errors = []
gs1 = data.get('Gs1ResponseMessage', {})
for resp in gs1.get('GS1Response', []):
    # Check TransactionResponse for ACCEPTED items
    for tr in resp.get('TransactionResponse', []):
        rsc = tr.get('ResponseStatusCode', '')
        ident = tr.get('TransactionIdentifier', {}).get('Value', '')
        if rsc == 'ACCEPTED':
            print(f'  ACCEPTED: {ident}')
    # Check TransactionException for errors
    for te in resp.get('TransactionException', []):
        for ce in te.get('CommandException', []):
            for de in ce.get('DocumentException', []):
                for ae in de.get('AttributeException', []):
                    for err in ae.get('GS1Error', []):
                        errors.append(f\"{err.get('ErrorCode','')}: {err.get('ErrorDescription','')[:150]}\")
    # Check MessageException
    for me in resp.get('MessageException', []):
        for err in me.get('GS1Error', []):
            errors.append(f\"{err.get('ErrorCode','')}: {err.get('ErrorDescription','')[:150]}\")
    # Check GS1Exception
    for ge in resp.get('GS1Exception', []):
        if isinstance(ge, dict):
            for err in ge.get('GS1Error', []):
                errors.append(f\"{err.get('ErrorCode','')}: {err.get('ErrorDescription','')[:150]}\")
            for ce in ge.get('CommandException', []):
                for de in ce.get('DocumentException', []):
                    for ae in de.get('AttributeException', []):
                        for err in ae.get('GS1Error', []):
                            errors.append(f\"{err.get('ErrorCode','')}: {err.get('ErrorDescription','')[:150]}\")
# Also check old-style Items format
for item in data.get('Items', []):
    for resp in item.get('Gs1Response', []):
        for exc in resp.get('AttributeException', []):
            code = exc.get('ExceptionMessageCode', '')
            desc = exc.get('ExceptionMessageDesciption', '')[:120]
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

# Throttle: Live/CreateMany is limited to 1/sec, 60/min, 500/hour
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

# --- Step 1: Create live products via Live/CreateMany (batches of 100) ---
BATCH_SIZE=100
LIVE_ACCEPTED=0
LIVE_FAILED=0
LIVE_REQUEST_IDS=()
PUBLISH_ITEMS=()

echo ""
echo "=== Step 1: Live/CreateMany (batches of $BATCH_SIZE) ==="

for ((bi=0; bi<TOTAL; bi+=BATCH_SIZE)); do
    BATCH_END=$((bi+BATCH_SIZE))
    [[ $BATCH_END -gt $TOTAL ]] && BATCH_END=$TOTAL
    BATCH_COUNT=$((BATCH_END-bi))
    BATCH_FILES=("${FILES[@]:$bi:$BATCH_COUNT}")

    echo "  Batch $((bi/BATCH_SIZE+1)): items $((bi+1))-$BATCH_END of $TOTAL"

    # Build the Live/CreateMany payload
    TMPFILE=$(mktemp)
    python3 -c "
import json, sys

files = sys.argv[1:]
items = []
for fpath in files:
    with open(fpath) as f:
        doc = json.load(f)
    draft = doc['DraftItem']
    items.append({
        'Identifier': draft['Identifier'],
        'TradeItem': draft['TradeItem']
    })

payload = {
    'DocumentCommand': 'Add',
    'Items': items
}
with open('$TMPFILE', 'w') as out:
    json.dump(payload, out)
print(f'Built payload with {len(items)} items')
" "${BATCH_FILES[@]}"

    # Retry loop for 429 rate limiting
    for attempt in 1 2 3; do
        RESPONSE=$(curl -s -w "\n%{http_code}" --max-time 300 -X POST "$API_BASE/CatalogueItem/Live/CreateMany" \
            -H 'Content-Type: application/json' \
            -H "Authorization: bearer $TOKEN" \
            -d @"$TMPFILE" 2>&1)

        HTTP_CODE=$(echo "$RESPONSE" | tail -1)
        BODY=$(echo "$RESPONSE" | sed '$d')

        if [[ "$HTTP_CODE" == "429" ]]; then
            RETRY_AFTER=$(echo "$BODY" | python3 -c "import json,sys; print(json.load(sys.stdin).get('retryAfter',60))" 2>/dev/null || echo 60)
            echo "    429 rate limited — waiting ${RETRY_AFTER}s (attempt $attempt/3)"
            sleep "$RETRY_AFTER"
            continue
        fi
        break
    done
    rm -f "$TMPFILE"

    if [[ "$HTTP_CODE" == "429" ]]; then
        echo "    FAIL: 429 rate limited after 3 retries"
        LIVE_FAILED=$((LIVE_FAILED+BATCH_COUNT))
        continue
    fi

    # Extract RequestIdentifier
    REQ_ID=$(echo "$BODY" | python3 -c "import json,sys; print(json.load(sys.stdin).get('RequestIdentifier',''))" 2>/dev/null || echo "")

    if [[ -n "$REQ_ID" && "$REQ_ID" != "None" ]]; then
        echo "    Submitted: $REQ_ID"
        LIVE_REQUEST_IDS+=("$REQ_ID")

        # Collect publish items from this batch
        for FILE in "${BATCH_FILES[@]}"; do
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
            [[ -n "$ITEM_INFO" ]] && PUBLISH_ITEMS+=("$ITEM_INFO")
        done
        LIVE_ACCEPTED=$((LIVE_ACCEPTED+BATCH_COUNT))
    else
        echo "    FAIL: $(echo "$BODY" | head -c 300)"
        LIVE_FAILED=$((LIVE_FAILED+BATCH_COUNT))
    fi

    # Throttle between batches
    if [[ $bi -lt $((TOTAL-BATCH_SIZE)) ]]; then
        sleep "$THROTTLE"
    fi
done

echo ""
echo "=== Live Creation Summary ==="
echo "Total:    $TOTAL"
echo "Submitted: $LIVE_ACCEPTED"
echo "Failed:   $LIVE_FAILED"
echo "Request IDs: ${LIVE_REQUEST_IDS[*]}"

# --- Wait for async processing and check results ---
if [[ ${#LIVE_REQUEST_IDS[@]} -gt 0 ]]; then
    echo ""
    echo "=== Checking Live/CreateMany results (waiting for async processing) ==="
    sleep 15

    for REQ_ID in "${LIVE_REQUEST_IDS[@]}"; do
        echo "  $REQ_ID:"
        curl -s --max-time 120 -X POST "$API_BASE/RequestStatus/Get" \
            -H 'Content-Type: application/json' \
            -H "Authorization: bearer $TOKEN" \
            -d "{\"RequestIdentifier\":\"$REQ_ID\",\"IncludeGs1Response\":true}" | parse_status
    done
fi

# --- Step 2: Publish all live items via AddMany ---
if [[ ${#PUBLISH_ITEMS[@]} -gt 0 ]]; then
    echo ""
    echo "=== Step 2: Publishing ${#PUBLISH_ITEMS[@]} items via AddMany ==="

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
