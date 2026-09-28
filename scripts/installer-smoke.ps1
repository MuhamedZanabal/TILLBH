# Installer smoke test, part 1 (Windows). Installs the built NSIS installer
# silently, starts the INSTALLED app with WebView2 remote debugging and drives
# its UI with scripts/installer-smoke.mjs, checks the database, the build
# commit and the logs, relaunches it, reinstalls over it, and uninstalls it.
# Business data in %ProgramData%\AMWAPOS must survive every step.
#
#   pwsh scripts/installer-smoke.ps1 -Installer <path to AMWAPOS_<ver>_x64-setup.exe> -ExpectSha <git sha>
param(
  [Parameter(Mandatory)] [string] $Installer,
  [Parameter(Mandatory)] [string] $ExpectSha,
  [string] $Shots = "installer-smoke"
)
$ErrorActionPreference = 'Stop'
function Fail($m) { throw "INSTALLER SMOKE FAILED: $m" }
function Step($m) { Write-Host "== $m" }

$exe = Get-Item $Installer
$version = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
$installDir = Join-Path $env:ProgramFiles 'AMWAPOS'
$app = Join-Path $installDir 'amwapos.exe'
$dataRoot = Join-Path $env:ProgramData 'AMWAPOS'
$db = Join-Path $dataRoot 'data\amwapos.db'
New-Item -ItemType Directory -Force $Shots | Out-Null

Step "Artifact"
$hash = (Get-FileHash $exe.FullName -Algorithm SHA256).Hash.ToLower()
Write-Host "file:    $($exe.Name)"
Write-Host "size:    $($exe.Length) bytes"
Write-Host "sha256:  $hash"
if ($exe.Name -ne "AMWAPOS_${version}_x64-setup.exe") { Fail "unexpected installer name $($exe.Name) for version $version" }
$sig = Get-AuthenticodeSignature $exe.FullName
Write-Host "signature: $($sig.Status)"

Step "Packaged files (no session, secrets, test data)"
$listing = & 7z l $exe.FullName | Out-String
if ($LASTEXITCODE -ne 0) { Fail "7-Zip could not list the installer" }
foreach ($bad in @('session\.db', '\.env\b', 'test-results', '\.amwapos-e2e', '\.amwapos-smoke', 'playwright', '\.pfx\b', 'id_rsa')) {
  if ($listing -match $bad) { Fail "the installer contains '$bad'" }
}
Write-Host ($listing -split "`n" | Select-String -Pattern '\.(exe|dll|traineddata)\s*$' | Out-String)

Step "Silent install"
if (Test-Path $dataRoot) { Fail "a previous AMWAPOS data folder exists on this machine" }
$p = Start-Process $exe.FullName -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { Fail "installer exit code $($p.ExitCode)" }
if (-not (Test-Path $app)) { Fail "amwapos.exe not installed in $installDir" }
$pv = (Get-Item $app).VersionInfo.ProductVersion
Write-Host "installed: $app (product version $pv)"
if (-not $pv.StartsWith($version)) { Fail "installed version $pv does not match $version" }
if (-not (Test-Path (Join-Path $dataRoot 'data'))) { Fail "the installer did not create the data folder" }

Step "Secret scan of the installed program"
$text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($app))
foreach ($pat in @('sk-ant-[A-Za-z0-9_-]{20,}', 'AIzaSy[A-Za-z0-9_-]{30,}', 'sk-or-v1-[a-f0-9]{20,}', '-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----', '/home/user/AMWAPOS')) {
  if ($text -match $pat) { Fail "the program contains a secret or developer path matching '$pat'" }
}

function Start-App {
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
  Start-Process $app | Out-Null
}
function Stop-App {
  # A normal close (WM_CLOSE), not a kill.
  & taskkill /IM amwapos.exe | Out-Null
  for ($i = 0; $i -lt 60; $i++) {
    if (-not (Get-Process amwapos -ErrorAction SilentlyContinue)) { return }
    Start-Sleep -Milliseconds 500
  }
  Fail "the app did not close normally within 30 s"
}
function Run-Ui($mode) {
  $env:SMOKE_SHOTS = $Shots
  node scripts/installer-smoke.mjs $mode
  if ($LASTEXITCODE -ne 0) { Fail "UI checks failed ($mode)" }
}

Step "First start: setup, catalogue, product pictures, WhatsApp, till, sale"
Start-App
Run-Ui first
Stop-App

Step "Database, migrations, build commit, logs"
$check = @"
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
v = c.execute('SELECT MAX(version) FROM schema_migrations').fetchone()[0]
p = c.execute("SELECT COUNT(*) FROM products WHERE name='Smoke Juice 250ml' AND image_hash IS NOT NULL").fetchone()[0]
s = c.execute('SELECT COUNT(*) FROM sales').fetchone()[0]
ok = c.execute('PRAGMA integrity_check').fetchone()[0]
print(f'schema={v} products={p} sales={s} integrity={ok}')
sys.exit(0 if (v >= 21 and p == 1 and s >= 1 and ok == 'ok') else 1)
"@
$check | Set-Content "$env:RUNNER_TEMP\dbcheck.py"
python "$env:RUNNER_TEMP\dbcheck.py" $db
if ($LASTEXITCODE -ne 0) { Fail "database check failed" }
$logs = Get-ChildItem (Join-Path $dataRoot 'logs') -Filter 'amwapos*.log' | Get-Content -Raw
$short = $ExpectSha.Substring(0, 12)
if ($logs -notmatch '"message":"AMWAPOS starting"') { Fail "no startup line in the log" }
if ($logs -notmatch "`"build`":`"$short`"") { Fail "the installed app was not built from $short" }
if ($logs -match 'startup failed') { Fail "the log reports a startup failure" }
Write-Host "log: started, build $short"

Step "Relaunch: data kept"
Start-App
Run-Ui relaunch
Stop-App

Step "Reinstall over the existing installation"
$p = Start-Process $exe.FullName -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { Fail "reinstall exit code $($p.ExitCode)" }
Start-App
Run-Ui relaunch
Stop-App

Step "Uninstall: program removed, business data kept"
$un = Join-Path $installDir 'uninstall.exe'
if (-not (Test-Path $un)) { Fail "no uninstaller" }
Start-Process $un -ArgumentList '/S' -Wait | Out-Null
for ($i = 0; $i -lt 60 -and (Test-Path $app); $i++) { Start-Sleep -Milliseconds 500 }
if (Test-Path $app) { Fail "amwapos.exe is still installed after uninstall" }
if (-not (Test-Path $db)) { Fail "uninstall removed the business database" }
Write-Host "INSTALLER SMOKE PASSED: $($exe.Name) sha256=$hash"
