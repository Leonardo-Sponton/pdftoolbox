//! Testes do idioma: resolução da preferência, preenchimento dos
//! marcadores e integridade do catálogo inglês contra o código-fonte.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::*;

#[test]
fn preference_resolution() {
    let pt = Some(Language::Portuguese);
    let en = Some(Language::English);
    // Automático: acompanha o sistema; idioma não suportado → português.
    assert_eq!(Preference::System.resolve(pt), Language::Portuguese);
    assert_eq!(Preference::System.resolve(en), Language::English);
    assert_eq!(Preference::System.resolve(None), Language::Portuguese);
    // Escolha manual vence o sistema.
    assert_eq!(Preference::Fixed(Language::Portuguese).resolve(en), Language::Portuguese);
    assert_eq!(Preference::Fixed(Language::English).resolve(pt), Language::English);
    assert_eq!(Preference::Fixed(Language::English).resolve(None), Language::English);
}

#[test]
fn preference_keys_round_trip_and_reject_garbage() {
    for pref in Preference::ALL {
        assert_eq!(Preference::from_key(pref.key()), pref);
    }
    assert_eq!(Preference::from_key(" en "), Preference::Fixed(Language::English));
    for bad in ["", "fr", "EN", "english", "pt_BR", "\u{0}"] {
        assert_eq!(Preference::from_key(bad), Preference::System, "{bad:?}");
    }
}

#[test]
fn language_names_are_native() {
    assert_eq!(Language::Portuguese.native_name(), "Português");
    assert_eq!(Language::English.native_name(), "English");
    // Não passam pelo catálogo: são iguais em qualquer idioma da interface.
    with_language(Language::English, || assert_eq!(Language::Portuguese.native_name(), "Português"));
}

#[test]
fn fill_replaces_named_placeholders() {
    assert_eq!(fill("{a} de {b}", &[("a", &1), ("b", &"dois")]), "1 de dois");
    assert_eq!(fill("{a}{a}", &[("a", &'x')]), "xx");
    assert_eq!(fill("{{literal}} {a}", &[("a", &3)]), "{literal} 3");
    assert_eq!(fill("sem marcadores", &[]), "sem marcadores");
    assert_eq!(fill("“{file}” — ç", &[("file", &"ã.pdf")]), "“ã.pdf” — ç");
}

#[test]
fn macros_translate_and_pick_the_plural() {
    with_language(Language::English, || {
        assert_eq!(tr!("Mesclar PDFs"), "Merge PDFs");
        let file = "a.pdf";
        assert_eq!(tr!("Lendo {file}", file), "Reading a.pdf");
        assert_eq!(
            trn!(1, "{n} página será copiada para um novo PDF.", "{n} páginas serão copiadas para um novo PDF.", n = 1),
            "1 page will be copied to a new PDF."
        );
        assert_eq!(
            trn!(0, "{n} página será copiada para um novo PDF.", "{n} páginas serão copiadas para um novo PDF.", n = 0),
            "0 pages will be copied to a new PDF."
        );
    });
    with_language(Language::Portuguese, || {
        assert_eq!(tr!("Mesclar PDFs"), "Mesclar PDFs");
        assert_eq!(
            trn!(2, "{n} página será copiada para um novo PDF.", "{n} páginas serão copiadas para um novo PDF.", n = 2),
            "2 páginas serão copiadas para um novo PDF."
        );
    });
}

#[test]
fn unknown_messages_fall_back_to_the_original() {
    assert_eq!(translate(Language::English, "mensagem que não está no catálogo"), "mensagem que não está no catálogo");
    assert_eq!(translate(Language::Portuguese, "Mesclar PDFs"), "Mesclar PDFs");
}

#[test]
fn thread_language_does_not_leak() {
    let before = current();
    with_language(Language::English, || {
        assert_eq!(current(), Language::English);
        std::thread::spawn(move || assert_eq!(current(), before)).join().unwrap();
    });
    assert_eq!(current(), before);
}

#[test]
fn library_messages_follow_the_language() {
    use crate::error::OpError;
    let err = OpError::PageOutOfRange { page: 9, total: 1 };
    assert_eq!(err.to_string(), "A página 9 não existe: o documento tem 1 página.");
    with_language(Language::English, || {
        assert_eq!(err.to_string(), "Page 9 does not exist: the document has 1 page.");
        assert_eq!(err.title(), "Invalid page selection");
        assert_eq!(crate::catalog::info(crate::catalog::ToolId::Merge).title(), "Merge PDFs");
        assert_eq!(
            crate::preview::remove("2", 3).unwrap().summary,
            "1 page will be removed; the new PDF will have 2 pages."
        );
        // A busca acha a ferramenta pelas palavras dos dois idiomas.
        let merge = crate::catalog::info(crate::catalog::ToolId::Merge);
        assert!(crate::catalog::matches(merge, "combine"));
        assert!(crate::catalog::matches(merge, "juntar"));
    });
}

// ---------------------------------------------------------------------------
// Integridade do catálogo contra o código-fonte.

/// Mensagens marcadas no código (`tr!`, `tr(`, `trn!`, `trn(`, `n_(`), com
/// o arquivo e a linha da primeira ocorrência.
fn source_messages() -> HashMap<String, String> {
    let mut found = HashMap::new();
    for file in rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src")) {
        let tokens = tokens_outside_tests(&file);
        for (i, token) in tokens.iter().enumerate() {
            let Token::Ident(name) = token else { continue };
            let mut j = i + 1;
            if tokens.get(j) == Some(&Token::Punct('!')) {
                j += 1;
            }
            if tokens.get(j) != Some(&Token::Punct('(')) || i > 0 && tokens[i - 1] == Token::Punct('.') {
                continue;
            }
            j += 1;
            let strings: Vec<(&String, usize)> = match name.as_str() {
                "tr" | "n_" => tokens.get(j).and_then(Token::string).into_iter().collect(),
                "trn" => {
                    // Pula a quantidade (primeiro argumento) até a vírgula.
                    let mut depth = 0;
                    while let Some(t) = tokens.get(j) {
                        match t {
                            Token::Punct('(' | '[' | '{') => depth += 1,
                            Token::Punct(')' | ']' | '}') => depth -= 1,
                            Token::Punct(',') if depth == 0 => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    [tokens.get(j + 1), tokens.get(j + 3)].into_iter().flatten().filter_map(Token::string).collect()
                }
                _ => continue,
            };
            for (text, line) in strings {
                found.entry(text.clone()).or_insert_with(|| format!("{}:{line}", file.display()));
            }
        }
    }
    found
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("i18n/en.rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

#[derive(Debug, PartialEq)]
enum Token {
    Ident(String),
    Str(String, usize),
    Punct(char),
}

impl Token {
    fn string(&self) -> Option<(&String, usize)> {
        match self {
            Token::Str(text, line) => Some((text, *line)),
            _ => None,
        }
    }
}

/// Tokens do arquivo até o módulo de testes (`#[cfg(test)]`, sempre no fim).
fn tokens_outside_tests(path: &Path) -> Vec<Token> {
    let text = std::fs::read_to_string(path).unwrap();
    if path.ends_with("i18n/tests.rs") {
        return Vec::new();
    }
    let code = text.find("#[cfg(test)]").map_or(text.as_str(), |i| &text[..i]);
    tokenize(code)
}

/// Tokenizador mínimo de Rust: identificadores, textos (com escapes e
/// continuação de linha) e pontuação; ignora comentários e caracteres.
fn tokenize(src: &str) -> Vec<Token> {
    let chars: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                line += usize::from(chars[i] == '\n');
                i += 1;
            }
            i += 2;
        } else if c == '\'' {
            // Caractere ('a', '\n', '\u{41}') ou tempo de vida ('static).
            if next == Some('\\') {
                i += 2;
                while chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if c == 'r' && (next == Some('"') || next == Some('#')) && !prev_is_ident(&chars, i) {
            let mut hashes = 0;
            let mut j = i + 1;
            while chars[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if chars[j] != '"' {
                out.push(Token::Ident("r".into()));
                i += 1;
                continue;
            }
            let start = j + 1;
            let mut k = start;
            loop {
                if chars[k] == '"' && (0..hashes).all(|h| chars.get(k + 1 + h) == Some(&'#')) {
                    break;
                }
                k += 1;
            }
            let s: String = chars[start..k].iter().collect();
            out.push(Token::Str(s.clone(), line));
            line += s.matches('\n').count();
            i = k + 1 + hashes;
        } else if c == '"' || (c == 'b' && next == Some('"')) {
            let start_line = line;
            i += if c == 'b' { 2 } else { 1 };
            let mut s = String::new();
            while chars[i] != '"' {
                if chars[i] == '\\' {
                    let e = chars[i + 1];
                    i += 2;
                    match e {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '0' => s.push('\0'),
                        '\\' | '"' | '\'' => s.push(e),
                        '\n' => {
                            line += 1;
                            while chars[i].is_whitespace() {
                                line += usize::from(chars[i] == '\n');
                                i += 1;
                            }
                        }
                        'u' => {
                            let end = (i..).find(|&k| chars[k] == '}').unwrap();
                            let hex: String = chars[i + 1..end].iter().collect();
                            s.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                            i = end + 1;
                        }
                        'x' => {
                            let hex: String = chars[i..i + 2].iter().collect();
                            s.push(char::from(u8::from_str_radix(&hex, 16).unwrap()));
                            i += 2;
                        }
                        other => panic!("escape desconhecido \\{other} na linha {line}"),
                    }
                } else {
                    line += usize::from(chars[i] == '\n');
                    s.push(chars[i]);
                    i += 1;
                }
            }
            i += 1;
            out.push(Token::Str(s, start_line));
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            if !c.is_whitespace() {
                out.push(Token::Punct(c));
            }
            i += 1;
        }
    }
    out
}

fn prev_is_ident(chars: &[char], i: usize) -> bool {
    i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')
}

fn catalog_keys() -> BTreeSet<&'static str> {
    en::MESSAGES.iter().map(|(k, _)| *k).collect()
}

#[test]
fn scanner_reads_the_marked_messages() {
    let tokens =
        tokenize("let a = tr!(\"um \\\n   dois\"); trn!(n.len(), \"x {n}\", \"xs {n}\", n); // tr!(\"não\")\n");
    let strings: Vec<&str> =
        tokens.iter().filter_map(|t| if let Token::Str(s, _) = t { Some(s.as_str()) } else { None }).collect();
    assert_eq!(strings, ["um dois", "x {n}", "xs {n}"]);
    // As mensagens deste módulo são de fato encontradas no código.
    let found = source_messages();
    assert!(found.contains_key("Mesclar PDFs"), "o catálogo de ferramentas usa n_");
    assert!(found.contains_key("{n} páginas serão copiadas para um novo PDF."), "trn! com quantidade");
}

#[test]
fn every_message_in_the_code_has_an_english_translation() {
    let keys = catalog_keys();
    let mut missing: Vec<String> = source_messages()
        .into_iter()
        .filter(|(text, _)| !keys.contains(text.as_str()))
        .map(|(text, at)| format!("{at}: {text:?}"))
        .collect();
    missing.sort();
    assert!(missing.is_empty(), "{} mensagens sem tradução:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn every_catalog_entry_is_used() {
    let used = source_messages();
    let unused: Vec<&str> = catalog_keys().into_iter().filter(|k| !used.contains_key(*k)).collect();
    assert!(unused.is_empty(), "traduções que o código não usa mais:\n{}", unused.join("\n"));
}

#[test]
fn catalog_has_no_duplicate_keys() {
    let mut seen = BTreeSet::new();
    let duplicates: Vec<&str> = en::MESSAGES.iter().map(|(k, _)| *k).filter(|k| !seen.insert(*k)).collect();
    assert!(duplicates.is_empty(), "chaves repetidas:\n{}", duplicates.join("\n"));
}

#[test]
fn translations_keep_placeholders_and_shape() {
    let mut problems = Vec::new();
    for (pt, en) in en::MESSAGES {
        let (mut a, mut b) = (placeholders(pt), placeholders(en));
        a.sort_unstable();
        b.sort_unstable();
        if a != b {
            problems.push(format!("marcadores diferentes: {pt:?} → {en:?}"));
        }
        if en.trim().is_empty() {
            problems.push(format!("tradução vazia: {pt:?}"));
        }
        if pt.starts_with(' ') != en.starts_with(' ') || pt.ends_with(' ') != en.ends_with(' ') {
            problems.push(format!("espaços nas pontas diferentes: {pt:?} → {en:?}"));
        }
        if pt.ends_with('.') != en.ends_with('.') || pt.ends_with('…') != en.ends_with('…') {
            problems.push(format!("pontuação final diferente: {pt:?} → {en:?}"));
        }
        if pt.matches('\n').count() != en.matches('\n').count() {
            problems.push(format!("quebras de linha diferentes: {pt:?} → {en:?}"));
        }
        // Exceção: o rótulo que cita o texto gravado em português.
        if en.contains(['ã', 'õ', 'ç', 'á', 'é', 'í', 'ó', 'ú', 'ê', 'ô', 'â']) && !en.contains("(in Portuguese)")
        {
            problems.push(format!("acento de português na tradução: {en:?}"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Textos com letras acentuadas do português fora de `tr!`/`trn!`/`n_` no
/// código da interface e das mensagens: provavelmente faltou marcar.
/// Ficam de fora o que não é interface (nomes de arquivos gerados, o que é
/// gravado no PDF, detalhes técnicos), listado aqui com o motivo.
#[test]
fn no_unmarked_portuguese_text() {
    const NOT_INTERFACE: &[&str] = &[
        // Nomes de arquivos e pastas gerados: o idioma não muda os arquivos.
        "{} - página {page:0digits$}.{}",
        "marca d’água",
        "página {one}",
        "páginas {list}",
        "{} páginas selecionadas",
        // Texto gravado no PDF pela numeração (opção da ferramenta).
        "Página {n} de {last}",
        // Nome do idioma na própria língua, igual em qualquer interface.
        "Português",
        // Bytes binários do cabeçalho do PDF.
        "%PDF-1.7\n%âãÏÓ\n",
        // Para quem programa: invariantes, detalhes técnicos e log.
        "ferramenta no catálogo",
        "saída inesperada do qpdf: {text:?}",
        "não é uma pasta",
        "pdftoolbox: não foi possível gravar as preferências: {e}",
    ];
    let marked = source_messages();
    let mut problems = Vec::new();
    for file in rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src")) {
        if file.ends_with("i18n/locale.rs") {
            continue;
        }
        // Folha de estilos (comentários CSS), não texto da interface.
        if file.ends_with("ui/theme.rs") {
            continue;
        }
        for token in tokens_outside_tests(&file) {
            let Token::Str(text, line) = token else { continue };
            let portuguese = text.contains(['ã', 'õ', 'ç', 'á', 'é', 'í', 'ó', 'ú', 'ê', 'ô', 'â', 'à', 'É', 'Á', 'Ç']);
            if portuguese && !marked.contains_key(&text) && !NOT_INTERFACE.contains(&text.as_str()) {
                problems.push(format!("{}:{line}: {text:?}", file.display()));
            }
        }
    }
    assert!(problems.is_empty(), "{} textos em português sem tradução:\n{}", problems.len(), problems.join("\n"));
}
