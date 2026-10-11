; Hooks for Tauri's NSIS installer (bundle.windows.nsis.installerHooks in tauri.conf.json).

; Install into a folder of our own. Tauri's template only defaults to "...\Kaanna" (productName,
; ASCII on purpose so paths stay plain): a folder
; chosen on the directory page (or passed with /D=) is used as is, so picking "D:\Apps" would
; spill the app's files straight into it. Runs before anything is copied; everything after
; (uninstaller, registry, shortcuts) uses $INSTDIR, so changing it here is enough.
;
; An existing install is left where it is, even one sitting directly in a folder like that:
; the installer pre-fills its location, and appending would leave a second copy behind.
!macro NSIS_HOOK_PREINSTALL
  Push $R0
  Push $R1

  ; Drop a trailing backslash (a drive root like "D:\") before comparing and appending.
  StrCpy $R0 "$INSTDIR" "" -1
  ${If} $R0 == "\"
    StrCpy $INSTDIR "$INSTDIR" -1
  ${EndIf}

  ${GetFileName} "$INSTDIR" $R0
  ReadRegStr $R1 SHCTX "${MANUPRODUCTKEY}" ""
  ${If} $R0 != "${PRODUCTNAME}"
  ${AndIf} $INSTDIR != $R1
  ${AndIfNot} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    StrCpy $INSTDIR "$INSTDIR\${PRODUCTNAME}"
    SetOutPath $INSTDIR
  ${EndIf}

  Pop $R1
  Pop $R0
!macroend
