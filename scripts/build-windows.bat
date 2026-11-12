@echo off
setlocal
cd /d "%~dp0.."
echo Vortal para Windows: app Rust
echo Requisitos: Rust 1.85+, ferramentas C++ do Visual Studio e NSIS.
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%CD%\scripts\build-windows.ps1"
if errorlevel 1 (
  echo.
  echo A compilacao falhou. Confira a mensagem acima.
  pause
  exit /b 1
)
echo.
echo Instalador pronto em dist\windows-native\Vortal-Setup-Rust-x64.exe
pause
