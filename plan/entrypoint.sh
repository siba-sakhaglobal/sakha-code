#!/bin/sh
set -e

# ── Validate required env vars ────────────────────────────────
if [ -z "$ANTHROPIC_API_KEY" ] && [ -z "$OPENAI_COMPATIBLE_API_KEY" ] && [ -z "$OPENAI_API_KEY" ] && [ "${AUTH_PROVIDER:-token}" != "apikey" ]; then
  echo "ERROR: no API key is configured." >&2
  echo "" >&2
  echo "  docker run -p 3000:3000 -e ANTHROPIC_API_KEY=sk-ant-... agent-workbench-web" >&2
  echo "  docker run -p 3000:3000 -e CLAUDE_CODE_USE_OPENAI_COMPATIBLE=true -e OPENAI_COMPATIBLE_BASE_URL=https://api.x.ai/v1 -e OPENAI_COMPATIBLE_API_KEY=... -e OPENAI_COMPATIBLE_MODEL=grok-4 agent-workbench-web" >&2
  echo "" >&2
  echo "  Or via docker-compose with a .env file:" >&2
  echo "    ANTHROPIC_API_KEY=sk-ant-... docker-compose up" >&2
  echo "    CLAUDE_CODE_USE_OPENAI_COMPATIBLE=true OPENAI_COMPATIBLE_API_KEY=... docker-compose up" >&2
  exit 1
fi

# The API key is forwarded to child PTY processes via process.env,
# so the agent CLI will pick it up automatically — no config file needed.

echo "Agent Workbench Web Terminal starting on port ${PORT:-3000}..."
if [ -n "$AUTH_TOKEN" ]; then
  echo "  Auth token protection: enabled"
fi
if [ -n "$ALLOWED_ORIGINS" ]; then
  echo "  Allowed origins: $ALLOWED_ORIGINS"
fi
echo "  Max sessions: ${MAX_SESSIONS:-5}"

# Hand off to the PTY WebSocket server
exec bun /app/src/server/web/pty-server.ts
