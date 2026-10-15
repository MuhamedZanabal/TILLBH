; TILLBH NSIS installer hooks.
; Business data is NEVER removed by install, upgrade or uninstall. Clean installs
; use %ProgramData%\TILLBH\data; deployed predecessor stores may remain in their
; legacy ProgramData directory while TILLBH opens them through compatibility code.

!macro NSIS_HOOK_PREINSTALL
  ; Preserve scratch registers used by the surrounding Tauri NSIS template.
  Push $R5
  Push $R6
  Push $R7
  Push $R8

  ; Tauri's stock NSIS identity uses PRODUCTNAME for its uninstall registry key
  ; and install directory. Because the visible product name changed, explicitly
  ; hand off a deployed predecessor install before copying TILLBH files.
  ;
  ; Security: do not execute an arbitrary UninstallString from the registry.
  ; Read only the predecessor's own per-machine product key, whose default value
  ; is the unquoted install directory written by the deployed Tauri installer.
  ; Derive uninstall.exe from that trusted location and fail closed when registry
  ; metadata is inconsistent or the predecessor uninstaller cannot complete.
  ReadRegStr $R7 HKLM "Software\AMWAPOS\AMWAPOS" ""
  ReadRegStr $R5 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\AMWAPOS" "UninstallString"

  ${If} $R7 == ""
  ${AndIf} $R5 != ""
    MessageBox MB_ICONSTOP "A previous installation is registered, but its trusted install-location metadata is missing. TILLBH installation has stopped to avoid leaving conflicting installs. Business data was not deleted."
    Abort
  ${EndIf}

  ${If} $R7 != ""
    StrCpy $R8 "$R7\uninstall.exe"
    IfFileExists "$R8" legacy_uninstaller_found legacy_uninstaller_missing

    legacy_uninstaller_found:
      DetailPrint "Migrating the previous installation to TILLBH..."
      ExecWait '"$R8" /S' $R6
      ${If} $R6 != 0
        MessageBox MB_ICONSTOP "The previous installation could not be removed safely (exit code $R6). TILLBH installation has stopped; business data was not deleted."
        Abort
      ${EndIf}
      ; The stock predecessor uninstaller can leave its product-location metadata.
      ; Remove that installer-only key after successful uninstall; never touch the
      ; predecessor ProgramData directory or credential store here.
      DeleteRegKey HKLM "Software\AMWAPOS\AMWAPOS"
      Goto legacy_handoff_done

    legacy_uninstaller_missing:
      MessageBox MB_ICONSTOP "A previous installation was detected, but its expected uninstaller is missing. TILLBH installation has stopped to avoid leaving conflicting installs. Business data was not deleted."
      Abort

    legacy_handoff_done:
  ${EndIf}

  Pop $R8
  Pop $R7
  Pop $R6
  Pop $R5
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Create the shared TILLBH data directory up front with explicit permissions:
  ; the till may be used under different Windows accounts, so local Users may
  ; modify files inside it (inheritance disabled; SYSTEM and Administrators keep
  ; full control). An empty directory does not hide a detected legacy store.
  ; Access to business functions is controlled by TILLBH staff PINs and roles.
  ; NSIS has no $COMMONAPPDATA constant: resolve %ProgramData% the same way
  ; the app does (src-tauri/src/lib.rs, data_dir), keeping $R9 intact.
  Push $R9
  ReadEnvStr $R9 PROGRAMDATA
  CreateDirectory "$R9\TILLBH\data"
  nsExec::Exec 'icacls "$R9\TILLBH" /inheritance:r /grant:r "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-544:(OI)(CI)F" "*S-1-5-32-545:(OI)(CI)M"'
  Pop $R9

  ; Remove deployed predecessor rules before registering the renamed executable.
  ; These names are compatibility-only identifiers and never recreate old rules.
  nsExec::Exec 'netsh advfirewall firewall delete rule name="AMWAPOS Hub"'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="AMWAPOS Discovery"'

  ; One inbound rule only: the hub API (TCP 47800), private networks, bound to
  ; the TILLBH executable. Terminals join by entering the hub address shown on
  ; the hub's Sync page. WhatsApp runs inside TILLBH (outbound only) and OCR
  ; runs the bundled Tesseract as a child process; neither listens on a port.
  ; The UDP discovery rule of earlier versions is removed.
  nsExec::Exec 'netsh advfirewall firewall delete rule name="TILLBH Hub"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="TILLBH Hub" dir=in action=allow program="$INSTDIR\tillbh.exe" protocol=TCP localport=47800 profile=private enable=yes'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="TILLBH Discovery"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'netsh advfirewall firewall delete rule name="TILLBH Hub"'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="TILLBH Discovery"'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="AMWAPOS Hub"'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="AMWAPOS Discovery"'
  Push $R9
  ReadEnvStr $R9 PROGRAMDATA
  DetailPrint "Business data is kept. Delete TILLBH or legacy data manually only after taking a backup."
  Pop $R9
!macroend
