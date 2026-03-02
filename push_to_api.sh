#!/usr/bin/env bash
# push_to_api.sh — Push firstbase JSON files to GS1 Catalogue Item API (Live/CreateMany)
# then query RequestStatus/Get for validation results.
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
BATCH_SIZE=50                # files per Live/CreateMany request
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
        --batch)    BATCH_SIZE="$2"; shift 2 ;;
        *)          echo "Unknown arg: $1"; exit 1 ;;
    esac
done

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
    RESULT=$(curl -s --max-time 120 -X POST "$API_BASE/RequestStatus/Get" \
        -H 'Content-Type: application/json' \
        -H "Authorization: bearer $TOKEN" \
        -d "{\"RequestIdentifier\":\"$REQUEST_ID\",\"IncludeGs1Response\":true}")

    echo "$RESULT" | python3 -c "
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
from collections import Counter
for pattern, count in Counter(errors).most_common(30):
    print(f'  {count:>4}x  {pattern}')
" 2>/dev/null || echo "$RESULT" | python3 -m json.tool 2>/dev/null || echo "$RESULT"
    exit 0
fi

# --- Collect files ---
FILES=()
for f in "$INPUT_DIR"/*.json; do
    base=$(basename "$f")
    # Skip batch files (start with firstbase_)
    [[ "$base" == firstbase_* ]] && continue
    FILES+=("$f")
done

TOTAL=${#FILES[@]}
echo "Found $TOTAL individual JSON files in $INPUT_DIR/"

if [[ $TOTAL -eq 0 ]]; then
    echo "No files to push."
    exit 0
fi

if $DRY_RUN; then
    echo "[DRY RUN] Would push $TOTAL files in batches of $BATCH_SIZE"
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

# --- Push in batches via Live/CreateMany ---
REQUEST_IDS=()
ACCEPTED=0
FAILED=0
BATCH_NUM=0

for ((i=0; i<TOTAL; i+=BATCH_SIZE)); do
    BATCH_NUM=$((BATCH_NUM+1))
    END=$((i+BATCH_SIZE))
    [[ $END -gt $TOTAL ]] && END=$TOTAL
    COUNT=$((END-i))

    echo ""
    echo "=== Batch $BATCH_NUM: files $((i+1))-$END of $TOTAL ==="

    # Build LiveItem array: [{DocumentCommand: "Add", DraftItem: <file content>}, ...]
    ITEMS="["
    FIRST=true
    for ((j=i; j<END; j++)); do
        FILE="${FILES[$j]}"
        CONTENT=$(cat "$FILE")
        if $FIRST; then
            FIRST=false
        else
            ITEMS+=","
        fi
        ITEMS+="{\"DocumentCommand\":\"Add\",\"DraftItem\":$( echo "$CONTENT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(json.dumps(d['DraftItem']))" 2>/dev/null )"
        # Add PublishToGln
        ITEMS+=",\"PublishToGln\":[\"$PUBLISH_GLN\"]}"
    done
    ITEMS+="]"

    RESPONSE=$(curl -s --max-time 120 -X POST "$API_BASE/CatalogueItem/Live/CreateMany" \
        -H 'Content-Type: application/json' \
        -H "Authorization: bearer $TOKEN" \
        -d "$ITEMS" 2>&1)

    # Extract RequestIdentifier
    REQ_ID=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('RequestIdentifier',''))" 2>/dev/null || echo "")

    if [[ -n "$REQ_ID" && "$REQ_ID" != "None" && "$REQ_ID" != "" ]]; then
        echo "  RequestIdentifier: $REQ_ID"
        REQUEST_IDS+=("$REQ_ID")
        ACCEPTED=$((ACCEPTED+COUNT))
    else
        echo "  FAILED: $RESPONSE"
        FAILED=$((FAILED+COUNT))
    fi
done

echo ""
echo "=== Summary ==="
echo "Total files:  $TOTAL"
echo "Accepted:     $ACCEPTED"
echo "Failed:       $FAILED"
echo "Request IDs:  ${REQUEST_IDS[*]:-none}"
echo ""

# --- Query status for each request ---
if [[ ${#REQUEST_IDS[@]} -gt 0 ]]; then
    echo "Waiting 5s for processing..."
    sleep 5

    for REQ_ID in "${REQUEST_IDS[@]}"; do
        echo ""
        echo "=== Status for $REQ_ID ==="
        RESULT=$(curl -s --max-time 120 -X POST "$API_BASE/RequestStatus/Get" \
            -H 'Content-Type: application/json' \
            -H "Authorization: bearer $TOKEN" \
            -d "{\"RequestIdentifier\":\"$REQ_ID\",\"IncludeGs1Response\":true}" 2>&1)

        echo "$RESULT" | python3 -c "
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
" 2>/dev/null || echo "$RESULT" | head -200
    done
fi

echo ""
echo "To re-query status later:"
for REQ_ID in "${REQUEST_IDS[@]}"; do
    echo "  ./push_to_api.sh --status $REQ_ID"
done
