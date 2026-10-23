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

# SHA-256 of the file called $name inside the NSIS installer $setup (7-Zip reads NSIS).
function Get-ExpectedExeHash([string]$setup, [string]$name) {
  $x = Join-Path $env:RUNNER_TEMP ("extract-" + [IO.Path]::GetFileNameWithoutExtension($setup))
  7z x $setup "-o$x" -y | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "7z could not extract $setup" }
  $f = @(Get-ChildItem -Recurse -File $x | Where-Object { $_.Name -eq $name })
  if ($f.Count -ne 1) { throw "found $($f.Count) '$name' in $setup" }
  (Get-FileHash $f[0].FullName).Hash
}

# Asserts exactly one install entry with DisplayVersion $v whose app exe is byte-identical
# to the one inside $setup. Returns the installed exe's SHA-256.
function Assert-Installed([string]$v, [string]$setup) {
  $e = Get-Entries
  if ($e.Count -ne 1) { throw "after $v`: expected 1 install entry, found $($e.Count): $($e.PSPath -join '; ')" }
  $e = $e[0]
  if ($e.DisplayVersion -ne $v) { throw "after $v`: DisplayVersion is '$($e.DisplayVersion)'" }
  $exe = Join-Path $e.InstallLocation.Trim('"') $e.MainBinaryName
  if (-not (Test-Path $exe)) { throw "after $v`: $exe missing" }
  $hash = (Get-FileHash $exe).Hash
  $want = Get-ExpectedExeHash $setup $e.MainBinaryName
  if ($hash -ne $want) { throw "after $v`: installed exe ($hash) is not the one in $(Split-Path -Leaf $setup) ($want)" }
  $vi = (Get-Item $exe).VersionInfo
  Write-Host "OK $v`: $($e.PSChildName) DisplayVersion=$($e.DisplayVersion) exe=$exe sha256=$hash ProductVersion=$($vi.ProductVersion) numeric=$($vi.FileMajorPart).$($vi.FileMinorPart).$($vi.FileBuildPart).$($vi.FilePrivatePart)"
  $hash
}

# X.Y.Z[-N] -> sortable key; a release (no -N) sorts after all its pre-releases.
function Get-VersionKey([string]$v) {
  if ($v -notmatch '^(\d+)\.(\d+)\.(\d+)(?:-(\d+))?$') { return $null }
  $pre = if ($Matches[4]) { [int]$Matches[4] } else { 99999 }
  '{0:D5}.{1:D5}.{2:D5}.{3:D5}' -f [int]$Matches[1], [int]$Matches[2], [int]$Matches[3], $pre
}
