@echo off
setlocal
cd /d "%~dp0.." || exit /b 1
if not exist target\release\runpost_mcp.exe (
  if not exist ui\dist\index.html (
    pushd ui || exit /b 1
    call npm ci --ignore-scripts 1>&2 || exit /b 1
    call npm run build 1>&2 || exit /b 1
    popd
  )
  cargo build --release --locked --quiet -p runpost-cli 1>&2 || exit /b 1
)
if not defined RUNPOST_WORKSPACE set "RUNPOST_WORKSPACE=%CD%\workspace"
target\release\runpost_mcp.exe mcp %*
exit /b %ERRORLEVEL%
