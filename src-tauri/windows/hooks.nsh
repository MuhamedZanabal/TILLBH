; TILLBH NSIS installer hooks.
; Business data lives in %ProgramData%\TILLBH\data and is NEVER removed by
; install, upgrade or uninstall. Only the program files are managed here.

!macro NSIS_HOOK_POSTINSTALL
  ; Create the shared data directory up front with explicit permissions: the till
  ; may be used under different Windows accounts, so local Users may modify files
  ; inside it (inheritance disabled; SYSTEM and Administrators keep full control).
  ; Access to business functions is controlled by TILLBH staff PINs and roles.
  ; NSIS has no $COMMONAPPDATA constant: resolve %ProgramData% the same way
  ; the app does (src-tauri/src/lib.rs, data_dir), keeping $R9 intact.
  Push $R9
  ReadEnvStr $R9 PROGRAMDATA
  CreateDirectory "$R9\TILLBH\data"
  nsExec::Exec 'icacls "$R9\TILLBH" /inheritance:r /grant:r "*S-1-5-18:(OI)(CI)F" "*S-1-5-32-544:(OI)(CI)F" "*S-1-5-32-545:(OI)(CI)M"'
  Pop $R9
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
  Push $R9
  ReadEnvStr $R9 PROGRAMDATA
  DetailPrint "Business data in $R9\TILLBH is kept. Delete it manually only after taking a backup."
  Pop $R9
!macroend
