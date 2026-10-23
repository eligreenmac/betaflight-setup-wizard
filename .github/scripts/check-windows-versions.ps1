# SPDX-License-Identifier: GPL-3.0-or-later
# Assert the numeric VERSIONINFO (fixed FileVersion and ProductVersion) of built Windows
# binaries equals the version derived from the app version (X.Y.Z-N -> X.Y.Z.N,
# X.Y.Z -> X.Y.Z.0; check-release-version.mjs output windows_version).
# Usage: check-windows-versions.ps1 -Expected 0.1.0.1 -Path setup.exe, app.exe
param(
  [Parameter(Mandatory)] [string] $Expected,
  [Parameter(Mandatory)] [string[]] $Path
)
$ErrorActionPreference = 'Stop'
if ($Expected -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw "bad expected version '$Expected'" }
$bad = 0
foreach ($p in $Path) {
  $f = Get-Item -LiteralPath $p
  $vi = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($f.FullName)
  $file = '{0}.{1}.{2}.{3}' -f $vi.FileMajorPart, $vi.FileMinorPart, $vi.FileBuildPart, $vi.FilePrivatePart
  $product = '{0}.{1}.{2}.{3}' -f $vi.ProductMajorPart, $vi.ProductMinorPart, $vi.ProductBuildPart, $vi.ProductPrivatePart
  Write-Host "$($f.Name): numeric FileVersion $file, ProductVersion $product (strings: FileVersion '$($vi.FileVersion)', ProductVersion '$($vi.ProductVersion)'); expected $Expected"
  if ($file -ne $Expected) { Write-Host "::error::$($f.Name) numeric FileVersion $file, expected $Expected"; $bad++ }
  if ($product -ne $Expected) { Write-Host "::error::$($f.Name) numeric ProductVersion $product, expected $Expected"; $bad++ }
}
if ($bad) { throw "$bad Windows version mismatch(es)" }
