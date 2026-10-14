//! Textos para pessoas: nomes de arquivo, tamanhos e números com o separador
//! decimal do idioma da interface (vírgula em português, ponto em inglês).
//! Tudo o que vira número ou nome na interface ou nas mensagens das operações
//! passa por aqui; frases e plurais ficam em `i18n`.

use std::path::Path;

use crate::i18n::{self, Language};

/// Nome do arquivo sem a pasta (ou o caminho inteiro, se não houver nome).
pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// “3,2 MB”, “512 B”.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{} {}", decimal(value, 1), UNITS[unit]) }
}

/// Número com `places` casas e o separador do idioma: “1,5” ou “1.5”.
pub fn decimal(value: f64, places: usize) -> String {
    let text = format!("{value:.places$}");
    match i18n::current() {
        Language::Portuguese => text.replace('.', ","),
        Language::English => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_decimal_comma() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1536), "1,5 KB");
        assert_eq!(size(3 * 1024 * 1024), "3,0 MB");
    }

    #[test]
    fn decimal_separator_follows_the_language() {
        assert_eq!(file_name(Path::new("/a/b c.pdf")), "b c.pdf");
        assert_eq!(decimal(1.54, 1), "1,5");
        i18n::with_language(Language::English, || {
            assert_eq!(decimal(1.54, 1), "1.5");
            assert_eq!(size(1536), "1.5 KB");
        });
    }
}
