#!/bin/bash
# Download EUDAMED devices (listing + details) and convert to firstbase JSON
# Usage: ./download.sh --10                            # download first 10 products
#        ./download.sh --100                           # download first 100 products
#        ./download.sh --srn CH-MF-000023141           # all products for this manufacturer SRN
#        ./download.sh --srn CH-MF-000023141 --50      # first 50 products for this SRN
#        ./download.sh --srn SRN1 SRN2 SRN3            # multiple SRNs into one file
#        ./download.sh --srn SRN1 SRN2 --50            # multiple SRNs, limit per SRN
set -euo pipefail

# Parse arguments
TOTAL=""
SRNS=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --srn)
            shift
            # Collect all following args that look like SRNs (not --flags)
            while [[ $# -gt 0 && ! "$1" =~ ^-- ]]; do
                SRNS+=("$1")
                shift
            done
            ;;
        --[0-9]*)
            TOTAL="${1#--}"
            shift
            ;;
        *)
            shift
            ;;
    esac
done

if [[ -z "$TOTAL" && ${#SRNS[@]} -eq 0 ]]; then
    echo "Usage: $0 [--N] [--srn <SRN> [SRN2 ...]]"
    echo "  --N              Number of products to download (e.g. --10, --100, --5000)"
    echo "  --srn <SRN> ...  Download all products for one or more SRNs"
    echo ""
    echo "Examples:"
    echo "  $0 --10                            # first 10 products"
    echo "  $0 --srn CH-MF-000023141           # all products for this SRN"
    echo "  $0 --srn CH-MF-000023141 --50      # first 50 for this SRN"
    echo "  $0 --srn SRN1 SRN2 SRN3            # multiple SRNs, combined output"
    echo "  $0 --srn SRN1 SRN2 --50            # multiple SRNs, limit 50 per SRN"
    exit 1
fi

BASE_URL="https://ec.europa.eu/tools/eudamed/api/devices/udiDiData"
USER_AGENT="Mozilla/5.0 (compatible; EUDAMED-downloader/2.0)"
PARALLEL=10
PAGE_SIZE=300
mkdir -p ndjson

# Set output file names
if [[ ${#SRNS[@]} -gt 0 ]]; then
    if [[ ${#SRNS[@]} -eq 1 ]]; then
        LABEL="${SRNS[0]}"
    else
        # Use first SRN + count to keep filename short
        LABEL="${SRNS[0]}_+$((${#SRNS[@]} - 1))srns"
    fi
else
    LABEL="$TOTAL"
fi
LISTING="ndjson/eudamed_${LABEL}.ndjson"
UUIDS="ndjson/uuids_${LABEL}.txt"
DETAILS="ndjson/eudamed_${LABEL}_details.ndjson"

# --- Step 1: Download listing ---
if [[ ${#SRNS[@]} -gt 0 ]]; then
    echo "=== Step 1: Downloading listings for ${#SRNS[@]} SRN(s)${TOTAL:+ (limit: $TOTAL per SRN)} ==="
    : > "$LISTING"
    collected=0
    for SRN in "${SRNS[@]}"; do
        echo "  --- SRN: $SRN ---"
        srn_collected=0
        p=0
        # Use server-side srn= filtering (API supports it for both manufacturer and AR SRN)
        while true; do
            p=$((p + 1))
            fetch_size=$PAGE_SIZE
            if [[ -n "$TOTAL" ]]; then
                remaining=$((TOTAL - srn_collected))
                fetch_size=$((remaining < PAGE_SIZE ? remaining : PAGE_SIZE))
            fi
            echo -n "  Page $p (fetch $fetch_size) ... "
            tmp=$(mktemp)
            if ! curl -fsSL "${BASE_URL}?page=${p}&pageSize=${fetch_size}&size=${fetch_size}&srn=${SRN}&iso2Code=en&languageIso2Code=en" \
                -A "$USER_AGENT" -o "$tmp" 2>/dev/null; then
                echo "FAILED"
                rm -f "$tmp"
                sleep 1
                continue
            fi
            count=$(jq -r '.content | length' "$tmp" 2>/dev/null || echo 0)
            if [[ $count -eq 0 ]]; then
                echo "empty page, done"
                rm -f "$tmp"
                break
            fi
            jq -c '.content[]' "$tmp" >> "$LISTING" 2>/dev/null
            srn_collected=$((srn_collected + count))
            collected=$((collected + count))
            echo "$count records (SRN total: $srn_collected, overall: $collected)"
            rm -f "$tmp"
            if [[ -n "$TOTAL" && $srn_collected -ge $TOTAL ]]; then
                break
            fi
            sleep 0.3
        done
    done
else
    echo "=== Step 1: Downloading $TOTAL listings ==="
    PAGES=$(( (TOTAL + PAGE_SIZE - 1) / PAGE_SIZE ))
    : > "$LISTING"
    collected=0
    for ((p=1; p<=PAGES; p++)); do
        remaining=$((TOTAL - collected))
        fetch_size=$((remaining < PAGE_SIZE ? remaining : PAGE_SIZE))
        echo -n "  Page $p/$PAGES (fetch $fetch_size) ... "
        tmp=$(mktemp)
        if ! curl -fsSL "${BASE_URL}?page=${p}&pageSize=${fetch_size}&size=${fetch_size}&iso2Code=en&languageIso2Code=en" \
            -A "$USER_AGENT" -o "$tmp" 2>/dev/null; then
            echo "FAILED"
            rm -f "$tmp"
            sleep 1
            continue
        fi
        count=$(jq -r '.content | length' "$tmp" 2>/dev/null || echo 0)
        jq -c '.content[]' "$tmp" >> "$LISTING" 2>/dev/null
        collected=$((collected + count))
        echo "$count records (total: $collected)"
        rm -f "$tmp"
        [[ $count -eq 0 ]] && break
        [[ $collected -ge $TOTAL ]] && break
        sleep 0.3
    done
fi
echo "  Listings: $collected → $LISTING"

if [[ $collected -eq 0 ]]; then
    echo "No products found. Exiting."
    exit 0
fi

# --- Step 2: Extract UUIDs ---
echo "=== Step 2: Extracting UUIDs ==="
jq -r '.uuid' "$LISTING" > "$UUIDS"
UUID_COUNT=$(wc -l < "$UUIDS")
echo "  $UUID_COUNT UUIDs → $UUIDS"

# --- Step 3: Download details ---
echo "=== Step 3: Downloading $UUID_COUNT details ($PARALLEL parallel) ==="
TMPDIR_DL=$(mktemp -d)

# Resume support
DONE=0
if [[ -f "$DETAILS" ]]; then
    DONE=$(wc -l < "$DETAILS")
    echo "  Resuming: $DONE already downloaded"
fi

REMAINING=$((UUID_COUNT - DONE))
if [[ $REMAINING -le 0 ]]; then
    echo "  All details already downloaded"
else
    # Create fetch script
    cat > "$TMPDIR_DL/fetch.sh" << 'FETCHEOF'
#!/bin/bash
uuid="$1"; outdir="$2"; base_url="$3"; ua="$4"
url="${base_url}/${uuid}?languageIso2Code=en"
for attempt in 1 2 3; do
    result=$(curl -fsSL "$url" -A "$ua" --connect-timeout 10 --max-time 30 2>/dev/null) && break
    sleep $((attempt * 2))
done
if [[ -n "${result:-}" ]]; then
    echo "$result" | jq -c '.' > "$outdir/$uuid.json" 2>/dev/null
fi
FETCHEOF
    chmod +x "$TMPDIR_DL/fetch.sh"

    # Process in batches
    BATCH_SIZE=50
    BATCH_DIR="$TMPDIR_DL/batches"
    mkdir -p "$BATCH_DIR"
    tail -n +"$((DONE + 1))" "$UUIDS" | split -l "$BATCH_SIZE" -d -a 4 - "$BATCH_DIR/batch_"

    PROCESSED=0
    for batch_file in "$BATCH_DIR"/batch_*; do
        [[ -f "$batch_file" ]] || continue
        batch_count=$(wc -l < "$batch_file")
        xargs -P "$PARALLEL" -I{} "$TMPDIR_DL/fetch.sh" {} "$TMPDIR_DL" "$BASE_URL" "$USER_AGENT" < "$batch_file"
        while IFS= read -r uuid; do
            if [[ -f "$TMPDIR_DL/$uuid.json" ]]; then
                cat "$TMPDIR_DL/$uuid.json" >> "$DETAILS"
                rm -f "$TMPDIR_DL/$uuid.json"
            fi
        done < "$batch_file"
        PROCESSED=$((PROCESSED + batch_count))
        echo "  Progress: $((DONE + PROCESSED)) / $UUID_COUNT"
    done
    rm -rf "$TMPDIR_DL"
fi

DETAIL_COUNT=$(wc -l < "$DETAILS")
echo "  Details: $DETAIL_COUNT → $DETAILS"

# --- Step 3b: Download Basic UDI-DI data (MDR mandatory fields) ---
BASIC_UDI_URL="https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData"
BASIC_UDI_CACHE="/tmp/basic_udi_cache"
mkdir -p "$BASIC_UDI_CACHE"

# Count how many UUIDs still need Basic UDI-DI data
NEED_BASIC=0
HAVE_BASIC=0
while IFS= read -r uuid; do
    if [[ -f "$BASIC_UDI_CACHE/$uuid.json" && -s "$BASIC_UDI_CACHE/$uuid.json" ]]; then
        HAVE_BASIC=$((HAVE_BASIC + 1))
    else
        NEED_BASIC=$((NEED_BASIC + 1))
    fi
done < "$UUIDS"

echo "=== Step 3b: Downloading Basic UDI-DI data ($NEED_BASIC needed, $HAVE_BASIC cached) ==="

if [[ $NEED_BASIC -gt 0 ]]; then
    TMPDIR_BDL=$(mktemp -d)

    # Create fetch script for Basic UDI-DI
    cat > "$TMPDIR_BDL/fetch_basic.sh" << 'FETCHEOF'
#!/bin/bash
uuid="$1"; cache_dir="$2"; base_url="$3"; ua="$4"
[[ -f "$cache_dir/$uuid.json" && -s "$cache_dir/$uuid.json" ]] && exit 0
url="${base_url}/${uuid}?languageIso2Code=en"
for attempt in 1 2 3; do
    result=$(curl -fsSL "$url" -A "$ua" --connect-timeout 10 --max-time 30 2>/dev/null) && break
    sleep $((attempt * 2))
done
if [[ -n "${result:-}" ]]; then
    echo "$result" > "$cache_dir/$uuid.json" 2>/dev/null
fi
FETCHEOF
    chmod +x "$TMPDIR_BDL/fetch_basic.sh"

    # Process in batches
    BATCH_DIR="$TMPDIR_BDL/batches"
    mkdir -p "$BATCH_DIR"
    # Only fetch UUIDs not already cached
    while IFS= read -r uuid; do
        if [[ ! -f "$BASIC_UDI_CACHE/$uuid.json" || ! -s "$BASIC_UDI_CACHE/$uuid.json" ]]; then
            echo "$uuid"
        fi
    done < "$UUIDS" | split -l 50 -d -a 4 - "$BATCH_DIR/batch_"

    BASIC_PROCESSED=0
    for batch_file in "$BATCH_DIR"/batch_*; do
        [[ -f "$batch_file" ]] || continue
        batch_count=$(wc -l < "$batch_file")
        xargs -P "$PARALLEL" -I{} "$TMPDIR_BDL/fetch_basic.sh" {} "$BASIC_UDI_CACHE" "$BASIC_UDI_URL" "$USER_AGENT" < "$batch_file"
        BASIC_PROCESSED=$((BASIC_PROCESSED + batch_count))
        echo "  Basic UDI-DI progress: $BASIC_PROCESSED / $NEED_BASIC"
    done
    rm -rf "$TMPDIR_BDL"
fi

BASIC_TOTAL=$(find "$BASIC_UDI_CACHE" -maxdepth 1 -name '*.json' -type f | wc -l | tr -d ' ')
echo "  Basic UDI-DI cache: $BASIC_TOTAL files in $BASIC_UDI_CACHE/"

# --- Step 4: Convert to firstbase JSON ---
echo "=== Step 4: Converting to firstbase JSON ==="
cargo run --quiet -- detail "$DETAILS" "$LISTING"

echo "=== Done! ==="
