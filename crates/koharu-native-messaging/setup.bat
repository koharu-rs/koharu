@echo off
setlocal enabledelayedexpansion

:: Determine script directory and manifest path dynamically
set "SCRIPT_DIR=%~dp0"
set "MANIFEST_PATH=%SCRIPT_DIR%com.koharu.native_host.json"

:: Determine repository root (2 levels up from crates\koharu-native-messaging)
for %%I in ("%SCRIPT_DIR%..\..") do set "REPO_ROOT=%%~fI"

:: Prefer release binary if built, otherwise fallback to debug
set "HOST_EXE=%REPO_ROOT%\target\release\koharu-native-messaging.exe"
if not exist "%HOST_EXE%" (
    set "HOST_EXE=%REPO_ROOT%\target\debug\koharu-native-messaging.exe"
)

echo Script Dir: %SCRIPT_DIR%
echo Host Binary: %HOST_EXE%
echo Manifest Path: %MANIFEST_PATH%

:: Dynamically update the absolute binary path in com.koharu.native_host.json
powershell -NoProfile -Command ^
    "$jsonPath = '%MANIFEST_PATH%';" ^
    "$exePath = '%HOST_EXE:\=\\%';" ^
    "$json = Get-Content $jsonPath -Raw | ConvertFrom-Json;" ^
    "$json.path = $exePath;" ^
    "$json | ConvertTo-Json -Depth 10 | Set-Content $jsonPath"

echo.
echo Registering Koharu Native Messaging Host with Google Chrome...
REG ADD "HKCU\Software\Google\Chrome\NativeMessagingHosts\com.koharu.native_host" /ve /t REG_SZ /d "%MANIFEST_PATH%" /f

echo Registering Koharu Native Messaging Host with Microsoft Edge...
REG ADD "HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.koharu.native_host" /ve /t REG_SZ /d "%MANIFEST_PATH%" /f

echo.
echo Done! Native Messaging Host successfully registered.
pause
