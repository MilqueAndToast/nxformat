param([switch]$Test)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    if ($Test) {
        & cargo +1.95.0 test --locked --manifest-path packer/Cargo.toml --lib
        if ($LASTEXITCODE -ne 0) { throw 'Packer tests failed' }
        & cargo +1.95.0 test --locked --release --manifest-path app/Cargo.toml
        if ($LASTEXITCODE -ne 0) { throw 'GUI tests failed' }
    }
    & cargo +1.95.0 build --locked --release --manifest-path app/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
    $package = Join-Path $PSScriptRoot 'dist-release/NXConvert-Local-Decryption'
    New-Item -ItemType Directory -Path $package -Force | Out-Null
    Copy-Item -LiteralPath 'app/target/release/nxconvert.exe' -Destination $package
    foreach ($document in @('LOCAL-DECRYPTION.md','TESTING.md','LICENSE')) {
        Copy-Item -LiteralPath $document -Destination $package
    }
    $hash = Get-FileHash -LiteralPath (Join-Path $package 'nxconvert.exe') -Algorithm SHA256
    Set-Content -LiteralPath (Join-Path $package 'SHA256SUMS.txt') -Value ($hash.Hash.ToLowerInvariant() + '  nxconvert.exe')
    $revision = if (Test-Path -LiteralPath '.git') { (& git rev-parse HEAD) } else { 'Source archive (see its filename for revision)' }
    @('NXConvert Local Decryption', ('Source revision: ' + $revision), ('Compiler: ' + (& rustc +1.95.0 --version)), 'Target: x86_64-pc-windows-msvc', ('Built UTC: ' + [DateTime]::UtcNow.ToString('o'))) | Set-Content -LiteralPath (Join-Path $package 'BUILD.txt')
    # Package an allowlist, never arbitrary files someone placed beside the EXE.
    $packageFiles = @('nxconvert.exe','LOCAL-DECRYPTION.md','TESTING.md','LICENSE','SHA256SUMS.txt','BUILD.txt') | ForEach-Object { Join-Path $package $_ }
    Compress-Archive -LiteralPath $packageFiles -DestinationPath (Join-Path $PSScriptRoot 'dist-release/NXConvert-Local-Decryption-Windows-x64.zip') -Force
    Write-Output "Built $package"
} finally {
    Pop-Location
}
