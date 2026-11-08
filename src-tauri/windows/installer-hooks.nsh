!macro NSIS_HOOK_POSTINSTALL
  ; Remove the development probe bundled by older releases.
  Delete "$INSTDIR\m1-probe.exe"
!macroend
