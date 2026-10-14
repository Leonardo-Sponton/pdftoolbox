//! Correção de inclinação para o OCR: mede o ângulo das linhas de texto de
//! uma página digitalizada e endireita a imagem antes do reconhecimento.
//!
//! O ângulo vem da própria imagem (perfil de projeção): os pixels escuros
//! são projetados sobre o eixo vertical girado de cada ângulo candidato, e o
//! ângulo em que as linhas de texto formam faixas mais nítidas (soma dos
//! quadrados das faixas, máxima) é o da inclinação. Assim não depende de uma
//! primeira leitura do Tesseract, que a partir de ~5° mistura as linhas.
//!
//! Só a imagem usada no reconhecimento é girada; a página do PDF não muda.
//! As caixas das palavras voltam ao sistema da imagem original por
//! `Rotation::to_original`.

use image::{GrayImage, Luma};

/// Maior inclinação procurada, em graus. Acima disso, a digitalização está
/// mais para “de lado” (tratada pela detecção de orientação).
pub const MAX_ANGLE: f32 = 15.0;
/// Abaixo disso, girar não melhora a leitura (o Tesseract tolera ~2°) e só
/// suavizaria as letras.
pub const MIN_ANGLE: f32 = 0.3;
/// Pontos escuros usados na medição (amostrados em grade).
const MAX_POINTS: usize = 250_000;
/// Abaixo deste valor de cinza (0–255), o pixel conta como tinta.
const INK: u8 = 128;

/// Coordenadas dos pixels escuros, amostrados em grade de `step` pixels.
fn ink_points(img: &GrayImage) -> (Vec<(f32, f32)>, u32) {
    let (w, h) = img.dimensions();
    let dark = |step: u32| {
        let mut n = 0usize;
        for y in (0..h).step_by(step as usize) {
            for x in (0..w).step_by(step as usize) {
                if img.get_pixel(x, y).0[0] < INK {
                    n += 1;
                }
            }
        }
        n
    };
    // Um passo de 2 px a 300 ppp ainda separa linhas de texto de 8 pt.
    let mut step = 2;
    while dark(step) > MAX_POINTS && step < 16 {
        step *= 2;
    }
    let mut points = Vec::new();
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            if img.get_pixel(x, y).0[0] < INK {
                points.push((x as f32, y as f32));
            }
        }
    }
    (points, step)
}

/// Nitidez das faixas horizontais depois de girar os pontos de `-angle`.
fn sharpness(points: &[(f32, f32)], angle: f32, bin: f32, bins: &mut [u32]) -> f64 {
    let (s, c) = angle.to_radians().sin_cos();
    bins.iter_mut().for_each(|b| *b = 0);
    // y' = −x·sen + y·cos: constante ao longo de uma linha inclinada `angle`.
    let offset = bins.len() as f32 / 2.0;
    for &(x, y) in points {
        let k = ((-x * s + y * c) / bin + offset) as isize;
        if let Some(b) = bins.get_mut(k.max(0) as usize) {
            *b += 1;
        }
    }
    bins.iter().map(|&b| (b as f64) * (b as f64)).sum()
}

/// Linhas de texto encontradas numa imagem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lines {
    /// Inclinação, em graus (positivo = descem para a direita na imagem).
    pub angle: f32,
    /// Quanto as faixas no melhor ângulo são mais nítidas que no pior
    /// (≈1 sem linhas horizontais; texto na horizontal passa de 1,5).
    pub contrast: f64,
    /// Ganho de nitidez do melhor ângulo sobre o 0°.
    gain: f64,
}

/// Mede as linhas horizontais (até `MAX_ANGLE` de inclinação). `None`
/// quando há pouca tinta para medir (página em branco).
pub fn analyze(img: &GrayImage) -> Option<Lines> {
    let (points, step) = ink_points(img);
    if points.len() < 500 {
        return None;
    }
    let (w, h) = img.dimensions();
    let bin = step as f32;
    let diag = ((w as f32).powi(2) + (h as f32).powi(2)).sqrt();
    let mut bins = vec![0u32; (2.0 * diag / bin) as usize + 2];
    let mut score = |a: f32| sharpness(&points, a, bin, &mut bins);
    let flat = score(0.0);
    let mut best = (0.0f32, flat);
    let mut worst = flat;
    let mut search = |from: f32, to: f32, by: f32, best: &mut (f32, f64), worst: &mut f64| {
        let mut a = from;
        while a <= to + 1e-4 {
            let s = score(a);
            if s > best.1 {
                *best = (a, s);
            }
            *worst = worst.min(s);
            a += by;
        }
    };
    search(-MAX_ANGLE, MAX_ANGLE, 0.5, &mut best, &mut worst);
    let coarse = best.0;
    search(coarse - 0.5, coarse + 0.5, 0.05, &mut best, &mut worst);
    Some(Lines { angle: best.0, contrast: best.1 / worst.max(1.0), gain: best.1 / flat.max(1.0) })
}

/// Inclinação das linhas de texto, em graus (positivo = descem para a
/// direita na imagem). `None` quando a página não tem texto suficiente ou
/// nenhum ângulo se destaca do 0° (fotos, páginas quase vazias).
pub fn estimate(img: &GrayImage) -> Option<f32> {
    let lines = analyze(img)?;
    // Uma página de texto inclinada tem faixas bem mais nítidas no ângulo certo.
    if lines.gain < 1.02 || lines.angle.abs() >= MAX_ANGLE - 0.25 {
        return None;
    }
    Some(lines.angle)
}

/// Como voltar do sistema da imagem endireitada ao da original.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    cos: f32,
    sin: f32,
    /// Origem da imagem endireitada no sistema girado (canto da tela ampliada).
    ox: f32,
    oy: f32,
}

impl Rotation {
    /// Ponto (X, Y) da imagem endireitada → ponto na imagem original.
    pub fn to_original(&self, x: f32, y: f32) -> (f32, f32) {
        let (gx, gy) = (x + self.ox, y + self.oy);
        (gx * self.cos - gy * self.sin, gx * self.sin + gy * self.cos)
    }

    /// Ponto da imagem original → ponto na imagem endireitada.
    pub fn to_straight(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.cos + y * self.sin - self.ox, -x * self.sin + y * self.cos - self.oy)
    }
}

/// Gira a imagem para que linhas inclinadas `angle` graus fiquem na
/// horizontal. A tela cresce para nada ser cortado; o fundo novo é branco.
pub fn straighten(img: &GrayImage, angle: f32) -> (GrayImage, Rotation) {
    let (sin, cos) = angle.to_radians().sin_cos();
    let (w, h) = (img.width() as f32, img.height() as f32);
    let fwd = |x: f32, y: f32| (x * cos + y * sin, -x * sin + y * cos);
    let corners = [fwd(0.0, 0.0), fwd(w, 0.0), fwd(0.0, h), fwd(w, h)];
    let (min_x, max_x) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c.0), b.max(c.0)));
    let (min_y, max_y) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c.1), b.max(c.1)));
    let rot = Rotation { cos, sin, ox: min_x.floor(), oy: min_y.floor() };
    let (nw, nh) = ((max_x - rot.ox).ceil() as u32, (max_y - rot.oy).ceil() as u32);
    let mut out = GrayImage::from_pixel(nw, nh, Luma([255]));
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let src = img.as_raw();
    let at = |x: i64, y: i64| -> f32 {
        if x < 0 || y < 0 || x >= iw || y >= ih { 255.0 } else { src[(y * iw + x) as usize] as f32 }
    };
    for (x, y, px) in out.enumerate_pixels_mut() {
        // Centro do pixel de destino → posição na original (interpolação bilinear).
        let (sx, sy) = rot.to_original(x as f32 + 0.5, y as f32 + 0.5);
        let (fx, fy) = (sx - 0.5, sy - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        if x0 < -1 || y0 < -1 || x0 >= iw || y0 >= ih {
            continue;
        }
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
        let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
        px.0[0] = (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8;
    }
    (out, rot)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// “Página” com linhas de texto simuladas, inclinadas `angle` graus:
    /// palavras de largura variada feitas de traços (como letras).
    fn page(angle: f32) -> GrayImage {
        let (w, h) = (1240u32, 1754u32);
        let mut img = GrayImage::from_pixel(w, h, Luma([255]));
        let t = angle.to_radians().tan();
        let mut seed = 7u32;
        let mut rand = |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) % n
        };
        for line in 0..30 {
            let y0 = 200.0 + line as f32 * 45.0;
            let mut x = 150 + rand(40);
            while x < 1050 {
                let word_end = (x + 25 + rand(90)).min(1090);
                while x < word_end {
                    // Uma “letra”: traços verticais de altura variada.
                    let (stroke, tall) = (2 + rand(3), 12 + rand(10));
                    for sx in x..(x + stroke).min(w) {
                        let base = y0 + sx as f32 * t;
                        for dy in 0..tall as i64 {
                            let y = base as i64 - dy;
                            if y >= 0 && (y as u32) < h {
                                img.put_pixel(sx, y as u32, Luma([20]));
                            }
                        }
                    }
                    x += stroke + 3 + rand(6);
                }
                x += 14 + rand(10);
            }
        }
        img
    }

    #[test]
    fn measures_the_tilt_of_text_lines() {
        for angle in [-8.0f32, -3.0, 1.2, 5.0, 10.0] {
            let got = estimate(&page(angle)).unwrap_or(0.0);
            assert!((got - angle).abs() <= 0.15, "esperava {angle}°, mediu {got}°");
        }
        assert!(estimate(&page(0.0)).is_none_or(|a| a.abs() < MIN_ANGLE));
    }

    #[test]
    fn text_lines_stand_out_only_when_horizontal() {
        let upright = analyze(&page(4.0)).unwrap();
        let sideways = analyze(&image::imageops::rotate90(&page(4.0))).unwrap();
        assert!(upright.contrast > 1.5, "{upright:?}");
        assert!(sideways.contrast < upright.contrast / 1.5, "{sideways:?}");
    }

    #[test]
    fn blank_and_noisy_pages_have_no_angle() {
        assert_eq!(estimate(&GrayImage::from_pixel(800, 1000, Luma([255]))), None);
        // Ruído uniforme (como uma foto granulada) não forma faixas.
        let mut seed = 12345u32;
        let noise = GrayImage::from_fn(800, 1000, |_, _| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            Luma([if (seed >> 16) % 7 == 0 { 0 } else { 255 }])
        });
        assert_eq!(estimate(&noise), None);
    }

    #[test]
    fn straightened_lines_are_horizontal_and_points_map_back() {
        let tilted = page(6.0);
        let (straight, rot) = straighten(&tilted, 6.0);
        assert!(straight.width() > tilted.width() && straight.height() > tilted.height(), "nada é cortado");
        let after = estimate(&straight).unwrap_or(0.0);
        assert!(after.abs() < 0.15, "ainda inclinada: {after}°");
        for (x, y) in [(0.0, 0.0), (1240.0, 0.0), (600.0, 900.0), (17.5, 1700.0)] {
            let (sx, sy) = rot.to_straight(x, y);
            assert!(sx >= -1.0 && sy >= -1.0 && sx <= straight.width() as f32 + 1.0);
            let (bx, by) = rot.to_original(sx, sy);
            assert!((bx - x).abs() < 0.01 && (by - y).abs() < 0.01, "({x}, {y}) → ({bx}, {by})");
        }
        // O início da 1ª linha (x = 150, y = 200 + 150·tg 6°) cai numa linha
        // horizontal da imagem endireitada.
        let (ax, ay) = rot.to_straight(150.0, 200.0 + 150.0 * 6f32.to_radians().tan());
        let (bx, by) = rot.to_straight(1080.0, 200.0 + 1080.0 * 6f32.to_radians().tan());
        assert!((ay - by).abs() < 0.5 && bx > ax, "({ax}, {ay}) e ({bx}, {by})");
    }
}
