Unicode true
Name "Vortal Rust"
Caption "Instalador do Vortal Rust"
OutFile "../../dist/windows-native/Vortal-Setup-Rust-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\VortalRust"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Icon "..\..\crates\vortal-desktop\assets\vortal.ico"
UninstallIcon "..\..\crates\vortal-desktop\assets\vortal.ico"
VIProductVersion "0.1.0.0"
VIAddVersionKey "ProductName" "Vortal"
VIAddVersionKey "FileDescription" "Vortal Rust desktop app"
VIAddVersionKey "ProductVersion" "0.1.0"
VIAddVersionKey "FileVersion" "0.1.0"
VIAddVersionKey "LegalCopyright" "Vortal contributors"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section "Vortal" InstallSection
  SetShellVarContext current
  SetRegView 64
  SetOutPath "$INSTDIR"
  File "..\..\target\x86_64-pc-windows-msvc\release\vortal-desktop.exe"
  File "..\..\target\x86_64-pc-windows-msvc\release\vortal.exe"
  File "..\..\target\x86_64-pc-windows-msvc\release\vortald.exe"
  File "..\..\target\x86_64-pc-windows-msvc\release\vortal-motion.exe"
  File "..\..\crates\vortal-desktop\assets\vortal.ico"
  File "..\..\README.md"
  File /oname=README-WINDOWS.md "README.md"
  File "..\..\LICENSE-APACHE"
  File "..\..\LICENSE-MIT"
  CreateDirectory "$SMPROGRAMS\Vortal Rust"
  CreateShortcut "$SMPROGRAMS\Vortal Rust\Vortal.lnk" "$INSTDIR\vortal-desktop.exe" "" "$INSTDIR\vortal.ico"
  CreateShortcut "$SMPROGRAMS\Vortal Rust\Desinstalar Vortal.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\Vortal Rust.lnk" "$INSTDIR\vortal-desktop.exe" "" "$INSTDIR\vortal.ico"
  StrCpy $0 "0"
  ReadRegDWORD $0 HKCU "Software\VortalRust" "StartupDisabled"
  StrCmp $0 "1" startup_done
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "VortalRust" '"$INSTDIR\vortal-desktop.exe" --background'
startup_done:
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VortalRust" "DisplayName" "Vortal Rust"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VortalRust" "DisplayVersion" "0.1.0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VortalRust" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VortalRust" "DisplayIcon" "$INSTDIR\vortal.ico"
SectionEnd

Section "Uninstall"
  SetRegView 64
  Delete "$SMPROGRAMS\Vortal Rust\Vortal.lnk"
  Delete "$SMPROGRAMS\Vortal Rust\Desinstalar Vortal.lnk"
  RMDir "$SMPROGRAMS\Vortal Rust"
  Delete "$DESKTOP\Vortal Rust.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VortalRust"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "VortalRust"
  DeleteRegKey HKCU "Software\VortalRust"
  Delete "$INSTDIR\vortal-desktop.exe"
  Delete "$INSTDIR\vortal.exe"
  Delete "$INSTDIR\vortald.exe"
  Delete "$INSTDIR\vortal-motion.exe"
  Delete "$INSTDIR\vortal.ico"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\README-WINDOWS.md"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
