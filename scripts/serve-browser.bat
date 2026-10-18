@echo off
setlocal
cd /d "%~dp0.." || exit /b 1
if not exist target\release\runpost_mcp.exe (
  call scripts\build.bat || exit /b 1
)
if not defined RUNPOST_WORKSPACE set "RUNPOST_WORKSPACE=%CD%\workspace"
target\release\runpost_mcp.exe serve --browser --open --port 7878 %*
exit /b %ERRORLEVEL%
