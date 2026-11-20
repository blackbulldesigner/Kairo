# Arma las imágenes para promocionar Kairo (promo\*.png) a partir de promo\fuente\*.html, con Edge sin ventana.
# Los recursos (logo, aviso, cuadro de un clip y capturas) van en promo\recursos (no se suben: tienen cosas de la
# pantalla de la persona). Para rehacerlos: --exportar-icono, --exportar-aviso, --exportar-cuadro y --captura-ui con
# KAIRO_CAPTURA_ATAJO (ver CLAUDE.md).
$ErrorActionPreference = 'Stop'
$raiz = Split-Path -Parent $PSScriptRoot
$edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
if (-not (Test-Path $edge)) { $edge = "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe" }
$perfil = Join-Path $env:TEMP 'kairo-promo-edge'
$diseños = @(
    @{ n = 'a-cuadrada'; w = 1080; h = 1080 }, @{ n = 'a-ancha'; w = 1920; h = 1080 },
    @{ n = 'b-cuadrada'; w = 1080; h = 1080 }, @{ n = 'b-ancha'; w = 1920; h = 1080 },
    @{ n = 'c-cuadrada'; w = 1080; h = 1080 }, @{ n = 'c-ancha'; w = 1920; h = 1080 }
)
foreach ($d in $diseños) {
    $html = Join-Path $raiz "promo\fuente\$($d.n).html"
    $png = Join-Path $raiz "promo\kairo-1.0-$($d.n).png"
    $url = 'file:///' + ($html -replace '\\', '/')
    $p = Start-Process $edge -ArgumentList '--headless=new', '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=1',
        "--user-data-dir=`"$perfil`"", "--window-size=$($d.w),$($d.h)", '--virtual-time-budget=3000', "--screenshot=`"$png`"", $url -PassThru -WindowStyle Hidden
    $null = $p.WaitForExit(60000)
    Write-Host ("{0}: {1}" -f $d.n, $(if (Test-Path $png) { 'listo' } else { 'FALTA' }))
}
