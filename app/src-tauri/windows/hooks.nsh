; Installer hooks (tauri.conf.json, bundle > windows > nsis > installerHooks).

; Utter registers itself to start at sign-in (Settings > Start with Windows). Uninstalling
; removes that entry, so nothing points at the removed program.
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Utter"
!macroend
