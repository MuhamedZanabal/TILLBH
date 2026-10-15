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

$currentRepo = (Get-Location).Path
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

# Read-only, deterministic compatibility assertions used on both the predecessor
# DB and migrated DB. This never changes either database while comparing them.
$dbCheck = @"
import json, sqlite3, sys
path = sys.argv[1]
c = sqlite3.connect('file:' + path.replace('\\', '/') + '?mode=ro', uri=True)
problems = []
def one(sql, args=()): return c.execute(sql, args).fetchone()[0]
pid = c.execute("SELECT product_id FROM products WHERE name='Smoke Juice 250ml'").fetchone()
if not pid:
    problems.append('product missing')
else:
    pid = pid[0]
    barcode_count = one('SELECT COUNT(*) FROM product_barcodes WHERE product_id=?', (pid,))
    price_count = one('SELECT COUNT(*) FROM product_prices WHERE product_id=?', (pid,))
    current_price = one("SELECT amount_minor FROM product_prices WHERE product_id=? AND effective_to IS NULL ORDER BY effective_from DESC LIMIT 1", (pid,))
    stock = one('SELECT COALESCE(SUM(qty_milli),0) FROM stock_levels WHERE product_id=?', (pid,))
    movements = one('SELECT COUNT(*) FROM stock_movements WHERE product_id=?', (pid,))
    image_count = one('SELECT COUNT(*) FROM products WHERE product_id=? AND image_hash IS NOT NULL', (pid,))
    historic_sale_price = one('SELECT unit_price_minor FROM sale_items WHERE product_id=? ORDER BY line_no LIMIT 1', (pid,))
    if barcode_count != 2: problems.append(f'barcodes={barcode_count}, expected 2')
    if price_count < 2: problems.append(f'price history={price_count}, expected >=2')
    if current_price != 1375: problems.append(f'current price={current_price}, expected 1375')
    if historic_sale_price != 1250: problems.append(f'historic sale price={historic_sale_price}, expected 1250')
    if stock != 11000: problems.append(f'stock={stock}, expected 11000')
    if movements < 3: problems.append(f'movements={movements}, expected >=3')
    if image_count != 1: problems.append('product image metadata missing')
customers = one("SELECT COUNT(*) FROM customers WHERE name='Upgrade Customer'")
suppliers = one("SELECT COUNT(*) FROM suppliers WHERE name='Upgrade Supplier'")
users = one("SELECT COUNT(*) FROM users WHERE display_name='Zana'")
sales = one('SELECT COUNT(*) FROM sales')
receipts = one("SELECT COUNT(*) FROM sales WHERE receipt_number IS NOT NULL AND receipt_number<>''")
payments = one('SELECT COUNT(*) FROM payments')
open_shifts = one("SELECT COUNT(*) FROM shifts WHERE status='open'")
devices = one('SELECT COUNT(*) FROM devices')
backup_raw = c.execute("SELECT value_json FROM settings WHERE key='local.backup'").fetchone()
backup = json.loads(backup_raw[0]) if backup_raw else {}
integrity = one('PRAGMA integrity_check')
if customers != 1: problems.append(f'customers={customers}, expected 1')
if suppliers != 1: problems.append(f'suppliers={suppliers}, expected 1')
if users != 1: problems.append(f'users={users}, expected 1')
if sales < 1: problems.append('sale missing')
if receipts < 1: problems.append('receipt number missing')
if payments < 1: problems.append('payment missing')
if open_shifts < 1: problems.append('open shift missing')
if devices != 1: problems.append(f'devices={devices}, expected 1')
if backup.get('interval_hours') != 12 or backup.get('keep') != 17 or backup.get('automatic') is not True:
    problems.append(f'backup settings mismatch: {backup}')
if integrity != 'ok': problems.append(f'integrity={integrity}')
print(f'compat-check path={path} sales={sales} users={users} customers={customers} suppliers={suppliers} receipts={receipts} payments={payments} open_shifts={open_shifts} devices={devices} integrity={integrity}')
if problems:
    print('problems: ' + '; '.join(problems))
    sys.exit(1)
"@
$dbCheck | Set-Content "$env:RUNNER_TEMP\upgrade-db-check.py"

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

Step "Seed realistic predecessor data through installed UI/backend"
Start-InstalledApp $legacyApp
Run-Smoke $LegacyRepo 'first' 'legacy-first'
node scripts/upgrade-seed.mjs
if ($LASTEXITCODE -ne 0) { Fail "upgrade-specific predecessor seed failed" }
Stop-InstalledApp 'amwapos.exe' 'amwapos'
if (-not (Test-Path $legacyDb)) { Fail "legacy UI did not create amwapos.db" }
python "$env:RUNNER_TEMP\upgrade-db-check.py" $legacyDb
if ($LASTEXITCODE -ne 0) { Fail "pre-upgrade compatibility data check failed" }
$legacyDbHashBefore = (Get-FileHash $legacyDb -Algorithm SHA256).Hash
$legacyDataFileCount = (Get-ChildItem $legacyData -Recurse -File -ErrorAction SilentlyContinue | Measure-Object).Count
Write-Host "legacy database sha256 before upgrade: $legacyDbHashBefore"
Write-Host "legacy data files before upgrade: $legacyDataFileCount"

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
Run-Smoke $currentRepo 'relaunch' 'tillbh-after-upgrade'
Stop-InstalledApp 'tillbh.exe' 'tillbh'
if (-not (Test-Path $currentDbInLegacyDir)) { Fail "TILLBH did not create tillbh.db beside the deployed legacy store" }
if ((Get-FileHash $legacyDb -Algorithm SHA256).Hash -ne $legacyDbHashBefore) {
  Fail "legacy rollback database was modified during upgrade"
}
python "$env:RUNNER_TEMP\upgrade-db-check.py" $currentDbInLegacyDir
if ($LASTEXITCODE -ne 0) { Fail "post-upgrade compatibility data check failed" }

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
