//! Página genérica de ferramenta: arquivos → opções → destino → executar →
//! progresso → resultado. As opções específicas ficam em `Options`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gio, glib};

use pdftoolbox::catalog::{self, Accepts, ToolId, ToolInfo};
use pdftoolbox::engines::Tool;
use pdftoolbox::engines::poppler;
use pdftoolbox::error::display_name;
use pdftoolbox::jobs::{self, CancelToken, JobEvent};
use pdftoolbox::operations::{
    self, CUSTOM_DPI, CompressLevel, Destination, DocFields, Estimate, ImageFormat, ImageLayout, MetadataChange,
    NumberFormat, NumberStyle, OcrOptions, Outcome, PageSize, Position, Request, Restrictions, Secret, SplitMode,
    WatermarkStyle, human_size, unsupported_chars,
};
use pdftoolbox::preview::Plan;
use pdftoolbox::{OpError, OpResult, pages, stamp};

use pdftoolbox::preview;

use super::page_preview::{LayoutPreview, PagePreview, StampPreview, TextPreview};
use super::thumbs::{self, FileFacts};
use super::{AppState, primary_menu, widgets};

enum Options {
    None,
    Split {
        mode: adw::ComboRow,
        ranges: adw::EntryRow,
        every: adw::SpinRow,
    },
    Extract {
        spec: adw::EntryRow,
        separate: adw::SwitchRow,
    },
    Remove {
        spec: adw::EntryRow,
    },
    Rotate {
        angle: adw::ComboRow,
        spec: adw::EntryRow,
    },
    Images {
        size: adw::ComboRow,
        margin: adw::SpinRow,
        one_file: adw::SwitchRow,
    },
    ToImages {
        format: adw::ComboRow,
        dpi: adw::SpinRow,
        quality: adw::SpinRow,
    },
    Compress(Box<CompressRows>),
    Numbers {
        position: adw::ComboRow,
        format: adw::ComboRow,
        first: adw::SpinRow,
        size: adw::SpinRow,
        spec: adw::EntryRow,
    },
    Watermark {
        text: adw::EntryRow,
        color: adw::ComboRow,
        opacity: adw::SpinRow,
        scale: adw::SpinRow,
        diagonal: adw::SwitchRow,
        spec: adw::EntryRow,
    },
    Metadata(Box<MetadataRows>),
    Protect(Box<ProtectRows>),
    Unlock {
        password: adw::PasswordEntryRow,
    },
    Text {
        layout: adw::SwitchRow,
        /// Reconhecer (OCR) as páginas digitalizadas antes de extrair.
        ocr: adw::SwitchRow,
        /// Idiomas do OCR: os escolhidos na ferramenta OCR, se instalados.
        ocr_languages: Rc<RefCell<Vec<String>>>,
    },
    Office,
    Ocr {
        languages: Rc<LanguageRows>,
        skip: adw::SwitchRow,
        straighten: adw::SwitchRow,
    },
}

/// Por que o formulário ainda não pode ser executado.
enum FormIssue {
    /// Falta preencher algo (aviso discreto; vazio = sem mensagem).
    Incomplete(String),
    /// Há um valor errado (mensagem em vermelho).
    Invalid(String),
}

struct MetadataRows {
    title: adw::EntryRow,
    author: adw::EntryRow,
    subject: adw::EntryRow,
    keywords: adw::EntryRow,
    remove_all: adw::SwitchRow,
    /// Somente leitura: programa de criação, produtor e datas.
    facts: [adw::ActionRow; 4],
    /// Arquivo cujos metadados preencheram os campos.
    loaded: RefCell<Option<PathBuf>>,
}

struct CompressRows {
    level: adw::ComboRow,
    dpi: adw::SpinRow,
    /// Tamanho previsto, calculado em segundo plano com uma amostra de páginas.
    estimate: adw::ActionRow,
    spinner: gtk::Spinner,
    estimates: RefCell<HashMap<(PathBuf, CompressLevel), Result<Estimate, String>>>,
    pending: RefCell<Option<CancelToken>>,
    generation: Cell<u64>,
}

impl CompressRows {
    fn level(&self) -> CompressLevel {
        level_from(self.level.selected(), self.dpi.value())
    }

    fn stop(&self) {
        if let Some(t) = self.pending.borrow_mut().take() {
            t.cancel();
        }
    }
}

impl Drop for CompressRows {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Idiomas do OCR: um interruptor por idioma instalado no Tesseract.
struct LanguageRows {
    row: adw::ExpanderRow,
    switches: RefCell<Vec<(String, adw::SwitchRow)>>,
}

impl LanguageRows {
    fn selected(&self) -> Vec<String> {
        self.switches.borrow().iter().filter(|(_, s)| s.is_active()).map(|(c, _)| c.clone()).collect()
    }

    fn update_subtitle(&self) {
        let chosen = self.selected();
        let text = if self.switches.borrow().is_empty() {
            "Nenhum idioma instalado".to_string()
        } else if chosen.is_empty() {
            "Nenhum escolhido".to_string()
        } else {
            let names: Vec<String> = chosen.iter().map(|c| operations::language_name(c)).collect();
            names.join(", ")
        };
        self.row.set_subtitle(&text);
    }
}

struct ProtectRows {
    password: adw::PasswordEntryRow,
    confirm: adw::PasswordEntryRow,
    restrict: adw::ExpanderRow,
    owner: adw::PasswordEntryRow,
    print: adw::SwitchRow,
    copy: adw::SwitchRow,
    edit: adw::SwitchRow,
}

pub struct ToolPage {
    app: Weak<AppState>,
    info: &'static ToolInfo,
    files: RefCell<Vec<PathBuf>>,
    file_info: Rc<RefCell<HashMap<PathBuf, FileFacts>>>,
    files_group: adw::PreferencesGroup,
    file_rows: RefCell<Vec<adw::ActionRow>>,
    empty_row: adw::ActionRow,
    options: Options,
    options_group: adw::PreferencesGroup,
    preview: Option<Rc<PagePreview>>,
    layout: Option<LayoutPreview>,
    stamp_preview: Option<Rc<StampPreview>>,
    text_preview: Option<Rc<TextPreview>>,
    /// Aviso de validação do formulário, logo abaixo das opções.
    form_hint: gtk::Label,
    plan_ok: Cell<bool>,
    /// Verdadeiro enquanto um clique na grade reescreve o campo de páginas.
    from_click: Cell<bool>,
    dest_dir: RefCell<Option<PathBuf>>,
    dest_custom: Cell<bool>,
    dest_row: adw::ActionRow,
    dest_reset: gtk::Button,
    name_row: adw::EntryRow,
    run_button: gtk::Button,
    cancel_button: gtk::Button,
    progress: gtk::ProgressBar,
    status: gtk::Label,
    result_slot: gtk::Box,
    scroll: gtk::ScrolledWindow,
    running: RefCell<Option<CancelToken>>,
    deps_ok: bool,
    /// OCR: páginas com texto e total, por arquivo (`None` = lendo).
    text_pages: RefCell<HashMap<PathBuf, Option<(u32, u32)>>>,
}

fn combo(title: &str, items: &[&str], selected: u32) -> adw::ComboRow {
    let row = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(items)).build();
    row.set_selected(selected);
    row
}

fn spin(title: &str, subtitle: &str, min: f64, max: f64, step: f64, value: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_value(value);
    row
}

fn entry(title: &str) -> adw::EntryRow {
    adw::EntryRow::builder().title(title).build()
}

fn run_label(id: ToolId) -> &'static str {
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

fn switch(title: &str, subtitle: &str, active: bool) -> adw::SwitchRow {
    adw::SwitchRow::builder().title(title).subtitle(subtitle).active(active).build()
}

fn password(title: &str) -> adw::PasswordEntryRow {
    adw::PasswordEntryRow::builder().title(title).build()
}

fn info_row(title: &str) -> adw::ActionRow {
    let row = widgets::plain_row(title, "—");
    row.set_subtitle_selectable(true);
    row.add_css_class("property");
    row
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

fn number_style(
    position: &adw::ComboRow,
    format: &adw::ComboRow,
    first: &adw::SpinRow,
    size: &adw::SpinRow,
) -> NumberStyle {
    NumberStyle {
        position: POSITIONS[(position.selected() as usize).min(5)],
        format: FORMATS[(format.selected() as usize).min(2)],
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
        color: COLORS[(color.selected() as usize).min(3)],
    }
}

/// “2026-10-14T09:59:35-03” → “14/10/2026, 09:59”. Formatos inesperados
/// são mostrados como vieram.
fn friendly_date(iso: &str) -> String {
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

fn spec_text(row: &adw::EntryRow) -> Option<String> {
    Some(row.text().trim().to_string()).filter(|s| !s.is_empty())
}

fn compress_explanation(level: CompressLevel) -> String {
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

fn level_from(i: u32, dpi: f64) -> CompressLevel {
    match i {
        0 => CompressLevel::Lossless,
        2 => CompressLevel::Strong,
        3 => CompressLevel::Custom(dpi as u32),
        _ => CompressLevel::Recommended,
    }
}

/// Texto da linha “Tamanho estimado” para os arquivos da lista.
fn estimate_text(results: &[(&Path, &Result<Estimate, String>)]) -> String {
    let ok: Vec<&Estimate> = results.iter().filter_map(|(_, r)| r.as_ref().ok()).collect();
    if let [(path, Err(e))] = results {
        return format!("Não foi possível estimar “{}” ({e}).", display_name(path));
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
        format!("{} → {approx}{}", human_size(original), human_size(estimated))
    } else {
        format!("{} arquivos: {} → {approx}{}", ok.len(), human_size(original), human_size(estimated))
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

/// Constrói as linhas de opções da ferramenta dentro de `group`.
fn build_options(info: &ToolInfo, group: &adw::PreferencesGroup) -> Options {
    match info.id {
        ToolId::Merge => {
            group.set_description(Some("Os arquivos são unidos na ordem da lista acima. Use as setas para reordenar."));
            Options::None
        }
        ToolId::Split => {
            let mode = combo("Modo", &["Por intervalos", "A cada N páginas", "Uma página por arquivo"], 0);
            let ranges = entry("Intervalos — cada um vira um arquivo (ex.: 1-3, 4-8, 9-)");
            let every = spin("Páginas por arquivo", "", 1.0, 10_000.0, 1.0, 2.0);
            every.set_visible(false);
            let (r, e) = (ranges.clone(), every.clone());
            mode.connect_selected_notify(move |m| {
                r.set_visible(m.selected() == 0);
                e.set_visible(m.selected() == 1);
            });
            group.add(&mode);
            group.add(&ranges);
            group.add(&every);
            Options::Split { mode, ranges, every }
        }
        ToolId::Extract => {
            let spec = entry("Páginas a extrair (ex.: 1-3, 7, 10-)");
            let separate = adw::SwitchRow::builder()
                .title("Um arquivo por página")
                .subtitle("Desligado: todas as páginas escolhidas em um único PDF, na ordem digitada")
                .build();
            group.add(&spec);
            group.add(&separate);
            Options::Extract { spec, separate }
        }
        ToolId::Remove => {
            let spec = entry("Páginas a remover (ex.: 2, 5-7)");
            group.add(&spec);
            Options::Remove { spec }
        }
        ToolId::Rotate => {
            let angle = combo(
                "Rotação",
                &["↻ 90° no sentido horário", "180° (de cabeça para baixo)", "↺ 90° no sentido anti-horário"],
                0,
            );
            let spec = entry("Páginas (deixe vazio para todas)");
            group.add(&angle);
            group.add(&spec);
            Options::Rotate { angle, spec }
        }
        ToolId::ImagesToPdf => {
            let size = combo("Tamanho da página", &["A4", "Carta", "Igual à imagem"], 0);
            let margin = spin("Margem", "Em milímetros", 0.0, 50.0, 1.0, 10.0);
            let one_file = adw::SwitchRow::builder()
                .title("Juntar em um único PDF")
                .subtitle("Desligado: um PDF para cada imagem")
                .active(true)
                .build();
            let m = margin.clone();
            size.connect_selected_notify(move |s| m.set_sensitive(s.selected() != 2));
            group.set_description(Some(
                "JPEGs são incorporados sem recompressão. A orientação gravada pela câmera (EXIF) é respeitada.",
            ));
            group.add(&size);
            group.add(&margin);
            group.add(&one_file);
            Options::Images { size, margin, one_file }
        }
        ToolId::PdfToImages => {
            let format = combo("Formato", &["PNG — sem perda", "JPEG — arquivos menores", "TIFF — sem perda (LZW)"], 0);
            let dpi =
                spin("Resolução", "Pontos por polegada: 150 para tela, 300 para impressão", 36.0, 1200.0, 10.0, 150.0);
            let quality = spin("Qualidade do JPEG", "De 10 a 100", 10.0, 100.0, 5.0, 90.0);
            quality.set_visible(false);
            let q = quality.clone();
            format.connect_selected_notify(move |f| q.set_visible(f.selected() == 1));
            group.add(&format);
            group.add(&dpi);
            group.add(&quality);
            Options::ToImages { format, dpi, quality }
        }
        ToolId::Compress => {
            let level = combo("Nível", &["Sem perda", "Recomendada", "Máxima", "Personalizada"], 1);
            let dpi = spin(
                "Resolução das imagens",
                "Em ppp: 72 a 100 para tela, 150 para e-mail, 300 para impressão",
                *CUSTOM_DPI.start() as f64,
                *CUSTOM_DPI.end() as f64,
                10.0,
                120.0,
            );
            dpi.set_visible(false);
            let estimate = widgets::plain_row("Tamanho estimado", "Adicione um arquivo para ver a estimativa.");
            let spinner = gtk::Spinner::builder().valign(gtk::Align::Center).visible(false).build();
            estimate.add_suffix(&spinner);
            estimate.set_subtitle_selectable(true);
            group.set_description(Some(&compress_explanation(CompressLevel::Recommended)));
            let (g, d) = (group.clone(), dpi.clone());
            level.connect_selected_notify(move |l| {
                d.set_visible(l.selected() == 3);
                g.set_description(Some(&compress_explanation(level_from(l.selected(), d.value()))))
            });
            group.add(&level);
            group.add(&dpi);
            group.add(&estimate);
            Options::Compress(Box::new(CompressRows {
                level,
                dpi,
                estimate,
                spinner,
                estimates: RefCell::new(HashMap::new()),
                pending: RefCell::new(None),
                generation: Cell::new(0),
            }))
        }
        ToolId::Organize | ToolId::Annotate => Options::None,
        ToolId::NumberPages => {
            let position = combo(
                "Posição",
                &[
                    "Embaixo, à direita",
                    "Embaixo, no centro",
                    "Embaixo, à esquerda",
                    "Em cima, à direita",
                    "Em cima, no centro",
                    "Em cima, à esquerda",
                ],
                1,
            );
            let format = combo("Formato", &["1, 2, 3…", "1 / 10", "Página 1 de 10"], 0);
            let first = spin("Começar em", "Número mostrado na primeira página numerada", 0.0, 99_999.0, 1.0, 1.0);
            let size = spin("Tamanho do texto", "Em pontos", 6.0, 36.0, 1.0, 11.0);
            let spec = entry("Páginas a numerar (vazio = todas; “2-” pula a capa)");
            group.set_description(Some(
                "O número é desenhado sobre a página, a 1 cm da borda; o conteúdo original não é alterado nem \
                 convertido em imagem.",
            ));
            for row in [
                position.upcast_ref::<gtk::Widget>(),
                format.upcast_ref(),
                first.upcast_ref(),
                size.upcast_ref(),
                spec.upcast_ref(),
            ] {
                group.add(row);
            }
            Options::Numbers { position, format, first, size, spec }
        }
        ToolId::Watermark => {
            let text = entry("Texto da marca d’água");
            text.set_text("CONFIDENCIAL");
            let color = combo("Cor", &["Cinza", "Vermelho", "Azul", "Preto"], 0);
            let opacity = spin("Opacidade", "Em %: quanto menor, mais transparente", 5.0, 100.0, 5.0, 30.0);
            let scale = spin("Tamanho", "Em % da diagonal da página", 10.0, 100.0, 5.0, 60.0);
            let diagonal = switch("Na diagonal", "Desligado: na horizontal, no centro da página", true);
            let sc = scale.clone();
            diagonal.connect_active_notify(move |d| {
                sc.set_subtitle(if d.is_active() {
                    "Em % da diagonal da página"
                } else {
                    "Em % da largura da página"
                })
            });
            let spec = entry("Páginas (deixe vazio para todas)");
            for row in [
                text.upcast_ref::<gtk::Widget>(),
                color.upcast_ref(),
                opacity.upcast_ref(),
                scale.upcast_ref(),
                diagonal.upcast_ref(),
                spec.upcast_ref(),
            ] {
                group.add(row);
            }
            Options::Watermark { text, color, opacity, scale, diagonal, spec }
        }
        ToolId::Metadata => {
            let title = entry("Título");
            let author = entry("Autor");
            let subject = entry("Assunto");
            let keywords = entry("Palavras-chave");
            let remove_all = switch(
                "Apagar todos os metadados",
                "Remove também programas e datas — útil antes de compartilhar o arquivo",
                false,
            );
            let facts =
                [info_row("Criado com"), info_row("Gerado por"), info_row("Criado em"), info_row("Modificado em")];
            group.set_description(Some("Campos deixados em branco são removidos do arquivo."));
            for row in
                [title.upcast_ref::<gtk::Widget>(), author.upcast_ref(), subject.upcast_ref(), keywords.upcast_ref()]
            {
                group.add(row);
            }
            for row in &facts {
                group.add(row);
            }
            group.add(&remove_all);
            let edits = [title.clone(), author.clone(), subject.clone(), keywords.clone()];
            remove_all.connect_active_notify(move |r| {
                for e in &edits {
                    e.set_sensitive(!r.is_active());
                }
            });
            Options::Metadata(Box::new(MetadataRows {
                title,
                author,
                subject,
                keywords,
                remove_all,
                facts,
                loaded: RefCell::new(None),
            }))
        }
        ToolId::Protect => {
            let password_row = password("Senha para abrir");
            let confirm = password("Repita a senha");
            let restrict = adw::ExpanderRow::builder()
                .title("Restringir impressão, cópia e edição")
                .subtitle("Exige uma segunda senha, só para mudar as permissões")
                .show_enable_switch(true)
                .enable_expansion(false)
                .build();
            let owner = password("Senha de permissões");
            let print = switch("Permitir imprimir", "", false);
            let copy = switch("Permitir copiar texto e imagens", "", false);
            let edit = switch("Permitir editar", "Inclui anotações e formulários", false);
            restrict.add_row(&owner);
            restrict.add_row(&print);
            restrict.add_row(&copy);
            restrict.add_row(&edit);
            group.set_description(Some(
                "Criptografia AES-256. Guarde a senha: sem ela, ninguém consegue abrir o arquivo — nem este \
                 aplicativo. O original continua sem senha.",
            ));
            group.add(&password_row);
            group.add(&confirm);
            group.add(&restrict);
            Options::Protect(Box::new(ProtectRows {
                password: password_row,
                confirm,
                restrict,
                owner,
                print,
                copy,
                edit,
            }))
        }
        ToolId::Unlock => {
            let password_row = password("Senha do documento");
            group.set_description(Some(
                "Informe a senha que você já conhece. Para remover restrições de impressão ou cópia, é preciso a \
                 senha de permissões (do proprietário). Nenhuma senha é adivinhada ou contornada.",
            ));
            group.add(&password_row);
            Options::Unlock { password: password_row }
        }
        ToolId::PdfToText => {
            let layout =
                switch("Manter o layout", "Preserva colunas e espaçamento; desligado, segue a ordem de leitura", false);
            let ocr = switch(
                "Reconhecer texto das páginas digitalizadas (OCR)",
                "Lendo os idiomas instalados…",
                Tool::Tesseract.find().is_some(),
            );
            ocr.set_sensitive(false);
            group.set_description(Some(
                "O texto selecionável é extraído como está. Páginas digitalizadas (imagens) só têm texto com o \
                 reconhecimento ligado; ele é feito numa cópia temporária e só o .txt é gravado.",
            ));
            group.add(&layout);
            group.add(&ocr);
            Options::Text { layout, ocr, ocr_languages: Rc::default() }
        }
        ToolId::Ocr => {
            let languages = adw::ExpanderRow::builder()
                .title("Idiomas do documento")
                .subtitle("Lendo os idiomas instalados…")
                .build();
            let skip = switch("Pular páginas que já têm texto", "Evita texto duplicado na busca e na cópia", true);
            let straighten = switch(
                "Endireitar páginas deitadas ou de cabeça para baixo",
                "Gira a página inteira, sem perda, quando o texto está de lado",
                true,
            );
            group.add(&languages);
            group.add(&skip);
            group.add(&straighten);
            Options::Ocr {
                languages: Rc::new(LanguageRows { row: languages, switches: RefCell::default() }),
                skip,
                straighten,
            }
        }
        ToolId::OfficeToPdf => {
            let isolated = if pdftoolbox::engines::network_isolation_available() {
                "A conversão roda isolada da rede."
            } else {
                "Neste sistema, a conversão não pode ser isolada da rede."
            };
            group.set_description(Some(&format!(
                "Word, Excel, PowerPoint, LibreOffice, RTF, CSV e TXT, convertidos pelo LibreOffice instalado. \
                 Macros não são executadas. {isolated} A primeira conversão pode levar alguns segundos."
            )));
            Options::Office
        }
    }
}

pub fn build(app: &Rc<AppState>, info: &'static ToolInfo, files: Vec<PathBuf>) -> adw::NavigationPage {
    let header = adw::HeaderBar::new();
    header.pack_end(&primary_menu());

    let body = gtk::Box::new(gtk::Orientation::Vertical, 24);
    body.set_margin_top(18);
    body.set_margin_bottom(32);
    body.set_margin_start(16);
    body.set_margin_end(16);

    let summary = gtk::Label::builder().label(info.summary).wrap(true).xalign(0.0).build();
    summary.add_css_class("dim-label");
    body.append(&summary);

    // Dependências
    let missing: Vec<Tool> = info.needs.iter().copied().filter(|t| t.find().is_none()).collect();
    let deps_ok = missing.is_empty();
    if !deps_ok {
        let group = adw::PreferencesGroup::builder()
            .title("Dependência ausente")
            .description("Esta ferramenta não pode ser usada até que o componente abaixo seja instalado. Rode o comando em um terminal e reabra a ferramenta.")
            .build();
        for tool in &missing {
            let row = widgets::plain_row(tool.label(), &tool.install_command());
            row.set_subtitle_selectable(true);
            row.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            group.add(&row);
        }
        let more = gtk::Button::builder().label("Ver todas as dependências").halign(gtk::Align::Start).build();
        more.set_action_name(Some("win.dependencies"));
        more.add_css_class("flat");
        group.set_header_suffix(Some(&more));
        body.append(&group);
    }

    // Arquivos
    let add_button = gtk::Button::builder()
        .label("Adicionar…")
        .valign(gtk::Align::Center)
        .tooltip_text("Adicionar arquivos (Ctrl+O)")
        .build();
    add_button.add_css_class("flat");
    let files_group = adw::PreferencesGroup::builder()
        .title(if info.multiple { "Arquivos" } else { "Arquivo" })
        .header_suffix(&add_button)
        .build();
    let empty_row = widgets::plain_row(
        "Nenhum arquivo",
        match info.accepts {
            Accepts::Pdf => "Arraste PDFs para esta janela ou clique em Adicionar",
            Accepts::Images => "Arraste imagens para esta janela ou clique em Adicionar",
            Accepts::Office => "Arraste documentos, planilhas ou apresentações para esta janela",
        },
    );
    empty_row.add_css_class("dim-label");
    files_group.add(&empty_row);
    body.append(&files_group);

    // Opções
    let options_group = adw::PreferencesGroup::builder().title("Opções").build();
    let options = build_options(info, &options_group);
    if !matches!(options, Options::None) || options_group.description().is_some() {
        body.append(&options_group);
    }
    let form_hint = gtk::Label::builder().wrap(true).xalign(0.0).visible(false).build();
    form_hint.set_accessible_role(gtk::AccessibleRole::Status);
    body.append(&form_hint);
    let stamp_preview = matches!(info.id, ToolId::NumberPages | ToolId::Watermark).then(StampPreview::new);
    if let Some(s) = &stamp_preview {
        body.append(&s.group);
    }
    let text_preview = (info.id == ToolId::PdfToText).then(TextPreview::new);
    if let Some(t) = &text_preview {
        body.append(&t.group);
    }

    // Pré-visualização
    let preview = match info.id {
        ToolId::Rotate | ToolId::Extract | ToolId::Remove | ToolId::NumberPages | ToolId::Watermark => {
            Some(PagePreview::new(true))
        }
        ToolId::Split => Some(PagePreview::new(false)),
        _ => None,
    };
    let layout = (info.id == ToolId::ImagesToPdf).then(LayoutPreview::new);
    if let Some(p) = &preview {
        body.append(&p.group);
    }
    if let Some(l) = &layout {
        body.append(&l.group);
    }

    // Destino
    let dest_group = adw::PreferencesGroup::builder()
        .title("Destino")
        .description("O original nunca é alterado. Se já existir um arquivo com o mesmo nome, o novo recebe um número, como “nome (2)”.")
        .build();
    let dest_row = widgets::plain_row("Pasta", "A mesma do primeiro arquivo");
    let dest_reset = widgets::icon_button("edit-undo-symbolic", "Voltar a usar a pasta do arquivo original");
    dest_reset.set_visible(false);
    let change = gtk::Button::builder().label("Alterar…").valign(gtk::Align::Center).build();
    dest_row.add_suffix(&dest_reset);
    dest_row.add_suffix(&change);
    dest_row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    let name_row = adw::EntryRow::builder().title("Nome do arquivo (opcional)").build();
    dest_group.add(&dest_row);
    dest_group.add(&name_row);
    body.append(&dest_group);

    // Ação e progresso
    let run_button = gtk::Button::builder().label(run_label(info.id)).halign(gtk::Align::Center).build();
    run_button.add_css_class("pill");
    run_button.add_css_class("suggested-action");
    run_button.set_tooltip_text(Some("Executar (Ctrl+Enter)"));
    let cancel_button = gtk::Button::builder().label("Cancelar").halign(gtk::Align::Center).visible(false).build();
    cancel_button.add_css_class("pill");
    let progress = gtk::ProgressBar::builder().visible(false).show_text(false).build();
    let status = gtk::Label::builder().wrap(true).visible(false).justify(gtk::Justification::Center).build();
    status.add_css_class("dim-label");
    status.set_accessible_role(gtk::AccessibleRole::Status);
    let action_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    action_box.append(&run_button);
    action_box.append(&progress);
    action_box.append(&status);
    action_box.append(&cancel_button);
    body.append(&action_box);

    let result_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&result_slot);

    let clamp = adw::Clamp::builder().maximum_size(720).child(&body).build();
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroll));
    let nav_page = adw::NavigationPage::with_tag(&toolbar, info.title, info.key);

    let page = Rc::new(ToolPage {
        app: Rc::downgrade(app),
        info,
        files: RefCell::new(Vec::new()),
        file_info: Rc::new(RefCell::new(HashMap::new())),
        files_group,
        file_rows: RefCell::new(Vec::new()),
        empty_row,
        options,
        options_group,
        preview,
        layout,
        stamp_preview,
        text_preview,
        form_hint,
        plan_ok: Cell::new(true),
        from_click: Cell::new(false),
        dest_dir: RefCell::new(None),
        dest_custom: Cell::new(false),
        dest_row,
        dest_reset: dest_reset.clone(),
        name_row,
        run_button: run_button.clone(),
        cancel_button: cancel_button.clone(),
        progress,
        status,
        result_slot,
        scroll: scroll.clone(),
        running: RefCell::new(None),
        deps_ok,
        text_pages: RefCell::new(HashMap::new()),
    });
    run_button.set_sensitive(deps_ok);

    // Ações da página (Ctrl+O e Ctrl+Enter)
    let group = gio::SimpleActionGroup::new();
    let add_action = gio::SimpleAction::new("add", None);
    let p = Rc::downgrade(&page);
    add_action.connect_activate(move |_, _| {
        if let Some(p) = p.upgrade() {
            p.pick_files();
        }
    });
    let run_action = gio::SimpleAction::new("run", None);
    let p = Rc::downgrade(&page);
    run_action.connect_activate(move |_, _| {
        if let Some(p) = p.upgrade() {
            p.run();
        }
    });
    group.add_action(&add_action);
    group.add_action(&run_action);
    nav_page.insert_action_group("page", Some(&group));
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Global);
    for (accel, action) in [("<Control>Return", "page.run"), ("<Control>KP_Enter", "page.run")] {
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(accel),
            Some(gtk::NamedAction::new(action)),
        ));
    }
    nav_page.add_controller(shortcuts);

    add_button.set_action_name(Some("page.add"));
    let p = Rc::downgrade(&page);
    run_button.connect_clicked(move |_| {
        if let Some(p) = p.upgrade() {
            p.run();
        }
    });
    let p = Rc::downgrade(&page);
    cancel_button.connect_clicked(move |b| {
        if let Some(p) = p.upgrade()
            && let Some(token) = p.running.borrow().as_ref()
        {
            token.cancel();
            b.set_sensitive(false);
            p.status.set_text("Cancelando…");
        }
    });
    let p = Rc::downgrade(&page);
    change.connect_clicked(move |_| {
        if let Some(p) = p.upgrade() {
            p.choose_folder();
        }
    });
    let p = Rc::downgrade(&page);
    dest_reset.connect_clicked(move |_| {
        if let Some(p) = p.upgrade() {
            p.dest_custom.set(false);
            p.update_destination();
        }
    });
    let p = Rc::downgrade(&page);
    widgets::add_file_drop(&nav_page, move |files| {
        if let Some(p) = p.upgrade() {
            p.add_files(files);
        }
    });
    // Qualquer mudança nas opções atualiza nome, pré-visualização e botão.
    let p = Rc::downgrade(&page);
    let refresh = move || {
        if let Some(p) = p.upgrade()
            && !p.from_click.get()
        {
            p.update_name_row();
            p.refresh_preview();
        }
    };
    match &page.options {
        Options::Split { mode, ranges, every } => {
            let r = refresh.clone();
            mode.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            ranges.connect_changed(move |_| r());
            let r = refresh.clone();
            every.connect_value_notify(move |_| r());
        }
        Options::Extract { spec, separate } => {
            let r = refresh.clone();
            separate.connect_active_notify(move |_| r());
            let r = refresh.clone();
            spec.connect_changed(move |_| r());
        }
        Options::Remove { spec } => {
            let r = refresh.clone();
            spec.connect_changed(move |_| r());
        }
        Options::Rotate { angle, spec } => {
            let r = refresh.clone();
            angle.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            spec.connect_changed(move |_| r());
        }
        Options::Images { size, margin, one_file } => {
            let r = refresh.clone();
            one_file.connect_active_notify(move |_| r());
            let r = refresh.clone();
            size.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            margin.connect_value_notify(move |_| r());
        }
        Options::ToImages { dpi, .. } => {
            let r = refresh.clone();
            dpi.connect_value_notify(move |_| r());
        }
        Options::Numbers { position, format, first, size, spec } => {
            let r = refresh.clone();
            position.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            format.connect_selected_notify(move |_| r());
            let r = refresh.clone();
            first.connect_value_notify(move |_| r());
            let r = refresh.clone();
            size.connect_value_notify(move |_| r());
            let r = refresh.clone();
            spec.connect_changed(move |_| r());
        }
        Options::Watermark { text, color, opacity, scale, diagonal, spec } => {
            for e in [text, spec] {
                let r = refresh.clone();
                e.connect_changed(move |_| r());
            }
            let r = refresh.clone();
            color.connect_selected_notify(move |_| r());
            for s in [opacity, scale] {
                let r = refresh.clone();
                s.connect_value_notify(move |_| r());
            }
            let r = refresh.clone();
            diagonal.connect_active_notify(move |_| r());
        }
        Options::Metadata(m) => {
            let r = refresh.clone();
            m.remove_all.connect_active_notify(move |_| r());
        }
        Options::Protect(p) => {
            for e in [&p.password, &p.confirm, &p.owner] {
                let r = refresh.clone();
                e.connect_changed(move |_| r());
            }
            let r = refresh.clone();
            p.restrict.connect_enable_expansion_notify(move |_| r());
        }
        Options::Unlock { password } => {
            let r = refresh.clone();
            password.connect_changed(move |_| r());
        }
        Options::Text { layout, ocr, ocr_languages } => {
            page.load_text_ocr(ocr.clone(), ocr_languages.clone());
            let p = Rc::downgrade(&page);
            layout.connect_active_notify(move |l| {
                if let Some(p) = p.upgrade()
                    && let Some(t) = &p.text_preview
                {
                    t.update(p.files.borrow().first().map(PathBuf::as_path), l.is_active());
                }
            });
        }
        Options::Ocr { languages, skip, .. } => {
            let r = refresh.clone();
            skip.connect_active_notify(move |_| r());
            page.load_languages(languages.clone(), refresh.clone());
        }
        Options::Compress(c) => {
            let p = Rc::downgrade(&page);
            c.level.connect_selected_notify(move |_| {
                if let Some(p) = p.upgrade() {
                    p.refresh_estimate();
                }
            });
            // Enquanto o valor muda (setas, digitação), espera parar antes de calcular.
            let p = Rc::downgrade(&page);
            c.dpi.connect_value_notify(move |_| {
                if let Some(p) = p.upgrade() {
                    p.refresh_estimate_later();
                }
            });
        }
        Options::None | Options::Office => {}
    }
    if let Some(preview) = &page.preview {
        let r = refresh.clone();
        preview.connect_loaded(r);
        let p = Rc::downgrade(&page);
        preview.connect_page_clicked(move |n| {
            if let Some(p) = p.upgrade() {
                p.toggle_page(n);
            }
        });
    }

    page.add_files(files);
    page.update_name_row();
    // Ferramentas que pedem algo digitado já abrem com o cursor no campo.
    let first_entry: Option<adw::EntryRow> = match &page.options {
        Options::Split { ranges, .. } => Some(ranges.clone()),
        Options::Extract { spec, .. } | Options::Remove { spec } | Options::Rotate { spec, .. } => Some(spec.clone()),
        Options::Numbers { spec, .. } => Some(spec.clone()),
        Options::Watermark { text, .. } => Some(text.clone()),
        Options::Metadata(m) => Some(m.title.clone()),
        Options::Protect(p) => Some(p.password.clone().upcast()),
        Options::Unlock { password } => Some(password.clone().upcast()),
        _ => None,
    };
    if let Some(entry) = first_entry {
        nav_page.connect_shown(move |_| {
            entry.grab_focus();
        });
    }
    // Enter no campo executa a ferramenta.
    for row in page.entries() {
        row.set_show_apply_button(false);
        let p = Rc::downgrade(&page);
        row.connect_entry_activated(move |_| {
            if let Some(p) = p.upgrade() {
                p.run();
            }
        });
    }
    // O estado vive enquanto o widget da página existir.
    let keep = page.clone();
    nav_page.connect_destroy(move |_| {
        let _ = &keep;
    });
    nav_page
}

impl ToolPage {
    /// Campos em que Enter executa a ferramenta.
    fn entries(&self) -> Vec<adw::EntryRow> {
        match &self.options {
            Options::Split { ranges, .. } => vec![ranges.clone()],
            Options::Extract { spec, .. } | Options::Remove { spec } | Options::Rotate { spec, .. } => {
                vec![spec.clone()]
            }
            Options::Numbers { spec, .. } => vec![spec.clone()],
            Options::Watermark { text, spec, .. } => vec![text.clone(), spec.clone()],
            Options::Metadata(m) => vec![m.title.clone(), m.author.clone(), m.subject.clone(), m.keywords.clone()],
            Options::Protect(p) => {
                vec![p.password.clone().upcast(), p.confirm.clone().upcast(), p.owner.clone().upcast()]
            }
            Options::Unlock { password } => vec![password.clone().upcast()],
            _ => vec![],
        }
    }

    /// Campo de páginas validado pela pré-visualização.
    fn spec_entry(&self) -> Option<adw::EntryRow> {
        match &self.options {
            Options::Split { ranges, .. } => Some(ranges.clone()),
            Options::Extract { spec, .. }
            | Options::Remove { spec }
            | Options::Rotate { spec, .. }
            | Options::Numbers { spec, .. }
            | Options::Watermark { spec, .. } => Some(spec.clone()),
            _ => None,
        }
    }

    fn window(&self) -> Option<adw::ApplicationWindow> {
        self.app.upgrade().map(|a| a.window.clone())
    }

    fn toast(&self, text: &str) {
        if let Some(app) = self.app.upgrade() {
            app.toast(text);
        }
    }

    fn pick_files(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let p = self.clone();
        glib::spawn_future_local(async move {
            let files = widgets::pick_files(&win, Some(p.info.accepts), p.info.multiple).await;
            p.add_files(files);
        });
    }

    fn choose_folder(self: &Rc<Self>) {
        let Some(win) = self.window() else { return };
        let p = self.clone();
        glib::spawn_future_local(async move {
            let initial = p.dest_dir.borrow().clone();
            if let Some(dir) = widgets::pick_folder(&win, initial.as_deref()).await {
                *p.dest_dir.borrow_mut() = Some(dir.clone());
                p.dest_custom.set(true);
                if let Some(app) = p.app.upgrade() {
                    let mut s = app.settings.borrow_mut();
                    s.last_dir = Some(dir);
                    s.save();
                }
                p.update_destination();
            }
        });
    }

    fn allows_duplicates(&self) -> bool {
        matches!(self.info.id, ToolId::Merge | ToolId::ImagesToPdf)
    }

    /// Acrescenta arquivos; PDFs com senha entram como cópia desbloqueada
    /// (a senha é pedida antes), exceto em “Remover senha”.
    pub fn add_files(self: &Rc<Self>, incoming: Vec<PathBuf>) {
        if incoming.is_empty() {
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        if self.info.id == ToolId::Unlock || self.info.accepts != Accepts::Pdf {
            self.add_ready_files(incoming);
            return;
        }
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let ready = super::unlock::prepare(&app, incoming).await;
            if let Some(p) = p.upgrade() {
                p.add_ready_files(ready);
            }
        });
    }

    fn add_ready_files(self: &Rc<Self>, incoming: Vec<PathBuf>) {
        if incoming.is_empty() {
            return;
        }
        let (ok, rejected): (Vec<PathBuf>, Vec<PathBuf>) =
            incoming.into_iter().partition(|f| catalog::extension_ok(self.info.accepts, f));
        if !rejected.is_empty() {
            let what = match self.info.accepts {
                Accepts::Pdf => "somente PDFs",
                Accepts::Images => "somente imagens JPEG, PNG, WebP, GIF, BMP ou TIFF",
                Accepts::Office => "somente documentos, planilhas e apresentações",
            };
            self.toast(&format!("{} arquivo(s) ignorado(s): esta ferramenta aceita {what}.", rejected.len()));
        }
        {
            let mut files = self.files.borrow_mut();
            if !self.info.multiple {
                files.clear();
                files.extend(ok.into_iter().take(1));
            } else {
                let mut skipped = 0;
                for f in ok {
                    if !self.allows_duplicates() && files.contains(&f) {
                        skipped += 1;
                    } else {
                        files.push(f);
                    }
                }
                if skipped > 0 {
                    self.toast(&format!("{skipped} arquivo(s) já estava(m) na lista."));
                }
            }
        }
        self.rebuild_files();
    }

    fn rebuild_files(self: &Rc<Self>) {
        for row in self.file_rows.borrow_mut().drain(..) {
            self.files_group.remove(&row);
        }
        let files = self.files.borrow().clone();
        self.empty_row.set_visible(files.is_empty());
        let ordered = self.info.multiple && matches!(self.info.id, ToolId::Merge | ToolId::ImagesToPdf);
        for (i, path) in files.iter().enumerate() {
            let is_pdf = self.info.accepts == Accepts::Pdf;
            let unlock_tool = self.info.id == ToolId::Unlock;
            let subtitle = self
                .file_info
                .borrow()
                .get(path)
                .map(|f| file_line(path, f, is_pdf, unlock_tool))
                .unwrap_or_else(|| "Lendo…".into());
            let row = widgets::plain_row(&display_name(path), &subtitle);
            row.set_tooltip_text(Some(&super::unlock::original(path).to_string_lossy()));
            if ordered && files.len() > 1 {
                let n = gtk::Label::new(Some(&(i + 1).to_string()));
                n.add_css_class("badge");
                n.add_css_class("acento");
                n.set_valign(gtk::Align::Center);
                n.set_tooltip_text(Some("Posição no resultado"));
                row.add_prefix(&n);
            }
            if thumbs::is_office(path) {
                let icon = gtk::Image::from_icon_name("pdftoolbox-office-para-pdf-symbolic");
                icon.set_pixel_size(32);
                icon.add_css_class("icone-documento");
                icon.set_margin_top(8);
                icon.set_margin_bottom(8);
                row.add_prefix(&icon);
            } else {
                let thumb = thumbs::picture(path, 40, 48);
                thumb.set_margin_top(6);
                thumb.set_margin_bottom(6);
                row.add_prefix(&thumb);
            }
            if ordered && files.len() > 1 {
                let up = widgets::icon_button("go-up-symbolic", "Mover para cima");
                let down = widgets::icon_button("go-down-symbolic", "Mover para baixo");
                up.set_sensitive(i > 0);
                down.set_sensitive(i + 1 < files.len());
                for (b, delta) in [(&up, -1isize), (&down, 1)] {
                    let p = Rc::downgrade(self);
                    b.connect_clicked(move |_| {
                        if let Some(p) = p.upgrade() {
                            let j = i as isize + delta;
                            p.files.borrow_mut().swap(i, j as usize);
                            p.rebuild_files();
                        }
                    });
                }
                row.add_suffix(&up);
                row.add_suffix(&down);
            }
            let remove = widgets::icon_button("list-remove-symbolic", "Remover da lista");
            let p = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.files.borrow_mut().remove(i);
                    p.rebuild_files();
                }
            });
            row.add_suffix(&remove);
            self.files_group.add(&row);
            self.file_rows.borrow_mut().push(row.clone());
            if !self.file_info.borrow().contains_key(path) {
                self.load_info(path.clone(), row);
            }
        }
        self.update_destination();
        self.update_name_row();
        if let Some(preview) = &self.preview {
            preview.set_files(&files);
        }
        if let (Some(t), Options::Text { layout, .. }) = (&self.text_preview, &self.options) {
            t.update(files.first().map(PathBuf::as_path), layout.is_active());
        }
        if let Options::Metadata(m) = &self.options {
            self.load_metadata(m, files.first().cloned());
        }
        if let Options::Compress(_) = &self.options {
            self.refresh_estimate();
        }
        if let Options::Ocr { .. } = &self.options {
            for path in &files {
                if self.text_pages.borrow().contains_key(path) {
                    continue;
                }
                self.text_pages.borrow_mut().insert(path.clone(), None);
                let p = Rc::downgrade(self);
                let path = path.clone();
                glib::spawn_future_local(async move {
                    let f = path.clone();
                    let counts = widgets::blocking(move || operations::text_pages(&f, &CancelToken::new()).ok()).await;
                    if let Some(p) = p.upgrade() {
                        if let Some(c) = counts {
                            p.text_pages.borrow_mut().insert(path, Some(c));
                        }
                        p.refresh_preview();
                    }
                });
            }
        }
        self.refresh_preview();
    }

    /// PDF para texto: o OCR usa os idiomas escolhidos na ferramenta OCR
    /// (ou português e inglês), entre os instalados.
    fn load_text_ocr(&self, row: adw::SwitchRow, languages: Rc<RefCell<Vec<String>>>) {
        if Tool::Tesseract.find().is_none() {
            row.set_subtitle(&format!("Requer o Tesseract: {}", Tool::Tesseract.install_command()));
            return;
        }
        let saved = self.app.upgrade().map(|a| a.settings.borrow().ocr_languages.clone()).unwrap_or_default();
        glib::spawn_future_local(async move {
            let installed =
                widgets::blocking(|| operations::installed_languages(&CancelToken::new()).unwrap_or_default()).await;
            let mut chosen: Vec<String> = installed.iter().filter(|c| saved.contains(c)).cloned().collect();
            if chosen.is_empty() {
                chosen = installed.iter().filter(|c| ["por", "eng"].contains(&c.as_str())).cloned().collect();
            }
            if chosen.is_empty() {
                chosen = installed.iter().take(1).cloned().collect();
            }
            if chosen.is_empty() {
                row.set_subtitle("Nenhum idioma do Tesseract instalado");
                row.set_active(false);
                return;
            }
            let names: Vec<String> = chosen.iter().map(|c| operations::language_name(c).to_lowercase()).collect();
            row.set_subtitle(&format!(
                "Só nas páginas sem texto · em {} (idiomas da ferramenta OCR) · leva alguns segundos por página",
                names.join(" e ")
            ));
            *languages.borrow_mut() = chosen;
            row.set_sensitive(true);
        });
    }

    /// OCR: um interruptor por idioma instalado, ligados os da última vez
    /// (ou português e inglês).
    fn load_languages(&self, rows: Rc<LanguageRows>, refresh: impl Fn() + Clone + 'static) {
        if !self.deps_ok {
            rows.row.set_subtitle("O Tesseract não está instalado");
            return;
        }
        let saved = self.app.upgrade().map(|a| a.settings.borrow().ocr_languages.clone()).unwrap_or_default();
        let app = self.app.clone();
        glib::spawn_future_local(async move {
            let installed =
                widgets::blocking(|| operations::installed_languages(&CancelToken::new()).unwrap_or_default()).await;
            let wanted: Vec<&String> = installed.iter().filter(|c| saved.contains(c)).collect();
            let defaults = ["por", "eng"];
            for code in &installed {
                let on = if wanted.is_empty() {
                    defaults.contains(&code.as_str()) || (installed.len() == 1)
                } else {
                    saved.contains(code)
                };
                let switch = adw::SwitchRow::builder().title(operations::language_name(code)).active(on).build();
                if operations::language_name(code) != *code {
                    switch.set_subtitle(code);
                }
                let (r, rows_weak, app) = (refresh.clone(), Rc::downgrade(&rows), app.clone());
                switch.connect_active_notify(move |_| {
                    if let Some(rows) = rows_weak.upgrade() {
                        rows.update_subtitle();
                        if let Some(app) = app.upgrade() {
                            let mut s = app.settings.borrow_mut();
                            s.ocr_languages = rows.selected();
                            s.save();
                        }
                    }
                    r();
                });
                rows.row.add_row(&switch);
                rows.switches.borrow_mut().push((code.clone(), switch));
            }
            let example = if installed.iter().any(|c| c == "spa") { "fra" } else { "spa" };
            let more = widgets::plain_row(
                "Outros idiomas",
                &format!(
                    "Instale os dados do Tesseract do idioma e reabra esta ferramenta. Exemplo ({}): {}",
                    operations::language_name(example).to_lowercase(),
                    operations::language_install(example)
                ),
            );
            more.set_subtitle_selectable(true);
            more.add_css_class("dim-label");
            rows.row.add_row(&more);
            rows.row.set_expanded(installed.is_empty());
            rows.update_subtitle();
            refresh();
        });
    }

    /// Preenche os campos com os metadados do arquivo (só ao trocar de arquivo,
    /// para não apagar o que o usuário já digitou).
    fn load_metadata(&self, m: &MetadataRows, file: Option<PathBuf>) {
        if *m.loaded.borrow() == file {
            return;
        }
        *m.loaded.borrow_mut() = file.clone();
        let rows = [m.title.clone(), m.author.clone(), m.subject.clone(), m.keywords.clone()];
        for r in &rows {
            r.set_text("");
        }
        for r in &m.facts {
            r.set_subtitle("—");
        }
        let Some(file) = file else { return };
        let facts = m.facts.clone();
        let loaded = file.clone();
        let Options::Metadata(_) = &self.options else { return };
        let weak_title = m.title.downgrade();
        glib::spawn_future_local(async move {
            let path = file.clone();
            let info =
                widgets::blocking(move || poppler::doc_info(&path, &pdftoolbox::jobs::CancelToken::new()).ok()).await;
            // A página pode ter sido fechada ou o arquivo trocado nesse meio-tempo.
            if weak_title.upgrade().is_none() || loaded != file {
                return;
            }
            let Some(info) = info else { return };
            for (row, value) in rows.iter().zip([&info.title, &info.author, &info.subject, &info.keywords]) {
                if row.text().is_empty() {
                    row.set_text(value);
                }
            }
            let dates = [friendly_date(&info.created), friendly_date(&info.modified)];
            let values = [&info.creator, &info.producer, &dates[0], &dates[1]];
            for (row, value) in facts.iter().zip(values) {
                row.set_subtitle(if value.is_empty() { "—" } else { value.as_str() });
            }
        });
    }

    /// Lê páginas/dimensões em segundo plano e atualiza a linha.
    fn load_info(self: &Rc<Self>, path: PathBuf, row: adw::ActionRow) {
        let is_pdf = self.info.accepts == Accepts::Pdf;
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let facts = thumbs::facts(&path).await;
            let unlock_tool = p.upgrade().is_some_and(|p| p.info.id == ToolId::Unlock);
            row.set_subtitle(&file_line(&path, &facts, is_pdf, unlock_tool));
            if facts.problem.is_some() || (facts.locked && !unlock_tool) {
                row.add_css_class("texto-erro");
            }
            if let Some(p) = p.upgrade() {
                p.file_info.borrow_mut().insert(path, facts);
                p.refresh_preview();
            }
        });
    }

    /// Plano da ferramenta para o arquivo mostrado na pré-visualização.
    /// `None` quando ainda não há o que mostrar (sem arquivo ou sem páginas digitadas).
    fn plan(&self) -> Option<Result<Plan, String>> {
        let total = self.preview.as_ref()?.total()?;
        let spec_of = |row: &adw::EntryRow| row.text().trim().to_string();
        let result = match &self.options {
            Options::Rotate { angle, spec } => preview::rotate(&spec_of(spec), angle_of(angle.selected()), total),
            Options::Extract { spec, separate } => {
                let t = spec_of(spec);
                if t.is_empty() {
                    return None;
                }
                preview::extract(&t, separate.is_active(), total)
            }
            Options::Remove { spec } => {
                let t = spec_of(spec);
                if t.is_empty() {
                    return None;
                }
                preview::remove(&t, total)
            }
            Options::Split { mode, ranges, every } => {
                let mode = match mode.selected() {
                    0 => {
                        let t = spec_of(ranges);
                        if t.is_empty() {
                            return None;
                        }
                        SplitMode::Ranges(t)
                    }
                    1 => SplitMode::Every(every.value() as u32),
                    _ => SplitMode::EachPage,
                };
                preview::split(&mode, total)
            }
            Options::Numbers { format, first, spec, .. } => preview::numbers(
                &spec_of(spec),
                FORMATS[(format.selected() as usize).min(2)],
                first.value() as u32,
                total,
            ),
            Options::Watermark { spec, .. } => preview::watermark(&spec_of(spec), total),
            _ => return None,
        };
        Some(result.map_err(|e| e.to_string()))
    }

    /// Atualiza pré-visualizações, resumos e o rótulo do botão principal.
    fn refresh_preview(&self) {
        let files = self.files.borrow().clone();
        match &self.options {
            Options::Images { size, margin, .. } => {
                if let Some(layout) = &self.layout {
                    let info = self.file_info.borrow();
                    let images: Vec<(PathBuf, Option<(f32, f32)>)> =
                        files.iter().map(|f| (f.clone(), info.get(f).and_then(|i| i.dims))).collect();
                    layout.update(&images, image_layout(size.selected(), margin.value()));
                }
            }
            Options::ToImages { dpi, .. } => {
                let info = self.file_info.borrow();
                let base = "Cada página vira um arquivo de imagem.";
                let text = match files.first().and_then(|f| info.get(f).map(|i| (f, i))) {
                    Some((f, facts)) if facts.dims.is_some() => {
                        let (w, h) = facts.dims.unwrap();
                        let d = dpi.value();
                        let (pw, ph) = ((w as f64 / 72.0 * d).round(), (h as f64 / 72.0 * d).round());
                        format!(
                            "{base} Em {} ppp, a página 1 de “{}” terá {} × {} px (≈ {} megapixels).",
                            d as u32,
                            display_name(f),
                            pw,
                            ph,
                            format!("{:.1}", pw * ph / 1e6).replace('.', ",")
                        )
                    }
                    _ => base.to_string(),
                };
                self.options_group.set_description(Some(&glib::markup_escape_text(&text)));
            }
            Options::Ocr { skip, .. } => {
                let text = match self.ocr_counts() {
                    Some((with_text, total)) => {
                        let mut t = ocr_summary(with_text, total, skip.is_active());
                        t.push_str(" Leva alguns segundos por página.");
                        t
                    }
                    None => "As páginas digitalizadas continuam como estão; o texto reconhecido entra numa \
                             camada invisível por cima, que permite buscar e copiar."
                        .into(),
                };
                self.options_group.set_description(Some(&text));
            }
            Options::Metadata(m) => {
                let text = if m.remove_all.is_active() {
                    "Título, autor, assunto, palavras-chave, programas e datas serão apagados."
                } else {
                    "Campos deixados em branco são removidos do arquivo."
                };
                self.options_group.set_description(Some(text));
            }
            Options::None if self.info.id == ToolId::Merge => {
                let info = self.file_info.borrow();
                let pages: Option<u32> = files.iter().map(|f| info.get(f).and_then(|i| i.pages)).sum();
                let mut text =
                    "Os arquivos são unidos na ordem da lista acima. Use as setas para reordenar.".to_string();
                if let (Some(n), true) = (pages, files.len() >= 2) {
                    text.push_str(&format!(" Resultado: um PDF com {n} páginas."));
                }
                self.options_group.set_description(Some(&text));
            }
            _ => {}
        }

        // Validação dos campos que não são páginas (senhas, texto…).
        let check = self.form_check();
        let (form_ok, hint, is_error) = match &check {
            Ok(None) => (true, None, false),
            Ok(Some(warning)) => (true, Some(warning.clone()), false),
            Err(FormIssue::Incomplete(msg)) => (false, Some(msg.clone()).filter(|m| !m.is_empty()), false),
            Err(FormIssue::Invalid(msg)) => (false, Some(msg.clone()), true),
        };
        self.form_hint.set_visible(hint.is_some());
        self.form_hint.set_text(hint.as_deref().unwrap_or(""));
        self.form_hint.set_css_classes(if is_error { &["texto-erro"] } else { &["texto-aviso"] });

        // Grade de páginas, validação ao vivo e botão principal.
        let Some(preview) = &self.preview else {
            self.run_button.set_sensitive(self.deps_ok && form_ok);
            if self.running.borrow().is_none() {
                self.run_button.set_label(&self.action_label(None));
            }
            return;
        };
        let plan = self.plan();
        let idle = match &self.options {
            Options::Extract { .. } => Some("Digite as páginas a extrair ou clique nelas abaixo."),
            Options::Remove { .. } => Some("Digite as páginas a remover ou clique nelas abaixo."),
            Options::Split { .. } => Some("Digite os intervalos; cada um vira um arquivo, com uma cor."),
            _ => None,
        };
        let entry = self.spec_entry();
        match &plan {
            Some(Ok(plan)) => preview.apply(Ok(plan), None, !self.from_click.get()),
            Some(Err(e)) => preview.apply(Err(e.clone()), None, false),
            None => preview.apply(Err(String::new()), idle, false),
        }
        let invalid = matches!(plan, Some(Err(_)));
        if let Some(entry) = entry {
            if invalid {
                entry.add_css_class("error");
            } else {
                entry.remove_css_class("error");
            }
        }
        self.plan_ok.set(!invalid);
        let waiting = plan.is_none() && idle.is_some() && preview.total().is_some();
        self.run_button.set_sensitive(self.deps_ok && !invalid && !waiting && form_ok);
        self.refresh_stamp(plan.as_ref().and_then(|p| p.as_ref().ok()), form_ok);
        if self.running.borrow().is_none() {
            self.run_button.set_label(&self.action_label(plan.as_ref().and_then(|p| p.as_ref().ok())));
        }
    }

    /// Problemas nos campos: `Err` impede executar; `Ok(Some)` é só um aviso.
    fn form_check(&self) -> Result<Option<String>, FormIssue> {
        match &self.options {
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
            Options::Unlock { password } => {
                if password.text().is_empty() {
                    Err(FormIssue::Incomplete(String::new()))
                } else {
                    Ok(None)
                }
            }
            Options::Ocr { languages, .. } if languages.selected().is_empty() => {
                Err(FormIssue::Incomplete(if languages.switches.borrow().is_empty() {
                    String::new()
                } else {
                    "Escolha pelo menos um idioma do documento.".into()
                }))
            }
            Options::Ocr { skip, .. } => match self.ocr_counts() {
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

    /// “Como vai ficar”: primeira página afetada, com o carimbo real.
    fn refresh_stamp(&self, plan: Option<&Plan>, form_ok: bool) {
        let Some(sp) = &self.stamp_preview else { return };
        let (Some(plan), Some(path), true) = (plan, self.preview.as_ref().and_then(|p| p.current()), form_ok) else {
            sp.hide();
            return;
        };
        let Some(page) = plan.first_marked() else {
            sp.hide();
            return;
        };
        let chosen = plan.marks.iter().filter(|m| m.kind != preview::MarkKind::Normal).count() as u32;
        match &self.options {
            Options::Numbers { position, format, first, size, .. } => {
                let style = number_style(position, format, first, size);
                let (n, last) = (style.first_number, style.first_number + chosen.saturating_sub(1));
                let label = stamp::number_text(style.format, n, last);
                let caption = format!("Página {page}, a primeira numerada: recebe “{label}”.");
                sp.show(&path, page, &caption, Box::new(move |size| stamp::number_page(size, n, last, &style)));
            }
            Options::Watermark { text, color, opacity, scale, diagonal, .. } => {
                let style = watermark_style(text, color, opacity, scale, diagonal);
                let caption = format!("Página {page}, a primeira com a marca d’água.");
                sp.show(&path, page, &caption, Box::new(move |size| stamp::watermark(&[size], &[1], &style).remove(0)));
            }
            _ => sp.hide(),
        }
    }

    /// Rótulo do botão que diz exatamente o que vai acontecer.
    fn action_label(&self, plan: Option<&Plan>) -> String {
        let base = run_label(self.info.id).to_string();
        let files = self.files.borrow().len();
        let many = |verb: &str, noun: &str| format!("{verb} {files} {noun}");
        match &self.options {
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
        let Some(plan) = plan else { return base };
        let count = |f: &dyn Fn(&preview::MarkKind) -> bool| plan.marks.iter().filter(|m| f(&m.kind)).count();
        let pages = |n: usize| if n == 1 { "1 página".to_string() } else { format!("{n} páginas") };
        match &self.options {
            Options::Rotate { .. } if self.files.borrow().len() > 1 => {
                format!("Girar {} arquivos", self.files.borrow().len())
            }
            Options::Rotate { .. } => {
                let n = count(&|k| *k == preview::MarkKind::Selected);
                if n == plan.marks.len() { "Girar todas as páginas".into() } else { format!("Girar {}", pages(n)) }
            }
            Options::Extract { spec, .. } => {
                let n = pages::parse_list(&spec.text(), plan.marks.len() as u32).map(|l| l.len()).unwrap_or(0);
                format!("Extrair {}", pages(n))
            }
            Options::Remove { .. } => format!("Remover {}", pages(count(&|k| *k == preview::MarkKind::Removed))),
            Options::Numbers { .. } => {
                let n = count(&|k| *k == preview::MarkKind::Selected);
                if n == plan.marks.len() { "Numerar todas as páginas".into() } else { format!("Numerar {}", pages(n)) }
            }
            Options::Watermark { .. } => {
                let n = count(&|k| *k == preview::MarkKind::Selected);
                if n == plan.marks.len() {
                    "Aplicar em todas as páginas".into()
                } else {
                    format!("Aplicar em {}", pages(n))
                }
            }
            Options::Split { .. } => {
                let n = plan
                    .marks
                    .iter()
                    .filter_map(|m| match m.kind {
                        preview::MarkKind::Group(g) => Some(g + 1),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                // Intervalos sobrepostos: o maior índice de grupo é a contagem de arquivos.
                if n == 1 { "Dividir em 1 arquivo".into() } else { format!("Dividir em {n} arquivos") }
            }
            _ => base,
        }
    }

    /// Clique numa página da pré-visualização: marca ou desmarca no campo.
    fn toggle_page(&self, page: u32) {
        let Some(total) = self.preview.as_ref().and_then(|p| p.total()) else { return };
        let (row, ordered) = match &self.options {
            Options::Rotate { spec, .. }
            | Options::Remove { spec }
            | Options::Numbers { spec, .. }
            | Options::Watermark { spec, .. } => (spec, false),
            Options::Extract { spec, .. } => (spec, true),
            _ => return,
        };
        let text = preview::toggle_page(&row.text(), page, total, ordered);
        // `set_text` apaga e reinsere o texto (dois avisos de mudança); o
        // estado intermediário vazio não deve ser mostrado nem mover a grade.
        self.from_click.set(true);
        row.set_text(&text);
        row.set_position(-1);
        self.update_name_row();
        self.refresh_preview();
        self.from_click.set(false);
    }

    fn update_destination(&self) {
        if !self.dest_custom.get() {
            let first =
                self.files.borrow().first().and_then(|f| super::unlock::original(f).parent().map(Path::to_path_buf));
            *self.dest_dir.borrow_mut() = first;
        }
        self.dest_reset.set_visible(self.dest_custom.get());
        let label = match (&*self.dest_dir.borrow(), self.dest_custom.get()) {
            (Some(d), true) => widgets::path_label(d),
            (Some(d), false) => format!("{} (pasta do original)", widgets::path_label(d)),
            (None, _) => "A mesma do primeiro arquivo".into(),
        };
        self.dest_row.set_subtitle(&label);
    }

    /// OCR: soma das páginas com texto e do total dos arquivos da lista,
    /// quando todos já foram lidos.
    fn ocr_counts(&self) -> Option<(u32, u32)> {
        let files = self.files.borrow();
        if files.is_empty() {
            return None;
        }
        let known = self.text_pages.borrow();
        files.iter().try_fold((0, 0), |(a, b), f| known.get(f).copied().flatten().map(|(t, n)| (a + t, b + n)))
    }

    fn single_output(&self) -> bool {
        let n = self.files.borrow().len();
        match &self.options {
            Options::Extract { separate, .. } => !separate.is_active(),
            Options::Images { one_file, .. } => one_file.is_active() || n <= 1,
            Options::Split { .. } | Options::ToImages { .. } => false,
            Options::Rotate { .. }
            | Options::Compress(_)
            | Options::Numbers { .. }
            | Options::Watermark { .. }
            | Options::Protect(_)
            | Options::Unlock { .. }
            | Options::Text { .. }
            | Options::Office
            | Options::Ocr { .. } => n <= 1,
            Options::Remove { .. } | Options::None | Options::Metadata(_) => true,
        }
    }

    fn update_name_row(&self) {
        self.name_row.set_visible(self.single_output());
    }

    fn request(&self) -> OpResult<Request> {
        let files = self.files.borrow().clone();
        if files.len() < self.info.min_files {
            return Err(if self.info.min_files > 1 {
                OpError::InvalidRange(format!("Adicione pelo menos {} arquivos.", self.info.min_files))
            } else {
                OpError::NoInput
            });
        }
        let first = files[0].clone();
        let need_text = |row: &adw::EntryRow, what: &str| -> OpResult<String> {
            let t = row.text().trim().to_string();
            if t.is_empty() { Err(OpError::InvalidRange(format!("Informe {what}."))) } else { Ok(t) }
        };
        Ok(match &self.options {
            Options::None => Request::Merge { inputs: files },
            Options::Split { mode, ranges, every } => Request::Split {
                input: first,
                mode: match mode.selected() {
                    0 => SplitMode::Ranges(need_text(ranges, "os intervalos de páginas")?),
                    1 => SplitMode::Every(every.value() as u32),
                    _ => SplitMode::EachPage,
                },
            },
            Options::Extract { spec, separate } => Request::Extract {
                input: first,
                spec: need_text(spec, "as páginas a extrair")?,
                separate: separate.is_active(),
            },
            Options::Remove { spec } => {
                Request::Remove { input: first, spec: need_text(spec, "as páginas a remover")? }
            }
            Options::Rotate { angle, spec } => Request::Rotate {
                inputs: files,
                spec: Some(spec.text().to_string()).filter(|s| !s.trim().is_empty()),
                angle: angle_of(angle.selected()),
            },
            Options::Images { size, margin, one_file } => Request::ImagesToPdf {
                inputs: files,
                layout: image_layout(size.selected(), margin.value()),
                one_file: one_file.is_active(),
            },
            Options::ToImages { format, dpi, quality } => Request::PdfToImages {
                inputs: files,
                format: match format.selected() {
                    0 => ImageFormat::Png,
                    1 => ImageFormat::Jpeg,
                    _ => ImageFormat::Tiff,
                },
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

    /// Recalcula a estimativa depois de uma pausa (resolução sendo digitada).
    fn refresh_estimate_later(self: &Rc<Self>) {
        let Options::Compress(c) = &self.options else { return };
        c.stop();
        let generation = c.generation.get() + 1;
        c.generation.set(generation);
        // O valor anterior não vale mais para a nova resolução.
        if !self.files.borrow().is_empty() {
            c.spinner.set_visible(true);
            c.spinner.set_spinning(true);
            c.estimate.set_subtitle("Calculando…");
        }
        let p = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
            if let Some(p) = p.upgrade()
                && let Options::Compress(c) = &p.options
                && c.generation.get() == generation
            {
                p.refresh_estimate();
            }
        });
    }

    /// Mostra a estimativa do nível escolhido, calculando o que faltar em
    /// segundo plano (um arquivo de cada vez; mudar o nível cancela o cálculo).
    fn refresh_estimate(self: &Rc<Self>) {
        let Options::Compress(c) = &self.options else { return };
        c.stop();
        let generation = c.generation.get() + 1;
        c.generation.set(generation);
        let files = self.files.borrow().clone();
        let level = c.level();
        let missing_tool = level.needs().iter().any(|t| t.find().is_none());
        c.estimate.set_visible(!missing_tool);
        c.spinner.set_visible(false);
        c.spinner.set_spinning(false);
        if files.is_empty() || missing_tool {
            c.estimate.set_subtitle("Adicione um arquivo para ver a estimativa.");
            return;
        }
        let todo: Vec<PathBuf> =
            files.iter().filter(|f| !c.estimates.borrow().contains_key(&((*f).clone(), level))).cloned().collect();
        if todo.is_empty() {
            let known = c.estimates.borrow();
            let results: Vec<(&Path, &Result<Estimate, String>)> =
                files.iter().map(|f| (f.as_path(), &known[&(f.clone(), level)])).collect();
            c.estimate.set_subtitle(&estimate_text(&results));
            return;
        }
        c.spinner.set_visible(true);
        c.spinner.set_spinning(true);
        c.estimate.set_subtitle(if files.len() == 1 {
            "Calculando… (comprimindo algumas páginas numa pasta temporária)"
        } else {
            "Calculando… (comprimindo algumas páginas de cada arquivo numa pasta temporária)"
        });
        let cancel = CancelToken::new();
        *c.pending.borrow_mut() = Some(cancel.clone());
        let p = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            for file in todo {
                let (f, token) = (file.clone(), cancel.clone());
                let result = widgets::blocking(move || operations::estimate_compression(&f, level, &token)).await;
                if cancel.is_cancelled() {
                    return;
                }
                let Some(page) = p.upgrade() else { return };
                let Options::Compress(c) = &page.options else { return };
                if c.generation.get() != generation {
                    return;
                }
                let result = result.map_err(|e| e.title().to_string());
                c.estimates.borrow_mut().insert((file, level), result);
            }
            if let Some(page) = p.upgrade() {
                page.refresh_estimate();
            }
        });
    }

    fn set_running(&self, running: bool) {
        // O foco acompanha o botão visível: sem isso, ao esconder o botão
        // focado o GTK leva o foco para o topo da janela.
        let had_focus = self.run_button.has_focus() || self.cancel_button.has_focus();
        self.run_button.set_visible(!running);
        self.cancel_button.set_visible(running);
        self.cancel_button.set_sensitive(true);
        self.progress.set_visible(running);
        self.status.set_visible(running);
        self.files_group.set_sensitive(!running);
        let focus_lost = self.window().is_some_and(|w| gtk::prelude::GtkWindowExt::focus(&w).is_none());
        if had_focus || running || focus_lost {
            if running {
                self.cancel_button.grab_focus();
            } else {
                self.run_button.grab_focus();
            }
        }
    }

    fn run(self: &Rc<Self>) {
        if !self.deps_ok || self.running.borrow().is_some() {
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        let request = match self.request() {
            Ok(r) => r,
            Err(e) => {
                self.show_error(&e);
                return;
            }
        };
        let Some(dir) = self.dest_dir.borrow().clone() else {
            self.show_error(&OpError::NoInput);
            return;
        };
        let name = self.single_output().then(|| self.name_row.text().to_string());
        let dest = Destination { dir, name };

        clear(&self.result_slot);
        self.set_running(true);
        self.progress.set_fraction(0.0);
        self.status.set_text("Preparando…");

        let n = self.files.borrow().len();
        let title = if n == 1 {
            format!("{} — {}", self.info.title, display_name(&self.files.borrow()[0]))
        } else {
            format!("{} — {n} arquivos", self.info.title)
        };
        let handle = jobs::spawn(move |ctx| operations::execute(request, &dest, ctx));
        *self.running.borrow_mut() = Some(handle.cancel.clone());
        let task = app.tasks.add(&title, handle.cancel.clone());

        let p = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = handle.events.recv().await {
                match event {
                    JobEvent::Queued => p.status.set_text("Na fila…"),
                    JobEvent::Progress(f, text) => {
                        p.progress.set_fraction(f);
                        p.status.set_text(&text);
                        task.progress(f, &text);
                    }
                    JobEvent::Finished(result) => {
                        *p.running.borrow_mut() = None;
                        p.set_running(false);
                        match &result {
                            Ok(outcome) => task.finish(&summary(outcome), folder_of(outcome)),
                            Err(e) => task.finish(e.title(), None),
                        }
                        p.show_result(result);
                        break;
                    }
                }
            }
        });
    }

    /// Rola a página até o resultado (ou erro) recém-mostrado.
    fn reveal_result(&self) {
        let (slot, scroll) = (self.result_slot.clone(), self.scroll.clone());
        // Espera o layout das linhas novas para saber a altura real.
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            if let Some(child) = scroll.child()
                && let Some(bounds) = slot.compute_bounds(&child)
            {
                let adj = scroll.vadjustment();
                let bottom = bounds.y() as f64 + bounds.height() as f64;
                if bottom > adj.value() + adj.page_size() {
                    adj.set_value((bottom - adj.page_size() + 24.0).min(bounds.y() as f64 - 12.0).max(0.0));
                }
            }
        });
    }

    fn show_error(&self, e: &OpError) {
        clear(&self.result_slot);
        self.result_slot.append(&error_group(e));
        self.reveal_result();
        let retry = matches!(e, OpError::ToolFailed { .. } | OpError::Io { .. } | OpError::OutputInvalid(_));
        if retry {
            self.run_button.set_label("Tentar novamente");
        } else {
            self.refresh_preview();
        }
    }

    fn show_result(self: &Rc<Self>, result: OpResult<Outcome>) {
        let outcome = match result {
            Ok(o) => o,
            Err(e) => {
                self.show_error(&e);
                return;
            }
        };
        self.refresh_preview();
        let Some(win) = self.window() else { return };
        let title = if outcome.cancelled {
            "Cancelado — alguns arquivos já haviam sido gravados"
        } else if outcome.failures.is_empty() {
            if outcome.outputs.is_empty() { "Nada a fazer" } else { "Concluído" }
        } else {
            "Concluído com falhas"
        };
        let group = adw::PreferencesGroup::builder().title(title).build();
        let mut notes = outcome.notes.clone();
        let from_copy = self.files.borrow().iter().any(|f| super::unlock::is_copy(f));
        if from_copy && !outcome.outputs.is_empty() && self.info.id != ToolId::Protect {
            notes.push(
                "O resultado foi salvo sem senha (o original continua protegido). Para protegê-lo, use \
                 “Proteger com senha”."
                    .into(),
            );
        }
        if !notes.is_empty() {
            group.set_description(Some(&glib::markup_escape_text(&notes.join("\n"))));
        }
        let new_button = gtk::Button::builder().label("Começar de novo").valign(gtk::Align::Center).build();
        new_button.add_css_class("flat");
        let p = Rc::downgrade(self);
        new_button.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.files.borrow_mut().clear();
                p.rebuild_files();
                clear(&p.result_slot);
            }
        });
        group.set_header_suffix(Some(&new_button));

        if let Some(folder) = &outcome.folder {
            let row =
                widgets::plain_row(&display_name(folder), &format!("Pasta com {} arquivo(s)", outcome.outputs.len()));
            row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
            let open = widgets::icon_button("folder-open-symbolic", "Abrir pasta");
            let (w, f) = (win.clone(), folder.clone());
            open.connect_clicked(move |_| widgets::show_in_folder(&w, &f));
            row.add_suffix(&open);
            group.add(&row);
        }
        const MAX_ROWS: usize = 12;
        for out in outcome.outputs.iter().take(if outcome.folder.is_some() { 3 } else { MAX_ROWS }) {
            let row = widgets::plain_row(&display_name(&out.path), &human_size(out.size));
            row.set_tooltip_text(Some(&out.path.to_string_lossy()));
            let open = widgets::icon_button("document-open-symbolic", "Abrir");
            let (w, path) = (win.clone(), out.path.clone());
            open.connect_clicked(move |_| widgets::open_file(&w, &path));
            let show = widgets::icon_button("folder-open-symbolic", "Mostrar na pasta");
            let (w, path) = (win.clone(), out.path.clone());
            show.connect_clicked(move |_| widgets::show_in_folder(&w, &path));
            row.add_suffix(&open);
            row.add_suffix(&show);
            row.set_activatable_widget(Some(&open));
            group.add(&row);
        }
        let shown = if outcome.folder.is_some() { 3 } else { MAX_ROWS };
        if outcome.outputs.len() > shown {
            group.add(&widgets::plain_row(&format!("e mais {} arquivo(s)", outcome.outputs.len() - shown), ""));
        }
        for (file, err) in &outcome.failures {
            group.add(&failure_row(&display_name(file), err));
        }
        self.result_slot.append(&group);
        self.reveal_result();
        match outcome.outputs.as_slice() {
            [] => {}
            [one] => self.toast(&format!("Salvo: {}", display_name(&one.path))),
            many => self.toast(&format!("{} arquivos salvos", many.len())),
        }
    }
}

/// Linha de detalhes de um arquivo da lista.
fn file_line(path: &Path, facts: &FileFacts, is_pdf: bool, unlock_tool: bool) -> String {
    if facts.locked && !unlock_tool {
        "🔒 Protegido por senha — adicione de novo e digite a senha".into()
    } else if super::unlock::is_copy(path) {
        format!("🔓 Aberto com a senha; o resultado sai sem senha · {}", facts.line(is_pdf))
    } else {
        facts.line(is_pdf)
    }
}

/// “2 de 5 páginas não têm texto selecionável e serão reconhecidas.” Sem
/// `skip`, todas as páginas são reconhecidas, inclusive as que têm texto.
fn ocr_summary(with_text: u32, total: u32, skip: bool) -> String {
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

fn summary(outcome: &Outcome) -> String {
    let mut s = match outcome.outputs.len() {
        0 => "Nenhum arquivo criado".to_string(),
        1 => format!("Concluído: {}", display_name(&outcome.outputs[0].path)),
        n => format!("Concluído: {n} arquivos"),
    };
    if !outcome.failures.is_empty() {
        s.push_str(&format!(" · {} falha(s)", outcome.failures.len()));
    }
    if outcome.cancelled {
        s.push_str(" · cancelado");
    }
    s
}

fn folder_of(outcome: &Outcome) -> Option<PathBuf> {
    outcome.folder.clone().or_else(|| outcome.outputs.first().map(|o| o.path.clone()))
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn failure_row(title: &str, err: &OpError) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::builder().use_markup(false).build();
    row.set_title(title);
    row.set_subtitle(&err.to_string());
    row.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
    let details = gtk::Label::builder()
        .label(err.technical())
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();
    details.add_css_class("monospace");
    details.add_css_class("caption");
    row.add_row(&details);
    row
}

fn error_group(e: &OpError) -> adw::PreferencesGroup {
    let group =
        adw::PreferencesGroup::builder().title(e.title()).description(glib::markup_escape_text(&e.to_string())).build();
    let technical = matches!(
        e,
        OpError::ToolFailed { .. } | OpError::Io { .. } | OpError::InvalidPdf { .. } | OpError::OutputInvalid(_)
    );
    if technical {
        let row = failure_row("Detalhes técnicos", e);
        row.set_subtitle("Útil para relatar um problema. Não inclui o conteúdo do documento.");
        group.add(&row);
    }
    if let OpError::MissingTool(t) = e {
        let row = widgets::plain_row(t.label(), &t.install_command());
        row.set_subtitle_selectable(true);
        group.add(&row);
    }
    group
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
