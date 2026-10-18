@echo off
setlocal
cd /d "%~dp0.." || exit /b 1
where cargo >nul 2>nul || (echo [runpost] Falta Rust: https://rustup.rs 1>&2 & exit /b 1)
where npm >nul 2>nul || (echo [runpost] Falta Node.js LTS 1>&2 & exit /b 1)
if not defined RUNPOST_WORKSPACE set "RUNPOST_WORKSPACE=%CD%\workspace"
pushd ui || exit /b 1
call npm ci --ignore-scripts || exit /b 1
popd
pushd apps\desktop || exit /b 1
call npm ci --ignore-scripts || exit /b 1
call npm run tauri -- dev -- -- --dev %*
exit /b %ERRORLEVEL%
