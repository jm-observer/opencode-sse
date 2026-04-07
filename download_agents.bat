@echo off
setlocal
set "URL=https://raw.githubusercontent.com/jm-observer/workspace-system-prompt/main/mcp-tool/AGENTS.md"
set "TARGET=AGENTS.md"
rem Directly download to target file in current directory
powershell -Command "Invoke-WebRequest -Uri \"%URL%\" -OutFile \"%TARGET%\" -ErrorAction Stop"
if errorlevel 1 (
  echo Download failed
  exit /b 1
)

echo AGENTS.md updated