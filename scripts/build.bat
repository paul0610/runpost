@echo off
setlocal
cd /d "%~dp0.." || exit /b 1
where cargo >nul 2>nul || (echo [runpost] Falta Rust: https://rustup.rs 1>&2 & exit /b 1)
where npm >nul 2>nul || (echo [runpost] Falta Node.js LTS 1>&2 & exit /b 1)
pushd ui || exit /b 1
call npm ci --ignore-scripts || exit /b 1
call npm run build || exit /b 1
popd
cargo build --release --locked -p runpost-cli || exit /b 1
pushd apps\desktop || exit /b 1
call npm ci --ignore-scripts || exit /b 1
rem SPEC-DEVIATION: CLI Tauri local fijado en package-lock.json; mismo comando Rust que cargo tauri, sin instalacion global.
call npm run tauri -- build --no-bundle --ci -- --locked || exit /b 1
popd
echo [runpost] OK: target\release\runpost_mcp.exe y target\release\runpost_app.exe
