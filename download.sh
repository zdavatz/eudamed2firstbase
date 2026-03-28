#!/bin/bash
# Download EUDAMED devices and convert to firstbase JSON
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
EUDAMED_JSON_DIR="eudamed_json"
DETAIL_DIR="$EUDAMED_JSON_DIR/detail"
BASIC_DIR="$EUDAMED_JSON_DIR/basic"
LOG_DIR="$EUDAMED_JSON_DIR/log"
DOWNLOAD_LOG="$LOG_DIR/download.log"
mkdir -p "$DETAIL_DIR" "$BASIC_DIR" "$LOG_DIR"

# Temp files for listing/UUIDs (not persisted)
LISTING=$(mktemp /tmp/eudamed_listing_XXXXXX.ndjson)
UUIDS=$(mktemp /tmp/eudamed_uuids_XXXXXX.txt)
trap "rm -f '$LISTING' '$UUIDS'" EXIT

# --- Step 1: Download listing ---
if [[ ${#SRNS[@]} -gt 0 ]]; then
    echo "=== Step 1: Downloading listings for ${#SRNS[@]} SRN(s)${TOTAL:+ (limit: $TOTAL per SRN)} ==="
    : > "$LISTING"
    collected=0
    for SRN in "${SRNS[@]}"; do
        echo "  --- SRN: $SRN ---"
        srn_collected=0
        p=-1
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
    for ((p=0; p<PAGES; p++)); do
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
echo "  Listings: $collected records"

if [[ $collected -eq 0 ]]; then
    echo "No products found. Exiting."
    exit 0
fi

# --- Step 2: Extract UUIDs ---
echo "=== Step 2: Extracting UUIDs ==="
jq -r '.uuid' "$LISTING" > "$UUIDS"
UUID_COUNT=$(wc -l < "$UUIDS")
echo "  $UUID_COUNT UUIDs extracted"

# --- Step 3: Download details as individual JSON files ---
echo "=== Step 3: Downloading details to $DETAIL_DIR/ ($PARALLEL parallel) ==="
TMPDIR_DL=$(mktemp -d)

# Resume support: count already downloaded files
NEED_DETAIL=0
HAVE_DETAIL=0
while IFS= read -r uuid; do
    if [[ -f "$DETAIL_DIR/$uuid.json" && -s "$DETAIL_DIR/$uuid.json" ]]; then
        HAVE_DETAIL=$((HAVE_DETAIL + 1))
    else
        NEED_DETAIL=$((NEED_DETAIL + 1))
    fi
done < "$UUIDS"

if [[ $HAVE_DETAIL -gt 0 ]]; then
    echo "  Resuming: $HAVE_DETAIL already downloaded, $NEED_DETAIL remaining"
fi

if [[ $NEED_DETAIL -gt 0 ]]; then
    # Create fetch script — saves to eudamed_json/detail/ and logs to download.log
    cat > "$TMPDIR_DL/fetch.sh" << 'FETCHEOF'
#!/bin/bash
uuid="$1"; outdir="$2"; base_url="$3"; ua="$4"; logfile="$5"
[[ -f "$outdir/$uuid.json" && -s "$outdir/$uuid.json" ]] && exit 0
url="${base_url}/${uuid}?languageIso2Code=en"
for attempt in 1 2 3; do
    result=$(curl -fsSL "$url" -A "$ua" --connect-timeout 10 --max-time 30 2>/dev/null) && break
    sleep $((attempt * 2))
done
if [[ -n "${result:-}" ]]; then
    echo "$result" | jq '.' > "$outdir/$uuid.json" 2>/dev/null
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') detail $uuid.json" >> "$logfile"
fi
FETCHEOF
    chmod +x "$TMPDIR_DL/fetch.sh"

    # Process in batches
    BATCH_SIZE=50
    BATCH_DIR="$TMPDIR_DL/batches"
    mkdir -p "$BATCH_DIR"
    # Only fetch UUIDs not already downloaded
    while IFS= read -r uuid; do
        if [[ ! -f "$DETAIL_DIR/$uuid.json" || ! -s "$DETAIL_DIR/$uuid.json" ]]; then
            echo "$uuid"
        fi
    done < "$UUIDS" | split -l "$BATCH_SIZE" -d -a 4 - "$BATCH_DIR/batch_"

    PROCESSED=0
    for batch_file in "$BATCH_DIR"/batch_*; do
        [[ -f "$batch_file" ]] || continue
        batch_count=$(wc -l < "$batch_file")
        xargs -P "$PARALLEL" -I{} "$TMPDIR_DL/fetch.sh" {} "$DETAIL_DIR" "$BASE_URL" "$USER_AGENT" "$DOWNLOAD_LOG" < "$batch_file"
        PROCESSED=$((PROCESSED + batch_count))
        echo "  Progress: $((HAVE_DETAIL + PROCESSED)) / $UUID_COUNT"
    done
fi
rm -rf "$TMPDIR_DL"

DETAIL_COUNT=$(find "$DETAIL_DIR" -maxdepth 1 -name '*.json' -type f | wc -l | tr -d ' ')
echo "  Details: $DETAIL_COUNT files in $DETAIL_DIR/"

# --- Step 3b: Download Basic UDI-DI data (MDR mandatory fields) ---
BASIC_UDI_URL="https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData"

# Count how many UUIDs still need Basic UDI-DI data
NEED_BASIC=0
HAVE_BASIC=0
while IFS= read -r uuid; do
    if [[ -f "$BASIC_DIR/$uuid.json" && -s "$BASIC_DIR/$uuid.json" ]]; then
        HAVE_BASIC=$((HAVE_BASIC + 1))
    else
        NEED_BASIC=$((NEED_BASIC + 1))
    fi
done < "$UUIDS"

echo "=== Step 3b: Downloading Basic UDI-DI data ($NEED_BASIC needed, $HAVE_BASIC cached) ==="

if [[ $NEED_BASIC -gt 0 ]]; then
    TMPDIR_BDL=$(mktemp -d)

    # Create fetch script for Basic UDI-DI with logging
    cat > "$TMPDIR_BDL/fetch_basic.sh" << 'FETCHEOF'
#!/bin/bash
uuid="$1"; cache_dir="$2"; base_url="$3"; ua="$4"; logfile="$5"
[[ -f "$cache_dir/$uuid.json" && -s "$cache_dir/$uuid.json" ]] && exit 0
url="${base_url}/${uuid}?languageIso2Code=en"
for attempt in 1 2 3; do
    result=$(curl -fsSL "$url" -A "$ua" --connect-timeout 10 --max-time 30 2>/dev/null) && break
    sleep $((attempt * 2))
done
if [[ -n "${result:-}" ]]; then
    echo "$result" > "$cache_dir/$uuid.json" 2>/dev/null
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') basic $uuid.json" >> "$logfile"
fi
FETCHEOF
    chmod +x "$TMPDIR_BDL/fetch_basic.sh"

    # Process in batches
    BATCH_DIR="$TMPDIR_BDL/batches"
    mkdir -p "$BATCH_DIR"
    # Only fetch UUIDs not already cached
    while IFS= read -r uuid; do
        if [[ ! -f "$BASIC_DIR/$uuid.json" || ! -s "$BASIC_DIR/$uuid.json" ]]; then
            echo "$uuid"
        fi
    done < "$UUIDS" | split -l 50 -d -a 4 - "$BATCH_DIR/batch_"

    BASIC_PROCESSED=0
    for batch_file in "$BATCH_DIR"/batch_*; do
        [[ -f "$batch_file" ]] || continue
        batch_count=$(wc -l < "$batch_file")
        xargs -P "$PARALLEL" -I{} "$TMPDIR_BDL/fetch_basic.sh" {} "$BASIC_DIR" "$BASIC_UDI_URL" "$USER_AGENT" "$DOWNLOAD_LOG" < "$batch_file"
        BASIC_PROCESSED=$((BASIC_PROCESSED + batch_count))
        echo "  Basic UDI-DI progress: $BASIC_PROCESSED / $NEED_BASIC"
    done
    rm -rf "$TMPDIR_BDL"
fi

BASIC_TOTAL=$(find "$BASIC_DIR" -maxdepth 1 -name '*.json' -type f | wc -l | tr -d ' ')
echo "  Basic UDI-DI: $BASIC_TOTAL files in $BASIC_DIR/"

# --- Step 3c: Completeness check — verify all 3 data levels exist ---
echo "=== Step 3c: Completeness check ==="
MISSING_DETAIL=0
MISSING_BASIC=0
MISSING_DETAIL_UUIDS=""
MISSING_BASIC_UUIDS=""
while IFS= read -r uuid; do
    if [[ ! -f "$DETAIL_DIR/$uuid.json" || ! -s "$DETAIL_DIR/$uuid.json" ]]; then
        MISSING_DETAIL=$((MISSING_DETAIL + 1))
        [[ $MISSING_DETAIL -le 5 ]] && MISSING_DETAIL_UUIDS="$MISSING_DETAIL_UUIDS    $uuid (Detail)\n"
    fi
    if [[ ! -f "$BASIC_DIR/$uuid.json" || ! -s "$BASIC_DIR/$uuid.json" ]]; then
        MISSING_BASIC=$((MISSING_BASIC + 1))
        [[ $MISSING_BASIC -le 5 ]] && MISSING_BASIC_UUIDS="$MISSING_BASIC_UUIDS    $uuid (Basic UDI-DI)\n"
    fi
done < "$UUIDS"

if [[ $MISSING_DETAIL -gt 0 || $MISSING_BASIC -gt 0 ]]; then
    echo "  WARNING: Incomplete data!"
    [[ $MISSING_DETAIL -gt 0 ]] && echo "    Missing Detail JSON: $MISSING_DETAIL"
    [[ $MISSING_BASIC -gt 0 ]] && echo "    Missing Basic UDI-DI: $MISSING_BASIC"
    echo "  Examples:"
    [[ -n "$MISSING_DETAIL_UUIDS" ]] && echo -e "$MISSING_DETAIL_UUIDS"
    [[ -n "$MISSING_BASIC_UUIDS" ]] && echo -e "$MISSING_BASIC_UUIDS"

    # Retry missing Basic UDI-DI downloads
    if [[ $MISSING_BASIC -gt 0 ]]; then
        echo "  Retrying $MISSING_BASIC missing Basic UDI-DI downloads..."
        TMPDIR_RETRY=$(mktemp -d)
        RETRY_LIST="$TMPDIR_RETRY/retry.txt"
        while IFS= read -r uuid; do
            if [[ ! -f "$BASIC_DIR/$uuid.json" || ! -s "$BASIC_DIR/$uuid.json" ]]; then
                echo "$uuid"
            fi
        done < "$UUIDS" > "$RETRY_LIST"

        cat > "$TMPDIR_RETRY/fetch_basic.sh" << 'FETCHEOF'
#!/bin/bash
uuid="$1"; cache_dir="$2"; base_url="$3"; ua="$4"
[[ -f "$cache_dir/$uuid.json" && -s "$cache_dir/$uuid.json" ]] && exit 0
url="${base_url}/${uuid}?languageIso2Code=en"
for attempt in 1 2 3 4 5; do
    result=$(curl -fsSL "$url" -A "$ua" --connect-timeout 15 --max-time 45 2>/dev/null) && break
    sleep $((attempt * 3))
done
if [[ -n "${result:-}" ]]; then
    echo "$result" > "$cache_dir/$uuid.json" 2>/dev/null
fi
FETCHEOF
        chmod +x "$TMPDIR_RETRY/fetch_basic.sh"
        xargs -P "$PARALLEL" -I{} "$TMPDIR_RETRY/fetch_basic.sh" {} "$BASIC_DIR" "$BASIC_UDI_URL" "$USER_AGENT" < "$RETRY_LIST"
        rm -rf "$TMPDIR_RETRY"

        # Re-check
        STILL_MISSING=0
        while IFS= read -r uuid; do
            if [[ ! -f "$BASIC_DIR/$uuid.json" || ! -s "$BASIC_DIR/$uuid.json" ]]; then
                STILL_MISSING=$((STILL_MISSING + 1))
            fi
        done < "$UUIDS"
        if [[ $STILL_MISSING -gt 0 ]]; then
            echo "  Still missing $STILL_MISSING Basic UDI-DI after retry"
        else
            echo "  All Basic UDI-DI downloads complete after retry"
        fi
    fi

    # Retry missing Detail downloads
    if [[ $MISSING_DETAIL -gt 0 ]]; then
        echo "  Retrying $MISSING_DETAIL missing Detail downloads..."
        TMPDIR_RETRY=$(mktemp -d)
        RETRY_LIST="$TMPDIR_RETRY/retry.txt"
        while IFS= read -r uuid; do
            if [[ ! -f "$DETAIL_DIR/$uuid.json" || ! -s "$DETAIL_DIR/$uuid.json" ]]; then
                echo "$uuid"
            fi
        done < "$UUIDS" > "$RETRY_LIST"

        cat > "$TMPDIR_RETRY/fetch_detail.sh" << 'FETCHEOF'
#!/bin/bash
uuid="$1"; outdir="$2"; base_url="$3"; ua="$4"
[[ -f "$outdir/$uuid.json" && -s "$outdir/$uuid.json" ]] && exit 0
url="${base_url}/${uuid}?languageIso2Code=en"
for attempt in 1 2 3 4 5; do
    result=$(curl -fsSL "$url" -A "$ua" --connect-timeout 15 --max-time 45 2>/dev/null) && break
    sleep $((attempt * 3))
done
if [[ -n "${result:-}" ]]; then
    echo "$result" | jq '.' > "$outdir/$uuid.json" 2>/dev/null
fi
FETCHEOF
        chmod +x "$TMPDIR_RETRY/fetch_detail.sh"
        xargs -P "$PARALLEL" -I{} "$TMPDIR_RETRY/fetch_detail.sh" {} "$DETAIL_DIR" "$BASE_URL" "$USER_AGENT" < "$RETRY_LIST"
        rm -rf "$TMPDIR_RETRY"

        STILL_MISSING=0
        while IFS= read -r uuid; do
            if [[ ! -f "$DETAIL_DIR/$uuid.json" || ! -s "$DETAIL_DIR/$uuid.json" ]]; then
                STILL_MISSING=$((STILL_MISSING + 1))
            fi
        done < "$UUIDS"
        if [[ $STILL_MISSING -gt 0 ]]; then
            echo "  Still missing $STILL_MISSING Detail JSON after retry"
        else
            echo "  All Detail downloads complete after retry"
        fi
    fi
else
    echo "  All $UUID_COUNT devices have both Detail and Basic UDI-DI data ✓"
fi

# --- Step 4: Convert to firstbase JSON ---
echo "=== Step 4: Converting to firstbase JSON ==="
cargo run --quiet -- firstbase

echo "=== Done! ==="
