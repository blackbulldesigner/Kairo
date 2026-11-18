# Cierra Kairo, compila la versión final y la vuelve a abrir.
# Uso: powershell -ExecutionPolicy Bypass -File scripts\actualizar.ps1 [-SinAbrir]
param([switch]$SinAbrir)

$ErrorActionPreference = 'Stop'
$raiz = Split-Path -Parent $PSScriptRoot

if (-not ('KairoActualizar.Win' -as [type])) {
    Add-Type -Namespace KairoActualizar -Name Win -MemberDefinition @'
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string clase, string titulo);
[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
'@
}

# 1. Cerrar con cuidado, como "Salir de Kairo": así se quita el ícono de la bandeja y el estado queda limpio.
#    (El título de los ajustes lleva un punto medio: se arma con [char] para no depender de la codificación.)
$WM_CLOSE = 0x0010
$ventanas = @(
    @{ Clase = 'KairoGrabadora'; Titulo = [NullString]::Value },
    @{ Clase = [NullString]::Value; Titulo = 'Kairo ' + [char]0x00B7 + ' Ajustes' }
)
foreach ($v in $ventanas) {
    $h = [KairoActualizar.Win]::FindWindow($v.Clase, $v.Titulo)
    if ($h -ne [IntPtr]::Zero) { [void][KairoActualizar.Win]::PostMessage($h, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) }
}
$limite = (Get-Date).AddSeconds(10)
while ((Get-Process kairo -ErrorAction SilentlyContinue) -and (Get-Date) -lt $limite) { Start-Sleep -Milliseconds 200 }
$quedan = Get-Process kairo -ErrorAction SilentlyContinue
if ($quedan) {
    Write-Host "Kairo no se cerró solo: se cierra a la fuerza."
    $quedan | Stop-Process -Force
}

# 2. Compilar la versión final.
Push-Location $raiz
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "La compilación falló: Kairo queda cerrado." }
} finally {
    Pop-Location
}

# 3. Abrir de nuevo (la grabadora en la bandeja, y se abren los ajustes).
if (-not $SinAbrir) {
    Start-Process (Join-Path $raiz 'target\release\kairo.exe')
    Write-Host "Kairo actualizado y abierto."
}
