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
; An update from inside Utter starts this installer, then Utter exits; the installer's own check
; for a running Utter came while it was still exiting, its forced close failed, and the update
; stopped on "Utter cannot be closed" with Utter already gone (Windows 11, 2026-10-20). During an
; update, wait here (up to 15 s) for every utter.exe to end before that check.
!macro NSIS_HOOK_PREINSTALL
  ${If} $UpdateMode = 1
    StrCpy $R9 0
    utter_wait_exit:
      nsExec::ExecToStack 'tasklist /FI "IMAGENAME eq ${MAINBINARYNAME}.exe" /NH /FO CSV'
      Pop $R8 ; exit code
      Pop $R7 ; output: a CSV line per process, or "INFO: No tasks..."
      StrCpy $R6 $R7 1
      ${If} $R6 == '"'
      ${AndIf} $R9 < 30
        Sleep 500
        IntOp $R9 $R9 + 1
        Goto utter_wait_exit
      ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$LOCALAPPDATA\Utter"
  ${EndIf}
!macroend
