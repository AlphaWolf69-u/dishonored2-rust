param(
    [string]$SourceDirectory = $PSScriptRoot,
    [string]$TargetDirectory = (Join-Path $PSScriptRoot 'target'),
    [switch]$Offline,
    [switch]$InstallOnly
)
$ErrorActionPreference = 'Stop'
$targetRoot = [IO.Path]::GetFullPath($TargetDirectory)
$releaseDirectory = Join-Path $targetRoot 'release'
$gameExecutable = Join-Path $releaseDirectory 'dishonored2-rust.exe'
$pendingExecutable = Join-Path $releaseDirectory 'dishonored2-rust.next.exe'

if (!$InstallOnly) {
    $sourceRoot = (Resolve-Path -LiteralPath $SourceDirectory).Path
    $snapshot = Join-Path ([IO.Path]::GetTempPath()) ('dh2-release-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $snapshot | Out-Null
    foreach ($item in @('Cargo.toml', 'Cargo.lock', 'src', 'assets')) {
        Copy-Item -LiteralPath (Join-Path $sourceRoot $item) -Destination $snapshot -Recurse
    }
    $manifestPath = Join-Path $snapshot 'Cargo.toml'
    $manifest = [IO.File]::ReadAllText($manifestPath)
    $gameTarget = [regex]::new('(\[\[bin\]\]\s*name\s*=\s*)"dishonored2-rust"')
    if ($gameTarget.Matches($manifest).Count -ne 1) { throw 'Expected one game binary target' }
    $manifest = $gameTarget.Replace($manifest, '$1"dishonored2-rust-build"', 1)
    $manifest = $manifest.Replace('path = "vendor/', 'path = "' + $sourceRoot.Replace('\', '/') + '/vendor/')
    [IO.File]::WriteAllText($manifestPath, $manifest)
    $previousTargetDirectory = $env:CARGO_TARGET_DIR
    try {
        $env:CARGO_TARGET_DIR = $targetRoot
        $arguments = @('build', '--manifest-path', $manifestPath, '--locked', '--release', '--features', 'game', '--bin', 'dishonored2-rust-build')
        if ($Offline) { $arguments += '--offline' }
        & cargo @arguments
        if ($LASTEXITCODE -ne 0) { throw "Release game build failed with exit code $LASTEXITCODE" }
    } finally {
        $env:CARGO_TARGET_DIR = $previousTargetDirectory
    }
    # Keep the snapshot: CARGO_MANIFEST_DIR locates runtime assets there.
    [IO.File]::Copy((Join-Path $releaseDirectory 'dishonored2-rust-build.exe'), $pendingExecutable, $true)
}

if (!(Test-Path -LiteralPath $pendingExecutable)) { throw "No pending release: $pendingExecutable" }
try {
    if (Test-Path -LiteralPath $gameExecutable) {
        [IO.File]::Replace($pendingExecutable, $gameExecutable, [NullString]::Value)
    } else {
        [IO.File]::Move($pendingExecutable, $gameExecutable)
    }
} catch {
    $installError = $_.Exception.InnerException
    if (!$installError) { $installError = $_.Exception }
    if (($installError.HResult -band 0xffff) -notin @(5, 32, 33)) { throw }
    $running = Get-Process -Name dishonored2-rust -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $gameExecutable }
    if (!$running) { throw }
    Write-Output "Release built: $pendingExecutable"
    Write-Output 'The game is running. After closing it, run build-game.ps1 -InstallOnly to install this release.'
    return
}
Write-Output "Release game ready: $gameExecutable"
