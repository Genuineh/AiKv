#!/usr/bin/env bash
# Export a Grafana panel as PNG by calling the Grafana Image Renderer (方式 B).
# Requires: Image Renderer running (e.g. otel docker-compose with grafana-image-renderer).
# When Renderer runs in Docker, GRAFANA_URL must be the URL the Renderer uses to reach Grafana (e.g. http://grafana:3000).
set -euo pipefail

print_usage() {
  cat <<'EOF'
Usage: scripts/grafana_export_panel.sh [options] --dashboard UID --panel ID --from FROM --to TO --output FILE

Export a Grafana panel as PNG via the Grafana Image Renderer service (direct call to Renderer HTTP API).
Time range is passed as from/to (e.g. now-1h, now, or Unix ms). Result is written to FILE.

Options:
  --dashboard UID   Dashboard UID (required)
  --panel ID        Panel ID (required)
  --from FROM       Start of time range (e.g. now-1h, now-6h, 1700000000000)
  --to TO           End of time range (e.g. now, 1700010000000)
  --output FILE     Output PNG path (required)
  --grafana-url URL Grafana base URL as reachable by the Renderer (default: http://grafana:3000)
  --renderer-url URL Renderer /render endpoint (default: http://localhost:8081/render)
  --width W         Image width in pixels (default: 800)
  --height H        Image height in pixels (default: 400)
  --token T         X-Auth-Token for Renderer (default: -)
  --timeout T       Render timeout in seconds (default: 60)
  --slug SLUG       Dashboard URL slug (default: d)
  -?, --help        Show this help message

Environment (override by options): GRAFANA_URL, RENDERER_URL, RENDERER_AUTH_TOKEN.

Example (Renderer and Grafana in Docker, script on host):
  scripts/grafana_export_panel.sh \
    --dashboard abc123 --panel 1 \
    --from now-1h --to now \
    --output /tmp/panel.png

EOF
}

GRAFANA_URL="${GRAFANA_URL:-http://grafana:3000}"
RENDERER_URL="${RENDERER_URL:-http://localhost:8081/render}"
RENDERER_AUTH_TOKEN="${RENDERER_AUTH_TOKEN:--}"
WIDTH="${GRAFANA_EXPORT_WIDTH:-800}"
HEIGHT="${GRAFANA_EXPORT_HEIGHT:-400}"
TIMEOUT="${GRAFANA_EXPORT_TIMEOUT:-60}"
DASHBOARD_SLUG="${GRAFANA_EXPORT_SLUG:-d}"

DASHBOARD_UID=""
PANEL_ID=""
FROM=""
TO=""
OUTPUT=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dashboard)
      DASHBOARD_UID="$2"; shift 2;;
    --panel)
      PANEL_ID="$2"; shift 2;;
    --from)
      FROM="$2"; shift 2;;
    --to)
      TO="$2"; shift 2;;
    --output)
      OUTPUT="$2"; shift 2;;
    --grafana-url)
      GRAFANA_URL="$2"; shift 2;;
    --renderer-url)
      RENDERER_URL="$2"; shift 2;;
    --width)
      WIDTH="$2"; shift 2;;
    --height)
      HEIGHT="$2"; shift 2;;
    --token)
      RENDERER_AUTH_TOKEN="$2"; shift 2;;
    --timeout)
      TIMEOUT="$2"; shift 2;;
    --slug)
      DASHBOARD_SLUG="$2"; shift 2;;
    -\?|--help)
      print_usage; exit 0;;
    *)
      echo "Unknown option: $1" >&2
      print_usage
      exit 1;;
  esac
done

if [[ -z "${DASHBOARD_UID}" || -z "${PANEL_ID}" || -z "${FROM}" || -z "${TO}" || -z "${OUTPUT}" ]]; then
  echo "Missing required arguments: --dashboard, --panel, --from, --to, --output" >&2
  print_usage
  exit 1
fi

# Panel URL that the Renderer's browser will load (must be reachable from Renderer container)
PANEL_PATH="/d-solo/${DASHBOARD_UID}/${DASHBOARD_SLUG}"
QUERY="panelId=${PANEL_ID}&from=${FROM}&to=${TO}&width=${WIDTH}&height=${HEIGHT}&theme=light"
PANEL_URL="${GRAFANA_URL}${PANEL_PATH}?${QUERY}"

echo "Exporting panel: ${PANEL_URL}" >&2
echo "Renderer: ${RENDERER_URL}" >&2
echo "Output: ${OUTPUT}" >&2

HTTP_CODE=$(curl -sS -w '%{http_code}' -o "${OUTPUT}" \
  -H "X-Auth-Token: ${RENDERER_AUTH_TOKEN}" \
  -G --data-urlencode "url=${PANEL_URL}" \
  --data-urlencode "width=${WIDTH}" \
  --data-urlencode "height=${HEIGHT}" \
  --data-urlencode "timeout=${TIMEOUT}" \
  --data-urlencode "encoding=png" \
  "${RENDERER_URL}")

if [[ "${HTTP_CODE}" != "200" ]]; then
  echo "Renderer returned HTTP ${HTTP_CODE}. Check Renderer logs and that GRAFANA_URL is reachable from the Renderer (e.g. http://grafana:3000 when both in Docker)." >&2
  if [[ -f "${OUTPUT}" ]]; then
    head -c 500 "${OUTPUT}" | cat -v >&2
    echo "" >&2
  fi
  exit 1
fi

# Basic check: PNG (file(1) is more reliable than grep on binary; works across environments)
if ! file -b "${OUTPUT}" | grep -q 'PNG'; then
  echo "Output is not a valid PNG (Renderer may have returned an error page)." >&2
  head -c 500 "${OUTPUT}" | cat -v >&2
  echo "" >&2
  exit 1
fi

echo "Saved PNG to ${OUTPUT}" >&2
