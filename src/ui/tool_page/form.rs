//! Leitura do formulário: valores das linhas → `Request`, plano da
//! pré-visualização, validação e o rótulo do botão principal.

use std::path::PathBuf;

use adw::prelude::*;

use pdftoolbox::operations::{
    CompressLevel, DocFields, ImageFormat, ImageLayout, MetadataChange, NumberFormat, NumberStyle, OcrOptions,
    PageSize, Position, Request, Restrictions, Secret, SplitMode, WatermarkStyle, unsupported_chars,
};
use pdftoolbox::preview::{self, MarkKind, Plan};
use pdftoolbox::{OpError, OpResult, pages, stamp};

use super::super::page_preview::LayerFn;
use super::options::Options;

/// Por que o formulário ainda não pode ser executado.
pub(super) enum FormIssue {
    /// Falta preencher algo (aviso discreto; vazio = sem mensagem).
    Incomplete(String),
    /// Há um valor errado (mensagem em vermelho).
    Invalid(String),
}

const POSITIONS: [Position; 6] = [
    Position::BottomRight,
    Position::BottomCenter,
    Position::BottomLeft,
    Position::TopRight,
    Position::TopCenter,
    Position::TopLeft,
];
const FORMATS: [NumberFormat; 3] = [NumberFormat::Number, NumberFormat::NumberOfTotal, NumberFormat::PageOfTotal];
const COLORS: [(f32, f32, f32); 4] = [(0.45, 0.45, 0.45), (0.78, 0.12, 0.12), (0.1, 0.3, 0.78), (0.0, 0.0, 0.0)];

/// Item escolhido de uma lista fixa (o último, se o índice passar dela).
fn pick<T: Copy, const N: usize>(items: [T; N], selected: u32) -> T {
    items[(selected as usize).min(N - 1)]
}

pub(super) fn level_from(selected: u32, dpi: f64) -> CompressLevel {
    match selected {
        0 => CompressLevel::Lossless,
        2 => CompressLevel::Strong,
        3 => CompressLevel::Custom(dpi as u32),
        _ => CompressLevel::Recommended,
    }
}

fn angle_of(selected: u32) -> i32 {
    match selected {
        0 => 90,
        1 => 180,
        _ => 270,
    }
}

fn image_layout(size: u32, margin: f64) -> ImageLayout {
    ImageLayout {
        size: match size {
            0 => PageSize::A4,
            1 => PageSize::Letter,
            _ => PageSize::FitImage,
        },
        margin_mm: margin as f32,
    }
}

/// Texto do campo sem espaços nas pontas; `None` se ficou vazio.
fn spec_text(row: &adw::EntryRow) -> Option<String> {
    Some(row.text().trim().to_string()).filter(|s| !s.is_empty())
}

/// Texto obrigatório: sem ele, o erro pede “Informe {what}.”
fn required_text(row: &adw::EntryRow, what: &str) -> OpResult<String> {
    spec_text(row).ok_or_else(|| OpError::InvalidRange(format!("Informe {what}.")))
}

fn number_style(
    position: &adw::ComboRow,
    format: &adw::ComboRow,
    first: &adw::SpinRow,
    size: &adw::SpinRow,
) -> NumberStyle {
    NumberStyle {
        position: pick(POSITIONS, position.selected()),
        format: pick(FORMATS, format.selected()),
        first_number: first.value() as u32,
        size: size.value() as f32,
        margin_mm: 10.0,
    }
}

fn watermark_style(
    text: &adw::EntryRow,
    color: &adw::ComboRow,
    opacity: &adw::SpinRow,
    scale: &adw::SpinRow,
    diagonal: &adw::SwitchRow,
) -> WatermarkStyle {
    WatermarkStyle {
        text: text.text().trim().to_string(),
        scale: scale.value() as f32 / 100.0,
        opacity: opacity.value() as f32 / 100.0,
        diagonal: diagonal.is_active(),
        color: pick(COLORS, color.selected()),
    }
}

/// Modo de divisão escolhido; `None` quando faltam os intervalos.
fn split_mode(mode: &adw::ComboRow, ranges: &adw::EntryRow, every: &adw::SpinRow) -> Option<SplitMode> {
    Some(match mode.selected() {
        0 => SplitMode::Ranges(spec_text(ranges)?),
        1 => SplitMode::Every(every.value() as u32),
        _ => SplitMode::EachPage,
    })
}

fn pages_text(n: usize) -> String {
    if n == 1 { "1 página".to_string() } else { format!("{n} páginas") }
}

impl Options {
    /// Plano da ferramenta para um documento de `total` páginas. `None`
    /// quando ainda não há o que mostrar (nenhuma página digitada).
    pub fn plan(&self, total: u32) -> Option<Result<Plan, String>> {
        let result = match self {
            Options::Rotate { angle, spec } => {
                preview::rotate(&spec_text(spec).unwrap_or_default(), angle_of(angle.selected()), total)
            }
            Options::Extract { spec, separate } => preview::extract(&spec_text(spec)?, separate.is_active(), total),
            Options::Remove { spec } => preview::remove(&spec_text(spec)?, total),
            Options::Split { mode, ranges, every } => preview::split(&split_mode(mode, ranges, every)?, total),
            Options::Numbers { format, first, spec, .. } => preview::numbers(
                &spec_text(spec).unwrap_or_default(),
                pick(FORMATS, format.selected()),
                first.value() as u32,
                total,
            ),
            Options::Watermark { spec, .. } => preview::watermark(&spec_text(spec).unwrap_or_default(), total),
            _ => return None,
        };
        Some(result.map_err(|e| e.to_string()))
    }

    /// Dica enquanto o campo de páginas está vazio (o botão fica desligado).
    pub fn idle_hint(&self) -> Option<&'static str> {
        match self {
            Options::Extract { .. } => Some("Digite as páginas a extrair ou clique nelas abaixo."),
            Options::Remove { .. } => Some("Digite as páginas a remover ou clique nelas abaixo."),
            Options::Split { .. } => Some("Digite os intervalos; cada um vira um arquivo, com uma cor."),
            _ => None,
        }
    }

    /// Problemas nos campos que não são páginas: `Err` impede executar;
    /// `Ok(Some)` é só um aviso. `ocr_counts`: páginas com texto e total dos
    /// arquivos (OCR), quando já lidos.
    pub fn check(&self, ocr_counts: Option<(u32, u32)>) -> Result<Option<String>, FormIssue> {
        match self {
            Options::Watermark { text, .. } => {
                let t = text.text();
                if t.trim().is_empty() {
                    text.add_css_class("error");
                    return Err(FormIssue::Incomplete("Digite o texto da marca d’água.".into()));
                }
                let (_, missing) = stamp::encode(t.trim());
                if !missing.is_empty() {
                    text.add_css_class("error");
                    return Err(FormIssue::Invalid(unsupported_chars(&missing)));
                }
                text.remove_css_class("error");
                Ok(None)
            }
            Options::Protect(p) => {
                let (pw, confirm) = (p.password.text(), p.confirm.text());
                p.confirm.remove_css_class("error");
                p.owner.remove_css_class("error");
                if pw.is_empty() {
                    return Err(FormIssue::Incomplete(String::new()));
                }
                if confirm.is_empty() {
                    return Err(FormIssue::Incomplete("Repita a senha para confirmar.".into()));
                }
                if pw != confirm {
                    p.confirm.add_css_class("error");
                    return Err(FormIssue::Invalid("As senhas não coincidem.".into()));
                }
                if p.restrict.enables_expansion() {
                    let owner = p.owner.text();
                    if owner.is_empty() {
                        return Err(FormIssue::Incomplete(
                            "Digite a senha de permissões (ou desligue as restrições).".into(),
                        ));
                    }
                    if owner == pw {
                        p.owner.add_css_class("error");
                        return Err(FormIssue::Invalid(
                            "A senha de permissões precisa ser diferente da senha para abrir.".into(),
                        ));
                    }
                }
                if pw.chars().count() < 8 {
                    return Ok(Some("Senha curta: com menos de 8 caracteres ela é fácil de adivinhar.".into()));
                }
                Ok(None)
            }
            Options::Unlock { password } if password.text().is_empty() => Err(FormIssue::Incomplete(String::new())),
            Options::Ocr { languages, .. } if languages.selected().is_empty() => {
                Err(FormIssue::Incomplete(if languages.switches.borrow().is_empty() {
                    String::new()
                } else {
                    "Escolha pelo menos um idioma do documento.".into()
                }))
            }
            Options::Ocr { skip, .. } => match ocr_counts {
                Some((with_text, total)) if with_text == total && skip.is_active() => Err(FormIssue::Incomplete(
                    "Não há páginas para reconhecer. Se o texto existente estiver errado ou ilegível, desligue \
                     “Pular páginas que já têm texto”."
                        .into(),
                )),
                Some((with_text, _)) if with_text > 0 && !skip.is_active() => Ok(Some(
                    "Páginas que já têm texto vão ganhar uma segunda cópia dele, e a busca pode encontrar tudo em \
                     dobro."
                        .into(),
                )),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Se a ferramenta gera um único arquivo com `files` arquivos na lista
    /// (só então o nome do arquivo pode ser escolhido).
    pub fn single_output(&self, files: usize) -> bool {
        match self {
            Options::Extract { separate, .. } => !separate.is_active(),
            Options::Images { one_file, .. } => one_file.is_active() || files <= 1,
            Options::Split { .. } | Options::ToImages { .. } => false,
            Options::Rotate { .. }
            | Options::Compress(_)
            | Options::Numbers { .. }
            | Options::Watermark { .. }
            | Options::Protect(_)
            | Options::Unlock { .. }
            | Options::Text { .. }
            | Options::Office
            | Options::Ocr { .. } => files <= 1,
            Options::Remove { .. } | Options::None | Options::Metadata(_) => true,
        }
    }

    /// Pedido de operação com os valores do formulário. `files` já tem a
    /// quantidade mínima de arquivos da ferramenta.
    pub fn request(&self, files: Vec<PathBuf>) -> OpResult<Request> {
        let first = files.first().cloned().ok_or(OpError::NoInput)?;
        Ok(match self {
            Options::None => Request::Merge { inputs: files },
            Options::Split { mode, ranges, every } => Request::Split {
                input: first,
                mode: split_mode(mode, ranges, every)
                    .ok_or_else(|| OpError::InvalidRange("Informe os intervalos de páginas.".into()))?,
            },
            Options::Extract { spec, separate } => Request::Extract {
                input: first,
                spec: required_text(spec, "as páginas a extrair")?,
                separate: separate.is_active(),
            },
            Options::Remove { spec } => {
                Request::Remove { input: first, spec: required_text(spec, "as páginas a remover")? }
            }
            Options::Rotate { angle, spec } => {
                Request::Rotate { inputs: files, spec: spec_text(spec), angle: angle_of(angle.selected()) }
            }
            Options::Images { size, margin, one_file } => Request::ImagesToPdf {
                inputs: files,
                layout: image_layout(size.selected(), margin.value()),
                one_file: one_file.is_active(),
            },
            Options::ToImages { format, dpi, quality } => Request::PdfToImages {
                inputs: files,
                format: pick([ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Tiff], format.selected()),
                dpi: dpi.value() as u32,
                quality: quality.value() as u8,
            },
            Options::Compress(c) => Request::Compress { inputs: files, level: c.level() },
            Options::Numbers { position, format, first, size, spec } => Request::NumberPages {
                inputs: files,
                spec: spec_text(spec),
                style: number_style(position, format, first, size),
            },
            Options::Watermark { text, color, opacity, scale, diagonal, spec } => Request::Watermark {
                inputs: files,
                spec: spec_text(spec),
                style: watermark_style(text, color, opacity, scale, diagonal),
            },
            Options::Metadata(m) => Request::Metadata {
                input: first,
                change: if m.remove_all.is_active() {
                    MetadataChange::RemoveAll
                } else {
                    MetadataChange::Set(DocFields {
                        title: m.title.text().to_string(),
                        author: m.author.text().to_string(),
                        subject: m.subject.text().to_string(),
                        keywords: m.keywords.text().to_string(),
                    })
                },
            },
            Options::Protect(p) => {
                if p.password.text() != p.confirm.text() {
                    return Err(OpError::InvalidRange("As senhas não coincidem.".into()));
                }
                Request::Protect {
                    inputs: files,
                    password: Secret::new(p.password.text().to_string()),
                    restrictions: p.restrict.enables_expansion().then(|| Restrictions {
                        owner: Secret::new(p.owner.text().to_string()),
                        allow_print: p.print.is_active(),
                        allow_copy: p.copy.is_active(),
                        allow_edit: p.edit.is_active(),
                    }),
                }
            }
            Options::Unlock { password } => {
                Request::Unlock { inputs: files, password: Secret::new(password.text().to_string()) }
            }
            Options::Text { layout, ocr, ocr_languages } => Request::ExtractText {
                inputs: files,
                layout: layout.is_active(),
                ocr: (ocr.is_active() && ocr.is_sensitive()).then(|| OcrOptions {
                    languages: ocr_languages.borrow().clone(),
                    skip_text_pages: true,
                    straighten: false,
                }),
            },
            Options::Office => Request::OfficeToPdf { inputs: files },
            Options::Ocr { languages, skip, straighten } => Request::Ocr {
                inputs: files,
                options: OcrOptions {
                    languages: languages.selected(),
                    skip_text_pages: skip.is_active(),
                    straighten: straighten.is_active(),
                },
            },
        })
    }

    /// Rótulo do botão que diz exatamente o que vai acontecer. `base` é o
    /// verbo da ferramenta; `files`, quantos arquivos há na lista.
    pub fn action_label(&self, base: &str, files: usize, plan: Option<&Plan>) -> String {
        let many = |verb: &str, noun: &str| format!("{verb} {files} {noun}");
        match self {
            Options::Metadata(m) if m.remove_all.is_active() => return "Apagar metadados".into(),
            Options::Protect(_) if files > 1 => return many("Proteger", "arquivos"),
            Options::Unlock { .. } if files > 1 => return many("Remover senha de", "arquivos"),
            Options::Text { .. } if files > 1 => return many("Extrair texto de", "arquivos"),
            Options::Office if files > 1 => return many("Converter", "documentos"),
            Options::Ocr { .. } if files > 1 => return many("Reconhecer texto de", "arquivos"),
            Options::Numbers { .. } if files > 1 => return many("Numerar", "arquivos"),
            Options::Watermark { .. } if files > 1 => return many("Aplicar marca d’água em", "arquivos"),
            _ => {}
        }
        let Some(plan) = plan else { return base.to_string() };
        let count = |kind: MarkKind| plan.marks.iter().filter(|m| m.kind == kind).count();
        let all_or = |n: usize, all: &str, some: &str| {
            if n == plan.marks.len() { all.to_string() } else { format!("{some} {}", pages_text(n)) }
        };
        match self {
            Options::Rotate { .. } if files > 1 => format!("Girar {files} arquivos"),
            Options::Rotate { .. } => all_or(count(MarkKind::Selected), "Girar todas as páginas", "Girar"),
            Options::Extract { spec, .. } => {
                let n = pages::parse_list(&spec.text(), plan.marks.len() as u32).map_or(0, |l| l.len());
                format!("Extrair {}", pages_text(n))
            }
            Options::Remove { .. } => format!("Remover {}", pages_text(count(MarkKind::Removed))),
            Options::Numbers { .. } => all_or(count(MarkKind::Selected), "Numerar todas as páginas", "Numerar"),
            Options::Watermark { .. } => all_or(count(MarkKind::Selected), "Aplicar em todas as páginas", "Aplicar em"),
            Options::Split { .. } => {
                // Intervalos sobrepostos: o maior índice de grupo é a contagem de arquivos.
                let n = plan
                    .marks
                    .iter()
                    .filter_map(|m| match m.kind {
                        MarkKind::Group(g) => Some(g + 1),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                if n == 1 { "Dividir em 1 arquivo".into() } else { format!("Dividir em {n} arquivos") }
            }
            _ => base.to_string(),
        }
    }

    /// “Como vai ficar”: legenda e camada da `page` (a primeira afetada)
    /// para numeração e marca d’água. `chosen` = páginas marcadas no plano.
    pub fn stamp_preview(&self, page: u32, chosen: u32) -> Option<(String, LayerFn)> {
        match self {
            Options::Numbers { position, format, first, size, .. } => {
                let style = number_style(position, format, first, size);
                let (n, last) = (style.first_number, style.first_number + chosen.saturating_sub(1));
                let label = stamp::number_text(style.format, n, last);
                let caption = format!("Página {page}, a primeira numerada: recebe “{label}”.");
                Some((caption, Box::new(move |size| stamp::number_page(size, n, last, &style))))
            }
            Options::Watermark { text, color, opacity, scale, diagonal, .. } => {
                let style = watermark_style(text, color, opacity, scale, diagonal);
                let caption = format!("Página {page}, a primeira com a marca d’água.");
                Some((caption, Box::new(move |size| stamp::watermark(&[size], &[1], &style).remove(0))))
            }
            _ => None,
        }
    }

    /// Pré-visualização de Imagens para PDF: o layout escolhido.
    pub fn image_layout(&self) -> Option<ImageLayout> {
        match self {
            Options::Images { size, margin, .. } => Some(image_layout(size.selected(), margin.value())),
            _ => None,
        }
    }
}
