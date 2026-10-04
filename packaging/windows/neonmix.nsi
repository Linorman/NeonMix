; NeonMix per-user installer. Build with:
;   makensis /DVERSION=0.1.0 /DSOURCE=<staged dir> /DOUTFILE=<installer.exe> neonmix.nsi
Unicode true
!include "MUI2.nsh"

!define REPOSITORY "https://github.com/Linorman/NeonMix"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\NeonMix"

Name "NeonMix ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\NeonMix"
InstallDirRegKey HKCU "Software\NeonMix" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

!macro StopNeonMix
  nsExec::Exec 'taskkill /F /T /IM neonmix-desktop.exe'
  nsExec::Exec 'taskkill /F /T /IM neonmix-background.exe'
  nsExec::Exec 'taskkill /F /T /IM neonmix-hub.exe'
  nsExec::Exec 'taskkill /F /T /IM neonmix-airplay-worker.exe'
  Sleep 800
!macroend

Section "NeonMix"
  !insertmacro StopNeonMix
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\plugins"
  RMDir /r "$INSTDIR\airplay"
  SetOutPath "$INSTDIR"
  File /r "${SOURCE}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\NeonMix" "InstallDir" "$INSTDIR"
  CreateShortcut "$SMPROGRAMS\NeonMix.lnk" "$INSTDIR\NeonMix.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "NeonMix"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "NeonMix"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "${REPOSITORY}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\NeonMix.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

; User data (%LOCALAPPDATA%\NeonMix: identity, paired devices, logs) is kept
; on uninstall so reinstalling does not break existing pairings.
Section "Uninstall"
  !insertmacro StopNeonMix
  Delete "$SMPROGRAMS\NeonMix.lnk"
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\plugins"
  RMDir /r "$INSTDIR\airplay"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\NeonMix.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\NeonMix"
SectionEnd
