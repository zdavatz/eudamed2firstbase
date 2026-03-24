#!/usr/bin/env bash
# push_to_api.sh — Push firstbase JSON files to GS1 Catalogue Item API
# All devices: Live/CreateMany (batches of 100) → AddMany (publish)
# Since 2026-03-10: 097.096 downgraded to warning — legacy devices publishable too
#
# Usage:
#   ./push_to_api.sh <PublishToGln>                    # push all UUID files in firstbase_json/
#   ./push_to_api.sh <PublishToGln> --dir /path/to/dir # push files from a custom directory
#   ./push_to_api.sh <PublishToGln> --dry-run          # show what would be pushed, no API calls
#   ./push_to_api.sh --status <reqid>                  # query status of a previous request
#
# Environment:
#   FIRSTBASE_EMAIL    (default: zdavatz@ywesee.com)
#   FIRSTBASE_PASSWORD (default: from script)
#   FIRSTBASE_GLN      (default: 7612345000480)

set -euo pipefail

API_BASE="https://test-webapi-firstbase.gs1.ch:5443"
INPUT_DIR="firstbase_json"
DB_PATH="db/version_tracking.db"

EMAIL="${FIRSTBASE_EMAIL:?Set FIRSTBASE_EMAIL in ~/.bashrc}"
PASSWORD="${FIRSTBASE_PASSWORD:?Set FIRSTBASE_PASSWORD in ~/.bashrc}"
GLN="${FIRSTBASE_GLN:-7612345000480}"

DRY_RUN=false
STATUS_MODE=false
REQUEST_ID=""
PUBLISH_GLN=""

# Parse args — first positional arg is PublishToGln (unless --status mode)
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)  DRY_RUN=true; shift ;;
        --status)   STATUS_MODE=true; REQUEST_ID="$2"; shift 2 ;;
        --dir)      INPUT_DIR="$2"; shift 2 ;;
        *)
            if [[ -z "$PUBLISH_GLN" && "$1" =~ ^[0-9]+$ ]]; then
                PUBLISH_GLN="$1"; shift
            else
                echo "Unknown arg: $1"; exit 1
            fi
            ;;
    esac
done

if ! $STATUS_MODE && [[ -z "$PUBLISH_GLN" ]]; then
    echo "Usage: $0 <PublishToGln> [--dir /path/to/dir] [--dry-run]"
    echo "       $0 --status <reqid>"
    echo ""
    echo "Example: $0 7612345000527"
    exit 1
fi

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

# --- Helper: log push results to SQLite DB ---
# Parses RequestStatus response JSON and logs per-item ACCEPTED/REJECTED to push_log table.
# Args: $1 = request_id, $2 = publish_gln, stdin = full RequestStatus JSON
log_push_results() {
    local req_id="$1"
    local pub_gln="$2"
    local json_file
    json_file=$(mktemp)
    cat > "$json_file"
    python3 -c "
import json, sqlite3, os, sys
from datetime import datetime, timezone

db_path, req_id, pub_gln, json_file = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
now = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')

with open(json_file) as f:
    data = json.load(f)

os.makedirs(os.path.dirname(db_path) or '.', exist_ok=True)
conn = sqlite3.connect(db_path)
conn.execute('''CREATE TABLE IF NOT EXISTS push_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uuid TEXT NOT NULL, gtin TEXT NOT NULL DEFAULT '',
    pushed_at TEXT NOT NULL, request_id TEXT, status TEXT NOT NULL,
    error_code TEXT, error_msg TEXT, publish_gln TEXT
)''')
conn.execute('CREATE INDEX IF NOT EXISTS idx_push_log_uuid ON push_log(uuid)')
conn.execute('CREATE INDEX IF NOT EXISTS idx_push_log_status ON push_log(status)')

rows = []
gs1 = data.get('Gs1ResponseMessage', {})
for resp in gs1.get('GS1Response', []):
    for tr in resp.get('TransactionResponse', []):
        rsc = tr.get('ResponseStatusCode', '')
        ident = tr.get('TransactionIdentifier', {}).get('Value', '')
        if rsc == 'ACCEPTED' and ident.startswith('Draft_'):
            rows.append((ident[6:], '', now, req_id, 'ACCEPTED', None, None, pub_gln))
    for te in resp.get('TransactionException', []):
        for ce in te.get('CommandException', []):
            for de in ce.get('DocumentException', []):
                doc_id = de.get('DocumentIdentifier', {}).get('Value', '')
                uuid = doc_id[6:] if doc_id.startswith('Draft_') else ''
                for ae in de.get('AttributeException', []):
                    for err in ae.get('GS1Error', []):
                        rows.append((uuid, '', now, req_id, 'REJECTED', err.get('ErrorCode',''), err.get('ErrorDescription','')[:200], pub_gln))
    for ge in resp.get('GS1Exception', []):
        if not isinstance(ge, dict): continue
        for ce in ge.get('CommandException', []):
            for de in ce.get('DocumentException', []):
                doc_id = de.get('DocumentIdentifier', {}).get('Value', '')
                uuid = doc_id[6:] if doc_id.startswith('Draft_') else ''
                for ae in de.get('AttributeException', []):
                    for err in ae.get('GS1Error', []):
                        rows.append((uuid, '', now, req_id, 'REJECTED', err.get('ErrorCode',''), err.get('ErrorDescription','')[:200], pub_gln))

if rows:
    conn.executemany('INSERT INTO push_log (uuid,gtin,pushed_at,request_id,status,error_code,error_msg,publish_gln) VALUES (?,?,?,?,?,?,?,?)', rows)
    conn.commit()
    accepted = sum(1 for r in rows if r[4] == 'ACCEPTED')
    rejected = sum(1 for r in rows if r[4] == 'REJECTED')
    print(f'  DB logged: {accepted} ACCEPTED, {rejected} REJECTED', file=sys.stderr)
conn.close()
" "$DB_PATH" "$req_id" "$pub_gln" "$json_file"
    rm -f "$json_file"
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

# --- Helper: detect regulatory act and GTIN from firstbase JSON ---
# Prints "ACT GTIN" (e.g. "MDR 08680941160296" or "UNKNOWN ")
get_file_info() {
    python3 -c "
import json, sys
with open(sys.argv[1]) as f:
    doc = json.load(f)
ti = doc.get('DraftItem', {}).get('TradeItem', {})
gtin = ti.get('Gtin', '')
# Only accept numeric GTINs (reject HIBC/IFA identifiers like +B976...)
if gtin and not gtin.isdigit():
    gtin = ''
act = 'UNKNOWN'
for ri in ti.get('RegulatedTradeItemModule', {}).get('RegulatoryInformation', []):
    a = ri.get('RegulatoryAct', '')
    if a:
        act = a
        break
print(f'{act} {gtin}')
" "$1" 2>/dev/null || echo "UNKNOWN "
}

# --- Collect and classify files (fast Rust scanner) ---
# Since 2026-03-10: 097.096 downgraded from error to warning — legacy devices
# (MDD/AIMDD/IVDD) can now be published via Live/CreateMany + AddMany too.
SCAN_TMP=$(mktemp)
cargo run --quiet -- scan "$INPUT_DIR" > "$SCAN_TMP"
LIVE_FILES=()
while IFS=$'\t' read -r filepath gtin; do
    [[ -n "$filepath" ]] && LIVE_FILES+=("$filepath")
done < "$SCAN_TMP"
rm -f "$SCAN_TMP"

LIVE_TOTAL=${#LIVE_FILES[@]}
TOTAL=$LIVE_TOTAL

if [[ $TOTAL -eq 0 ]]; then
    echo "No files to push."
    exit 0
fi

# Throttle: write endpoints limited to 1/sec, 60/min, 500/hour
# Small batches (<=60): 1s delay. Large batches: 8s delay to stay within 500/hour.
if [[ $TOTAL -le 60 ]]; then
    THROTTLE=1
else
    THROTTLE=8
fi
echo "Throttle: ${THROTTLE}s between requests"

PROCESSED_DIR="$INPUT_DIR/processed"

if $DRY_RUN; then
    echo "[DRY RUN] Would push $LIVE_TOTAL files via Live/CreateMany + AddMany"
    [[ $LIVE_TOTAL -gt 0 ]] && echo "First: ${LIVE_FILES[0]}"
    exit 0
fi

# Track successfully sent files for moving to processed/
SENT_FILES=()

# --- Get token (with retry) ---
TOKEN=""
for token_attempt in 1 2 3; do
    echo "Getting token (attempt $token_attempt/3)..."
    TOKEN=$(curl -s --max-time 30 -X POST "$API_BASE/Account/Token" \
        -H 'Content-Type: application/json' \
        -d "{\"UserEmail\":\"$EMAIL\",\"Password\":\"$PASSWORD\",\"Gln\":\"$GLN\"}" 2>&1 | tr -d '"')

    if [[ ${#TOKEN} -gt 20 ]]; then
        echo "Token obtained (${#TOKEN} chars)"
        break
    fi
    echo "  Failed: ${TOKEN:-(empty response)}"
    if [[ $token_attempt -lt 3 ]]; then
        echo "  Retrying in 10s..."
        sleep 10
    fi
done

if [[ ${#TOKEN} -lt 20 ]]; then
    echo "ERROR: Failed to get token after 3 attempts. API may be down."
    echo "  URL: $API_BASE/Account/Token"
    exit 1
fi

# --- Step 1: Create live products via Live/CreateMany (batches of 100) ---
BATCH_SIZE=100
LIVE_ACCEPTED=0
LIVE_FAILED=0
LIVE_REQUEST_IDS=()
PUBLISH_ITEMS=()

if [[ $LIVE_TOTAL -gt 0 ]]; then
echo ""
echo "=== Step 1: Live/CreateMany (batches of $BATCH_SIZE) ==="

for ((bi=0; bi<LIVE_TOTAL; bi+=BATCH_SIZE)); do
    BATCH_END=$((bi+BATCH_SIZE))
    [[ $BATCH_END -gt $LIVE_TOTAL ]] && BATCH_END=$LIVE_TOTAL
    BATCH_COUNT=$((BATCH_END-bi))
    BATCH_FILES=("${LIVE_FILES[@]:$bi:$BATCH_COUNT}")

    echo "  Batch $((bi/BATCH_SIZE+1)): items $((bi+1))-$BATCH_END of $LIVE_TOTAL"

    # Build the Live/CreateMany payload (keep packaging hierarchy nested)
    TMPFILE=$(mktemp)
    python3 -c "
import json, sys

files = sys.argv[1:]
items = []
for fpath in files:
    with open(fpath) as f:
        doc = json.load(f)
    draft = doc['DraftItem']
    item = {
        'Identifier': draft['Identifier'],
        'TradeItem': draft['TradeItem']
    }
    # Keep CatalogueItemChildItemLink nested (API requires children inline)
    if 'CatalogueItemChildItemLink' in draft:
        item['CatalogueItemChildItemLink'] = draft['CatalogueItemChildItemLink']
    items.append(item)

payload = {
    'DocumentCommand': 'Add',
    'Items': items
}
with open('$TMPFILE', 'w') as out:
    json.dump(payload, out)
children = sum(len(doc.get('DraftItem',{}).get('CatalogueItemChildItemLink',[])) for fpath in files for doc in [json.load(open(fpath))])
print(f'    Payload: {len(items)} items ({children} with children)')
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

        # Collect publish items from this batch (including children) and track sent files
        for FILE in "${BATCH_FILES[@]}"; do
            while IFS= read -r line; do
                [[ -n "$line" ]] && PUBLISH_ITEMS+=("$line")
            done < <(python3 -c "
import json
with open('$FILE') as f:
    doc = json.load(f)
di = doc['DraftItem']

def emit(ident, ti):
    gtin = ti.get('Gtin', '') or ti.get('TradeItemIdentification', {}).get('Gtin', '')
    tm = ti.get('TargetMarket', {}).get('TargetMarketCountryCode', {}).get('Value', '097')
    if gtin:
        print(f'{ident}|{gtin}|{tm}')

emit(di.get('Identifier', ''), di.get('TradeItem', {}))
for child in di.get('CatalogueItemChildItemLink', []):
    cat = child.get('CatalogueItem', {})
    emit(cat.get('Identifier', ''), cat.get('TradeItem', {}))
    for gc in cat.get('CatalogueItemChildItemLink', []):
        g = gc.get('CatalogueItem', {})
        emit(g.get('Identifier', ''), g.get('TradeItem', {}))
" 2>/dev/null)
            SENT_FILES+=("$FILE")
        done
        LIVE_ACCEPTED=$((LIVE_ACCEPTED+BATCH_COUNT))
    else
        echo "    FAIL: $(echo "$BODY" | head -c 300)"
        LIVE_FAILED=$((LIVE_FAILED+BATCH_COUNT))
    fi

    # Throttle between batches
    if [[ $bi -lt $((LIVE_TOTAL-BATCH_SIZE)) ]]; then
        sleep "$THROTTLE"
    fi
done

echo ""
echo "=== Live Creation Summary ==="
echo "Total: $LIVE_TOTAL"
echo "Submitted:      $LIVE_ACCEPTED"
echo "Failed:         $LIVE_FAILED"
echo "Request IDs:    ${LIVE_REQUEST_IDS[*]}"

MAX_POLLS=24  # 24 * 15s = 6 minutes max wait

# --- Wait for async processing until all requests are Done ---
if [[ ${#LIVE_REQUEST_IDS[@]} -gt 0 ]]; then
    echo ""
    echo "=== Waiting for Live/CreateMany async processing ==="
    for REQ_ID in "${LIVE_REQUEST_IDS[@]}"; do
        echo "  $REQ_ID:"
        for ((poll=1; poll<=MAX_POLLS; poll++)); do
            sleep 15
            STATUS_OUT=$(curl -s --max-time 120 -X POST "$API_BASE/RequestStatus/Get" \
                -H 'Content-Type: application/json' \
                -H "Authorization: bearer $TOKEN" \
                -d "{\"RequestIdentifier\":\"$REQ_ID\",\"IncludeGs1Response\":true}")
            STATUS=$(echo "$STATUS_OUT" | python3 -c "import json,sys; print(json.load(sys.stdin).get('Status','unknown'))" 2>/dev/null || echo "unknown")
            echo "    Poll $poll: $STATUS"
            if [[ "$STATUS" == "Done" || "$STATUS" == "Failed" ]]; then
                echo "$STATUS_OUT" | parse_status
                echo "$STATUS_OUT" | log_push_results "$REQ_ID" "$PUBLISH_GLN"
                # Write full response as HTML log
                mkdir -p log
                LOG_FILE="log/$(date '+%M.%H_%d.%m.%Y').log.html"
                echo "$STATUS_OUT" | python3 -c "
import json, sys, html
from datetime import datetime, timezone

data = json.load(sys.stdin)
req_id = '$REQ_ID'
pub_gln = '$PUBLISH_GLN'
now = datetime.now(timezone.utc).strftime('%Y-%m-%d %H:%M:%S UTC')

accepted = []
rejected = []
gs1 = data.get('Gs1ResponseMessage', {})
for resp in gs1.get('GS1Response', []):
    for tr in resp.get('TransactionResponse', []):
        rsc = tr.get('ResponseStatusCode', '')
        ident = tr.get('TransactionIdentifier', {}).get('Value', '')
        if rsc == 'ACCEPTED':
            accepted.append(ident)
    for te in resp.get('TransactionException', []):
        for ce in te.get('CommandException', []):
            for de in ce.get('DocumentException', []):
                doc_id = de.get('DocumentIdentifier', {}).get('Value', '')
                for ae in de.get('AttributeException', []):
                    for err in ae.get('GS1Error', []):
                        rejected.append((doc_id, err.get('ErrorCode',''), err.get('ErrorDescription','')[:200]))
    for ge in resp.get('GS1Exception', []):
        if not isinstance(ge, dict): continue
        for ce in ge.get('CommandException', []):
            for de in ce.get('DocumentException', []):
                doc_id = de.get('DocumentIdentifier', {}).get('Value', '')
                for ae in de.get('AttributeException', []):
                    for err in ae.get('GS1Error', []):
                        rejected.append((doc_id, err.get('ErrorCode',''), err.get('ErrorDescription','')[:200]))

out = f'''<!DOCTYPE html>
<html><head><meta charset=\"utf-8\"><title>Push Log {html.escape(req_id)}</title>
<style>
body {{ font-family: monospace; margin: 20px; }}
h1 {{ font-size: 18px; }}
table {{ border-collapse: collapse; width: 100%; margin: 10px 0; }}
th, td {{ border: 1px solid #ccc; padding: 6px 10px; text-align: left; }}
th {{ background: #f0f0f0; }}
.accepted {{ color: green; }}
.rejected {{ color: red; }}
.summary {{ background: #f8f8f8; padding: 10px; margin: 10px 0; }}
pre {{ background: #f4f4f4; padding: 10px; overflow-x: auto; max-height: 600px; font-size: 12px; }}
</style></head><body>
<h1>GS1 Firstbase Push Log</h1>
<div class=\"summary\">
<b>Timestamp:</b> {now}<br>
<b>Request ID:</b> {html.escape(req_id)}<br>
<b>Publish GLN:</b> {html.escape(pub_gln)}<br>
<b>Status:</b> {html.escape(data.get('Status','unknown'))}<br>
<b>Accepted:</b> <span class=\"accepted\">{len(accepted)}</span> |
<b>Rejected:</b> <span class=\"rejected\">{len(rejected)}</span>
</div>
'''

if accepted:
    out += '<h2 class=\"accepted\">Accepted</h2><table><tr><th>#</th><th>Identifier</th></tr>'
    for i, ident in enumerate(accepted, 1):
        out += f'<tr><td>{i}</td><td>{html.escape(ident)}</td></tr>'
    out += '</table>'

if rejected:
    out += '<h2 class=\"rejected\">Rejected</h2><table><tr><th>#</th><th>Identifier</th><th>Error Code</th><th>Description</th></tr>'
    for i, (doc_id, code, desc) in enumerate(rejected, 1):
        out += f'<tr><td>{i}</td><td>{html.escape(doc_id)}</td><td>{html.escape(code)}</td><td>{html.escape(desc)}</td></tr>'
    out += '</table>'

out += '<h2>Full JSON Response</h2><pre>' + html.escape(json.dumps(data, indent=2)) + '</pre>'
out += '</body></html>'
print(out)
" > "$LOG_FILE"
                echo "    Log written: $LOG_FILE"
                break
            fi
        done
        if [[ "$STATUS" != "Done" && "$STATUS" != "Failed" ]]; then
            echo "    WARNING: Request still not done after $((MAX_POLLS*15))s — AddMany may fail"
        fi
    done
fi

fi # end if LIVE_TOTAL > 0

# --- Step 2: Publish all live items via AddMany ---
if [[ ${#PUBLISH_ITEMS[@]} -gt 0 ]]; then
    # Refresh token before AddMany (CreateMany polling may have taken minutes)
    echo ""
    echo "Refreshing token before AddMany..."
    TOKEN=$(curl -s --max-time 60 -X POST "$API_BASE/Account/Token" \
        -H 'Content-Type: application/json' \
        -d "{\"UserEmail\":\"$EMAIL\",\"Password\":\"$PASSWORD\",\"Gln\":\"$GLN\"}" | tr -d '"')
    if [[ ${#TOKEN} -lt 20 ]]; then
        echo "ERROR: Failed to refresh token: $TOKEN"
        echo "AddMany skipped — items are live but NOT published"
    else
        echo "Token refreshed (${#TOKEN} chars)"
    fi

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

        # Retry loop for 429 rate limiting
        for pub_attempt in 1 2 3; do
            FULL_RESPONSE=$(curl -s -w "\n%{http_code}" --max-time 180 -X POST "$API_BASE/CatalogueItemPublication/AddMany" \
                -H 'Content-Type: application/json' \
                -H "Authorization: bearer $TOKEN" \
                -d @"$TMPFILE")

            PUB_HTTP_CODE=$(echo "$FULL_RESPONSE" | tail -1)
            RESPONSE=$(echo "$FULL_RESPONSE" | sed '$d')

            if [[ "$PUB_HTTP_CODE" == "429" ]]; then
                PUB_RETRY_AFTER=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('retryAfter',60))" 2>/dev/null || echo 60)
                echo "    AddMany 429 rate limited — waiting ${PUB_RETRY_AFTER}s (attempt $pub_attempt/3)"
                sleep "$PUB_RETRY_AFTER"
                continue
            fi
            break
        done
        rm -f "$TMPFILE"
        echo "    AddMany HTTP $PUB_HTTP_CODE"

        # Extract RequestIdentifier from AddMany response
        PUB_REQ_ID=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('RequestIdentifier',''))" 2>/dev/null || echo "")

        if [[ -n "$PUB_REQ_ID" && "$PUB_REQ_ID" != "None" ]]; then
            echo "    AddMany submitted: $PUB_REQ_ID"

            # Poll until Done/Failed
            for ((pub_poll=1; pub_poll<=MAX_POLLS; pub_poll++)); do
                sleep 15
                PUB_STATUS_OUT=$(curl -s --max-time 120 -X POST "$API_BASE/RequestStatus/Get" \
                    -H 'Content-Type: application/json' \
                    -H "Authorization: bearer $TOKEN" \
                    -d "{\"RequestIdentifier\":\"$PUB_REQ_ID\",\"IncludeGs1Response\":true}")
                PUB_STATUS=$(echo "$PUB_STATUS_OUT" | python3 -c "import json,sys; print(json.load(sys.stdin).get('Status','unknown'))" 2>/dev/null || echo "unknown")
                echo "    AddMany poll $pub_poll: $PUB_STATUS"
                if [[ "$PUB_STATUS" == "Done" || "$PUB_STATUS" == "Failed" ]]; then
                    echo "$PUB_STATUS_OUT" | parse_status
                    # Append AddMany result to HTML log
                    mkdir -p log
                    LOG_FILE="log/$(date '+%M.%H_%d.%m.%Y').log.html"
                    echo "$PUB_STATUS_OUT" | python3 -c "
import json, sys, html
from datetime import datetime, timezone

data = json.load(sys.stdin)
req_id = '$PUB_REQ_ID'
pub_gln = '$PUBLISH_GLN'
now = datetime.now(timezone.utc).strftime('%Y-%m-%d %H:%M:%S UTC')

accepted = []
rejected = []
gs1 = data.get('Gs1ResponseMessage', {})
for resp in gs1.get('GS1Response', []):
    for tr in resp.get('TransactionResponse', []):
        rsc = tr.get('ResponseStatusCode', '')
        ident = tr.get('TransactionIdentifier', {}).get('Value', '')
        if rsc == 'ACCEPTED':
            accepted.append(ident)
    for te in resp.get('TransactionException', []):
        for ce in te.get('CommandException', []):
            for de in ce.get('DocumentException', []):
                doc_id = de.get('DocumentIdentifier', {}).get('Value', '')
                for ae in de.get('AttributeException', []):
                    for err in ae.get('GS1Error', []):
                        rejected.append((doc_id, err.get('ErrorCode',''), err.get('ErrorDescription','')[:200]))
    for ge in resp.get('GS1Exception', []):
        if not isinstance(ge, dict): continue
        for ce in ge.get('CommandException', []):
            for de in ce.get('DocumentException', []):
                doc_id = de.get('DocumentIdentifier', {}).get('Value', '')
                for ae in de.get('AttributeException', []):
                    for err in ae.get('GS1Error', []):
                        rejected.append((doc_id, err.get('ErrorCode',''), err.get('ErrorDescription','')[:200]))

out = f'''<!DOCTYPE html>
<html><head><meta charset=\"utf-8\"><title>Push Log {html.escape(req_id)}</title>
<style>
body {{ font-family: monospace; margin: 20px; }}
h1 {{ font-size: 18px; }}
table {{ border-collapse: collapse; width: 100%; margin: 10px 0; }}
th, td {{ border: 1px solid #ccc; padding: 6px 10px; text-align: left; }}
th {{ background: #f0f0f0; }}
.accepted {{ color: green; }}
.rejected {{ color: red; }}
.summary {{ background: #f8f8f8; padding: 10px; margin: 10px 0; }}
pre {{ background: #f4f4f4; padding: 10px; overflow-x: auto; max-height: 600px; font-size: 12px; }}
</style></head><body>
<h1>GS1 Firstbase AddMany Publication Log</h1>
<div class=\"summary\">
<b>Timestamp:</b> {now}<br>
<b>Request ID:</b> {html.escape(req_id)}<br>
<b>Publish GLN:</b> {html.escape(pub_gln)}<br>
<b>Status:</b> {html.escape(data.get('Status','unknown'))}<br>
<b>Published:</b> <span class=\"accepted\">{len(accepted)}</span> |
<b>Failed:</b> <span class=\"rejected\">{len(rejected)}</span>
</div>
'''
if accepted:
    out += '<h2 class=\"accepted\">Published</h2><table><tr><th>#</th><th>Identifier</th></tr>'
    for i, ident in enumerate(accepted, 1):
        out += f'<tr><td>{i}</td><td>{html.escape(ident)}</td></tr>'
    out += '</table>'
if rejected:
    out += '<h2 class=\"rejected\">Failed</h2><table><tr><th>#</th><th>Identifier</th><th>Error Code</th><th>Description</th></tr>'
    for i, (doc_id, code, desc) in enumerate(rejected, 1):
        out += f'<tr><td>{i}</td><td>{html.escape(doc_id)}</td><td>{html.escape(code)}</td><td>{html.escape(desc)}</td></tr>'
    out += '</table>'
out += '<h2>Full JSON Response</h2><pre>' + html.escape(json.dumps(data, indent=2)) + '</pre>'
out += '</body></html>'
print(out)
" > "$LOG_FILE"
                    echo "    AddMany log written: $LOG_FILE"
                    if [[ "$PUB_STATUS" == "Failed" ]]; then
                        echo "    ERROR: AddMany publication FAILED for batch $((pi/PUB_BATCH+1))"
                    fi
                    break
                fi
            done
            if [[ "$PUB_STATUS" != "Done" && "$PUB_STATUS" != "Failed" ]]; then
                echo "    WARNING: AddMany still not done after $((MAX_POLLS*15))s — publication may not have completed"
            fi
        else
            echo "    ERROR: AddMany failed — no RequestIdentifier returned"
            echo "    Response: $(echo "$RESPONSE" | head -c 500)"
        fi

        # Throttle between publish batches
        sleep "$THROTTLE"
    done
fi

# --- Move successfully sent files to processed/ ---
if [[ ${#SENT_FILES[@]} -gt 0 ]]; then
    mkdir -p "$PROCESSED_DIR"
    MOVED=0
    for FILE in "${SENT_FILES[@]}"; do
        BASE=$(basename "$FILE")
        if mv "$FILE" "$PROCESSED_DIR/$BASE" 2>/dev/null; then
            MOVED=$((MOVED+1))
        else
            echo "  Warning: could not move $BASE to processed/"
        fi
    done
    echo ""
    echo "Moved $MOVED file(s) to $PROCESSED_DIR/"
fi
