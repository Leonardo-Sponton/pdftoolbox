//! Linhas de opções de cada ferramenta: montagem, ligações entre as linhas
//! e quais delas atualizam a pré-visualização.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;

use pdftoolbox::catalog::{ToolId, ToolInfo};
use pdftoolbox::engines::{self, Tool};
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::{self, CUSTOM_DPI, CompressLevel, Estimate};

use super::super::widgets;
use super::form::level_from;
use super::texts::compress_explanation;

pub(super) enum Options {
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

pub(super) struct MetadataRows {
    pub title: adw::EntryRow,
    pub author: adw::EntryRow,
    pub subject: adw::EntryRow,
    pub keywords: adw::EntryRow,
    pub remove_all: adw::SwitchRow,
    /// Somente leitura: programa de criação, produtor e datas.
    pub facts: [adw::ActionRow; 4],
    /// Arquivo cujos metadados preencheram os campos (compartilhado com a
    /// leitura em segundo plano, que descarta o resultado se ele mudou).
    pub loaded: Rc<RefCell<Option<PathBuf>>>,
}

impl MetadataRows {
    /// Campos editáveis, na ordem do formulário.
    pub fn fields(&self) -> [adw::EntryRow; 4] {
        [self.title.clone(), self.author.clone(), self.subject.clone(), self.keywords.clone()]
    }
}

pub(super) struct CompressRows {
    pub level: adw::ComboRow,
    pub dpi: adw::SpinRow,
    /// Tamanho previsto, calculado em segundo plano com uma amostra de páginas.
    pub estimate: adw::ActionRow,
    pub spinner: gtk::Spinner,
    pub estimates: RefCell<HashMap<(PathBuf, CompressLevel), Result<Estimate, String>>>,
    pub pending: RefCell<Option<CancelToken>>,
    /// Cada novo pedido de estimativa invalida as respostas anteriores.
    pub generation: Cell<u64>,
}

impl CompressRows {
    pub fn level(&self) -> CompressLevel {
        level_from(self.level.selected(), self.dpi.value())
    }

    /// Cancela o cálculo em andamento e devolve a geração do novo pedido.
    pub fn restart(&self) -> u64 {
        self.stop();
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        generation
    }

    pub fn show_calculating(&self, text: &str) {
        self.spinner.set_visible(true);
        self.spinner.set_spinning(true);
        self.estimate.set_subtitle(text);
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
pub(super) struct LanguageRows {
    pub row: adw::ExpanderRow,
    pub switches: RefCell<Vec<(String, adw::SwitchRow)>>,
}

impl LanguageRows {
    pub fn selected(&self) -> Vec<String> {
        self.switches.borrow().iter().filter(|(_, s)| s.is_active()).map(|(c, _)| c.clone()).collect()
    }

    pub fn update_subtitle(&self) {
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

pub(super) struct ProtectRows {
    pub password: adw::PasswordEntryRow,
    pub confirm: adw::PasswordEntryRow,
    pub restrict: adw::ExpanderRow,
    pub owner: adw::PasswordEntryRow,
    pub print: adw::SwitchRow,
    pub copy: adw::SwitchRow,
    pub edit: adw::SwitchRow,
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

fn add_rows(group: &adw::PreferencesGroup, rows: &[&gtk::Widget]) {
    for row in rows {
        group.add(*row);
    }
}

/// Chama `refresh` sempre que o valor de `row` muda.
fn on_change(row: &gtk::Widget, refresh: &Rc<dyn Fn()>) {
    let r = refresh.clone();
    // SpinRow também é editável: o valor (não o texto) é o que conta.
    if let Some(spin) = row.downcast_ref::<adw::SpinRow>() {
        spin.connect_value_notify(move |_| r());
    } else if let Some(combo) = row.downcast_ref::<adw::ComboRow>() {
        combo.connect_selected_notify(move |_| r());
    } else if let Some(switch) = row.downcast_ref::<adw::SwitchRow>() {
        switch.connect_active_notify(move |_| r());
    } else if let Some(expander) = row.downcast_ref::<adw::ExpanderRow>() {
        expander.connect_enable_expansion_notify(move |_| r());
    } else if let Some(editable) = row.dynamic_cast_ref::<gtk::Editable>() {
        editable.connect_changed(move |_| r());
    }
}

impl Options {
    /// Constrói as linhas de opções da ferramenta dentro de `group`.
    pub fn build(info: &ToolInfo, group: &adw::PreferencesGroup) -> Options {
        match info.id {
            ToolId::Merge => {
                group.set_description(Some(
                    "Os arquivos são unidos na ordem da lista acima. Use as setas para reordenar.",
                ));
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
                add_rows(group, &[mode.upcast_ref(), ranges.upcast_ref(), every.upcast_ref()]);
                Options::Split { mode, ranges, every }
            }
            ToolId::Extract => {
                let spec = entry("Páginas a extrair (ex.: 1-3, 7, 10-)");
                let separate = switch(
                    "Um arquivo por página",
                    "Desligado: todas as páginas escolhidas em um único PDF, na ordem digitada",
                    false,
                );
                add_rows(group, &[spec.upcast_ref(), separate.upcast_ref()]);
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
                add_rows(group, &[angle.upcast_ref(), spec.upcast_ref()]);
                Options::Rotate { angle, spec }
            }
            ToolId::ImagesToPdf => {
                let size = combo("Tamanho da página", &["A4", "Carta", "Igual à imagem"], 0);
                let margin = spin("Margem", "Em milímetros", 0.0, 50.0, 1.0, 10.0);
                let one_file = switch("Juntar em um único PDF", "Desligado: um PDF para cada imagem", true);
                let m = margin.clone();
                size.connect_selected_notify(move |s| m.set_sensitive(s.selected() != 2));
                group.set_description(Some(
                    "JPEGs são incorporados sem recompressão. A orientação gravada pela câmera (EXIF) é respeitada.",
                ));
                add_rows(group, &[size.upcast_ref(), margin.upcast_ref(), one_file.upcast_ref()]);
                Options::Images { size, margin, one_file }
            }
            ToolId::PdfToImages => {
                let format =
                    combo("Formato", &["PNG — sem perda", "JPEG — arquivos menores", "TIFF — sem perda (LZW)"], 0);
                let dpi = spin(
                    "Resolução",
                    "Pontos por polegada: 150 para tela, 300 para impressão",
                    36.0,
                    1200.0,
                    10.0,
                    150.0,
                );
                let quality = spin("Qualidade do JPEG", "De 10 a 100", 10.0, 100.0, 5.0, 90.0);
                quality.set_visible(false);
                let q = quality.clone();
                format.connect_selected_notify(move |f| q.set_visible(f.selected() == 1));
                add_rows(group, &[format.upcast_ref(), dpi.upcast_ref(), quality.upcast_ref()]);
                Options::ToImages { format, dpi, quality }
            }
            ToolId::Compress => Options::Compress(Box::new(build_compress(group))),
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
                add_rows(
                    group,
                    &[
                        position.upcast_ref(),
                        format.upcast_ref(),
                        first.upcast_ref(),
                        size.upcast_ref(),
                        spec.upcast_ref(),
                    ],
                );
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
                    });
                });
                let spec = entry("Páginas (deixe vazio para todas)");
                add_rows(
                    group,
                    &[
                        text.upcast_ref(),
                        color.upcast_ref(),
                        opacity.upcast_ref(),
                        scale.upcast_ref(),
                        diagonal.upcast_ref(),
                        spec.upcast_ref(),
                    ],
                );
                Options::Watermark { text, color, opacity, scale, diagonal, spec }
            }
            ToolId::Metadata => Options::Metadata(Box::new(build_metadata(group))),
            ToolId::Protect => Options::Protect(Box::new(build_protect(group))),
            ToolId::Unlock => {
                let password_row = password("Senha do documento");
                group.set_description(Some(
                    "Informe a senha que você já conhece. Para remover restrições de impressão ou cópia, é preciso \
                     a senha de permissões (do proprietário). Nenhuma senha é adivinhada ou contornada.",
                ));
                group.add(&password_row);
                Options::Unlock { password: password_row }
            }
            ToolId::PdfToText => {
                let layout = switch(
                    "Manter o layout",
                    "Preserva colunas e espaçamento; desligado, segue a ordem de leitura",
                    false,
                );
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
                add_rows(group, &[layout.upcast_ref(), ocr.upcast_ref()]);
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
                add_rows(group, &[languages.upcast_ref(), skip.upcast_ref(), straighten.upcast_ref()]);
                Options::Ocr {
                    languages: Rc::new(LanguageRows { row: languages, switches: RefCell::default() }),
                    skip,
                    straighten,
                }
            }
            ToolId::OfficeToPdf => {
                let isolated = if engines::network_isolation_available() {
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

    /// Faz `refresh` ser chamado quando muda uma opção que afeta o nome do
    /// arquivo, a pré-visualização ou o botão. (Nível de compressão, layout
    /// do texto e idiomas do OCR têm tratamento próprio na página.)
    pub fn connect_changes(&self, refresh: &Rc<dyn Fn()>) {
        let rows: Vec<gtk::Widget> = match self {
            Options::Split { mode, ranges, every } => {
                vec![mode.clone().upcast(), ranges.clone().upcast(), every.clone().upcast()]
            }
            Options::Extract { spec, separate } => vec![separate.clone().upcast(), spec.clone().upcast()],
            Options::Remove { spec } => vec![spec.clone().upcast()],
            Options::Rotate { angle, spec } => vec![angle.clone().upcast(), spec.clone().upcast()],
            Options::Images { size, margin, one_file } => {
                vec![one_file.clone().upcast(), size.clone().upcast(), margin.clone().upcast()]
            }
            Options::ToImages { dpi, .. } => vec![dpi.clone().upcast()],
            Options::Numbers { position, format, first, size, spec } => vec![
                position.clone().upcast(),
                format.clone().upcast(),
                first.clone().upcast(),
                size.clone().upcast(),
                spec.clone().upcast(),
            ],
            Options::Watermark { text, color, opacity, scale, diagonal, spec } => vec![
                text.clone().upcast(),
                spec.clone().upcast(),
                color.clone().upcast(),
                opacity.clone().upcast(),
                scale.clone().upcast(),
                diagonal.clone().upcast(),
            ],
            Options::Metadata(m) => vec![m.remove_all.clone().upcast()],
            Options::Protect(p) => vec![
                p.password.clone().upcast(),
                p.confirm.clone().upcast(),
                p.owner.clone().upcast(),
                p.restrict.clone().upcast(),
            ],
            Options::Unlock { password } => vec![password.clone().upcast()],
            Options::Ocr { skip, .. } => vec![skip.clone().upcast()],
            Options::Text { .. } | Options::Compress(_) | Options::None | Options::Office => vec![],
        };
        for row in &rows {
            on_change(row, refresh);
        }
    }

    /// Campos em que Enter executa a ferramenta; o primeiro recebe o foco
    /// quando a página abre.
    pub fn entries(&self) -> Vec<adw::EntryRow> {
        match self {
            Options::Split { ranges, .. } => vec![ranges.clone()],
            Options::Extract { spec, .. }
            | Options::Remove { spec }
            | Options::Rotate { spec, .. }
            | Options::Numbers { spec, .. } => vec![spec.clone()],
            Options::Watermark { text, spec, .. } => vec![text.clone(), spec.clone()],
            Options::Metadata(m) => m.fields().to_vec(),
            Options::Protect(p) => {
                vec![p.password.clone().upcast(), p.confirm.clone().upcast(), p.owner.clone().upcast()]
            }
            Options::Unlock { password } => vec![password.clone().upcast()],
            _ => vec![],
        }
    }

    /// Campo de páginas validado pela pré-visualização.
    pub fn spec_entry(&self) -> Option<&adw::EntryRow> {
        match self {
            Options::Split { ranges, .. } => Some(ranges),
            Options::Extract { spec, .. }
            | Options::Remove { spec }
            | Options::Rotate { spec, .. }
            | Options::Numbers { spec, .. }
            | Options::Watermark { spec, .. } => Some(spec),
            _ => None,
        }
    }

    /// Campo que um clique numa página da grade reescreve, e se a ordem das
    /// páginas nele importa (Extrair).
    pub fn clickable_spec(&self) -> Option<(&adw::EntryRow, bool)> {
        match self {
            Options::Rotate { spec, .. }
            | Options::Remove { spec }
            | Options::Numbers { spec, .. }
            | Options::Watermark { spec, .. } => Some((spec, false)),
            Options::Extract { spec, .. } => Some((spec, true)),
            _ => None,
        }
    }
}

fn build_compress(group: &adw::PreferencesGroup) -> CompressRows {
    let level = combo("Nível", &["Sem perda", "Recomendada", "Máxima", "Personalizada"], 1);
    let dpi = spin(
        "Resolução das imagens",
        "Em ppp: 72 a 100 para tela, 150 para e-mail, 300 para impressão",
        f64::from(*CUSTOM_DPI.start()),
        f64::from(*CUSTOM_DPI.end()),
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
        g.set_description(Some(&compress_explanation(level_from(l.selected(), d.value()))));
    });
    add_rows(group, &[level.upcast_ref(), dpi.upcast_ref(), estimate.upcast_ref()]);
    CompressRows {
        level,
        dpi,
        estimate,
        spinner,
        estimates: RefCell::new(HashMap::new()),
        pending: RefCell::new(None),
        generation: Cell::new(0),
    }
}

fn build_metadata(group: &adw::PreferencesGroup) -> MetadataRows {
    let rows = MetadataRows {
        title: entry("Título"),
        author: entry("Autor"),
        subject: entry("Assunto"),
        keywords: entry("Palavras-chave"),
        remove_all: switch(
            "Apagar todos os metadados",
            "Remove também programas e datas — útil antes de compartilhar o arquivo",
            false,
        ),
        facts: [info_row("Criado com"), info_row("Gerado por"), info_row("Criado em"), info_row("Modificado em")],
        loaded: Rc::new(RefCell::new(None)),
    };
    group.set_description(Some("Campos deixados em branco são removidos do arquivo."));
    for row in rows.fields() {
        group.add(&row);
    }
    for row in &rows.facts {
        group.add(row);
    }
    group.add(&rows.remove_all);
    let fields = rows.fields();
    rows.remove_all.connect_active_notify(move |r| {
        for field in &fields {
            field.set_sensitive(!r.is_active());
        }
    });
    rows
}

fn build_protect(group: &adw::PreferencesGroup) -> ProtectRows {
    let rows = ProtectRows {
        password: password("Senha para abrir"),
        confirm: password("Repita a senha"),
        restrict: adw::ExpanderRow::builder()
            .title("Restringir impressão, cópia e edição")
            .subtitle("Exige uma segunda senha, só para mudar as permissões")
            .show_enable_switch(true)
            .enable_expansion(false)
            .build(),
        owner: password("Senha de permissões"),
        print: switch("Permitir imprimir", "", false),
        copy: switch("Permitir copiar texto e imagens", "", false),
        edit: switch("Permitir editar", "Inclui anotações e formulários", false),
    };
    rows.restrict.add_row(&rows.owner);
    rows.restrict.add_row(&rows.print);
    rows.restrict.add_row(&rows.copy);
    rows.restrict.add_row(&rows.edit);
    group.set_description(Some(
        "Criptografia AES-256. Guarde a senha: sem ela, ninguém consegue abrir o arquivo — nem este \
         aplicativo. O original continua sem senha.",
    ));
    add_rows(group, &[rows.password.upcast_ref(), rows.confirm.upcast_ref(), rows.restrict.upcast_ref()]);
    rows
}
