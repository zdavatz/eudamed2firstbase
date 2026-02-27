#!/bin/bash
set -euo pipefail
BASE_URL="https://ec.europa.eu/tools/eudamed/api/devices/udiDiData"
USER_AGENT="Mozilla/5.0 (compatible; EUDAMED-downloader/1.0)"
OUTPUT="ndjson/eudamed_10k.ndjson"
PAGE_SIZE=300
TOTAL=10200
PAGES=$(( (TOTAL + PAGE_SIZE - 1) / PAGE_SIZE ))

: > "$OUTPUT"
collected=0

for ((p=1; p<=PAGES; p++)); do
    echo -n "Page $p/$PAGES ... "
    tmp=$(mktemp)
    if ! curl -fsSL "${BASE_URL}?page=${p}&pageSize=${PAGE_SIZE}&size=${PAGE_SIZE}&iso2Code=en&languageIso2Code=en" -A "$USER_AGENT" -o "$tmp" 2>/dev/null; then
        echo "FAILED"
        rm -f "$tmp"
        sleep 1
        continue
    fi
    count=$(jq -r '.content | length' "$tmp" 2>/dev/null || echo 0)
    jq -c '.content[]' "$tmp" >> "$OUTPUT" 2>/dev/null
    collected=$((collected + count))
    echo "$count records (total: $collected)"
    rm -f "$tmp"
    [[ $count -eq 0 ]] && break
    sleep 0.3
done

echo "Done! $collected records → $OUTPUT ($(du -h "$OUTPUT" | cut -f1))"
