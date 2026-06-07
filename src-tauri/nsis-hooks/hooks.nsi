; -----------------------------------------------------------------------------
; Unduhin NSIS installer hooks.
;
; Tauri's NSIS bundler includes this file inside the generated installer
; (see `bundle.windows.nsis.installerHooks` in tauri.conf.json) and calls
; the documented macro hooks during install/uninstall.
;
; The installer wires the per-browser Native Messaging registry keys and:
; (1) drops the host binary + manifest at `$INSTDIR\native-host\` via
;     `bundle.resources` in tauri.conf.json, and
; (2) rewrites the manifest's `"path": "PLACEHOLDER_ABS_PATH"` to the
;     real absolute path so Chrome can locate the binary.
;
; To register a different host name later: change `NM_HOST_NAME` below.
; -----------------------------------------------------------------------------

!define NM_HOST_NAME "com.unduhin.host"
!define NM_MANIFEST_REL "native-host\${NM_HOST_NAME}.json"
!define NM_HOST_BIN_REL "native-host\unduhin-native-host.exe"

; -------------------------------------------------------------------
; Substring replace. Stack on entry (top → bottom):
;   REPLACEMENT
;   NEEDLE
;   HAYSTACK
; Leaves the rewritten string on top of the stack.
; -------------------------------------------------------------------
Function UnduhinStrRep
    Exch $R4 ; REPLACEMENT
    Exch
    Exch $R3 ; NEEDLE
    Exch 2
    Exch $R2 ; HAYSTACK
    Push $R5
    Push $R6
    Push $R7
    Push $R8

    StrLen $R5 $R3
    StrCpy $R7 0
    StrCpy $R8 ""

    sr_loop:
        StrCpy $R6 $R2 $R5 $R7
        StrCmp $R6 "" sr_done
        StrCmp $R6 $R3 sr_hit
        StrCpy $R6 $R2 1 $R7
        StrCpy $R8 "$R8$R6"
        IntOp $R7 $R7 + 1
        Goto sr_loop
    sr_hit:
        StrCpy $R8 "$R8$R4"
        IntOp $R7 $R7 + $R5
        Goto sr_loop
    sr_done:
    StrCpy $R2 $R8

    Pop $R8
    Pop $R7
    Pop $R6
    Pop $R5
    Pop $R4
    Pop $R3
    Exch $R2
FunctionEnd

; -------------------------------------------------------------------
; In-place text replace for a small file (≤ a few KB). Slurps the
; whole file, runs UnduhinStrRep, writes the result back. Used to
; rewrite the host manifest's PLACEHOLDER_ABS_PATH to the install path.
; -------------------------------------------------------------------
; UNIQ must be a string that's unique per !insertmacro invocation in
; the enclosing scope. Pass `${__LINE__}` at the call site — it
; evaluates once there, producing one stable value all four labels
; below share. Using `${__LINE__}` *inside* this macro body doesn't
; work: NSIS evaluates it per source line, so each label below would
; get a different number and the Goto/IfErrors references wouldn't
; resolve.
!macro UnduhinReplaceInFile FILE NEEDLE REPLACEMENT UNIQ
    Push $0 ; file handle
    Push $1 ; accumulated content
    Push $2 ; chunk
    Push $3 ; rewritten
    Push $4 ; captured REPLACEMENT (preserved across the $1 file-read)
    ; Capture REPLACEMENT before $1 is clobbered by the file read below.
    ; The caller passes "$1" here; once `StrCpy $1 ""` and the read loop
    ; run, `${REPLACEMENT}` (which expanded to the literal `$1`) would
    ; otherwise resolve to the file's own contents.
    StrCpy $4 "${REPLACEMENT}"

    ClearErrors
    FileOpen $0 "${FILE}" r
    IfErrors uri_done_${UNIQ}

    StrCpy $1 ""
    uri_read_${UNIQ}:
        FileRead $0 $2
        IfErrors uri_eof_${UNIQ}
        StrCpy $1 "$1$2"
        Goto uri_read_${UNIQ}
    uri_eof_${UNIQ}:
    FileClose $0

    Push "$1"
    Push "${NEEDLE}"
    Push "$4"
    Call UnduhinStrRep
    Pop $3

    ClearErrors
    FileOpen $0 "${FILE}" w
    IfErrors uri_done_${UNIQ}
    FileWrite $0 "$3"
    FileClose $0

    uri_done_${UNIQ}:
    Pop $4
    Pop $3
    Pop $2
    Pop $1
    Pop $0
!macroend

; -------------------------------------------------------------------
; Terminate running Unduhin processes so Windows releases the lock on
; their .exe images. This matters because the native-messaging host is
; owned by the *browser* (a long-lived `connectNative` port keeps it
; alive), not by the app or the installer. Without this:
;   - on uninstall, the still-running `native-host\unduhin-native-host.exe`
;     leaves an orphaned process AND the locked binary on disk — deleting
;     the registry keys alone only stops *future* launches; and
;   - on upgrade, the staged host binary can't be overwritten.
;
; Process images: `Unduhin.exe` is the installed/bundled main app (Tauri
; renames the `unduhin-app` cargo bin to `productName`); `unduhin-app.exe`
; is the dev-build name, killed too so a dev machine cleans up. taskkill
; exits non-zero when a process isn't running — we Pop and ignore it.
; `nsExec::Exec` runs hidden (no console flash), unlike the `Exec` instr.
; -------------------------------------------------------------------
!macro UnduhinKillProcesses
    DetailPrint "Stopping any running Unduhin processes..."
    Push $0
    nsExec::Exec 'taskkill /F /IM "unduhin-native-host.exe"'
    Pop $0
    nsExec::Exec 'taskkill /F /IM "Unduhin.exe"'
    Pop $0
    nsExec::Exec 'taskkill /F /IM "unduhin-app.exe"'
    Pop $0
    Pop $0
!macroend

; Tauri's per-user NSIS install runs under HKCU; HKLM keys would require
; elevation. The browsers below all honour HKCU for native messaging.

; Fired before files are staged. On an upgrade the previous host process
; is still alive (browser keeps the port open), so kill it first or the
; resource copy fails on the locked binary.
!macro NSIS_HOOK_PREINSTALL
    !insertmacro UnduhinKillProcesses
!macroend

!macro NSIS_HOOK_POSTINSTALL
    DetailPrint "Registering Native Messaging hooks for supported browsers..."
    StrCpy $0 "$INSTDIR\${NM_MANIFEST_REL}"
    WriteRegStr HKCU "Software\Google\Chrome\NativeMessagingHosts\${NM_HOST_NAME}" "" "$0"
    WriteRegStr HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NM_HOST_NAME}" "" "$0"
    WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\${NM_HOST_NAME}" "" "$0"

    DetailPrint "Rewriting host manifest path to $INSTDIR\${NM_HOST_BIN_REL}..."
    ; JSON requires backslashes to be escaped, so the raw install path
    ; must be turned from "C:\foo\bar" into "C:\\foo\\bar" before we
    ; inject it into the manifest's `"path"` value.
    Push "$INSTDIR\${NM_HOST_BIN_REL}"
    Push "\"
    Push "\\"
    Call UnduhinStrRep
    Pop $1
    !insertmacro UnduhinReplaceInFile "$INSTDIR\${NM_MANIFEST_REL}" "PLACEHOLDER_ABS_PATH" "$1" "${__LINE__}"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
    ; Kill the browser-owned host (and the app) first: the host's binary
    ; is locked while it runs, so without this the uninstaller can't
    ; delete it and an orphan process survives the uninstall.
    !insertmacro UnduhinKillProcesses
    DetailPrint "Removing Native Messaging hooks..."
    DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\${NM_HOST_NAME}"
    DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NM_HOST_NAME}"
    DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\${NM_HOST_NAME}"
!macroend
