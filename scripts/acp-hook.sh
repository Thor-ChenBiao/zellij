#!/bin/bash
# ACP Hook — writes structured tool call events to the ACP room streams.
#
# Works with: Claude Code, Codex CLI, Gemini CLI
# Safe outside Zellij: silently exits if ZELLIJ_AGENT_SHARED_ID is not set.
#
# Receives JSON on stdin from the provider's hook system.
# Writes to:
#   - events.log / events.jsonl  (all tool calls)
#   - code.log / code.jsonl      (Edit/Write with file content)

# --- Guard: only run inside an ACP room ---
ROOM_ID="${ZELLIJ_AGENT_SHARED_ID:-}"
[ -z "$ROOM_ID" ] && exit 0

# --- Resolve ACP cache directory ---
if [ "$(uname)" = "Darwin" ]; then
    ACP_ROOT="$HOME/Library/Caches/org.Zellij-Contributors.Zellij/agent-control-plane"
else
    ACP_ROOT="${XDG_CACHE_HOME:-$HOME/.cache}/zellij/agent-control-plane"
fi
ROOM_DIR="$ACP_ROOT/$ROOM_ID"
[ ! -d "$ROOM_DIR" ] && exit 0

# --- Read hook JSON from stdin ---
HOOK_JSON=$(cat)
[ -z "$HOOK_JSON" ] && exit 0

# --- Parse fields ---
TOOL=$(echo "$HOOK_JSON" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    # Claude Code: tool_name, Codex: tool_name, Gemini: function_name or tool_name
    tool = d.get('tool_name') or d.get('function_name') or d.get('tool', {}).get('name', 'unknown')
    print(tool)
except:
    print('unknown')
" 2>/dev/null)

HOOK_EVENT=$(echo "$HOOK_JSON" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    # Claude: hook_type or event, Codex: event, Gemini: event
    print(d.get('hook_type') or d.get('event') or 'unknown')
except:
    print('unknown')
" 2>/dev/null)

PROVIDER="${ZELLIJ_AGENT_PROVIDER:-unknown}"
ENDPOINT="${ZELLIJ_AGENT_ENDPOINT_ID:-unknown}"
NOW=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# --- Write to events stream ---
EVENT_LINE="$NOW | $PROVIDER | hook | tool_call | $TOOL ($HOOK_EVENT)"
echo "$EVENT_LINE" >> "$ROOM_DIR/events.log"

EVENT_JSON=$(python3 -c "
import sys, json
hook = json.loads('''$HOOK_JSON''') if '''$HOOK_JSON''' else {}
event = {
    'time': '$NOW',
    'stream': 'events',
    'kind': 'tool_call',
    'shared_id': '$ROOM_ID',
    'endpoint_id': '$ENDPOINT',
    'provider': '$PROVIDER',
    'role': '${ZELLIJ_AGENT_ROLE:-driver}',
    'message': '$TOOL ($HOOK_EVENT)',
    'payload': {
        'tool': '$TOOL',
        'hook_event': '$HOOK_EVENT',
        'source': 'provider_hook',
        'hook_data': hook,
    }
}
print(json.dumps(event))
" 2>/dev/null)
[ -n "$EVENT_JSON" ] && echo "$EVENT_JSON" >> "$ROOM_DIR/events.jsonl"

# --- For Edit/Write: also write to code stream ---
case "$TOOL" in
    Edit|Write|MultiEdit|edit|write|multi_edit)
        FILE_PATH=$(echo "$HOOK_JSON" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    # Try various field names
    inp = d.get('tool_input') or d.get('input') or d.get('args') or {}
    if isinstance(inp, str):
        import json as j
        inp = j.loads(inp)
    path = inp.get('file_path') or inp.get('path') or inp.get('filename') or ''
    print(path)
except:
    print('')
" 2>/dev/null)

        if [ -n "$FILE_PATH" ]; then
            # Read file content for code stream
            if [ -f "$FILE_PATH" ]; then
                LINES=$(wc -l < "$FILE_PATH" | tr -d ' ')
                PREVIEW=$(head -60 "$FILE_PATH" | cat -n)
                CODE_LINE="$NOW | $PROVIDER | code_change | ━━━ $TOOL: $FILE_PATH ($LINES lines) ━━━"
                echo "" >> "$ROOM_DIR/code.log"
                echo "$CODE_LINE" >> "$ROOM_DIR/code.log"
                echo "$PREVIEW" >> "$ROOM_DIR/code.log"
            else
                CODE_LINE="$NOW | $PROVIDER | code_change | ━━━ $TOOL: $FILE_PATH (file not found) ━━━"
                echo "$CODE_LINE" >> "$ROOM_DIR/code.log"
            fi

            CODE_JSON=$(python3 -c "
import sys, json
event = {
    'time': '$NOW',
    'stream': 'code',
    'kind': 'code_change',
    'shared_id': '$ROOM_ID',
    'endpoint_id': '$ENDPOINT',
    'provider': '$PROVIDER',
    'role': '${ZELLIJ_AGENT_ROLE:-driver}',
    'message': '$TOOL: $FILE_PATH',
    'payload': {
        'tool': '$TOOL',
        'file_path': '$FILE_PATH',
        'source': 'provider_hook',
    }
}
print(json.dumps(event))
" 2>/dev/null)
            [ -n "$CODE_JSON" ] && echo "$CODE_JSON" >> "$ROOM_DIR/code.jsonl"
        fi
        ;;
esac

exit 0
