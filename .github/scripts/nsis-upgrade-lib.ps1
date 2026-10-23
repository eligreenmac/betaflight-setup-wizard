# SPDX-License-Identifier: GPL-3.0-or-later
# Helpers for .github/workflows/nsis-upgrade.yml: silent NSIS installs and assertions on
# the resulting uninstall entry. Dot-source it: . ./.github/scripts/nsis-upgrade-lib.ps1
$ErrorActionPreference = 'Stop'
$script:Product = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).productName
$script:UninstallRoots = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
                         'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
                         'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'

function Get-Entries {
  @(foreach ($r in $script:UninstallRoots) {
    if (Test-Path $r) {
      Get-ChildItem $r | Get-ItemProperty | Where-Object { $_.DisplayName -eq $script:Product }
    }
  })
}

# Runs "<setup> /S" and returns its exit code.
function Install-Silent([string]$setup) {
  $p = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
  Write-Host "$(Split-Path -Leaf $setup) /S -> exit $($p.ExitCode)"
  $p.ExitCode
}

# The WinUSB helper the app runs elevated (built by .github/actions/build-wdi-simple,
# bundled via tauri.conf.json resources "drivers/*"), relative to the install dir.
$script:HelperRelPath = 'drivers\bfsw-wdi-simple.exe'

# SHA-256 of $relPath inside the NSIS installer $setup (7-Zip reads NSIS; extracted once
# per installer).
function Get-InstallerFileHash([string]$setup, [string]$relPath) {
  $x = Join-Path $env:RUNNER_TEMP ("extract-" + [IO.Path]::GetFileNameWithoutExtension($setup))
  if (-not (Test-Path $x)) {
    7z x $setup "-o$x" -y | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "7z could not extract $setup" }
  }
  $f = Join-Path $x $relPath
  if (-not (Test-Path -LiteralPath $f -PathType Leaf)) { throw "'$relPath' is not in $(Split-Path -Leaf $setup)" }
  (Get-FileHash -LiteralPath $f).Hash
}

# Asserts that $relPath exists under $installDir and is byte-identical to the copy
# inside $setup. Returns its SHA-256.
function Assert-InstalledFile([string]$v, [string]$setup, [string]$installDir, [string]$relPath) {
  $path = Join-Path $installDir $relPath
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "after $v`: $path missing" }
  $hash = (Get-FileHash -LiteralPath $path).Hash
  $want = Get-InstallerFileHash $setup $relPath
  if ($hash -ne $want) { throw "after $v`: installed $relPath ($hash) is not the one in $(Split-Path -Leaf $setup) ($want)" }
  $hash
}

# Asserts exactly one install entry with DisplayVersion $v, and that the app exe and the
# WinUSB helper are installed and byte-identical to the copies inside $setup. Returns the
# installed app exe's SHA-256.
function Assert-Installed([string]$v, [string]$setup) {
  $e = Get-Entries
  if ($e.Count -ne 1) { throw "after $v`: expected 1 install entry, found $($e.Count): $($e.PSPath -join '; ')" }
  $e = $e[0]
  if ($e.DisplayVersion -ne $v) { throw "after $v`: DisplayVersion is '$($e.DisplayVersion)'" }
  $dir = $e.InstallLocation.Trim('"')
  $hash = Assert-InstalledFile $v $setup $dir $e.MainBinaryName
  $exe = Join-Path $dir $e.MainBinaryName
  $vi = (Get-Item $exe).VersionInfo
  Write-Host "OK $v`: $($e.PSChildName) DisplayVersion=$($e.DisplayVersion) exe=$exe sha256=$hash ProductVersion=$($vi.ProductVersion) numeric=$($vi.FileMajorPart).$($vi.FileMinorPart).$($vi.FileBuildPart).$($vi.FilePrivatePart)"
  $hh = Assert-InstalledFile $v $setup $dir $script:HelperRelPath
  Write-Host "OK $v helper: $(Join-Path $dir $script:HelperRelPath) sha256=$hh matches $(Split-Path -Leaf $setup)"
  $hash
}

# X.Y.Z[-N] -> sortable key; a release (no -N) sorts after all its pre-releases.
function Get-VersionKey([string]$v) {
  if ($v -notmatch '^(\d+)\.(\d+)\.(\d+)(?:-(\d+))?$') { return $null }
  $pre = if ($Matches[4]) { [int]$Matches[4] } else { 99999 }
  '{0:D5}.{1:D5}.{2:D5}.{3:D5}' -f [int]$Matches[1], [int]$Matches[2], [int]$Matches[3], $pre
}
