#!/usr/bin/env bash
set -e
URL="https://raw.githubusercontent.com/jm-observer/workspace-system-prompt/main/mcp-tool/AGENTS.md"
TARGET="AGENTS.md"
# Directly download to target location (current directory)
curl -fsSL "$URL" -o "$TARGET"
if [ $? -ne 0 ]; then
  echo "Download failed"
  exit 1
fi
echo "AGENTS.md updated"