#!/bin/sh
# Health check for oxicrab gateway.
# Checks the HTTP health endpoint. Used by Docker HEALTHCHECK and
# can be called remotely for dual-VPS monitoring.
set -e

PORT="${OXICRAB_PORT:-18790}"
URL="http://localhost:${PORT}/api/health"

response=$(curl -sf --max-time 5 "$URL" 2>/dev/null) || exit 1
echo "$response" | grep -q '"status":"ok"' || exit 1
