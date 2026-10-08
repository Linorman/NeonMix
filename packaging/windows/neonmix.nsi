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
Var OldInstallDir

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

!macro StopNeonMix
  nsExec::ExecToLog '"$PLUGINSDIR\neonmix-maintenance.exe" --shutdown-installation --installation-root "$INSTDIR" --state-dir "$LOCALAPPDATA\NeonMix"'
  Pop $0
  ${If} $0 != 0
    IfSilent +2
    MessageBox MB_OK|MB_ICONSTOP "本安装实例未能完整停止。请退出该实例后重试；程序文件尚未替换。"
    SetErrorLevel 30
    Abort
  ${EndIf}
!macroend

!include "LogicLib.nsh"

Function .onInit
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=neonmix-maintenance.exe "${SOURCE}\..\neonmix-maintenance.exe"
FunctionEnd

Function un.onInit
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=neonmix-maintenance.exe "${SOURCE}\..\neonmix-maintenance.exe"
FunctionEnd

Section "NeonMix"
  ReadRegStr $OldInstallDir HKCU "Software\NeonMix" "InstallDir"
  ${If} $OldInstallDir != ""
  ${AndIf} $OldInstallDir != $INSTDIR
    nsExec::ExecToLog '"$PLUGINSDIR\neonmix-maintenance.exe" --shutdown-installation --installation-root "$OldInstallDir" --state-dir "$LOCALAPPDATA\NeonMix"'
    Pop $0
    ${If} $0 != 0
      SetErrorLevel 30
      Abort "旧安装实例未能完整停止，请退出后重试。"
    ${EndIf}
  ${EndIf}
  ; Refuse an occupied unmarked directory before any recursive deletion.
  IfFileExists "$INSTDIR\*.*" 0 new_install
  IfFileExists "$INSTDIR\neonmix-installation.json" existing_install
  IfFileExists "$INSTDIR\NeonMix.exe" existing_install
  IfSilent +2
  MessageBox MB_OK|MB_ICONSTOP "安装目录已有其他文件，请选择 NeonMix 专属目录。"
  SetErrorLevel 31
  Abort
existing_install:
  !insertmacro StopNeonMix
new_install:
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\plugins"
  RMDir /r "$INSTDIR\airplay"
  SetOutPath "$INSTDIR"
  File /r "${SOURCE}\*.*"
  ; Preserve network-instance.ini across same-directory upgrades. It is not
  ; included in the stage, and its GUID never depends on version or username.
  DetailPrint "允许同子网设备在公用、专用和域网络连接 NeonMix；网络规则维护会单独请求提升权限。"
  nsExec::ExecToLog '"$INSTDIR\bin\neonmix-network-helper.exe" install'
  Pop $1
  ${If} $1 != 0
    IfSilent +2
    MessageBox MB_OK|MB_ICONINFORMATION "程序已安装，网络配置未完成。可运行安装目录下 bin\neonmix-network-helper.exe repair 重新配置。此配置允许同子网设备在公用、专用和域网络连接。"
    SetErrorLevel 20
  ${EndIf}
  ${If} $1 == 0
  ${AndIf} $OldInstallDir != ""
  ${AndIf} $OldInstallDir != $INSTDIR
    IfFileExists "$OldInstallDir\bin\neonmix-network-helper.exe" 0 old_rules_done
    nsExec::ExecToLog '"$OldInstallDir\bin\neonmix-network-helper.exe" remove'
    Pop $0
    ${If} $0 != 0
      DetailPrint "旧路径规则清理未完成，已保留原安装记录，可用原路径 helper remove 重试。"
      SetErrorLevel 20
    ${EndIf}
old_rules_done:
  ${EndIf}
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\NeonMix" "InstallDir" "$INSTDIR"
  CreateShortcut "$SMPROGRAMS\NeonMix.lnk" "$INSTDIR\NeonMix.exe"
  CreateShortcut "$SMPROGRAMS\NeonMix 网络修复.lnk" "$INSTDIR\bin\neonmix-network-helper.exe" "repair"
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
  nsExec::ExecToLog '"$INSTDIR\bin\neonmix-network-helper.exe" remove'
  Pop $1
  ${If} $1 != 0
    ; Keep the exact helper and instance record for an administrator retry.
    IfSilent +2
    MessageBox MB_OK|MB_ICONINFORMATION "网络规则清理未完成，已保留实例信息。请重试卸载；用户配对资料未删除。"
    SetErrorLevel 20
    Abort
  ${EndIf}
  ReadRegStr $OldInstallDir HKCU "Software\NeonMix" "InstallDir"
  ${If} $OldInstallDir == $INSTDIR
    Delete "$SMPROGRAMS\NeonMix.lnk"
    Delete "$SMPROGRAMS\NeonMix 网络修复.lnk"
  ${EndIf}
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\plugins"
  RMDir /r "$INSTDIR\airplay"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\NeonMix.exe"
  Delete "$INSTDIR\neonmix-installation.json"
  Delete "$INSTDIR\network-instance.ini"
  Delete "$INSTDIR\network-journal.ini"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  ${If} $OldInstallDir == $INSTDIR
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
    DeleteRegKey HKCU "Software\NeonMix"
  ${EndIf}
SectionEnd
