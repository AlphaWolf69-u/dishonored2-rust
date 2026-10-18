$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $project
try {
    cargo build --locked --release --features game --bin dishonored2-rust
    if ($LASTEXITCODE -ne 0) { throw "Release game build failed with exit code $LASTEXITCODE" }
    $source = Join-Path $project 'target/release/dishonored2-rust.exe'
    Write-Output "Release game ready: $source"
} finally {
    Pop-Location
}
