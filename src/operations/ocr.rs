//! OCR: torna pesquisável o texto de documentos digitalizados.
//!
//! Cada página sem texto é renderizada (`pdftoppm`), o Tesseract gera só a
//! camada de texto invisível dela, e o qpdf sobrepõe essas camadas às páginas
//! originais. A imagem digitalizada não é recomprimida nem rasterizada de
//! novo: o arquivo cresce poucos KB por página.

use std::path::{Path, PathBuf};

use image::GrayImage;

use super::{Destination, Outcome, batch, batch_output_name, parallel_map, verify_pdf};
use crate::deskew;
use crate::engines::{install_line, poppler, qpdf, tesseract};
use crate::error::{OpError, OpResult};
use crate::human;
use crate::i18n::{n_, tr};
use crate::jobs::JobCtx;
use crate::pdfwriter::{self, InvisiblePage, InvisibleWord};
use crate::storage::{self, JobDir, stem};

/// Nomes dos idiomas mais comuns do Tesseract (código → nome, em minúsculas
/// como no meio de uma frase em português; traduzidos ao exibir).
const LANGUAGE_NAMES: &[(&str, &str)] = &[
    ("por", n_("português")),
    ("eng", n_("inglês")),
    ("spa", n_("espanhol")),
    ("fra", n_("francês")),
    ("deu", n_("alemão")),
    ("ita", n_("italiano")),
    ("nld", n_("holandês")),
    ("cat", n_("catalão")),
    ("glg", n_("galego")),
    ("eus", n_("basco")),
    ("lat", n_("latim")),
    ("pol", n_("polonês")),
    ("ces", n_("tcheco")),
    ("slk", n_("eslovaco")),
    ("hun", n_("húngaro")),
    ("ron", n_("romeno")),
    ("swe", n_("sueco")),
    ("dan", n_("dinamarquês")),
    ("nor", n_("norueguês")),
    ("fin", n_("finlandês")),
    ("ell", n_("grego")),
    ("tur", n_("turco")),
    ("rus", n_("russo")),
    ("ukr", n_("ucraniano")),
    ("ara", n_("árabe")),
    ("heb", n_("hebraico")),
    ("hin", n_("hindi")),
    ("jpn", n_("japonês")),
    ("kor", n_("coreano")),
    ("chi_sim", n_("chinês simplificado")),
    ("chi_tra", n_("chinês tradicional")),
    ("vie", n_("vietnamita")),
    ("tha", n_("tailandês")),
];

/// Nome do idioma como aparece no meio de uma frase (“português”,
/// “Portuguese”); códigos sem nome conhecido (`frk`, `script/Latin`…)
/// aparecem como estão.
pub fn language_name_in_text(code: &str) -> String {
    LANGUAGE_NAMES.iter().find(|(c, _)| *c == code).map_or_else(|| code.to_string(), |(_, n)| tr(n).to_string())
}

/// Nome do idioma sozinho, com inicial maiúscula (“Português”).
pub fn language_name(code: &str) -> String {
    let name = language_name_in_text(code);
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// Idiomas instalados para o Tesseract, com os mais comuns primeiro
/// (português, inglês e os da tabela acima, nessa ordem).
pub fn installed_languages(cancel: &crate::jobs::CancelToken) -> OpResult<Vec<String>> {
    let mut langs = tesseract::languages(cancel)?;
    let rank = |c: &String| LANGUAGE_NAMES.iter().position(|(k, _)| k == c).unwrap_or(usize::MAX);
    langs.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    Ok(langs)
}

/// Comando que instala os dados de um idioma (exemplo mostrado na interface).
pub fn language_install(code: &str) -> String {
    install_line(
        &format!("tesseract-ocr-{}", code.replace('_', "-").to_lowercase()),
        &format!("tesseract-data-{code}"),
        &format!("tesseract-langpack-{code}"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrOptions {
    /// Códigos do Tesseract (`por`, `eng`…), na ordem de preferência.
    pub languages: Vec<String>,
    /// Não reconhecer páginas que já têm texto selecionável (evita texto
    /// duplicado na busca e na cópia).
    pub skip_text_pages: bool,
    /// Girar (sem perda, como “Girar PDF”) as páginas que o reconhecimento
    /// encontrou deitadas ou de cabeça para baixo.
    pub straighten: bool,
}

/// Resolução do reconhecimento: 300 ppp, menos em páginas enormes (plantas,
/// cartazes) para a imagem não passar de `MAX_SIDE` pixels.
const DPI: u32 = 300;
const MAX_SIDE: f32 = 7000.0;

fn dpi_for((w, h): (f32, f32)) -> u32 {
    let inches = w.max(h) / 72.0;
    if inches <= 0.0 { DPI } else { ((MAX_SIDE / inches).floor() as u32).clamp(72, DPI) }
}

/// Uma página “tem texto” se o `pdftotext` encontra letras ou números nela.
fn has_text(page: &str) -> bool {
    page.chars().any(char::is_alphanumeric)
}

/// Quantas páginas já têm texto selecionável, e o total (para a interface
/// mostrar o que será reconhecido antes de executar).
pub fn text_pages(pdf: &Path, cancel: &crate::jobs::CancelToken) -> OpResult<(u32, u32)> {
    let flags = pages_with_text(pdf, cancel)?;
    Ok((flags.iter().filter(|&&t| t).count() as u32, flags.len() as u32))
}

/// Para cada página (na ordem), se ela já tem texto selecionável.
pub fn pages_with_text(pdf: &Path, cancel: &crate::jobs::CancelToken) -> OpResult<Vec<bool>> {
    let total = qpdf::page_count(pdf, cancel)?;
    Ok(poppler::texts_per_page(pdf, total, cancel)?.iter().map(|t| has_text(t)).collect())
}

/// Páginas a reconhecer (1 = primeira).
fn targets(texts: &[String], skip_text_pages: bool) -> Vec<u32> {
    texts.iter().enumerate().filter(|(_, t)| !(skip_text_pages && has_text(t))).map(|(i, _)| i as u32 + 1).collect()
}

/// Idiomas conferidos e a fonte da camada de texto, prontos para o OCR.
pub(super) struct Engine {
    langs: String,
    font: Vec<u8>,
}

pub(super) fn engine(opts: &OcrOptions, ctx: &JobCtx) -> OpResult<Engine> {
    if opts.languages.is_empty() {
        return Err(OpError::InvalidRange(tr!("Escolha pelo menos um idioma do documento.").into()));
    }
    let installed = tesseract::languages(&ctx.cancel)?;
    if let Some(code) = opts.languages.iter().find(|c| !installed.contains(c)) {
        return Err(OpError::MissingOcrLanguage {
            language: language_name_in_text(code),
            install: language_install(code),
        });
    }
    Ok(Engine { langs: opts.languages.join("+"), font: tesseract::glyphless_font(&ctx.cancel)? })
}

/// PDF com a camada de texto, ainda na pasta da tarefa.
pub(super) struct Searchable {
    pub pdf: PathBuf,
    analyzed: usize,
    recognized: usize,
    /// Páginas que já tinham texto (e foram puladas, se pedido).
    already: usize,
    /// Páginas deitadas ou de cabeça para baixo giradas sem perda.
    turned: usize,
    /// Inclinações corrigidas para a leitura.
    tilted: Vec<f32>,
}

/// Aviso que acompanha todo resultado de OCR.
pub(super) fn caveat() -> &'static str {
    tr!("O reconhecimento automático pode errar letras e números, principalmente em digitalizações tortas, \
         manchadas ou com letra pequena. Confira trechos importantes antes de copiar ou confiar na busca.")
}

/// Reconhece as páginas de `input` (as sem texto, se `skip_text_pages`) e
/// grava em `job` o PDF com a camada de texto. `None` quando não havia o que
/// reconhecer. `slot` = (índice, total) do arquivo no lote, para o progresso.
pub(super) fn searchable(
    input: &Path,
    opts: &OcrOptions,
    engine: &Engine,
    job: &JobDir,
    ctx: &JobCtx,
    (i, n): (usize, usize),
) -> OpResult<Option<Searchable>> {
    ctx.report(i as f64 / n as f64, &tr!("Analisando {file}", file = stem(input)));
    let total = qpdf::page_count(input, &ctx.cancel)?;
    let sizes = poppler::page_sizes(input, total, &ctx.cancel)?;
    let texts = poppler::texts_per_page(input, total, &ctx.cancel)?;
    let pages = targets(&texts, opts.skip_text_pages);
    let already = texts.iter().filter(|t| has_text(t)).count();
    if pages.is_empty() {
        return Ok(None);
    }
    let reads = recognize(input, &pages, &sizes, &engine.langs, job, ctx, (i, n))?;
    let tilted: Vec<f32> = reads.iter().filter_map(|r| r.skew).collect();
    let layers: Vec<InvisiblePage> = reads.into_iter().map(|r| r.layer).collect();

    ctx.report((i as f64 + 0.97) / n as f64, &tr!("Montando {file}", file = stem(input)));
    // O qpdf encaixa cada página da camada na página de destino, então o
    // arredondamento de pixels da renderização não desloca o texto.
    let layer_pdf = job.file("camada-texto.pdf");
    pdfwriter::invisible_text_pdf(&layer_pdf, &layers, &engine.font)
        .map_err(|e| OpError::io(tr!("gravar a camada de texto temporária"), e))?;
    let mut tmp = job.file("ocr.pdf");
    qpdf::overlay_pages(input, &layer_pdf, &pages, &tmp, &ctx.cancel)?;
    let mut turned: Vec<(i32, Vec<u32>)> = Vec::new();
    if opts.straighten {
        for (layer, &page) in layers.iter().zip(&pages).filter(|(l, _)| l.rotation != 0) {
            match turned.iter_mut().find(|(a, _)| *a == layer.rotation as i32) {
                Some((_, list)) => list.push(page),
                None => turned.push((layer.rotation as i32, vec![page])),
            }
        }
    }
    if !turned.is_empty() {
        let straight = job.file("ocr-endireitado.pdf");
        qpdf::rotate(&tmp, &turned, &straight, &ctx.cancel)?;
        tmp = straight;
    }
    verify_pdf(&tmp, total, ctx)?;

    let after = poppler::texts_per_page(&tmp, total, &ctx.cancel)?;
    let recognized = pages.iter().filter(|&&p| has_text(&after[p as usize - 1])).count();
    if recognized == 0 {
        return Err(OpError::OutputInvalid(tr!(
            "nenhum texto foi reconhecido em “{file}”. Confira se o idioma escolhido é o do documento e se a \
             digitalização está legível",
            file = human::file_name(input)
        )));
    }
    Ok(Some(Searchable {
        pdf: tmp,
        analyzed: pages.len(),
        recognized,
        already,
        turned: turned.iter().map(|(_, p)| p.len()).sum(),
        tilted,
    }))
}

/// ““x.pdf”: texto reconhecido em 3 de 4 páginas analisadas. …”
pub(super) fn summary(input: &Path, s: &Searchable, skip_text_pages: bool) -> String {
    let mut parts = vec![trn!(
        s.analyzed,
        "“{file}”: texto reconhecido em {recognized} de {analyzed} página analisada.",
        "“{file}”: texto reconhecido em {recognized} de {analyzed} páginas analisadas.",
        file = human::file_name(input),
        recognized = s.recognized,
        analyzed = s.analyzed
    )];
    if s.turned > 0 {
        parts.push(trn!(
            s.turned,
            "{n} página estava deitada ou de cabeça para baixo e foi girada.",
            "{n} páginas estavam deitadas ou de cabeça para baixo e foram giradas.",
            n = s.turned
        ));
    }
    if !s.tilted.is_empty() {
        let max = s.tilted.iter().fold(0.0f32, |m, a| m.max(a.abs()));
        let angle = human::decimal(f64::from(max), 1);
        parts.push(trn!(
            s.tilted.len(),
            "{n} página estava torta ({angle}°) e foi lida numa cópia endireitada.",
            "{n} páginas estavam tortas (até {angle}°) e foram lidas em cópias endireitadas.",
            n = s.tilted.len(),
            angle
        ));
    }
    if skip_text_pages && s.already > 0 {
        parts.push(trn!(
            s.already,
            "{n} página já tinha texto e ficou como estava.",
            "{n} páginas já tinham texto e ficaram como estavam.",
            n = s.already
        ));
    }
    parts.join(" ")
}

pub fn ocr(inputs: &[PathBuf], opts: OcrOptions, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    if inputs.is_empty() {
        return Err(OpError::NoInput);
    }
    let engine = engine(&opts, ctx)?;
    let n = inputs.len();
    let mut outcome = batch(inputs, |i, input, outcome| {
        let job = JobDir::new()?;
        let Some(done) = searchable(input, &opts, &engine, &job, ctx, (i, n))? else {
            outcome.notes.push(tr!(
                "“{file}” já tem texto selecionável em todas as páginas; não havia o que reconhecer e nenhum \
                 arquivo foi criado.",
                file = human::file_name(input)
            ));
            return Ok(());
        };
        let base = format!("{} - OCR", stem(input));
        let name = batch_output_name(dest, n, &base, "pdf");
        outcome.push(storage::commit(&done.pdf, &dest.dir, &name)?);
        outcome.notes.push(summary(input, &done, opts.skip_text_pages));
        Ok(())
    })?;
    if !outcome.outputs.is_empty() {
        outcome.notes.push(caveat().into());
    }
    Ok(outcome)
}

/// Página da camada a partir das linhas reconhecidas numa imagem de
/// `image_px` pixels em `dpi`. Cada linha vira texto horizontal numa linha de
/// base só (a mediana das palavras): o Poppler — e com ele a busca de vários
/// leitores — quebra linhas cujas palavras sobem ou descem, como acontece
/// em digitalizações levemente tortas.
fn layer_page(lines: &[Vec<tesseract::Word>], image_px: (u32, u32), dpi: u32, rotation: u32) -> InvisiblePage {
    let scale = 72.0 / dpi as f32;
    // Palavras no sistema da imagem endireitada.
    let upright = if rotation % 180 == 90 { (image_px.1, image_px.0) } else { image_px };
    let height = upright.1 as f32 * scale;
    let median = |mut v: Vec<f32>| -> f32 {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let mut words = Vec::new();
    for line in lines.iter().filter(|l| !l.is_empty()) {
        let baseline = median(line.iter().map(|w| w.top + w.height).collect());
        let size = median(line.iter().map(|w| w.height).collect()) * scale;
        for w in line {
            words.push(InvisibleWord {
                text: w.text.clone(),
                x: w.left * scale,
                baseline: height - baseline * scale,
                width: w.width * scale,
                size,
            });
        }
    }
    InvisiblePage { width: image_px.0 as f32 * scale, height: image_px.1 as f32 * scale, rotation, words }
}

/// Leva as palavras para outro sistema de coordenadas pelo centro de cada
/// uma (largura e altura ficam: a diferença entre os sistemas é uma
/// inclinação pequena).
fn map_words(lines: Vec<Vec<tesseract::Word>>, to: impl Fn(f32, f32) -> (f32, f32)) -> Vec<Vec<tesseract::Word>> {
    lines
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|w| {
                    let (cx, cy) = to(w.left + w.width / 2.0, w.top + w.height / 2.0);
                    tesseract::Word { left: cx - w.width / 2.0, top: cy - w.height / 2.0, ..w }
                })
                .collect()
        })
        .collect()
}

fn turned(img: &GrayImage, rotation: u32) -> GrayImage {
    match rotation {
        90 => image::imageops::rotate90(img),
        180 => image::imageops::rotate180(img),
        270 => image::imageops::rotate270(img),
        _ => img.clone(),
    }
}

/// Uma leitura da página girada `rotation` graus (sentido horário) e, se as
/// linhas estiverem inclinadas, endireitada. As palavras já voltaram ao
/// sistema da página girada (sem a correção de inclinação).
struct Attempt {
    rotation: u32,
    skew: Option<f32>,
    text: tesseract::Recognition,
    /// Imagem lida (para a detecção de orientação); apagada ao descartar.
    image: PathBuf,
}

impl Drop for Attempt {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.image);
    }
}

fn attempt(
    original: &GrayImage,
    rotation: u32,
    page: u32,
    langs: &str,
    dpi: u32,
    job: &JobDir,
    ctx: &JobCtx,
) -> OpResult<Attempt> {
    let img = turned(original, rotation);
    let tilt = deskew::estimate(&img).filter(|a| a.abs() >= deskew::MIN_ANGLE);
    let (img, rot) = match tilt {
        Some(angle) => {
            let (straight, rot) = deskew::straighten(&img, angle);
            (straight, Some(rot))
        }
        None => (img, None),
    };
    let image = job.file(&format!("p{page}-{rotation}.pgm"));
    img.save(&image).map_err(|e| {
        OpError::OutputInvalid(tr!("não foi possível preparar a página {page}: {error}", page, error = e))
    })?;
    drop(img);
    // A partir daqui o arquivo é apagado junto com a tentativa.
    let mut result = Attempt { rotation, skew: tilt, text: Default::default(), image };
    let text = tesseract::recognize(&result.image, &job.file(&format!("t{page}-{rotation}")), langs, dpi, &ctx.cancel)?;
    result.text = match rot {
        Some(rot) => tesseract::Recognition { lines: map_words(text.lines, |x, y| rot.to_original(x, y)) },
        None => text,
    };
    Ok(result)
}

/// Resultado da leitura de uma página.
#[derive(Debug, Clone)]
struct PageRead {
    layer: InvisiblePage,
    /// Inclinação corrigida para a leitura, em graus.
    skew: Option<f32>,
}

/// Lê uma página renderizada. As linhas inclinadas são lidas numa cópia
/// endireitada da imagem. Páginas deitadas aparecem já na medição das
/// linhas (elas correm na vertical); de cabeça para baixo, pela leitura
/// ruim — nos dois casos a orientação é confirmada, a imagem é girada, lida
/// de novo, e a melhor leitura vence.
fn read_page(image: &Path, page: u32, langs: &str, dpi: u32, job: &JobDir, ctx: &JobCtx) -> OpResult<PageRead> {
    let original = image::open(image)
        .map_err(|_| OpError::OutputInvalid(tr!("imagem da página {page} ilegível", page)))?
        .into_luma8();
    let dims = original.dimensions();
    let lines_flat = deskew::analyze(&original);
    let lines_side = lines_flat.and_then(|_| deskew::analyze(&image::imageops::rotate90(&original)));
    // Linhas bem mais nítidas com a página girada 90°: o texto está de lado.
    let sideways = match (lines_flat, lines_side) {
        (Some(f), Some(s)) => s.contrast > 1.8 && s.contrast > f.contrast * 1.5,
        _ => false,
    };
    let first = attempt(&original, if sideways { 90 } else { 0 }, page, langs, dpi, job, ctx)?;
    let vertical = first.text.mostly_vertical();
    let poor = first.text.words().next().is_some() && (vertical || first.text.mean_conf() < 60.0);
    let second_turn = if sideways {
        // 90° ou 270°: se a leitura saiu ruim, era o outro lado.
        (poor || first.text.words().next().is_none()).then_some(270)
    } else if poor {
        let osd = tesseract::orientation(&first.image, &ctx.cancel)?;
        Some(match osd {
            Some(r @ (90 | 180 | 270)) => r,
            // Pouco texto para a detecção: a direção das linhas resolve; com
            // linhas horizontais e leitura ruim, testa de cabeça para baixo.
            _ if vertical => first.text.vertical_rotation(),
            _ => 180,
        })
    } else {
        None
    };
    let mut best = first;
    if let Some(r) = second_turn {
        let second = attempt(&original, r, page, langs, dpi, job, ctx)?;
        // Linhas verticais não podem ficar como estão (a camada é horizontal):
        // a nova leitura vale, a menos que seja muito pior.
        let better = if vertical {
            second.text.score() * 4 >= best.text.score() * 3
        } else {
            second.text.score() > best.text.score()
        };
        if better {
            best = second;
        }
    }
    let layer = layer_page(&best.text.lines, dims, dpi, best.rotation);
    Ok(PageRead { layer, skew: best.skew })
}

/// Até quantos Tesseract rodam ao mesmo tempo (um processo por página).
const MAX_WORKERS: usize = 4;

/// Reconhece as páginas em paralelo. Devolve as leituras na ordem de
/// `pages`. `(file_index, file_count)` situa o arquivo no lote (progresso).
fn recognize(
    input: &Path,
    pages: &[u32],
    sizes: &[(f32, f32)],
    langs: &str,
    job: &JobDir,
    ctx: &JobCtx,
    (file_index, file_count): (usize, usize),
) -> OpResult<Vec<PageRead>> {
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1)).clamp(1, MAX_WORKERS);
    let count = pages.len();
    let report = |done: usize| {
        ctx.report(
            (file_index as f64 + 0.95 * done as f64 / count as f64) / file_count as f64,
            &trn!(
                count,
                "{file}: reconhecendo {done} de {count} página",
                "{file}: reconhecendo {done} de {count} páginas",
                file = stem(input),
                done,
                count
            ),
        );
    };
    report(0);
    parallel_map(
        pages,
        workers,
        &ctx.cancel,
        |&page| {
            let dpi = dpi_for(sizes[page as usize - 1]);
            let image = poppler::render_gray(input, page, dpi, &job.file(&format!("p{page}")), &ctx.cancel)?;
            let result = read_page(&image, page, langs, dpi, job, ctx);
            let _ = std::fs::remove_file(&image);
            result
        },
        report,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_pages_get_a_lower_resolution() {
        assert_eq!(dpi_for((595.0, 842.0)), 300);
        // A0 (841 × 1189 mm): 300 ppp passaria de 14 000 px.
        let a0 = (2384.0, 3370.0);
        let dpi = dpi_for(a0);
        assert!(dpi < 300 && (3370.0 / 72.0 * dpi as f32) <= MAX_SIDE);
        assert_eq!(dpi_for((0.0, 0.0)), 300);
    }

    #[test]
    fn skewed_lines_are_flattened_onto_one_baseline() {
        let word = |text: &str, left: f32, top: f32| tesseract::Word {
            text: text.into(),
            left,
            top,
            width: 100.0,
            height: 40.0,
            conf: 90.0,
        };
        // Linha subindo para a direita (digitalização torta), 300 ppp.
        let line = vec![word("um", 300.0, 1000.0), word("dois", 450.0, 990.0), word("três", 600.0, 980.0)];
        let page = layer_page(&[line], (2480, 3508), 300, 0);
        assert!((page.height - 3508.0 * 0.24).abs() < 0.01 && (page.width - 2480.0 * 0.24).abs() < 0.01);
        let bases: Vec<f32> = page.words.iter().map(|w| w.baseline).collect();
        assert!(bases.iter().all(|b| *b == bases[0]), "uma linha de base só: {bases:?}");
        // Mediana: a palavra do meio, base em 1030 px a partir do topo.
        assert!((bases[0] - (3508.0 - 1030.0) * 0.24).abs() < 0.01);
        assert!((page.words[1].x - 108.0).abs() < 0.01 && (page.words[1].width - 24.0).abs() < 0.01);
        assert!((page.words[0].size - 9.6).abs() < 0.01);
    }

    #[test]
    fn pages_with_text_are_skipped_only_when_asked() {
        let texts = vec!["Título\n".to_string(), "  \n ".to_string(), "".to_string(), "— ·".to_string()];
        assert_eq!(targets(&texts, true), [2, 3, 4]);
        assert_eq!(targets(&texts, false), [1, 2, 3, 4]);
    }
}
