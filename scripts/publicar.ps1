# Publica una versión nueva de Kairo en GitHub (Releases). Los Kairo instalados la detectan solos (una vez
# por día, o con "Buscar ahora" en Acerca de) y avisan junto al reloj.
#
# Antes: subir "version" en Cargo.toml (por ejemplo 0.7.0 -> 0.7.1) y hacer commit.
# Uso:   powershell -ExecutionPolicy Bypass -File scripts\publicar.ps1 [-Notas "qué trae"] [-NotasArchivo notas.md]
#
# Sube la rama, arma dist\KairoSetup-<versión>.exe y su .sha256, y crea el release v<versión> con los dos.
# Si no se pasan notas, se arman con los títulos de los commits desde la versión anterior.
param([string]$Notas, [string]$NotasArchivo)

$ErrorActionPreference = 'Stop'
$raiz = Split-Path -Parent $PSScriptRoot
Push-Location $raiz
try {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "Falta GitHub CLI (gh): https://cli.github.com" }
    if (git status --porcelain) { throw "Hay cambios sin commit: haz commit antes de publicar." }
    # Notas largas (con tablas o imágenes) desde un archivo UTF-8.
    if ($NotasArchivo) { $Notas = [IO.File]::ReadAllText((Resolve-Path $NotasArchivo), [Text.Encoding]::UTF8) }

    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $tag = "v$version"
    git fetch --tags --quiet
    if (git tag --list $tag) { throw "La versión $tag ya está publicada: sube 'version' en Cargo.toml." }

    # Notas: los commits desde el último release (o los últimos 15 si es el primero). git escribe en UTF-8:
    # sin esto, Windows PowerShell rompe los acentos ("Grabaci├│n").
    if (-not $Notas) {
        [Console]::OutputEncoding = [Text.Encoding]::UTF8
        # (En Windows PowerShell, un error de git con 2>$null corta el script: por eso el try.)
        $anterior = try { git describe --tags --abbrev=0 2>$null } catch { $null }
        $rango = if ($anterior) { "$anterior..HEAD" } else { "-15" }
        $Notas = (git log $rango --no-merges --format='- %s') -join "`n"
    }

    git push origin HEAD
    if ($LASTEXITCODE -ne 0) { throw "No se pudo subir la rama." }

    & (Join-Path $PSScriptRoot 'instalador.ps1')
    $exe = "dist\KairoSetup-$version.exe"
    $hash = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
    # Formato de sha256sum: "<hash>  <archivo>".
    [IO.File]::WriteAllText((Join-Path $raiz "$exe.sha256"), "$hash  KairoSetup-$version.exe`n")

    # Las notas van en un archivo (UTF-8 sin BOM): Windows PowerShell parte los argumentos que tienen comillas.
    $archivoNotas = Join-Path $raiz "dist\notas-$version.md"
    [IO.File]::WriteAllText($archivoNotas, $Notas, (New-Object Text.UTF8Encoding $false))
    gh release create $tag $exe "$exe.sha256" --title "Kairo $version" --notes-file $archivoNotas --target (git rev-parse HEAD)
    if ($LASTEXITCODE -ne 0) { throw "No se pudo crear el release." }
    git fetch --tags --quiet
    Write-Host "Publicado: Kairo $version ($tag). Los Kairo instalados lo van a detectar solos."
} finally {
    Pop-Location
}
