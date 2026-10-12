; Installer hooks (tauri.conf.json, bundle > windows > nsis > installerHooks).
;
; The uninstaller wears Utter's mark too (tauri.conf.json's installerIcon covers only the
; installer); this file is read before the pages are defined.
!ifndef MUI_UNICON
  !define MUI_UNICON "${__FILEDIR__}\..\icons\icon.ico"
!endif
;
; Tauri's uninstaller already removes Utter's start-at-sign-in entry, and skips that during an
; update so Start with Windows survives it. Its "delete the data" check box (relabelled in
; English.nsh) clears only the WebView's folder, named after the bundle id; Utter keeps its
; models, runtime and settings in %LOCALAPPDATA%\Utter (utter_core::data_dir), so that goes too.
; Never during an update, and never unless the person ticked the box: it is unticked by
; default, so a reinstall keeps gigabytes of models.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$LOCALAPPDATA\Utter"
  ${EndIf}
!macroend
