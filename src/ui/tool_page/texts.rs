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
        ToolId::Merge => tr!("Mesclar"),
        ToolId::Split => tr!("Dividir"),
        ToolId::Extract => tr!("Extrair"),
        ToolId::Remove => tr!("Remover páginas"),
        ToolId::Rotate => tr!("Girar"),
        ToolId::ImagesToPdf | ToolId::PdfToImages => tr!("Converter"),
        ToolId::Compress => tr!("Comprimir"),
        ToolId::Organize | ToolId::Annotate => tr!("Salvar"),
        ToolId::NumberPages => tr!("Numerar"),
        ToolId::Watermark => tr!("Aplicar marca d’água"),
        ToolId::Metadata => tr!("Salvar metadados"),
        ToolId::Protect => tr!("Proteger"),
        ToolId::Unlock => tr!("Remover senha"),
        ToolId::PdfToText => tr!("Extrair texto"),
        ToolId::OfficeToPdf => tr!("Converter"),
        ToolId::Ocr => tr!("Reconhecer texto"),
    }
}

/// “2026-10-14T09:59:35-03” → “14/10/2026, 09:59” (em inglês,
/// “2026-10-14, 09:59”). Formatos inesperados são mostrados como vieram.
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
        tr!(
            "{day}/{month}/{year}, {time}",
            day = &iso[8..10],
            month = &iso[5..7],
            year = &iso[0..4],
            time = &iso[11..16]
        )
    } else {
        iso.to_string()
    }
}

/// O que o nível de compressão faz, e o que ele exige.
pub fn compress_explanation(level: CompressLevel) -> String {
    let base = match level {
        CompressLevel::Lossless => {
            tr!("Reorganiza a estrutura do arquivo sem alterar imagens nem texto. A qualidade é idêntica; \
             a economia costuma ser pequena em PDFs já otimizados.")
        }
        CompressLevel::Recommended => {
            tr!("Reduz as imagens para 150 ppp e as recomprime em JPEG. Bom para tela e e-mail; há perda leve \
             de qualidade nas imagens. O texto continua pesquisável.")
        }
        CompressLevel::Strong => {
            tr!("Reduz as imagens para 96 ppp. Gera o menor arquivo, com perda visível ao ampliar ou imprimir.")
        }
        CompressLevel::Custom(_) => {
            tr!("Reduz as imagens para a resolução escolhida abaixo e as recomprime em JPEG. Imagens que já \
             estão abaixo dela não são ampliadas. O texto continua pesquisável.")
        }
    };
    let mut parts = vec![base];
    if level != CompressLevel::Lossless {
        parts.push(tr!(
            "O documento é reescrito pelo Ghostscript: confira o resultado se ele tiver formulários, links ou anexos."
        ));
        if Tool::Ghostscript.find().is_none() {
            parts.push(tr!("⚠ O Ghostscript não está instalado; este nível não está disponível."));
        }
    }
    parts.join(" ")
}

/// Texto da linha “Tamanho estimado” para os arquivos da lista.
pub fn estimate_text(results: &[(&Path, &Result<Estimate, String>)]) -> String {
    let ok: Vec<&Estimate> = results.iter().filter_map(|(_, r)| r.as_ref().ok()).collect();
    if let [(path, Err(e))] = results {
        return tr!("Não foi possível estimar “{file}” ({error}).", file = human::file_name(path), error = e);
    }
    if ok.is_empty() {
        return tr!("Não foi possível estimar o resultado.").into();
    }
    let original: u64 = ok.iter().map(|e| e.original).sum();
    let estimated: u64 = ok.iter().map(|e| e.estimated).sum();
    let exact = ok.iter().all(|e| e.exact);
    let total = Estimate { original, estimated, exact };
    let approx = if exact { "" } else { "≈ " };
    let sizes = format!("{} → {approx}{}", human::size(original), human::size(estimated));
    let mut text = if results.len() == 1 { sizes } else { tr!("{n} arquivos: {sizes}", n = ok.len(), sizes) };
    if total.worthwhile() {
        text.push_str(&format!(" (−{:.0}%)", total.saving() * 100.0));
    } else if results.len() == 1 {
        text.push_str(tr!(". Quase nenhuma economia: nenhum arquivo novo seria criado."));
    } else {
        text.push_str(tr!(". Quase nenhuma economia: os arquivos que não diminuírem não serão gravados."));
    }
    if results.len() == 1 && total.worthwhile() {
        text.push_str(if exact {
            tr!(". Calculado com o documento inteiro.")
        } else {
            tr!(". Estimado com uma amostra das páginas.")
        });
    }
    let failed = results.len() - ok.len();
    if failed > 0 {
        text.push(' ');
        text.push_str(&trn!(
            failed,
            "Sem estimativa para {n} arquivo.",
            "Sem estimativa para {n} arquivos.",
            n = failed
        ));
    }
    text
}

/// “2 de 5 páginas não têm texto selecionável e serão reconhecidas.” Sem
/// `skip`, todas as páginas são reconhecidas, inclusive as que têm texto.
pub fn ocr_summary(with_text: u32, total: u32, skip: bool) -> String {
    if !skip && with_text > 0 {
        return match (with_text, total) {
            (1, 1) => tr!("A página será reconhecida, embora já tenha texto selecionável.").into(),
            (w, n) if w == n => {
                tr!("As {n} páginas serão reconhecidas, embora todas já tenham texto selecionável.", n)
            }
            (w, n) => trn!(
                w as usize,
                "As {n} páginas serão reconhecidas, inclusive {w} que já tem texto selecionável.",
                "As {n} páginas serão reconhecidas, inclusive {w} que já têm texto selecionável.",
                n,
                w
            ),
        };
    }
    match (total - with_text, total) {
        (0, 1) => tr!("A página já tem texto selecionável.").into(),
        (0, n) => tr!("Todas as {n} páginas já têm texto selecionável.", n),
        (1, 1) => tr!("A página não tem texto selecionável e será reconhecida.").into(),
        (w, n) if w == n => tr!("Nenhuma das {n} páginas tem texto selecionável: todas serão reconhecidas.", n),
        (w, n) => trn!(
            w as usize,
            "{w} de {n} páginas não tem texto selecionável e será reconhecida.",
            "{w} de {n} páginas não têm texto selecionável e serão reconhecidas.",
            w,
            n
        ),
    }
}

/// Resumo da tarefa na lista de tarefas.
pub fn outcome_summary(outcome: &Outcome) -> String {
    let mut s = match outcome.outputs.as_slice() {
        [] => tr!("Nenhum arquivo criado").to_string(),
        [one] => tr!("Concluído: {file}", file = human::file_name(&one.path)),
        many => tr!("Concluído: {n} arquivos", n = many.len()),
    };
    let failures = outcome.failures.len();
    if failures > 0 {
        s.push_str(" · ");
        s.push_str(&trn!(failures, "{n} falha", "{n} falhas", n = failures));
    }
    if outcome.cancelled {
        s.push_str(" · ");
        s.push_str(tr!("cancelado"));
    }
    s
}

#[cfg(test)]
mod tests {
    use pdftoolbox::i18n::{Language, with_language};

    use super::{friendly_date, ocr_summary, outcome_summary, run_label};

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

    #[test]
    fn texts_in_english_agree_in_number_and_use_unambiguous_dates() {
        with_language(Language::English, || {
            assert_eq!(ocr_summary(2, 3, true), "1 of 3 pages has no selectable text and will be recognized.");
            assert_eq!(ocr_summary(1, 3, true), "2 of 3 pages have no selectable text and will be recognized.");
            assert_eq!(
                ocr_summary(1, 5, false),
                "All 5 pages will be recognized, including 1 that already has selectable text."
            );
            assert_eq!(friendly_date("2026-10-14T09:59:35-03"), "2026-10-14, 09:59");
            assert_eq!(run_label(pdftoolbox::catalog::ToolId::Merge), "Merge");
            let mut outcome = pdftoolbox::operations::Outcome::default();
            assert_eq!(outcome_summary(&outcome), "No file created");
            outcome.cancelled = true;
            assert_eq!(outcome_summary(&outcome), "No file created · cancelled");
        });
    }
}
