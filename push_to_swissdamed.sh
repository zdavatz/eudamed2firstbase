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

INPUT_DIR="eudamed_json/detail"
BASIC_UDI_CACHE="eudamed_json/basic"
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
    local token_url="$API_BASE/oauth2/token"  # TODO: confirm token endpoint from OpenAPI
    $VERBOSE && echo "  POST $token_url"

    TOKEN=$(curl -s $CURL_VERBOSE --max-time 30 -X POST "$token_url" \
        -H 'Content-Type: application/x-www-form-urlencoded' \
        -d "grant_type=client_credentials&client_id=$CLIENT_ID&client_secret=$CLIENT_SECRET" \
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
    # Skip if no basic file
    if [[ ! -f "$BASIC_UDI_CACHE/$uuid.json" ]]; then
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
    budi_file="$BASIC_UDI_CACHE/$uuid.json"

    # Determine endpoint from legislation
    ENDPOINT=$(python3 -c "
import json
with open('$budi_file') as f:
    bd = json.load(f)

mc = bd.get('multiComponent', {})
mc_code = mc.get('code', '') if isinstance(mc, dict) else ''
suffix = mc_code.rsplit('.', 1)[-1] if '.' in mc_code else mc_code

if suffix in ('system', 'procedure-pack', 'spp-procedure-pack'):
    print('spp')
else:
    leg = bd.get('legislation', {})
    leg_code = leg.get('code', '') if isinstance(leg, dict) else ''
    act = leg_code.rsplit('.', 1)[-1] if '.' in leg_code else leg_code
    print(act.lower())
" 2>/dev/null)

    API_PATH="/v1/m2m/udi/data/$ENDPOINT"

    # Build Swissdamed JSON payload
    PAYLOAD=$(python3 -c "
import json, sys

with open('$f') as fh:
    device = json.load(fh)
with open('$budi_file') as fh:
    budi = json.load(fh)

def lang_texts(mlt):
    if not mlt or not isinstance(mlt, dict):
        return []
    texts = mlt.get('texts', [])
    if not texts:
        return []
    result = []
    for t in texts:
        text = t.get('text', '')
        if not text:
            continue
        lang_obj = t.get('language', {})
        lang = lang_obj.get('isoCode', 'en') if isinstance(lang_obj, dict) else 'en'
        result.append({'language': lang, 'text': text})
    return result

def extract_issuing_entity(code):
    if not code:
        return 'GS1'
    suffix = code.rsplit('.', 1)[-1] if '.' in code else code
    return suffix.upper()

def di_code(di_obj):
    if not di_obj or not isinstance(di_obj, dict):
        return None
    code = di_obj.get('code', '')
    if not code:
        return None
    agency = di_obj.get('issuingAgency', {})
    agency_code = agency.get('code', '') if isinstance(agency, dict) else ''
    return {'diCode': code, 'issuingEntityCode': extract_issuing_entity(agency_code)}

# Primary DI
primary = di_code(device.get('primaryDi'))
if not primary:
    sys.exit(1)

# Secondary DI
secondary = di_code(device.get('secondaryDi'))

# Basic UDI-DI identifier
basic_udi_id = di_code(budi.get('basicUdi'))
if not basic_udi_id:
    basic_udi_id = {'diCode': '', 'issuingEntityCode': 'GS1'}

# Risk class
rc = budi.get('riskClass', {})
rc_code = rc.get('code', '') if isinstance(rc, dict) else ''
risk_class = rc_code.rsplit('.', 1)[-1].upper().replace('-', '_') if rc_code else 'CLASS_I'

# Multi component type
mc = budi.get('multiComponent', {})
mc_code_raw = mc.get('code', '') if isinstance(mc, dict) else ''
mc_suffix = mc_code_raw.rsplit('.', 1)[-1] if '.' in mc_code_raw else mc_code_raw
mc_type = {'system': 'SYSTEM', 'procedure-pack': 'PROCEDURE_PACK', 'spp-procedure-pack': 'SPP_PROCEDURE_PACK'}.get(mc_suffix, 'DEVICE')

# Manufacturer SRN
mfr = budi.get('manufacturer', {})
mfr_srn = mfr.get('srn', '') if isinstance(mfr, dict) else ''

# Nomenclature codes
noms = device.get('cndNomenclatures', [])
nom_codes = [n.get('code', '') for n in (noms or []) if n.get('code')]

# Production identifiers
pi = device.get('udiPiType', {})
prod_ids = []
if pi and isinstance(pi, dict):
    if pi.get('batchNumber'): prod_ids.append('BATCH_NUMBER')
    if pi.get('serializationNumber'): prod_ids.append('SERIALISATION_NUMBER')
    if pi.get('manufacturingDate'): prod_ids.append('MANUFACTURING_DATE')
    if pi.get('expirationDate'): prod_ids.append('EXPIRATION_DATE')
    if pi.get('softwareIdentification'): prod_ids.append('SOFTWARE_IDENTIFICATION')

# Storage handling
shc_list = []
for shc in (device.get('storageHandlingConditions') or []):
    tc = shc.get('typeCode', '')
    suffix = tc.rsplit('.', 1)[-1] if '.' in tc else tc
    descs = lang_texts(shc.get('description'))
    shc_list.append({'type': suffix, 'description': descs})

# Critical warnings
warn_list = []
for w in (device.get('criticalWarnings') or []):
    tc = w.get('typeCode', '')
    suffix = tc.rsplit('.', 1)[-1] if '.' in tc else tc
    descs = lang_texts(w.get('description'))
    warn_list.append({'type': suffix, 'description': descs})

# Packages (containedItem hierarchy)
packages = []
# TODO: flatten containedItem tree to PackageUdiDiDto list

endpoint = '$ENDPOINT'
uuid = '$uuid'

if endpoint == 'spp':
    payload = {
        'correlationId': uuid,
        'basicUdi': {
            'deviceName': budi.get('deviceName'),
            'modelName': budi.get('deviceModel'),
            'identifier': basic_udi_id,
            'riskClass': risk_class,
            'type': mc_type,
            'medicinalPurpose': lang_texts(budi.get('medicalPurpose')),
            'prActorCode': mfr_srn,
        },
        'udiDi': {
            'tradeNames': lang_texts(device.get('tradeName')),
            'referenceNumber': device.get('reference', '') or '',
            'additionalDescription': lang_texts(device.get('additionalDescription')),
            'website': device.get('additionalInformationUrl'),
            'sterile': device.get('sterile', False) or False,
            'sterilization': device.get('sterilization', False) or False,
            'nomenclatureCodes': nom_codes,
            'storageHandlingConditions': shc_list,
            'criticalWarnings': warn_list,
            'identifier': primary,
            'secondaryIdentifier': secondary,
            'productionIdentifiers': prod_ids,
            'packages': packages,
        },
    }
else:
    payload = {
        'correlationId': uuid,
        'basicUdi': {
            'deviceName': budi.get('deviceName'),
            'modelName': budi.get('deviceModel'),
            'animalTissuesCells': budi.get('animalTissues', False) or False,
            'humanTissuesCells': budi.get('humanTissues', False) or False,
            'type': mc_type,
            'active': budi.get('active', False) or False,
            'administeringMedicine': budi.get('administeringMedicine', False) or False,
            'humanProductCheck': budi.get('humanProduct', False) or False,
            'implantable': budi.get('implantable', False) or False,
            'measuringFunction': budi.get('measuringFunction', False) or False,
            'medicinalProductCheck': budi.get('medicinalProduct', False) or False,
            'reusable': budi.get('reusable', False) or False,
            'identifier': basic_udi_id,
            'riskClass': risk_class,
            'mfActorCode': mfr_srn,
        },
        'udiDi': {
            'tradeNames': lang_texts(device.get('tradeName')),
            'referenceNumber': device.get('reference', '') or '',
            'additionalDescription': lang_texts(device.get('additionalDescription')),
            'website': device.get('additionalInformationUrl'),
            'sterile': device.get('sterile', False) or False,
            'sterilization': device.get('sterilization', False) or False,
            'nomenclatureCodes': nom_codes,
            'storageHandlingConditions': shc_list,
            'criticalWarnings': warn_list,
            'identifier': primary,
            'secondaryIdentifier': secondary,
            'productionIdentifiers': prod_ids,
            'packages': packages,
            'baseQuantity': device.get('baseQuantity', 1) or 1,
            'numberOfReuses': device.get('maxNumberOfReuses', -1) if device.get('maxNumberOfReuses') is not None else -1,
            'latex': device.get('latex', False) or False,
            'clinicalSizes': [],
            'reprocessed': device.get('reprocessed', False) or False,
            'cmrSubstances': [],
            'endocrineSubstances': [],
        },
    }

# Remove None values
def clean(obj):
    if isinstance(obj, dict):
        return {k: clean(v) for k, v in obj.items() if v is not None}
    elif isinstance(obj, list):
        return [clean(i) for i in obj]
    return obj

print(json.dumps(clean(payload)))
" 2>/dev/null)

    if [[ -z "$PAYLOAD" ]]; then
        ((SKIPPED++)) || true
        continue
    fi

    $VERBOSE && echo "  POST $API_BASE$API_PATH ($uuid → $ENDPOINT)"

    # Submit to Swissdamed
    RESPONSE=$(curl -s $CURL_VERBOSE -w "\n%{http_code}" --max-time 60 \
        -X POST "$API_BASE$API_PATH" \
        -H 'Content-Type: application/json' \
        -H "Authorization: Bearer $TOKEN" \
        -d "$PAYLOAD")

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
