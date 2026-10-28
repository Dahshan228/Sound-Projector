; Sound Projector installer. Build it with installer\build.ps1, which passes AppVersion and AppIcon
; and downloads VB-CABLE into target\vbcable.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Keep AppId the same forever: it's how a new installer recognises an existing install.
AppId={{94FB8615-2B2A-40CD-8AC5-FEAAE91290BF}
AppName=Sound Projector
AppVersion={#AppVersion}
AppPublisher=Dahshan228
AppPublisherURL=https://github.com/Dahshan228/Sound-Projector
; Per-user install: no admin prompt for the app itself (VB-CABLE asks for its own).
PrivilegesRequired=lowest
DefaultDirName={autopf}\Sound Projector
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
; The installer can't replace the exe while the app runs; the app holds this mutex.
AppMutex=SoundProjector.SingleInstance
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=SoundProjectorSetup
SetupIconFile={#AppIcon}
UninstallDisplayIcon={app}\Sound Projector.exe
UninstallDisplayName=Sound Projector
WizardStyle=modern
Compression=lzma2
SolidCompression=yes

[Messages]
; The only page Setup shows, so the VB-CABLE credit lives here.
ReadyLabel1=This installs Sound Projector and adds it to your Desktop and Start menu.
ReadyLabel2b=If VB-CABLE isn't installed yet, Setup installs it too. Sound Projector uses it as a virtual mic, so friends can hear your songs.%n%nVB-CABLE is a free virtual audio cable by VB-Audio Software (vb-cable.com). It's donationware: if you find it useful, you can donate on their site. Installing it means you accept VB-Audio's license, which is in "VB-CABLE readme.txt" next to the app.%n%nWindows will ask you to allow the VB-CABLE driver, and then to restart.%n%nClick Install to continue.

[Files]
Source: "..\target\release\sound-projector.exe"; DestDir: "{app}"; DestName: "Sound Projector.exe"; Flags: ignoreversion
Source: "..\target\vbcable\readme.txt"; DestDir: "{app}"; DestName: "VB-CABLE readme.txt"; Flags: ignoreversion
Source: "..\target\vbcable\*"; DestDir: "{tmp}\vbcable"; Flags: deleteafterinstall recursesubdirs

[Icons]
Name: "{autodesktop}\Sound Projector"; Filename: "{app}\Sound Projector.exe"
Name: "{autoprograms}\Sound Projector"; Filename: "{app}\Sound Projector.exe"

[Run]
Filename: "{app}\Sound Projector.exe"; Description: "Open Sound Projector"; Flags: nowait postinstall

[Code]
var
  CableInstalled: Boolean;

// The app reports whether Windows already has the VB-CABLE devices (exit code 0).
function HasCable: Boolean;
var
  Code: Integer;
begin
  Result := Exec(ExpandConstant('{app}\Sound Projector.exe'), '--has-vbcable', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  if (CurStep = ssPostInstall) and not HasCable then
  begin
    WizardForm.StatusLabel.Caption := 'Installing VB-CABLE by VB-Audio...';
    // VB-CABLE's own setup needs admin, so this shows the Windows permission prompt. -i -h installs without its window.
    if ShellExec('runas', ExpandConstant('{tmp}\vbcable\VBCABLE_Setup_x64.exe'), '-i -h', ExpandConstant('{tmp}\vbcable'), SW_SHOW, ewWaitUntilTerminated, Code) then
      CableInstalled := True
    else
      MsgBox('VB-CABLE wasn''t installed, so friends won''t hear Sound Projector yet. You can install it later from vb-cable.com; the setup guide in Sound Projector''s Settings has the link.', mbInformation, MB_OK);
  end;
end;

// VB-CABLE needs a restart before its devices work.
function NeedRestart: Boolean;
begin
  Result := CableInstalled;
end;
