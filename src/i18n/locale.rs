//! Idioma do sistema, pelas mesmas regras do gettext.
//!
//! Ordem: `LANGUAGE` (lista de preferência “pt_BR:en”, ignorada quando o
//! locale efetivo é “C”/“POSIX”), depois o locale efetivo de mensagens, o
//! primeiro definido entre `LC_ALL`, `LC_MESSAGES` e `LANG`. Sem nenhuma
//! dessas variáveis (sessões iniciadas sem o ambiente do login), vale o
//! locale do sistema em `/etc/locale.conf` (systemd). Vence o primeiro
//! idioma suportado da lista; nomes como “pt_BR.UTF-8@euro” ou “en_GB.utf8”
//! valem pelo código do idioma.

use super::Language;

const LOCALE_CONF: &str = "/etc/locale.conf";

/// Idioma suportado preferido no ambiente deste processo, se houver.
pub fn system_language() -> Option<Language> {
    detect_language(|name| std::env::var(name).ok(), || std::fs::read_to_string(LOCALE_CONF).ok())
}

/// `env` lê uma variável de ambiente; `locale_conf` lê o conteúdo de
/// `/etc/locale.conf` (só chamado se o ambiente não definir o locale).
pub fn detect_language(
    env: impl Fn(&str) -> Option<String>,
    locale_conf: impl FnOnce() -> Option<String>,
) -> Option<Language> {
    let defined = |name: &str| env(name).filter(|v| !v.trim().is_empty());
    if defined("LANGUAGE").is_some() || ["LC_ALL", "LC_MESSAGES", "LANG"].iter().any(|v| defined(v).is_some()) {
        return from_variables(defined);
    }
    let text = locale_conf()?;
    from_variables(|name| conf_value(&text, name))
}

fn from_variables(var: impl Fn(&str) -> Option<String>) -> Option<Language> {
    let effective = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| var(name));
    let mut candidates: Vec<String> = Vec::new();
    if !effective.as_deref().is_some_and(is_c_locale)
        && let Some(list) = var("LANGUAGE")
    {
        candidates.extend(list.split(':').map(str::to_string));
    }
    candidates.extend(effective);
    candidates.iter().find_map(|name| language_of(name))
}

/// “pt_BR.UTF-8@euro” → Português; “C”, “fr_FR” → nenhum suportado.
fn language_of(locale: &str) -> Option<Language> {
    let code = locale.trim().split(['_', '-', '.', '@']).next()?.to_ascii_lowercase();
    Language::from_code(&code)
}

fn is_c_locale(locale: &str) -> bool {
    let base = locale.trim().split(['.', '@']).next().unwrap_or("");
    base == "C" || base == "POSIX"
}

/// Valor de `NOME=valor` (com ou sem aspas) num arquivo no formato do locale.conf.
fn conf_value(text: &str, name: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .filter(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().trim_matches(['"', '\'']).to_string())
        .rfind(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(vars: &[(&str, &str)], conf: Option<&str>) -> Option<Language> {
        detect_language(
            |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()),
            || conf.map(str::to_string),
        )
    }

    #[test]
    fn reads_the_locale_variables_in_gettext_order() {
        assert_eq!(detect(&[("LANG", "pt_BR.UTF-8")], None), Some(Language::Portuguese));
        assert_eq!(detect(&[("LANG", "en_US.UTF-8")], None), Some(Language::English));
        assert_eq!(detect(&[("LANG", "pt_BR.UTF-8"), ("LC_MESSAGES", "en_GB.utf8")], None), Some(Language::English));
        assert_eq!(detect(&[("LC_MESSAGES", "en_GB"), ("LC_ALL", "pt_PT@euro")], None), Some(Language::Portuguese));
        // Variável vazia não conta.
        assert_eq!(detect(&[("LC_ALL", ""), ("LANG", "en_US")], None), Some(Language::English));
    }

    #[test]
    fn language_list_wins_with_its_first_supported_entry() {
        assert_eq!(detect(&[("LANGUAGE", "fr:en_US:pt"), ("LANG", "pt_BR.UTF-8")], None), Some(Language::English));
        // Nada suportado na lista: vale o locale.
        assert_eq!(detect(&[("LANGUAGE", "fr:de"), ("LANG", "pt_BR.UTF-8")], None), Some(Language::Portuguese));
        // Locale C: o gettext ignora LANGUAGE.
        assert_eq!(detect(&[("LANGUAGE", "en"), ("LC_ALL", "C.UTF-8")], None), None);
    }

    #[test]
    fn unsupported_or_c_locales_give_no_language() {
        assert_eq!(detect(&[("LANG", "fr_FR.UTF-8")], None), None);
        assert_eq!(detect(&[("LANG", "C")], None), None);
        assert_eq!(detect(&[("LANG", "POSIX")], None), None);
        assert_eq!(detect(&[], None), None);
    }

    #[test]
    fn falls_back_to_locale_conf_only_without_environment() {
        let conf = "# gerado pelo localectl\nLANG=\"en_US.UTF-8\"\nLC_TIME=pt_BR.UTF-8\n";
        assert_eq!(detect(&[], Some(conf)), Some(Language::English));
        assert_eq!(detect(&[("LANG", "pt_BR.UTF-8")], Some(conf)), Some(Language::Portuguese));
        assert_eq!(detect(&[], Some("LANG=pt_BR.UTF-8\nLC_MESSAGES='en_IE.UTF-8'\n")), Some(Language::English));
        assert_eq!(detect(&[], Some("lixo sem formato")), None);
    }
}
