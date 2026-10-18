@echo off
setlocal
cd /d "%~dp0.." || exit /b 1
if not exist target\release\runpost_app.exe (
  call scripts\build.bat || exit /b 1
)
if not defined RUNPOST_WORKSPACE set "RUNPOST_WORKSPACE=%CD%\workspace"
start "" target\release\runpost_app.exe %*
