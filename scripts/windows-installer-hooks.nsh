; The GNU executable needs this DLL alongside ruiss.exe. Read the build output
; only when NSIS runs, after Cargo has generated it. NSIS accepts cross-drive
; paths, unlike Tauri's resource path normalization which strips drive letters.
!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  File "/oname=WebView2Loader.dll" "$%CARGO_TARGET_DIR%\release\WebView2Loader.dll"
!macroend

; Run after Tauri has checked/closed the app, so the DLL is no longer locked.
!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\WebView2Loader.dll"
  RMDir "$INSTDIR"
!macroend
