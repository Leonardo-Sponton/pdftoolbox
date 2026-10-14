//! Idioma da interface: português (o texto-fonte) e inglês.
//!
//! As mensagens são escritas em português no próprio código, dentro de
//! `tr!`, `trn!` ou `n_`, e esse texto é a chave do catálogo inglês
//! (`en.rs`), como no gettext. Assim o código continua legível e a tradução
//! fica num único lugar. Uma mensagem sem tradução aparece em português.
//!
//! - `tr!("Lendo {file}", file = nome)` → texto traduzido, com `{nome}`
//!   preenchidos (sem argumentos, devolve `&'static str`);
//! - `trn!(n, "{n} página", "{n} páginas", n)` → escolhe singular ou plural
//!   (português e inglês usam a mesma regra: singular só para 1);
//! - `n_("…")` marca textos em tabelas constantes, traduzidos depois com `tr`.
//!
//! O idioma afeta só textos para pessoas: nomes de arquivos gerados, o que
//! é gravado nos PDFs e os detalhes técnicos não passam por aqui.

mod en;
mod locale;

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::{Display, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{LazyLock, OnceLock};

pub use locale::{detect_language, system_language};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Portuguese,
    English,
}

impl Language {
    /// Idioma usado quando o do sistema não é suportado.
    pub const FALLBACK: Language = Language::Portuguese;

    /// Código ISO 639-1 (“pt”, “en”).
    pub fn code(self) -> &'static str {
        match self {
            Language::Portuguese => "pt",
            Language::English => "en",
        }
    }

    pub fn from_code(code: &str) -> Option<Language> {
        match code {
            "pt" => Some(Language::Portuguese),
            "en" => Some(Language::English),
            _ => None,
        }
    }

    /// Nome do idioma nele mesmo: reconhecível em qualquer idioma da interface.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::Portuguese => "Português",
            Language::English => "English",
        }
    }
}

/// Escolha do usuário: acompanhar o sistema ou fixar um idioma.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    #[default]
    System,
    Fixed(Language),
}

impl Preference {
    pub const ALL: [Preference; 3] =
        [Preference::System, Preference::Fixed(Language::Portuguese), Preference::Fixed(Language::English)];

    /// Valor gravado na configuração: “sistema”, “pt” ou “en”.
    pub fn key(self) -> &'static str {
        match self {
            Preference::System => "sistema",
            Preference::Fixed(language) => language.code(),
        }
    }

    /// Valor desconhecido ou vazio → acompanhar o sistema.
    pub fn from_key(key: &str) -> Preference {
        Language::from_code(key.trim()).map_or(Preference::System, Preference::Fixed)
    }

    /// Idioma efetivo, dado o idioma suportado detectado no sistema (se houver).
    pub fn resolve(self, system: Option<Language>) -> Language {
        match self {
            Preference::Fixed(language) => language,
            Preference::System => system.unwrap_or(Language::FALLBACK),
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THREAD_LANGUAGE: Cell<Option<Language>> = const { Cell::new(None) };
}

/// Idioma em que as mensagens estão sendo produzidas.
pub fn current() -> Language {
    THREAD_LANGUAGE.with(Cell::get).unwrap_or_else(|| match CURRENT.load(Ordering::Relaxed) {
        1 => Language::English,
        _ => Language::Portuguese,
    })
}

/// Executa `f` com as mensagens desta thread em `language`, sem mudar o
/// idioma das demais. Para testes, que rodam em paralelo.
pub fn with_language<R>(language: Language, f: impl FnOnce() -> R) -> R {
    let previous = THREAD_LANGUAGE.with(|l| l.replace(Some(language)));
    let result = f();
    THREAD_LANGUAGE.with(|l| l.set(previous));
    result
}

/// Idioma suportado do sistema, detectado uma vez (antes de
/// `align_system_libraries` alterar o ambiente) e guardado.
pub fn system() -> Option<Language> {
    static SYSTEM: OnceLock<Option<Language>> = OnceLock::new();
    *SYSTEM.get_or_init(system_language)
}

/// Passa a produzir as mensagens no idioma da preferência; devolve o idioma.
pub fn apply(preference: Preference) -> Language {
    let language = preference.resolve(system());
    set_current(language);
    language
}

pub fn set_current(language: Language) {
    CURRENT.store(
        match language {
            Language::Portuguese => 0,
            Language::English => 1,
        },
        Ordering::Relaxed,
    );
}

static ENGLISH: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| en::MESSAGES.iter().copied().collect());

/// Texto de `message` no idioma `language`. Sem tradução, devolve o original.
pub fn translate(language: Language, message: &'static str) -> &'static str {
    match language {
        Language::Portuguese => message,
        Language::English => ENGLISH.get(message).copied().unwrap_or(message),
    }
}

/// Texto de `message` no idioma atual.
pub fn tr(message: &'static str) -> &'static str {
    translate(current(), message)
}

/// Singular (`one`, só para 1) ou plural (`many`), no idioma atual.
pub fn trn(count: usize, one: &'static str, many: &'static str) -> &'static str {
    tr(if count == 1 { one } else { many })
}

/// Marca um texto de uma tabela constante para tradução; traduza-o com `tr`
/// ao exibi-lo.
pub const fn n_(message: &'static str) -> &'static str {
    message
}

/// Valor original de `LANGUAGE`, guardado quando `align_system_libraries` o troca.
static INHERITED_LANGUAGE: OnceLock<Option<OsString>> = OnceLock::new();

/// Faz os textos próprios do GTK e da libadwaita (menu de contexto dos
/// campos, diálogo “Sobre”) usarem o idioma da interface, pela variável
/// `LANGUAGE` do gettext. Nada muda quando o idioma já é o do sistema.
/// O GTK lê a variável ao iniciar: uma troca posterior só vale para esses
/// textos na próxima execução.
///
/// # Safety
/// Altera o ambiente do processo: chame no início de `main`, antes de criar
/// qualquer outra thread.
pub unsafe fn align_system_libraries(language: Language, system: Option<Language>) {
    if system == Some(language) || INHERITED_LANGUAGE.set(std::env::var_os("LANGUAGE")).is_err() {
        return;
    }
    let value = match language {
        Language::Portuguese => "pt_BR:pt",
        Language::English => "en",
    };
    // SAFETY: garantido por quem chama (nenhuma outra thread existe).
    unsafe { std::env::set_var("LANGUAGE", value) };
}

/// `LANGUAGE` como o processo a recebeu (`Some(None)` = não definida), se
/// `align_system_libraries` a alterou. Os programas externos recebem o
/// valor original: o idioma da interface não muda o comportamento deles.
pub fn inherited_language_variable() -> Option<Option<&'static OsString>> {
    INHERITED_LANGUAGE.get().map(Option::as_ref)
}

/// Substitui cada `{nome}` de `template` pelo argumento de mesmo nome.
/// `{{` e `}}` produzem chaves literais. Um nome sem argumento fica como está.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(end) = tail.find('}').filter(|_| tail.starts_with('{'))
            && let Some((_, value)) = args.iter().find(|(name, _)| *name == &tail[1..end])
        {
            let _ = write!(out, "{value}");
            rest = &tail[end + 1..];
            continue;
        }
        debug_assert!(false, "argumento ausente em “{template}”");
        out.push_str(&tail[..1]);
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

/// Nomes dos `{marcadores}` de um texto, em ordem e sem repetição.
pub fn placeholders(template: &str) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            rest = &tail[2..];
        } else if let (true, Some(end)) = (tail.starts_with('{'), tail.find('}')) {
            if !names.contains(&&tail[1..end]) {
                names.push(&tail[1..end]);
            }
            rest = &tail[end + 1..];
        } else {
            rest = &tail[1..];
        }
    }
    names
}

/// Texto traduzido; com argumentos `nome = valor` (ou só `nome`, para uma
/// variável de mesmo nome), preenche os `{nome}` e devolve `String`.
#[macro_export]
macro_rules! tr {
    ($message:literal) => {
        $crate::i18n::tr($message)
    };
    ($message:literal, $($name:ident $(= $value:expr)?),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($message), &[$((stringify!($name), &$crate::__tr_arg!($name $(= $value)?) as &dyn ::std::fmt::Display)),+])
    };
}

/// Como `tr!`, escolhendo entre singular e plural pela quantidade `count`.
#[macro_export]
macro_rules! trn {
    ($count:expr, $one:literal, $many:literal) => {
        $crate::i18n::trn($count, $one, $many)
    };
    ($count:expr, $one:literal, $many:literal, $($name:ident $(= $value:expr)?),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::trn($count, $one, $many), &[$((stringify!($name), &$crate::__tr_arg!($name $(= $value)?) as &dyn ::std::fmt::Display)),+])
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __tr_arg {
    ($name:ident) => {
        $name
    };
    ($name:ident = $value:expr) => {
        $value
    };
}

#[cfg(test)]
mod tests;
