//! Utilidades mínimas de H.264 en formato Annex B (unidades NAL separadas por códigos de inicio).

/// Unidades NAL de un flujo Annex B (sin los códigos de inicio).
pub fn unidades_nal(datos: &[u8]) -> Vec<&[u8]> {
    let mut inicios = Vec::new();
    let mut i = 0;
    while i + 3 <= datos.len() {
        if datos[i] == 0 && datos[i + 1] == 0 && datos[i + 2] == 1 {
            inicios.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nal = Vec::new();
    for (k, &ini) in inicios.iter().enumerate() {
        let mut fin = inicios.get(k + 1).map(|&s| s - 3).unwrap_or(datos.len());
        // El código de inicio de 4 bytes deja un cero de más al final de la unidad anterior.
        while fin > ini && datos[fin - 1] == 0 && k + 1 < inicios.len() {
            fin -= 1;
        }
        if fin > ini {
            nal.push(&datos[ini..fin]);
        }
    }
    nal
}

fn tipo(n: &[u8]) -> u8 {
    n[0] & 0x1F
}

/// ¿Trae SPS (7) y PPS (8)? Un cuadro clave sin ellos no se puede decodificar solo.
pub fn tiene_encabezado(datos: &[u8]) -> bool {
    let nal = unidades_nal(datos);
    nal.iter().any(|n| tipo(n) == 7) && nal.iter().any(|n| tipo(n) == 8)
}

/// SPS y PPS en Annex B: el encabezado que necesita el MP4.
pub fn encabezado(datos: &[u8]) -> Option<Vec<u8>> {
    let nal = unidades_nal(datos);
    let sps = nal.iter().find(|n| tipo(n) == 7)?;
    let pps = nal.iter().find(|n| tipo(n) == 8)?;
    let mut h = vec![0, 0, 0, 1];
    h.extend_from_slice(sps);
    h.extend_from_slice(&[0, 0, 0, 1]);
    h.extend_from_slice(pps);
    Some(h)
}

/// ¿Está en Annex B? (Empieza con un código de inicio.)
pub fn es_annex_b(datos: &[u8]) -> bool {
    datos.starts_with(&[0, 0, 0, 1]) || datos.starts_with(&[0, 0, 1])
}
