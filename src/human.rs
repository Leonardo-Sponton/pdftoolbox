//! Textos para pessoas (pt-BR): nomes de arquivo, tamanhos, plurais e
//! números com vírgula decimal. Tudo o que vira texto na interface ou nas
//! mensagens das operações passa por aqui.

use std::path::Path;

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

/// “1 página”, “3 páginas”.
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Número com `places` casas e vírgula decimal: “1,5”.
pub fn decimal(value: f64, places: usize) -> String {
    format!("{value:.places$}").replace('.', ",")
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
    fn plural_and_names() {
        assert_eq!(plural(1, "página", "páginas"), "1 página");
        assert_eq!(plural(0, "página", "páginas"), "0 páginas");
        assert_eq!(file_name(Path::new("/a/b c.pdf")), "b c.pdf");
        assert_eq!(decimal(1.54, 1), "1,5");
    }
}
