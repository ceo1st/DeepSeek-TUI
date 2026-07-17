@echo off
setlocal

set "OHOS_LINKER_SCRIPT=%~dp0ohos-clang.ps1"
if not exist "%OHOS_LINKER_SCRIPT%" (
    echo error: OpenHarmony linker script is missing: %OHOS_LINKER_SCRIPT% 1>&2
    exit /b 1
)

"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%OHOS_LINKER_SCRIPT%" %*
exit /b %ERRORLEVEL%
