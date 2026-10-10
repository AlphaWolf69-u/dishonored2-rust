param(
    [string]$Project = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'
$output = Join-Path $Project 'analysis\decompiled'
$headless = 'D:\Programs\Ghidra\ghidra_12.1.4_PUBLIC\support\analyzeHeadless.bat'
$status = Join-Path $output 'pipeline-status.txt'
New-Item -ItemType Directory -Path $output -Force | Out-Null

try {
    [System.IO.File]::WriteAllText($status, "RUNNING: exporting Dishonored2.exe pseudocode and then indexing it.`n")
    $env:JAVA_HOME = 'D:\Programs\Java\jdk-21.0.12.1+1'
    & $headless (Join-Path $Project 'analysis\ghidra') 'Dishonored2' `
        -process 'Dishonored2.exe' -noanalysis -readOnly `
        -scriptPath (Join-Path $Project 'knowledge') `
        -postScript 'ExportDecomp.java' $output '0' `
        -log (Join-Path $output 'ghidra.log') `
        *> (Join-Path $output 'ghidra-console.log')
    if ($LASTEXITCODE -ne 0) { throw "Ghidra exited with code $LASTEXITCODE" }
    $exportStatus = Get-Content -LiteralPath (Join-Path $output 'status.txt') -Raw
    if (-not $exportStatus.StartsWith('COMPLETE:')) { throw "Export incomplete: $exportStatus" }
    & python (Join-Path $Project 'knowledge\db.py') index-tree $output --label dishonored2-exe `
        *> (Join-Path $output 'index.log')
    if ($LASTEXITCODE -ne 0) { throw "SQLite import exited with code $LASTEXITCODE" }
    [System.IO.File]::WriteAllText($status, "READY: Ghidra export complete and indexed in knowledge\dh2.sqlite.`n$exportStatus")
}
catch {
    [System.IO.File]::WriteAllText($status, "FAILED: $($_.Exception.Message)`nCheck ghidra.log, ghidra-console.log, and index.log.`n")
    exit 1
}
