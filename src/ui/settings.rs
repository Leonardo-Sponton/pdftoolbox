//! Preferências locais em `~/.config/pdftoolbox/config` (chave=valor).
//! Guarda apenas tema, ferramentas recentes e a última pasta de destino —
//! nunca nomes de documentos processados.

use std::path::PathBuf;

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

#[derive(Debug, Clone)]
pub struct Settings {
    pub theme: Theme,
    pub recent: Vec<String>,
    pub last_dir: Option<PathBuf>,
}

const MAX_RECENT: usize = 4;

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("config")
    }

    pub fn load() -> Settings {
        let mut s = Settings { theme: Theme::System, recent: Vec::new(), last_dir: None };
        let Ok(text) = std::fs::read_to_string(Self::path()) else { return s };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k.trim() {
                "tema" => s.theme = Theme::from_key(v.trim()),
                "recentes" => {
                    s.recent = v.split(',').map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect()
                }
                "ultima_pasta" => s.last_dir = Some(PathBuf::from(v)).filter(|p| p.is_dir()),
                _ => {}
            }
        }
        s.recent.truncate(MAX_RECENT);
        s
    }

    pub fn save(&self) {
        let dir = config_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let mut text = format!("tema={}\nrecentes={}\n", self.theme.key(), self.recent.join(","));
        if let Some(d) = &self.last_dir {
            text.push_str(&format!("ultima_pasta={}\n", d.to_string_lossy()));
        }
        let tmp = dir.join(".config.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, Self::path());
        }
    }

    pub fn touch_recent(&mut self, key: &str) {
        self.recent.retain(|k| k != key);
        self.recent.insert(0, key.to_string());
        self.recent.truncate(MAX_RECENT);
    }
}
