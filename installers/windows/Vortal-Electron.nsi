Unicode true
Name "Vortal"
Caption "Instalador do Vortal"
OutFile "/tmp/vortal-dist/windows-electron/Vortal-Setup-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\Vortal"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 32
Icon "..\..\crates\vortal-desktop\assets\vortal.ico"
UninstallIcon "..\..\crates\vortal-desktop\assets\vortal.ico"
VIProductVersion "0.1.0.0"
VIAddVersionKey "ProductName" "Vortal"
VIAddVersionKey "FileDescription" "Vortal Electron desktop app"
VIAddVersionKey "ProductVersion" "0.1.0"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section "Vortal"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File /r "..\..\dist\windows-electron\win-unpacked\*.*"
  CreateDirectory "$SMPROGRAMS\Vortal"
  CreateShortcut "$SMPROGRAMS\Vortal\Vortal.lnk" "$INSTDIR\Vortal.exe" "" "$INSTDIR\Vortal.exe"
  CreateShortcut "$SMPROGRAMS\Vortal\Desinstalar Vortal.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\Vortal.lnk" "$INSTDIR\Vortal.exe" "" "$INSTDIR\Vortal.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vortal" "DisplayName" "Vortal"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vortal" "DisplayVersion" "0.1.0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vortal" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vortal" "DisplayIcon" "$INSTDIR\Vortal.exe"
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$SMPROGRAMS\Vortal\Vortal.lnk"
  Delete "$SMPROGRAMS\Vortal\Desinstalar Vortal.lnk"
  RMDir "$SMPROGRAMS\Vortal"
  Delete "$DESKTOP\Vortal.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vortal"
  RMDir /r "$INSTDIR"
SectionEnd
