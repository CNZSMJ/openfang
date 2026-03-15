#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${MINIMAX_BASE_URL:-https://api.minimaxi.com/v1}"
API_KEY="${MINIMAX_API_KEY:-}"
MODEL="${MINIMAX_MODEL:-MiniMax-M2.5}"
PROMPT="${MINIMAX_PROMPT:-Reply with exactly: pong}"
TIMEOUT_SECONDS="${MINIMAX_TIMEOUT_SECONDS:-30}"

if [[ -z "$API_KEY" ]]; then
  echo "ERROR: MINIMAX_API_KEY is not set."
  exit 2
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "ERROR: curl is required."
  exit 2
fi

mask_key() {
  local key="$1"
  local n=${#key}
  if (( n <= 10 )); then
    printf "***"
    return
  fi
  printf "%s...%s(len=%d)" "${key:0:6}" "${key:n-4:4}" "$n"
}

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

echo "MiniMax OpenAI-compat probe"
echo "base_url: $BASE_URL"
echo "model: $MODEL"
echo "api_key: $(mask_key "$API_KEY")"
echo

fail=0

models_status="$(
  curl -sS -m "$TIMEOUT_SECONDS" \
    -H "Authorization: Bearer $API_KEY" \
    -o "$tmp_dir/models.json" \
    -w "%{http_code}" \
    "$BASE_URL/models"
)"
echo "GET /models -> HTTP $models_status"
if [[ "$models_status" == 2* ]]; then
  echo "models_probe: OK"
else
  echo "models_probe: non-blocking (MiniMax may not expose this endpoint in OpenAI mode)"
  echo "Response:"
  cat "$tmp_dir/models.json"
  echo
fi

chat_payload="$tmp_dir/chat_payload.json"
cat >"$chat_payload" <<EOF
{
  "model": "$MODEL",
  "messages": [
    {"role":"user","content":"$PROMPT"}
  ],
  "max_tokens": 16,
  "temperature": 0
}
EOF

chat_status="$(
  curl -sS -m "$TIMEOUT_SECONDS" \
    -H "Authorization: Bearer $API_KEY" \
    -H "Content-Type: application/json" \
    -o "$tmp_dir/chat.json" \
    -w "%{http_code}" \
    -d @"$chat_payload" \
    "$BASE_URL/chat/completions"
)"
echo "POST /chat/completions -> HTTP $chat_status"
if [[ "$chat_status" == 2* ]]; then
  if command -v jq >/dev/null 2>&1; then
    echo "assistant_content: $(jq -r '.choices[0].message.content // "<missing>"' "$tmp_dir/chat.json")"
    echo "finish_reason: $(jq -r '.choices[0].finish_reason // "<missing>"' "$tmp_dir/chat.json")"
    echo "usage: $(jq -c '.usage // {}' "$tmp_dir/chat.json")"
  else
    echo "Response snippet:"
    head -c 400 "$tmp_dir/chat.json"
    echo
  fi
else
  fail=1
  echo "Response:"
  cat "$tmp_dir/chat.json"
  echo
fi

echo
if (( fail == 0 )); then
  echo "PASS: MiniMax OpenAI-compatible endpoints are reachable."
else
  echo "FAIL: One or more probes failed."
  exit 1
fi
