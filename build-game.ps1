$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $project
try {
    cargo build --locked --release --features game --bin dishonored2-rust
    if ($LASTEXITCODE -ne 0) { throw "Release game build failed with exit code $LASTEXITCODE" }
    $source = Join-Path $project 'target/release/dishonored2-rust.exe'
    New-Item -ItemType Directory -Path (Join-Path $project 'bin') -Force | Out-Null
    $destination = Join-Path $project 'bin/dishonored2-rust.exe'
    Copy-Item -LiteralPath $source -Destination $destination -Force
    $built = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    $shipped = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
    if ($built -ne $shipped) { throw 'The copied game differs from the release build' }
    Write-Output "Release game ready: $destination"
} finally {
    Pop-Location
}
