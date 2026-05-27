; NSIS hooks for Nidavellir - Windows Service + optional PawnIO bundled installer

!macro NidavellirFindServiceExe OUT_VAR
  FindFirst $0 ${OUT_VAR} "$INSTDIR\nidavellir-service-*.exe"
  FindClose $0
!macroend

!macro NidavellirStopService
  nsExec::ExecToLog "sc stop NidavellirCore"
  Pop $0
  Sleep 1500
  nsExec::ExecToLog "sc delete NidavellirCore"
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro NidavellirStopService
  !insertmacro NidavellirFindServiceExe $R0
  StrCmp $R0 "" +3 0
    nsExec::ExecToLog "cmd /c taskkill /F /IM $R0 /T"
    Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro NidavellirFindServiceExe $R0
  StrCmp $R0 "" service_missing 0
    StrCpy $R1 "$INSTDIR\$R0"
    nsExec::ExecToLog "sc create NidavellirCore binPath= \"$R1\" start= auto DisplayName= \"Nidavellir Core Service\""
    Pop $0
    nsExec::ExecToLog "sc description NidavellirCore \"Privileged hardware access for Nidavellir (MSR, PawnIO).\""
    Pop $0
    nsExec::ExecToLog "sc start NidavellirCore"
    Pop $0
    Goto pawnio_prompt
  service_missing:
    MessageBox MB_ICONEXCLAMATION "Nidavellir Core Service binary was not found in the install folder. Reinstall or contact support."

  pawnio_prompt:
  IfFileExists "$INSTDIR\resources\third_party\pawnio\PawnIO-Setup.exe" 0 skip_pawnio
    MessageBox MB_YESNO|MB_ICONQUESTION "Install the bundled PawnIO kernel driver now?$\r$\n(Recommended for CPU MSR access. Requires a reboot if Windows prompts.)" IDYES run_pawnio IDNO skip_pawnio
  run_pawnio:
    ExecWait '"$INSTDIR\resources\third_party\pawnio\PawnIO-Setup.exe" /S' $0
  skip_pawnio:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro NidavellirStopService
  !insertmacro NidavellirFindServiceExe $R0
  StrCmp $R0 "" +3 0
    nsExec::ExecToLog "cmd /c taskkill /F /IM $R0 /T"
    Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; PawnIO uninstall is separate product - we do not remove it automatically
!macroend
