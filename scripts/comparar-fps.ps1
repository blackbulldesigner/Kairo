# Compara cuánto le cuesta a un juego que lo graben, sin jugar: corre una escena de prueba siempre igual (pesada,
# en movimiento y sin límite de FPS) sin nada, con Kairo grabando, con Medal grabando y sin nada otra vez, y
# muestra los FPS y lo que usó cada grabador (procesador, tarjeta, encoder y RAM).
#
# Uso: powershell -ExecutionPolicy Bypass -File scripts\comparar-fps.ps1 [-Segundos 60] [-SinMedal]
# Dura unos 5 minutos y tapa la pantalla en cada ronda (Esc la corta). Medal tiene que estar instalado y configurado
# para grabar la pantalla completa. Kairo de la bandeja se cierra durante la prueba y se vuelve a abrir al final.
param([int]$Segundos = 60, [switch]$SinMedal)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$raiz = Split-Path -Parent $PSScriptRoot
Push-Location $raiz
$instalado = Join-Path $env:LOCALAPPDATA 'Programs\Kairo\Kairo.exe'
$habiaKairo = $false
try {
    Write-Host 'Compilando la prueba (la primera vez tarda unos minutos)...'
    # (cargo escribe su avance por la salida de errores: en Windows PowerShell eso cortaría el script.)
    $ErrorActionPreference = 'Continue'
    cargo build --release --features dev --target-dir target/medir 2>&1 | ForEach-Object { "$_" } | Select-Object -Last 1 | Out-Host
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0) { throw 'No se pudo compilar la prueba.' }
    $exe = Join-Path $raiz 'target\medir\release\kairo.exe'
    $dir = Join-Path $env:TEMP ('kairo-fps-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    New-Item -ItemType Directory -Force $dir | Out-Null

    # Que no grabe nada más: Kairo de la bandeja se cierra (vuelve al final) y Medal tiene que estar cerrado.
    $bandeja = @(Get-Process kairo -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $instalado })
    if ($bandeja.Count -gt 0) {
        $habiaKairo = $true
        Write-Host 'Cierro Kairo de la bandeja mientras dura la prueba (al final se vuelve a abrir).'
        $bandeja | Stop-Process -Force
        Start-Sleep -Seconds 1
    }
    $carpetaMedal = Join-Path $env:LOCALAPPDATA 'Medal'
    function Test-Medal {
        @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($carpetaMedal, [StringComparison]::OrdinalIgnoreCase) }).Count -gt 0
    }
    function Wait-SinMedal {
        while (Test-Medal) {
            Read-Host 'Cierra Medal del todo (clic derecho en su ícono junto al reloj, Salir) y aprieta Enter' | Out-Null
        }
    }
    function Measure-Ronda([string]$nombre) {
        $ErrorActionPreference = 'Continue'
        $json = Join-Path $dir "$nombre.json"
        & $exe --prueba-fps medir $Segundos $script:carga $json 2>&1 | ForEach-Object { "$_" } | Out-Host
        if (-not (Test-Path $json)) { throw "La ronda «$nombre» no terminó (¿se cortó con Esc?)." }
        Get-Content $json -Raw -Encoding UTF8 | ConvertFrom-Json
    }

    Wait-SinMedal
    Write-Host ''
    Write-Host 'Calibrando la escena para que ande a unos 200 FPS sin grabar...'
    $ErrorActionPreference = 'Continue'
    $salida = @(& $exe --prueba-fps calibrar 2>&1 | ForEach-Object { "$_" })
    $ErrorActionPreference = 'Stop'
    $salida | Out-Host
    $script:carga = [int](($salida | Select-String '^carga (\d+)$' | Select-Object -Last 1).Matches[0].Groups[1].Value)

    $r = [ordered]@{}
    Write-Host ''
    Write-Host 'Ronda 1 de 4: sin grabar'
    $r['Sin nada'] = Measure-Ronda 'sin-nada-1'

    Write-Host ''
    Write-Host 'Ronda 2 de 4: con Kairo grabando (tus ajustes, todo el escritorio)'
    $kairoTxt = Join-Path $dir 'kairo.txt'
    $k = Start-Process $exe -ArgumentList '--grabar-fondo', ($Segundos + 12) -PassThru -WindowStyle Hidden -RedirectStandardOutput $kairoTxt
    Start-Sleep -Seconds 4
    $r['Kairo'] = Measure-Ronda 'kairo'
    $null = $k.WaitForExit(30000)
    $detalleKairo = (Get-Content $kairoTxt -Encoding UTF8 -ErrorAction SilentlyContinue | Select-String 'grabando:' | Select-Object -Last 1).Line

    if (-not $SinMedal) {
        Write-Host ''
        Write-Host 'Ronda 3 de 4: con Medal grabando'
        Read-Host 'Abre Medal, revisa que grabe la PANTALLA COMPLETA (no solo juegos) a 1080p y 60 FPS, espera a que esté grabando y aprieta Enter' | Out-Null
        $r['Medal'] = Measure-Ronda 'medal'
        Wait-SinMedal
    }

    Write-Host ''
    Write-Host 'Ronda 4 de 4: sin grabar otra vez (por si la PC se calentó)'
    $sinNada2 = Measure-Ronda 'sin-nada-2'

    # Sin nada = el promedio de la primera y la última ronda.
    $base = ($r['Sin nada'].fps_promedio + $sinNada2.fps_promedio) / 2
    $baseBajo = ($r['Sin nada'].bajo_1 + $sinNada2.bajo_1) / 2
    $filas = foreach ($nombre in $r.Keys) {
        $x = $r[$nombre]
        $uso = if ($nombre -ne 'Sin nada') { $x.grabadores.$nombre } else { $null }
        $fps = if ($nombre -eq 'Sin nada') { $base } else { $x.fps_promedio }
        $bajo = if ($nombre -eq 'Sin nada') { $baseBajo } else { $x.bajo_1 }
        [pscustomobject]@{
            'Ronda'      = $nombre
            'FPS'        = [math]::Round($fps, 1)
            'Pierde'     = if ($nombre -eq 'Sin nada') { '-' } else { '{0:N1} %' -f ((1 - $fps / $base) * 100) }
            '1 % bajo'   = [math]::Round($bajo, 1)
            'Pierde (1 %)' = if ($nombre -eq 'Sin nada') { '-' } else { '{0:N1} %' -f ((1 - $bajo / $baseBajo) * 100) }
            'Procesador' = if ($uso) { '{0:N2} %' -f $uso.cpu } else { '-' }
            'Tarjeta'    = if ($uso) { '{0:N1} %' -f $uso.gpu } else { '-' }
            'Encoder'    = if ($uso) { '{0:N1} %' -f $uso.encoder } else { '-' }
            'RAM'        = if ($uso) { '{0:N0} MB' -f $uso.ram_mb } else { '-' }
        }
    }
    Write-Host ''
    Write-Host ("Escena: {0}x{1}, carga {2}, {3} s por ronda{4}" -f $r['Sin nada'].ancho, $r['Sin nada'].alto, $carga, $Segundos, $(if ($r['Sin nada'].sin_limite) { ', sin límite de FPS' } else { '' }))
    if ($detalleKairo) { Write-Host "Kairo: $detalleKairo" }
    $filas | Format-Table -AutoSize | Out-Host
    foreach ($nombre in @('Kairo', 'Medal')) {
        if ($r.Contains($nombre) -and (-not $r[$nombre].grabadores.$nombre -or $r[$nombre].grabadores.$nombre.encoder -lt 1)) {
            Write-Host "Ojo: $nombre casi no usó el encoder de video. ¿Estaba grabando la pantalla completa?" -ForegroundColor Yellow
        }
    }

    # El resultado también en un archivo, para compartirlo.
    # (Entre paréntesis: en PowerShell la coma va antes que el +.)
    $columnas = @($filas[0].PSObject.Properties.Name)
    $md = @(('| ' + ($columnas -join ' | ') + ' |'), ('|' + ('---|' * $columnas.Count)))
    foreach ($f in $filas) { $md += '| ' + ($f.PSObject.Properties.Value -join ' | ') + ' |' }
    $archivo = Join-Path $dir 'resultado.md'
    [IO.File]::WriteAllLines($archivo, $md, (New-Object Text.UTF8Encoding $false))
    Write-Host "Resultado guardado en $archivo"
} finally {
    if ($habiaKairo -and -not (Get-Process kairo -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $instalado })) {
        Start-Process $instalado -ArgumentList '--fondo'
        Write-Host 'Kairo de la bandeja se volvió a abrir.'
    }
    Pop-Location
}
