# Windows upgrade smoke: install the last pre-rename AMWAPOS build, seed a real
# store through its installed WebView UI, then install TILLBH over it and prove
# executable handoff, DB migration and business-data persistence.
param(
  [Parameter(Mandatory)] [string] $LegacyInstaller,
  [Parameter(Mandatory)] [string] $LegacyRepo,
  [Parameter(Mandatory)] [string] $CurrentInstaller,
  [string] $Shots = "upgrade-smoke"
)
$ErrorActionPreference = 'Stop'
function Fail($m) { throw "UPGRADE SMOKE FAILED: $m" }
function Step($m) { Write-Host "== $m" }

$legacyExe = Get-Item $LegacyInstaller
$currentExe = Get-Item $CurrentInstaller
$legacyInstallDir = Join-Path $env:ProgramFiles 'AMWAPOS'
$currentInstallDir = Join-Path $env:ProgramFiles 'TILLBH'
$legacyApp = Join-Path $legacyInstallDir 'amwapos.exe'
$currentApp = Join-Path $currentInstallDir 'tillbh.exe'
$legacyRoot = Join-Path $env:ProgramData 'AMWAPOS'
$currentRoot = Join-Path $env:ProgramData 'TILLBH'
$legacyData = Join-Path $legacyRoot 'data'
$legacyDb = Join-Path $legacyData 'amwapos.db'
$currentDbInLegacyDir = Join-Path $legacyData 'tillbh.db'
$legacyProductKey = 'HKLM:\Software\AMWAPOS\AMWAPOS'
$legacyUninstallKey = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\AMWAPOS'
New-Item -ItemType Directory -Force $Shots | Out-Null

function Start-InstalledApp([string] $Path) {
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
  Start-Process $Path | Out-Null
}
function Stop-InstalledApp([string] $Image, [string] $ProcessName) {
  & taskkill /IM $Image 2>$null | Out-Null
  for ($i = 0; $i -lt 60; $i++) {
    if (-not (Get-Process $ProcessName -ErrorAction SilentlyContinue)) { return }
    Start-Sleep -Milliseconds 500
  }
  Fail "$Image did not close within 30 seconds"
}
function Run-Smoke([string] $Repo, [string] $Mode, [string] $Prefix) {
  $old = $env:SMOKE_SHOTS
  $env:SMOKE_SHOTS = Join-Path (Resolve-Path $Shots) $Prefix
  New-Item -ItemType Directory -Force $env:SMOKE_SHOTS | Out-Null
  Push-Location $Repo
  try {
    node scripts/installer-smoke.mjs $Mode
    if ($LASTEXITCODE -ne 0) { Fail "$Prefix UI smoke failed ($Mode)" }
  } finally {
    Pop-Location
    $env:SMOKE_SHOTS = $old
  }
}

Step "Preconditions"
foreach ($path in @($legacyRoot, $currentRoot, $legacyInstallDir, $currentInstallDir)) {
  if (Test-Path $path) { Fail "runner is not clean; unexpected path exists: $path" }
}

Step "Install deployed AMWAPOS predecessor"
$p = Start-Process $legacyExe.FullName -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { Fail "legacy installer exit code $($p.ExitCode)" }
if (-not (Test-Path $legacyApp)) { Fail "legacy executable was not installed" }
if (-not (Test-Path $legacyProductKey)) { Fail "legacy trusted product-location registry key was not created" }
if (-not (Test-Path $legacyUninstallKey)) { Fail "legacy uninstall registry key was not created" }
$registeredLocation = (Get-Item $legacyProductKey).GetValue('')
if ([string]::IsNullOrWhiteSpace($registeredLocation)) { Fail "legacy product-location registry value is empty" }
if (([IO.Path]::GetFullPath($registeredLocation)).TrimEnd('\') -ne ([IO.Path]::GetFullPath($legacyInstallDir)).TrimEnd('\')) {
  Fail "legacy registry install location '$registeredLocation' does not match '$legacyInstallDir'"
}

Step "Seed realistic predecessor data through installed UI"
Start-InstalledApp $legacyApp
Run-Smoke $LegacyRepo 'first' 'legacy-first'
Stop-InstalledApp 'amwapos.exe' 'amwapos'
if (-not (Test-Path $legacyDb)) { Fail "legacy UI did not create amwapos.db" }
$legacyDbHashBefore = (Get-FileHash $legacyDb -Algorithm SHA256).Hash
$legacyDataFileCount = (Get-ChildItem $legacyData -Recurse -File -ErrorAction SilentlyContinue | Measure-Object).Count
Write-Host "legacy database sha256 before upgrade: $legacyDbHashBefore"
Write-Host "legacy data files before upgrade: $legacyDataFileCount"

$checkLegacy = @"
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
p = c.execute("SELECT COUNT(*) FROM products WHERE name='Smoke Juice 250ml'").fetchone()[0]
s = c.execute('SELECT COUNT(*) FROM sales').fetchone()[0]
u = c.execute("SELECT COUNT(*) FROM users WHERE name='Zana'").fetchone()[0]
ok = c.execute('PRAGMA integrity_check').fetchone()[0]
print(f'legacy products={p} sales={s} users={u} integrity={ok}')
sys.exit(0 if p == 1 and s >= 1 and u == 1 and ok == 'ok' else 1)
"@
$checkLegacy | Set-Content "$env:RUNNER_TEMP\upgrade-legacy-check.py"
python "$env:RUNNER_TEMP\upgrade-legacy-check.py" $legacyDb
if ($LASTEXITCODE -ne 0) { Fail "pre-upgrade legacy data check failed" }

Step "Install TILLBH over deployed predecessor"
$p = Start-Process $currentExe.FullName -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { Fail "TILLBH upgrade installer exit code $($p.ExitCode)" }
if (-not (Test-Path $currentApp)) { Fail "tillbh.exe was not installed" }
if (Test-Path $legacyApp) { Fail "legacy executable remains after handoff" }
if (Test-Path $legacyUninstallKey) { Fail "legacy Add/Remove Programs entry remains after handoff" }
if (Test-Path $legacyProductKey) { Fail "legacy installer location metadata remains after handoff" }
if (-not (Test-Path $legacyDb)) { Fail "upgrade removed the legacy rollback database" }
if (-not (Test-Path (Join-Path $currentRoot 'data'))) { Fail "TILLBH installer did not create its clean-install data directory" }

Step "First TILLBH launch performs safe DB migration and opens old store"
Start-InstalledApp $currentApp
Run-Smoke (Get-Location).Path 'relaunch' 'tillbh-after-upgrade'
Stop-InstalledApp 'tillbh.exe' 'tillbh'
if (-not (Test-Path $currentDbInLegacyDir)) { Fail "TILLBH did not create tillbh.db beside the deployed legacy store" }
if ((Get-FileHash $legacyDb -Algorithm SHA256).Hash -ne $legacyDbHashBefore) {
  Fail "legacy rollback database was modified during upgrade"
}

$checkCurrent = @"
import sqlite3, sys
legacy, current = sys.argv[1], sys.argv[2]
for label, path in [('legacy', legacy), ('current', current)]:
  c = sqlite3.connect(path)
  p = c.execute("SELECT COUNT(*) FROM products WHERE name='Smoke Juice 250ml'").fetchone()[0]
  s = c.execute('SELECT COUNT(*) FROM sales').fetchone()[0]
  u = c.execute("SELECT COUNT(*) FROM users WHERE name='Zana'").fetchone()[0]
  ok = c.execute('PRAGMA integrity_check').fetchone()[0]
  print(f'{label} products={p} sales={s} users={u} integrity={ok}')
  if not (p == 1 and s >= 1 and u == 1 and ok == 'ok'):
    sys.exit(1)
"@
$checkCurrent | Set-Content "$env:RUNNER_TEMP\upgrade-current-check.py"
python "$env:RUNNER_TEMP\upgrade-current-check.py" $legacyDb $currentDbInLegacyDir
if ($LASTEXITCODE -ne 0) { Fail "post-upgrade data check failed" }

Step "TILLBH uninstall keeps both rollback and migrated business data"
$un = Join-Path $currentInstallDir 'uninstall.exe'
if (-not (Test-Path $un)) { Fail "TILLBH uninstaller was not installed" }
$p = Start-Process $un -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { Fail "TILLBH uninstaller exit code $($p.ExitCode)" }
for ($i = 0; $i -lt 60 -and (Test-Path $currentApp); $i++) { Start-Sleep -Milliseconds 500 }
if (Test-Path $currentApp) { Fail "tillbh.exe remains after uninstall" }
if (-not (Test-Path $legacyDb)) { Fail "uninstall removed legacy rollback DB" }
if (-not (Test-Path $currentDbInLegacyDir)) { Fail "uninstall removed migrated TILLBH DB" }

Write-Host "UPGRADE SMOKE PASSED: $($legacyExe.Name) -> $($currentExe.Name)"
