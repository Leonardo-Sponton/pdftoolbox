//! Textos da página de ferramenta que dependem só de dados: rótulos,
//! explicações, resumos e datas.

use std::path::Path;

use pdftoolbox::catalog::ToolId;
use pdftoolbox::engines::Tool;
use pdftoolbox::human;
use pdftoolbox::operations::{CompressLevel, Estimate, Outcome};

/// Verbo do botão principal de cada ferramenta.
pub fn run_label(id: ToolId) -> &'static str {
    match id {
        ToolId::Merge => "Mesclar",
        ToolId::Split => "Dividir",
        ToolId::Extract => "Extrair",
        ToolId::Remove => "Remover páginas",
        ToolId::Rotate => "Girar",
        ToolId::ImagesToPdf | ToolId::PdfToImages => "Converter",
        ToolId::Compress => "Comprimir",
        ToolId::Organize | ToolId::Annotate => "Salvar",
        ToolId::NumberPages => "Numerar",
        ToolId::Watermark => "Aplicar marca d’água",
        ToolId::Metadata => "Salvar metadados",
        ToolId::Protect => "Proteger",
        ToolId::Unlock => "Remover senha",
        ToolId::PdfToText => "Extrair texto",
        ToolId::OfficeToPdf => "Converter",
        ToolId::Ocr => "Reconhecer texto",
    }
}

/// “2026-10-14T09:59:35-03” → “14/10/2026, 09:59”. Formatos inesperados
/// são mostrados como vieram.
pub fn friendly_date(iso: &str) -> String {
    let b = iso.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    if b.len() >= 16
        && digits(0..4)
        && digits(5..7)
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && digits(14..16)
    {
        format!("{}/{}/{}, {}", &iso[8..10], &iso[5..7], &iso[0..4], &iso[11..16])
    } else {
        iso.to_string()
    }
}

/// O que o nível de compressão faz, e o que ele exige.
pub fn compress_explanation(level: CompressLevel) -> String {
    let base = match level {
        CompressLevel::Lossless => {
            "Reorganiza a estrutura do arquivo sem alterar imagens nem texto. A qualidade é idêntica; \
             a economia costuma ser pequena em PDFs já otimizados."
        }
        CompressLevel::Recommended => {
            "Reduz as imagens para 150 ppp e as recomprime em JPEG. Bom para tela e e-mail; há perda leve \
             de qualidade nas imagens. O texto continua pesquisável."
        }
        CompressLevel::Strong => {
            "Reduz as imagens para 96 ppp. Gera o menor arquivo, com perda visível ao ampliar ou imprimir."
        }
        CompressLevel::Custom(_) => {
            "Reduz as imagens para a resolução escolhida abaixo e as recomprime em JPEG. Imagens que já \
             estão abaixo dela não são ampliadas. O texto continua pesquisável."
        }
    };
    let mut text = base.to_string();
    if level != CompressLevel::Lossless {
        text.push_str(
            " O documento é reescrito pelo Ghostscript: confira o resultado se ele tiver formulários, links ou anexos.",
        );
        if Tool::Ghostscript.find().is_none() {
            text.push_str(" ⚠ O Ghostscript não está instalado; este nível não está disponível.");
        }
    }
    text
}

/// Texto da linha “Tamanho estimado” para os arquivos da lista.
pub fn estimate_text(results: &[(&Path, &Result<Estimate, String>)]) -> String {
    let ok: Vec<&Estimate> = results.iter().filter_map(|(_, r)| r.as_ref().ok()).collect();
    if let [(path, Err(e))] = results {
        return format!("Não foi possível estimar “{}” ({e}).", human::file_name(path));
    }
    if ok.is_empty() {
        return "Não foi possível estimar o resultado.".into();
    }
    let original: u64 = ok.iter().map(|e| e.original).sum();
    let estimated: u64 = ok.iter().map(|e| e.estimated).sum();
    let exact = ok.iter().all(|e| e.exact);
    let total = Estimate { original, estimated, exact };
    let approx = if exact { "" } else { "≈ " };
    let mut text = if results.len() == 1 {
        format!("{} → {approx}{}", human::size(original), human::size(estimated))
    } else {
        format!("{} arquivos: {} → {approx}{}", ok.len(), human::size(original), human::size(estimated))
    };
    if total.worthwhile() {
        text.push_str(&format!(" (−{:.0}%)", total.saving() * 100.0));
    } else if results.len() == 1 {
        text.push_str(". Quase nenhuma economia: nenhum arquivo novo seria criado.");
    } else {
        text.push_str(". Quase nenhuma economia: os arquivos que não diminuírem não serão gravados.");
    }
    if results.len() == 1 && total.worthwhile() {
        text.push_str(if exact {
            ". Calculado com o documento inteiro."
        } else {
            ". Estimado com uma amostra das páginas."
        });
    }
    let failed = results.len() - ok.len();
    if failed > 0 {
        text.push_str(&format!(" Sem estimativa para {failed} arquivo(s)."));
    }
    text
}

/// “2 de 5 páginas não têm texto selecionável e serão reconhecidas.” Sem
/// `skip`, todas as páginas são reconhecidas, inclusive as que têm texto.
pub fn ocr_summary(with_text: u32, total: u32, skip: bool) -> String {
    if !skip && with_text > 0 {
        return match (with_text, total) {
            (1, 1) => "A página será reconhecida, embora já tenha texto selecionável.".into(),
            (w, n) if w == n => {
                format!("As {n} páginas serão reconhecidas, embora todas já tenham texto selecionável.")
            }
            (1, n) => format!("As {n} páginas serão reconhecidas, inclusive 1 que já tem texto selecionável."),
            (w, n) => format!("As {n} páginas serão reconhecidas, inclusive {w} que já têm texto selecionável."),
        };
    }
    match (total - with_text, total) {
        (0, 1) => "A página já tem texto selecionável.".into(),
        (0, n) => format!("Todas as {n} páginas já têm texto selecionável."),
        (1, 1) => "A página não tem texto selecionável e será reconhecida.".into(),
        (w, n) if w == n => format!("Nenhuma das {n} páginas tem texto selecionável: todas serão reconhecidas."),
        (1, n) => format!("1 de {n} páginas não tem texto selecionável e será reconhecida."),
        (w, n) => format!("{w} de {n} páginas não têm texto selecionável e serão reconhecidas."),
    }
}

/// Resumo da tarefa na lista de tarefas.
pub fn outcome_summary(outcome: &Outcome) -> String {
    let mut s = match outcome.outputs.as_slice() {
        [] => "Nenhum arquivo criado".to_string(),
        [one] => format!("Concluído: {}", human::file_name(&one.path)),
        many => format!("Concluído: {} arquivos", many.len()),
    };
    if !outcome.failures.is_empty() {
        s.push_str(&format!(" · {} falha(s)", outcome.failures.len()));
    }
    if outcome.cancelled {
        s.push_str(" · cancelado");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{friendly_date, ocr_summary};

    #[test]
    fn ocr_summary_agrees_in_number() {
        assert_eq!(ocr_summary(2, 3, true), "1 de 3 páginas não tem texto selecionável e será reconhecida.");
        assert_eq!(ocr_summary(1, 3, true), "2 de 3 páginas não têm texto selecionável e serão reconhecidas.");
        assert_eq!(ocr_summary(0, 1, true), "A página não tem texto selecionável e será reconhecida.");
        assert_eq!(ocr_summary(4, 4, true), "Todas as 4 páginas já têm texto selecionável.");
    }

    #[test]
    fn ocr_summary_counts_every_page_when_not_skipping() {
        assert_eq!(
            ocr_summary(293, 308, false),
            "As 308 páginas serão reconhecidas, inclusive 293 que já têm texto selecionável."
        );
        assert_eq!(
            ocr_summary(1, 5, false),
            "As 5 páginas serão reconhecidas, inclusive 1 que já tem texto selecionável."
        );
        assert_eq!(ocr_summary(0, 5, false), "Nenhuma das 5 páginas tem texto selecionável: todas serão reconhecidas.");
    }

    #[test]
    fn dates_are_shown_in_brazilian_format() {
        assert_eq!(friendly_date("2026-10-14T09:59:35-03"), "14/10/2026, 09:59");
        assert_eq!(friendly_date("2026-10-14T09:59:35Z"), "14/10/2026, 09:59");
        assert_eq!(friendly_date("ontem"), "ontem");
        assert_eq!(friendly_date(""), "");
    }
}
