//! El ícono de Kairo dibujado por código: cuadrado redondeado de color con un anillo abierto
//! (rebobinar) y un punto (grabar). Lo usan la ventana de ajustes y la bandeja.

/// Imagen RGBA (sin premultiplicar) de `n`×`n` píxeles con el fondo del color dado.
pub fn rgba(n: usize, color: [u8; 3]) -> Vec<u8> {
    const SUB: usize = 4;
    // El dibujo está pensado en una grilla de 64×64.
    let escala = 64.0 / n as f32;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (mut fondo, mut blanco) = (0.0f32, 0.0f32);
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let fx = (x as f32 + (sx as f32 + 0.5) / SUB as f32) * escala;
                    let fy = (y as f32 + (sy as f32 + 0.5) / SUB as f32) * escala;
                    // Cuadrado redondeado de 2 a 62 con radio 14.
                    let (cx, cy) = (fx.clamp(16.0, 48.0), fy.clamp(16.0, 48.0));
                    if (fx - cx).powi(2) + (fy - cy).powi(2) <= 14.0f32.powi(2) {
                        fondo += 1.0;
                    }
                    let (dx, dy) = (fx - 32.0, fy - 32.0);
                    let d = (dx * dx + dy * dy).sqrt();
                    let angulo = dy.atan2(dx).to_degrees();
                    let anillo = (14.0..=19.0).contains(&d) && !(-75.0..=-25.0).contains(&angulo);
                    if anillo || d <= 6.5 {
                        blanco += 1.0;
                    }
                }
            }
            let total = (SUB * SUB) as f32;
            let f = fondo / total;
            let b = if f > 0.0 { (blanco / total).min(f) / f } else { 0.0 };
            let i = (y * n + x) * 4;
            for c in 0..3 {
                px[i + c] = (color[c] as f32 * (1.0 - b) + 255.0 * b).round() as u8;
            }
            px[i + 3] = (f * 255.0).round() as u8;
        }
    }
    px
}
