$ErrorActionPreference = 'Stop'
$reviewRepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location -LiteralPath $reviewRepoRoot
try {
    cargo build --workspace --locked
    if ($LASTEXITCODE -ne 0) { throw 'Workspace build failed' }
    $reviewPackLib = Get-ChildItem target/debug/deps/liboxidize_pack-*.rlib | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    $reviewTransportLib = Get-ChildItem target/debug/deps/liboxidize_transport-*.rlib | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    rustc --edition=2021 docs/review/parser_probe.rs -L dependency=target/debug/deps --extern "oxidize_pack=$($reviewPackLib.FullName)" --extern "oxidize_transport=$($reviewTransportLib.FullName)" -o target/review-parser.exe
    if ($LASTEXITCODE -ne 0) { throw 'Parser probe compilation failed' }
    & ./target/review-parser.exe
    if ($LASTEXITCODE -ne 0) { throw 'Parser probe exited unsuccessfully' }
} finally {
    Pop-Location
}
