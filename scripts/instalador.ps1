# Arma el instalador: compila la versión final y la copia como dist\KairoSetup-<versión>.exe.
# (El instalador es el mismo exe de Kairo: con ese nombre se abre la ventana de instalación.)
$ErrorActionPreference = 'Stop'
$raiz = Split-Path -Parent $PSScriptRoot
Push-Location $raiz
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "La compilación falló." }
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    New-Item -ItemType Directory -Force dist | Out-Null
    $destino = "dist\KairoSetup-$version.exe"
    Copy-Item target\release\kairo.exe $destino -Force
    Write-Host "Instalador listo: $destino"
} finally {
    Pop-Location
}
