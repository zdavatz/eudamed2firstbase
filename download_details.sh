#!/bin/bash
# Download EUDAMED device details with controlled parallelism
set -eu

BASE_URL="https://ec.europa.eu/tools/eudamed/api/devices/udiDiData"
USER_AGENT="Mozilla/5.0 (compatible; EUDAMED-downloader/2.0)"
UUID_FILE="ndjson/uuids.txt"
OUTPUT="ndjson/eudamed_10k_details.ndjson"
TMPDIR_DL=$(mktemp -d)
PARALLEL=10
TOTAL=$(wc -l < "$UUID_FILE")

# Resume support
DONE=0
if [[ -f "$OUTPUT" ]]; then
    DONE=$(wc -l < "$OUTPUT")
    echo "Resuming: $DONE already downloaded out of $TOTAL"
fi

REMAINING=$((TOTAL - DONE))
echo "Downloading $REMAINING device details ($PARALLEL parallel) → $OUTPUT"
echo "Temp dir: $TMPDIR_DL"

# Create a fetch script that writes one compacted JSON line to outdir/uuid.json
cat > "$TMPDIR_DL/fetch.sh" << 'FETCHEOF'
#!/bin/bash
uuid="$1"
outdir="$2"
base_url="$3"
ua="$4"
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

# Split remaining UUIDs into batch files of 50
BATCH_SIZE=50
BATCH_DIR="$TMPDIR_DL/batches"
mkdir -p "$BATCH_DIR"

tail -n +"$((DONE + 1))" "$UUID_FILE" | split -l "$BATCH_SIZE" -d -a 4 - "$BATCH_DIR/batch_"

PROCESSED=0
for batch_file in "$BATCH_DIR"/batch_*; do
    [[ -f "$batch_file" ]] || continue
    batch_count=$(wc -l < "$batch_file")

    # Run batch in parallel
    xargs -P "$PARALLEL" -I{} "$TMPDIR_DL/fetch.sh" {} "$TMPDIR_DL" "$BASE_URL" "$USER_AGENT" < "$batch_file"

    # Combine results in order
    while IFS= read -r uuid; do
        if [[ -f "$TMPDIR_DL/$uuid.json" ]]; then
            cat "$TMPDIR_DL/$uuid.json" >> "$OUTPUT"
            rm -f "$TMPDIR_DL/$uuid.json"
        fi
    done < "$batch_file"

    PROCESSED=$((PROCESSED + batch_count))
    echo "Progress: $((DONE + PROCESSED)) / $TOTAL ($PROCESSED new)"
done

rm -rf "$TMPDIR_DL"
FINAL=$(wc -l < "$OUTPUT")
echo "Done! $FINAL detail records → $OUTPUT ($(du -h "$OUTPUT" | cut -f1))"
