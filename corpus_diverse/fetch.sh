#!/usr/bin/env bash
# Downloads the diverse-producer corpus described in MANIFEST.md.
#
# The PDFs are not in the repository: they are third-party content and 45 MB of it, which git
# would keep forever. Provenance lives in sources.txt so the set is reproducible instead.
set -uo pipefail
cd "$(dirname "$0")"
ok=0
while read -r name url; do
  [ -z "${name:-}" ] && continue
  [ -f "$name" ] && { echo "have $name"; ok=$((ok+1)); continue; }
  code=$(curl -sL --max-time 120 -A "rustium-pdf test corpus" -w "%{http_code}" -o "$name" "$url")
  if [ "$code" = "200" ] && head -c 5 "$name" | grep -q "%PDF"; then
    echo "got  $name ($(( $(stat -c%s "$name") / 1024 ))KB)"; ok=$((ok+1))
  else
    echo "FAIL $name (http $code)"; rm -f "$name"
  fi
  sleep 2
done < sources.txt
echo "$ok/$(grep -c . sources.txt) present"
