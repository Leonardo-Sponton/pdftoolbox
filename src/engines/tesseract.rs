//! Reconhecimento de texto (OCR) com o Tesseract. Só a camada de texto
//! invisível é gerada (`textonly_pdf`); a página original continua intacta e
//! recebe essa camada por cima com o qpdf.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{Tool, run_ok};
use crate::error::{OpError, OpResult};
use crate::jobs::CancelToken;

/// Idiomas instalados (`por`, `eng`…), sem o `osd` de orientação.
pub fn languages(cancel: &CancelToken) -> OpResult<Vec<String>> {
    let out = run_ok(Tool::Tesseract, ["--list-langs"], &[0], cancel)?;
    Ok(parse_languages(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_languages(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| !l.starts_with("List of available languages"))
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "osd")
        .map(str::to_string)
        .collect()
}

/// Palavra reconhecida, em pixels da imagem (origem no canto superior esquerdo).
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    /// Confiança do Tesseract, de 0 a 100.
    pub conf: f32,
}

/// Linhas reconhecidas numa imagem.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recognition {
    pub lines: Vec<Vec<Word>>,
}

impl Recognition {
    pub fn words(&self) -> impl Iterator<Item = &Word> {
        self.lines.iter().flatten()
    }

    /// Caracteres reconhecidos com boa confiança: compara duas leituras da
    /// mesma página (por exemplo, antes e depois de endireitá-la).
    pub fn score(&self) -> usize {
        self.words().filter(|w| w.conf >= 60.0).map(|w| w.text.chars().count()).sum()
    }

    pub fn mean_conf(&self) -> f32 {
        let (sum, n) = self.words().fold((0.0, 0), |(s, n), w| (s + w.conf, n + 1));
        if n == 0 { 0.0 } else { sum / n as f32 }
    }

    /// A maioria das linhas com mais de uma palavra corre na vertical: a
    /// página está deitada.
    pub fn mostly_vertical(&self) -> bool {
        let center = |w: &Word| (w.left + w.width / 2.0, w.top + w.height / 2.0);
        let (mut vertical, mut total) = (0, 0);
        for line in self.lines.iter().filter(|l| l.len() >= 2) {
            let (a, b) = (center(&line[0]), center(&line[line.len() - 1]));
            total += 1;
            if (b.1 - a.1).abs() > (b.0 - a.0).abs() {
                vertical += 1;
            }
        }
        total > 0 && vertical * 2 > total
    }

    /// Para uma página deitada: quantos graus (horário) a endireitam,
    /// pela direção em que as palavras de cada linha avançam (para baixo:
    /// o texto foi girado 90° no sentido horário e volta com 270°).
    pub fn vertical_rotation(&self) -> u32 {
        let center_y = |w: &Word| w.top + w.height / 2.0;
        let down: f32 = self
            .lines
            .iter()
            .filter(|l| l.len() >= 2)
            .map(|l| (center_y(&l[l.len() - 1]) - center_y(&l[0])).signum())
            .sum();
        if down >= 0.0 { 270 } else { 90 }
    }
}

/// Reconhece `image` e devolve as linhas de texto, cada uma com suas
/// palavras na ordem de leitura. Usa a saída TSV: a camada de PDF do próprio
/// Tesseract inclina cada linha junto com a digitalização, e o Poppler quebra
/// linhas inclinadas (a busca deixa de achar palavras).
pub fn recognize(image: &Path, out_base: &Path, langs: &str, dpi: u32, cancel: &CancelToken) -> OpResult<Recognition> {
    let args: Vec<OsString> = vec![
        image.into(),
        out_base.into(),
        "-l".into(),
        langs.into(),
        "--dpi".into(),
        dpi.to_string().into(),
        "tsv".into(),
    ];
    run_ok(Tool::Tesseract, args, &[0], cancel)?;
    let produced = out_base.with_extension("tsv");
    let tsv = std::fs::read_to_string(&produced)
        .map_err(|_| OpError::OutputInvalid("o Tesseract não gerou o resultado do reconhecimento".into()))?;
    Ok(Recognition { lines: parse_tsv(&tsv) })
}

/// Quantos graus (sentido horário) girar a imagem para o texto ficar de pé,
/// segundo a detecção de orientação do Tesseract (`--psm 0`). `None` quando
/// não há texto suficiente ou os dados de orientação (`osd`) faltam.
pub fn orientation(image: &Path, cancel: &CancelToken) -> OpResult<Option<u32>> {
    let args: Vec<OsString> = vec![image.into(), "-".into(), "--psm".into(), "0".into()];
    let out = super::run(Tool::Tesseract, args, cancel)?;
    if out.code != Some(0) {
        return Ok(None);
    }
    Ok(parse_rotate(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_rotate(text: &str) -> Option<u32> {
    let value = text.lines().find_map(|l| l.strip_prefix("Rotate:"))?.trim().parse::<u32>().ok()?;
    matches!(value, 0 | 90 | 180 | 270).then_some(value)
}

/// Linhas do TSV (nível 5 = palavra), agrupadas por bloco, parágrafo e linha.
fn parse_tsv(tsv: &str) -> Vec<Vec<Word>> {
    // (bloco, parágrafo, linha) da página → palavras.
    type LineKey = (u32, u32, u32, u32);
    let mut lines: Vec<(LineKey, Vec<Word>)> = Vec::new();
    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.splitn(12, '\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let text = cols[11].trim();
        let num = |i: usize| cols[i].parse::<f32>().ok();
        let id = |i: usize| cols[i].parse::<u32>().unwrap_or(0);
        let (Some(left), Some(top), Some(width), Some(height)) = (num(6), num(7), num(8), num(9)) else { continue };
        if text.is_empty() || width <= 0.0 || height <= 0.0 {
            continue;
        }
        let key = (id(1), id(2), id(3), id(4));
        let conf = num(10).unwrap_or(0.0);
        let word = Word { text: text.to_string(), left, top, width, height, conf };
        match lines.last_mut() {
            Some((k, words)) if *k == key => words.push(word),
            _ => lines.push((key, vec![word])),
        }
    }
    lines.into_iter().map(|(_, w)| w).collect()
}

/// Fonte “sem desenho” que o Tesseract instala para a camada de texto
/// invisível (`pdf.ttf`, na pasta dos idiomas).
pub fn glyphless_font(cancel: &CancelToken) -> OpResult<Vec<u8>> {
    let out = run_ok(Tool::Tesseract, ["--list-langs"], &[0], cancel)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let dir = text.lines().find_map(|l| l.split('"').nth(1)).map(PathBuf::from);
    let candidates = dir.into_iter().chain(std::env::var_os("TESSDATA_PREFIX").map(PathBuf::from));
    for dir in candidates {
        if let Ok(bytes) = std::fs::read(dir.join("pdf.ttf")) {
            return Ok(bytes);
        }
    }
    Err(OpError::OutputInvalid(
        "a fonte da camada de texto do Tesseract (pdf.ttf) não foi encontrada; reinstale o pacote do Tesseract".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_words_by_line() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t1700\t2200\t-1\t\n\
                   4\t1\t1\t1\t1\t0\t159\t133\t372\t43\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t159\t144\t161\t32\t92.6\tDeclaração\n\
                   5\t1\t1\t1\t1\t2\t331\t141\t32\t25\t92.3\tde\n\
                   5\t1\t1\t1\t1\t3\t373\t133\t10\t10\t10.0\t \n\
                   5\t1\t1\t2\t1\t1\t161\t209\t48\t28\t92.2\tEu,\n";
        let lines = parse_tsv(tsv);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Declaração", "de"]);
        assert_eq!(
            lines[1][0],
            Word { text: "Eu,".into(), left: 161.0, top: 209.0, width: 48.0, height: 28.0, conf: 92.2 }
        );
    }

    #[test]
    fn reads_the_rotation_and_spots_sideways_pages() {
        let osd = "Page number: 0\nOrientation in degrees: 90\nRotate: 270\nOrientation confidence: 26.35\n";
        assert_eq!(parse_rotate(osd), Some(270));
        assert_eq!(parse_rotate("Rotate: 45\n"), None);
        assert_eq!(parse_rotate(""), None);
        let w = |left: f32, top: f32| Word { text: "ab".into(), left, top, width: 10.0, height: 40.0, conf: 90.0 };
        let sideways = Recognition { lines: vec![vec![w(100.0, 500.0), w(100.0, 400.0)]] };
        assert!(sideways.mostly_vertical());
        let upright = Recognition { lines: vec![vec![w(100.0, 500.0), w(200.0, 498.0)], vec![w(5.0, 5.0)]] };
        assert!(!upright.mostly_vertical());
        assert_eq!(upright.score(), 6);
        assert_eq!(sideways.vertical_rotation(), 90, "palavras subindo");
        let down = Recognition { lines: vec![vec![w(100.0, 400.0), w(100.0, 500.0)]] };
        assert_eq!(down.vertical_rotation(), 270, "palavras descendo");
    }

    #[test]
    fn lists_languages_without_osd() {
        let out = "List of available languages in \"/usr/share/tessdata/\" (3):\neng\nosd\npor\n";
        assert_eq!(parse_languages(out), ["eng", "por"]);
        assert!(parse_languages("").is_empty());
    }
}
