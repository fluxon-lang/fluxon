# Fluxon installer for Windows (PowerShell).
#
#   irm https://raw.githubusercontent.com/fluxon-lang/fluxon/master/install.ps1 | iex
#
# Downloads the latest (or a pinned) release binary, verifies its SHA-256
# checksum, and installs it to %LOCALAPPDATA%\Fluxon\bin, adding that directory
# to the user PATH.
#
# Env knobs:
#   $env:FLUXON_VERSION       pin a version, e.g. v0.1.0 (default: latest)
#   $env:FLUXON_INSTALL_DIR   install target (default: %LOCALAPPDATA%\Fluxon\bin)

$ErrorActionPreference = 'Stop'
$Repo    = 'fluxon-lang/fluxon'
$BinName = 'fluxon'

function Info($msg) { Write-Host "fluxon " -ForegroundColor DarkGray -NoNewline; Write-Host $msg }
function Ok($msg)   { Write-Host "$([char]0x2713) " -ForegroundColor Green -NoNewline; Write-Host $msg }
function Die($msg)  { Write-Host "error: " -ForegroundColor Red -NoNewline; Write-Host $msg; exit 1 }

# --- detect arch ------------------------------------------------------------
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
  'AMD64' { 'x86_64' }
  'ARM64' { 'aarch64' }
  default { Die "unsupported architecture '$($env:PROCESSOR_ARCHITECTURE)'." }
}

# --- resolve version --------------------------------------------------------
$version = $env:FLUXON_VERSION
if (-not $version) {
  Info 'resolving the latest release...'
  try {
    $rel = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest" `
      -Headers @{ 'User-Agent' = 'fluxon-installer' }
    $version = $rel.tag_name
  } catch {
    Die "could not determine the latest release. Set `$env:FLUXON_VERSION='vX.Y.Z' and retry."
  }
}
$tag = if ($version.StartsWith('v')) { $version } else { "v$version" }

$asset   = "$BinName-$tag-windows-$arch.zip"
$baseUrl = "https://github.com/$Repo/releases/download/$tag"

Info "installing fluxon $tag (windows/$arch)"

# --- download into a temp dir ----------------------------------------------
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("fluxon-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
try {
  $zipPath = Join-Path $tmp $asset
  Info "downloading $asset"
  try {
    Invoke-WebRequest "$baseUrl/$asset" -OutFile $zipPath -UseBasicParsing
  } catch {
    Die "download failed. Is '$tag' published for windows/$arch? See https://github.com/$Repo/releases"
  }

  # --- verify checksum (best-effort) ----------------------------------------
  try {
    $sumsPath = Join-Path $tmp 'SHA256SUMS.txt'
    Invoke-WebRequest "$baseUrl/SHA256SUMS.txt" -OutFile $sumsPath -UseBasicParsing
    $line = (Get-Content $sumsPath | Where-Object { $_ -match [Regex]::Escape($asset) + '$' } | Select-Object -First 1)
    if ($line) {
      $expected = ($line -split '\s+')[0].ToLower()
      $actual   = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower()
      if ($expected -ne $actual) { Die "checksum mismatch for $asset - refusing to install." }
      Ok 'checksum verified'
    }
  } catch { } # no checksum file published yet -> skip

  # --- unpack ---------------------------------------------------------------
  Expand-Archive -Path $zipPath -DestinationPath $tmp -Force
  $exe = Join-Path $tmp "$BinName.exe"
  if (-not (Test-Path $exe)) { Die "archive did not contain $BinName.exe." }

  # --- install --------------------------------------------------------------
  $installDir = $env:FLUXON_INSTALL_DIR
  if (-not $installDir) { $installDir = Join-Path $env:LOCALAPPDATA 'Fluxon\bin' }
  New-Item -ItemType Directory -Path $installDir -Force | Out-Null
  Copy-Item $exe (Join-Path $installDir "$BinName.exe") -Force
  Ok "installed $BinName to $installDir\$BinName.exe"

  # --- add to user PATH -----------------------------------------------------
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (($userPath -split ';') -notcontains $installDir) {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
    Info "added $installDir to your user PATH - open a new terminal for it to take effect."
  }
} finally {
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ''
Ok "run $BinName --help to get started"
