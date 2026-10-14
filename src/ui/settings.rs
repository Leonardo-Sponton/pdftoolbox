//! Preferências locais em `~/.config/pdftoolbox/config` (chave=valor).
//! Guarda apenas tema, idioma, ferramentas recentes, a última pasta de
//! destino e os idiomas do OCR —
//! nunca nomes de documentos processados.

use std::path::{Path, PathBuf};

use pdftoolbox::i18n::Preference;
use pdftoolbox::storage::config_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn key(self) -> &'static str {
        match self {
            Theme::System => "sistema",
            Theme::Light => "claro",
            Theme::Dark => "escuro",
        }
    }

    pub fn from_key(key: &str) -> Theme {
        match key {
            "claro" => Theme::Light,
            "escuro" => Theme::Dark,
            _ => Theme::System,
        }
    }

    pub fn apply(self) {
        adw::StyleManager::default().set_color_scheme(match self {
            Theme::System => adw::ColorScheme::Default,
            Theme::Light => adw::ColorScheme::ForceLight,
            Theme::Dark => adw::ColorScheme::ForceDark,
        });
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: Theme,
    /// Idioma da interface escolhido no menu (padrão: seguir o sistema).
    pub language: Preference,
    pub recent: Vec<String>,
    pub last_dir: Option<PathBuf>,
    /// Idiomas escolhidos da última vez no OCR (vazio = padrão).
    pub ocr_languages: Vec<String>,
    /// Linhas que esta versão não conhece (de versões mais novas, por
    /// exemplo): regravadas como estão, para não se perderem.
    unknown: Vec<String>,
}

const MAX_RECENT: usize = 4;

/// “a, b,,c” → [a, b, c].
fn parse_list(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect()
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            theme: Theme::System,
            language: Preference::System,
            recent: Vec::new(),
            last_dir: None,
            ocr_languages: Vec::new(),
            unknown: Vec::new(),
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("config")
    }

    /// Preferências gravadas; sem arquivo (primeira execução) ou ilegível,
    /// os padrões.
    pub fn load() -> Settings {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path).map(|text| Self::parse(&text)).unwrap_or_default()
    }

    /// Lê o formato `chave=valor`; valores inválidos ficam no padrão.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                if !line.trim().is_empty() {
                    s.unknown.push(line.to_string());
                }
                continue;
            };
            match k.trim() {
                "tema" => s.theme = Theme::from_key(v.trim()),
                "idioma" => s.language = Preference::from_key(v),
                "recentes" => s.recent = parse_list(v),
                "ocr_idiomas" => s.ocr_languages = parse_list(v),
                "ultima_pasta" => s.last_dir = Some(PathBuf::from(v)).filter(|p| p.is_dir()),
                _ => s.unknown.push(line.to_string()),
            }
        }
        s.recent.truncate(MAX_RECENT);
        s
    }

    pub fn to_text(&self) -> String {
        let mut text = format!("tema={}\nrecentes={}\n", self.theme.key(), self.recent.join(","));
        if self.language != Preference::System {
            text.push_str(&format!("idioma={}\n", self.language.key()));
        }
        if let Some(d) = &self.last_dir {
            text.push_str(&format!("ultima_pasta={}\n", d.to_string_lossy()));
        }
        if !self.ocr_languages.is_empty() {
            text.push_str(&format!("ocr_idiomas={}\n", self.ocr_languages.join(",")));
        }
        for line in &self.unknown {
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    /// Grava as preferências. Uma falha (disco cheio, pasta sem permissão)
    /// não interrompe nada: o aplicativo segue com as preferências em memória.
    pub fn save(&self) {
        if let Err(e) = self.save_to(&Self::path()) {
            eprintln!("pdftoolbox: não foi possível gravar as preferências: {e}");
        }
    }

    /// Grava num arquivo temporário e renomeia: um arquivo pela metade
    /// nunca substitui o anterior.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(".config.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(tmp, path)
    }

    pub fn touch_recent(&mut self, key: &str) {
        self.recent.retain(|k| k != key);
        self.recent.insert(0, key.to_string());
        self.recent.truncate(MAX_RECENT);
    }
}

#[cfg(test)]
mod tests {
    use pdftoolbox::i18n::Language;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdftoolbox-config-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn starts_with_defaults_without_a_config_file() {
        let dir = temp_dir("sem-arquivo");
        let s = Settings::load_from(&dir.join("config"));
        assert_eq!(s, Settings::default());
        assert_eq!(s.language, Preference::System);
    }

    #[test]
    fn invalid_or_garbage_config_falls_back_to_defaults() {
        let s = Settings::parse("idioma=klingon\ntema=roxo\n\u{0}lixo\n=\nrecentes=\n");
        assert_eq!(s.language, Preference::System);
        assert_eq!(s.theme, Theme::System);
        assert!(s.recent.is_empty());
        // Arquivo binário (ou que não é UTF-8): padrões, sem pânico.
        let dir = temp_dir("binario");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert_eq!(Settings::load_from(&dir.join("config")), Settings::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn language_choice_persists_and_can_return_to_automatic() {
        let dir = temp_dir("idioma");
        let path = dir.join("config");
        let mut s = Settings::load_from(&path);
        s.language = Preference::Fixed(Language::English);
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path).language, Preference::Fixed(Language::English));

        let mut s = Settings::load_from(&path);
        s.language = Preference::Fixed(Language::Portuguese);
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path).language, Preference::Fixed(Language::Portuguese));

        // Voltar ao automático não deixa preferência manual gravada.
        let mut s = Settings::load_from(&path);
        s.language = Preference::System;
        s.save_to(&path).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("idioma="));
        assert_eq!(Settings::load_from(&path).language, Preference::System);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn changing_the_language_keeps_unrelated_settings() {
        let dir = temp_dir("outras");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        let original = format!(
            "tema=escuro\nrecentes=mesclar,ocr\nultima_pasta={}\nocr_idiomas=por,eng\nchave_futura=42\n",
            dir.display()
        );
        std::fs::write(&path, &original).unwrap();
        let mut s = Settings::load_from(&path);
        s.language = Preference::Fixed(Language::English);
        s.save_to(&path).unwrap();
        let saved = Settings::load_from(&path);
        assert_eq!(saved.theme, Theme::Dark);
        assert_eq!(saved.recent, ["mesclar", "ocr"]);
        assert_eq!(saved.last_dir.as_deref(), Some(dir.as_path()));
        assert_eq!(saved.ocr_languages, ["por", "eng"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("chave_futura=42"), "linha desconhecida preservada: {text}");
        assert!(text.contains("idioma=en"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unwritable_location_reports_an_error_instead_of_panicking() {
        let dir = temp_dir("bloqueado");
        std::fs::create_dir_all(&dir).unwrap();
        // Um arquivo no lugar da pasta: create_dir_all falha.
        std::fs::write(dir.join("arquivo"), b"x").unwrap();
        assert!(Settings::default().save_to(&dir.join("arquivo").join("config")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
