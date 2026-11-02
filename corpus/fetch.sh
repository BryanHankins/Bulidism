#!/usr/bin/env bash
# Download the corpus models listed in corpus/manifest.json into corpus/cache/, checking each
# file's SHA-256. Files already present with the right hash are kept.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p cache
python3 -c 'import json; [print(m["file"], m["sha256"], m["url"]) for m in json.load(open("manifest.json"))["models"]]' |
while read -r file sha url; do
  if [ -f "cache/$file" ] && echo "$sha  cache/$file" | sha256sum -c --status; then
    echo "ok      $file"; continue
  fi
  echo "fetch   $file"
  curl -fsSL --retry 3 -o "cache/$file.part" "$url"
  echo "$sha  cache/$file.part" | sha256sum -c --status || { echo "checksum mismatch: $file" >&2; rm -f "cache/$file.part"; exit 1; }
  mv "cache/$file.part" "cache/$file"
done
