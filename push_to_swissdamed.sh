#!/usr/bin/env bash
# push_to_swissdamed.sh — Push EUDAMED devices to Swissdamed M2M API
#
# Usage:
#   ./push_to_swissdamed.sh                          # push all eudamed_json/detail/ files
#   ./push_to_swissdamed.sh --playground              # use playground environment (default)
#   ./push_to_swissdamed.sh --production              # use production environment
#   ./push_to_swissdamed.sh --status <correlationId>  # check submit status
#   ./push_to_swissdamed.sh --market-status           # set market status for submitted devices
#   ./push_to_swissdamed.sh --dry-run                 # show what would be pushed
#   ./push_to_swissdamed.sh -v                        # verbose output
#
# Environment:
#   SWISSDAMED_CLIENT_ID     (required)
#   SWISSDAMED_CLIENT_SECRET (required)

set -euo pipefail

# --- Configuration ---
PLAYGROUND_BASE="https://playground.swissdamed.ch"
PRODUCTION_BASE=""  # TODO: production URL not yet published
API_BASE="$PLAYGROUND_BASE"

INPUT_DIR="swissdamed_json"
DB_PATH="db/version_tracking.db"

CLIENT_ID="${SWISSDAMED_CLIENT_ID:?Set SWISSDAMED_CLIENT_ID in ~/.bashrc}"
CLIENT_SECRET="${SWISSDAMED_CLIENT_SECRET:?Set SWISSDAMED_CLIENT_SECRET in ~/.bashrc}"

DRY_RUN=false
VERBOSE=false
STATUS_MODE=false
MARKET_STATUS_MODE=false
CORRELATION_ID=""

# --- Parse args ---
while [[ $# -gt 0 ]]; do
    case "$1" in
        -v|--verbose)     VERBOSE=true; shift ;;
        --dry-run)        DRY_RUN=true; shift ;;
        --playground)     API_BASE="$PLAYGROUND_BASE"; shift ;;
        --production)
            if [[ -z "$PRODUCTION_BASE" ]]; then
                echo "ERROR: Production URL not yet published by Swissmedic"
                exit 1
            fi
            API_BASE="$PRODUCTION_BASE"; shift ;;
        --status)         STATUS_MODE=true; CORRELATION_ID="$2"; shift 2 ;;
        --market-status)  MARKET_STATUS_MODE=true; shift ;;
        --dir)            INPUT_DIR="$2"; shift 2 ;;
        *)                echo "Unknown arg: $1"; exit 1 ;;
    esac
done

CURL_VERBOSE=""
$VERBOSE && CURL_VERBOSE="-v"

# --- Helper: Get OAuth2 token ---
get_token() {
    # Azure CIAM token endpoint (from OpenAPI spec securitySchemes)
    local token_url="https://3a5c95df-c59f-418a-96fc-b8531bf24be8.ciamlogin.com/3a5c95df-c59f-418a-96fc-b8531bf24be8/oauth2/v2.0/token"
    local scope="8d64e26d-ea71-4ab8-90d6-2acd795eb668/.default"
    $VERBOSE && echo "  POST $token_url"

    TOKEN=$(curl -s $CURL_VERBOSE --max-time 30 -X POST "$token_url" \
        -H 'Content-Type: application/x-www-form-urlencoded' \
        -d "grant_type=client_credentials&client_id=$CLIENT_ID&client_secret=$CLIENT_SECRET&scope=$scope" \
        | python3 -c "import json,sys; print(json.load(sys.stdin).get('access_token',''))" 2>/dev/null || echo "")

    if [[ ${#TOKEN} -lt 20 ]]; then
        echo "ERROR: Failed to get OAuth2 token"
        $VERBOSE && echo "  Response: $TOKEN"
        exit 1
    fi
    echo "Token obtained (${#TOKEN} chars)"
}

# --- Status check mode ---
if $STATUS_MODE; then
    echo "Getting token..."
    get_token
    echo "Checking status for $CORRELATION_ID..."
    curl -s $CURL_VERBOSE --max-time 60 -X POST "$API_BASE/v1/m2m/udi/data/udi-di-request-status" \
        -H 'Content-Type: application/json' \
        -H "Authorization: Bearer $TOKEN" \
        -d "{\"correlationIds\":[\"$CORRELATION_ID\"]}" | python3 -c "
import json, sys
data = json.load(sys.stdin)
print(json.dumps(data, indent=2))
" 2>/dev/null
    exit 0
fi

# --- Ensure swissdamed_push_log table exists ---
python3 -c "
import sqlite3, os
os.makedirs(os.path.dirname('$DB_PATH') or '.', exist_ok=True)
conn = sqlite3.connect('$DB_PATH')
conn.execute('''CREATE TABLE IF NOT EXISTS swissdamed_push_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uuid TEXT NOT NULL, correlation_id TEXT, pushed_at TEXT NOT NULL,
    endpoint TEXT NOT NULL, status TEXT NOT NULL,
    error_code TEXT, error_msg TEXT
)''')
conn.execute('CREATE INDEX IF NOT EXISTS idx_swissdamed_uuid ON swissdamed_push_log(uuid)')
conn.execute('CREATE INDEX IF NOT EXISTS idx_swissdamed_status ON swissdamed_push_log(status)')
conn.commit()
conn.close()
" 2>/dev/null

# --- Collect files to push (skip already pushed) ---
echo "Scanning $INPUT_DIR/ for unpushed EUDAMED JSON files..."
PUSHED_UUIDS=$(python3 -c "
import sqlite3
conn = sqlite3.connect('$DB_PATH')
try:
    rows = conn.execute('SELECT DISTINCT uuid FROM swissdamed_push_log WHERE status=\"ACCEPTED\"').fetchall()
    for r in rows:
        print(r[0])
except:
    pass
conn.close()
" 2>/dev/null)

PUSH_FILES=()
SKIPPED_PUSHED=0
TOTAL_FILES=0
for f in "$INPUT_DIR"/*.json; do
    [[ -f "$f" ]] || continue
    TOTAL_FILES=$((TOTAL_FILES + 1))
    uuid=$(basename "$f" .json)
    # Skip if already pushed to Swissdamed
    if echo "$PUSHED_UUIDS" | grep -q "^${uuid}$" 2>/dev/null; then
        SKIPPED_PUSHED=$((SKIPPED_PUSHED + 1))
        continue
    fi
    PUSH_FILES+=("$f")
done

echo "Found $TOTAL_FILES files, ${#PUSH_FILES[@]} to push, $SKIPPED_PUSHED already pushed"

if [[ ${#PUSH_FILES[@]} -eq 0 ]]; then
    echo "No new files to push."
    exit 0
fi

if $DRY_RUN; then
    echo "[DRY RUN] Would push ${#PUSH_FILES[@]} files to $API_BASE"
    exit 0
fi

# --- Get token ---
echo "Getting token..."
get_token

# --- Push devices ---
SUBMITTED=0
FAILED=0
PUSH_TOTAL=${#PUSH_FILES[@]}

for f in "${PUSH_FILES[@]}"; do
    [[ -f "$f" ]] || continue
    uuid=$(basename "$f" .json)

    # Read pre-built Swissdamed JSON (from cargo run swissdamed)
    PAYLOAD=$(cat "$f")

    # Extract endpoint from correlationId — determine from basicUdi fields
    ENDPOINT=$(python3 -c "
import json
d = json.load(open('$f'))
bu = d.get('basicUdi', {})
if 'prActorCode' in bu and 'medicinalPurpose' in bu:
    print('spp')
elif 'mfActorCode' not in bu:
    print('mdr')
else:
    print('mdr')
" 2>/dev/null || echo "mdr")

    API_PATH="/v1/m2m/udi/data/$ENDPOINT"

    $VERBOSE && echo "  POST $API_BASE$API_PATH ($uuid → $ENDPOINT)"

    # Submit to Swissdamed
    RESPONSE=$(curl -s $CURL_VERBOSE -w "\n%{http_code}" --max-time 60 \
        -X POST "$API_BASE$API_PATH" \
        -H 'Content-Type: application/json' \
        -H "Authorization: Bearer $TOKEN" \
        -d @"$f")

    HTTP_CODE=$(echo "$RESPONSE" | tail -1)
    BODY=$(echo "$RESPONSE" | sed '$d')

    # Log result to swissdamed_push_log
    log_swissdamed() {
        local s_uuid="$1" s_status="$2" s_endpoint="$3" s_err_code="$4" s_err_msg="$5"
        python3 -c "
import sqlite3
from datetime import datetime, timezone
conn = sqlite3.connect('$DB_PATH')
now = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
conn.execute('INSERT INTO swissdamed_push_log (uuid,correlation_id,pushed_at,endpoint,status,error_code,error_msg) VALUES (?,?,?,?,?,?,?)',
    ('$s_uuid','$s_uuid',now,'$s_endpoint','$s_status','$s_err_code','$s_err_msg'))
conn.commit()
conn.close()
" 2>/dev/null
    }

    if [[ "$HTTP_CODE" == "202" ]]; then
        ((SUBMITTED++)) || true
        log_swissdamed "$uuid" "ACCEPTED" "$ENDPOINT" "" ""
        $VERBOSE && echo "    202 Accepted"
    elif [[ "$HTTP_CODE" == "429" ]]; then
        echo "    429 Rate limited — waiting 60s"
        sleep 60
        # Retry once
        RESPONSE=$(curl -s -w "\n%{http_code}" --max-time 60 \
            -X POST "$API_BASE$API_PATH" \
            -H 'Content-Type: application/json' \
            -H "Authorization: Bearer $TOKEN" \
            -d "$PAYLOAD")
        HTTP_CODE=$(echo "$RESPONSE" | tail -1)
        BODY=$(echo "$RESPONSE" | sed '$d')
        if [[ "$HTTP_CODE" == "202" ]]; then
            ((SUBMITTED++)) || true
            log_swissdamed "$uuid" "ACCEPTED" "$ENDPOINT" "" ""
        else
            ((FAILED++)) || true
            log_swissdamed "$uuid" "REJECTED" "$ENDPOINT" "$HTTP_CODE" "$(echo "$BODY" | head -c 200)"
            echo "    FAIL ($uuid): HTTP $HTTP_CODE"
        fi
    else
        ((FAILED++)) || true
        log_swissdamed "$uuid" "REJECTED" "$ENDPOINT" "$HTTP_CODE" "$(echo "$BODY" | head -c 200)"
        echo "  FAIL ($uuid): HTTP $HTTP_CODE — $(echo "$BODY" | head -c 200)"
    fi

    # Progress
    DONE=$((SUBMITTED + FAILED))
    [[ $((DONE % 100)) -eq 0 && $DONE -gt 0 ]] && echo "  Progress: $DONE/$PUSH_TOTAL (submitted=$SUBMITTED failed=$FAILED)"

    # Throttle
    sleep 0.5
done

echo ""
echo "=== Swissdamed Push Summary ==="
echo "Total files:  $FILE_COUNT"
echo "Submitted:    $SUBMITTED"
echo "Failed:       $FAILED"
echo "Skipped:      $SKIPPED"
echo "API Base:     $API_BASE"
echo ""
echo "Next: Wait 5+ minutes, then check status with:"
echo "  ./push_to_swissdamed.sh --status <correlationId>"
echo "Then set market status with:"
echo "  ./push_to_swissdamed.sh --market-status"
