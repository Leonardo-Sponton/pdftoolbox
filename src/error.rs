use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::engines::Tool;
use crate::human;

pub type OpResult<T> = Result<T, OpError>;

/// Erros das operações. `Display` produz a mensagem para o usuário (pt-BR);
/// `technical()` produz o detalhe técnico, que não contém conteúdo do documento.
#[derive(Debug)]
pub enum OpError {
    NoInput,
    MissingTool(Tool),
    InvalidPdf {
        file: PathBuf,
        detail: String,
    },
    PasswordProtected(PathBuf),
    /// A senha informada não abre o documento.
    WrongPassword(PathBuf),
    /// A senha abre o documento, mas não é a de permissões, e o autor
    /// restringiu impressão, cópia ou edição.
    OwnerPasswordRequired(PathBuf),
    /// Remover senha de um arquivo que não tem senha.
    NotEncrypted(PathBuf),
    UnsupportedImage {
        file: PathBuf,
        detail: String,
    },
    InvalidRange(String),
    PageOutOfRange {
        page: u32,
        total: u32,
    },
    EmptySelection,
    Io {
        context: String,
        source: io::Error,
    },
    ToolFailed {
        tool: Tool,
        code: Option<i32>,
        stderr: String,
    },
    OutputInvalid(String),
    /// Dados do idioma do OCR não instalados (nome do idioma, comando).
    MissingOcrLanguage {
        language: String,
        install: String,
    },
    Cancelled,
}

impl OpError {
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        OpError::Io { context: context.into(), source }
    }

    /// `action` é o que falhou, já traduzido e com `{file}` para o nome do arquivo.
    pub fn io_path(action: &str, path: &Path, source: io::Error) -> Self {
        Self::io(crate::i18n::fill(action, &[("file", &human::file_name(path))]), source)
    }

    pub fn title(&self) -> &'static str {
        match self {
            OpError::NoInput => tr!("Nenhum arquivo selecionado"),
            OpError::MissingTool(_) | OpError::MissingOcrLanguage { .. } => tr!("Dependência ausente"),
            OpError::InvalidPdf { .. } => tr!("Documento inválido"),
            OpError::PasswordProtected(_) => tr!("Documento protegido por senha"),
            OpError::WrongPassword(_) => tr!("Senha incorreta"),
            OpError::OwnerPasswordRequired(_) => tr!("É preciso a senha de permissões"),
            OpError::NotEncrypted(_) => tr!("O documento não tem senha"),
            OpError::UnsupportedImage { .. } => tr!("Imagem não suportada"),
            OpError::InvalidRange(_) | OpError::PageOutOfRange { .. } | OpError::EmptySelection => {
                tr!("Seleção de páginas inválida")
            }
            OpError::Io { source, .. } => match io_kind(source) {
                IoKind::NoSpace => tr!("Espaço em disco insuficiente"),
                IoKind::Permission => tr!("Permissão insuficiente"),
                IoKind::ReadOnly => tr!("Destino somente leitura"),
                IoKind::NotFound => tr!("Arquivo não encontrado"),
                IoKind::Other => tr!("Erro de leitura ou gravação"),
            },
            OpError::ToolFailed { .. } => tr!("O motor de processamento falhou"),
            OpError::OutputInvalid(_) => tr!("O resultado não passou na verificação"),
            OpError::Cancelled => tr!("Operação cancelada"),
        }
    }

    /// Detalhe técnico para a seção "Detalhes" e para o log.
    pub fn technical(&self) -> String {
        match self {
            OpError::Io { context, source } => format!("{context}: {source}"),
            OpError::ToolFailed { tool, code, stderr } => {
                let status = match code {
                    Some(code) => tr!("{program} saiu com código {code}", program = tool.program(), code),
                    None => tr!("{program} foi encerrado por um sinal", program = tool.program()),
                };
                format!("{status}\n{}", stderr.trim())
            }
            OpError::InvalidPdf { detail, .. } | OpError::UnsupportedImage { detail, .. } => detail.clone(),
            other => format!("{other:?}"),
        }
    }
}

enum IoKind {
    NoSpace,
    Permission,
    ReadOnly,
    NotFound,
    Other,
}

fn io_kind(e: &io::Error) -> IoKind {
    match e.raw_os_error() {
        Some(28 | 122) => return IoKind::NoSpace, // ENOSPC, EDQUOT
        Some(30) => return IoKind::ReadOnly,      // EROFS
        _ => {}
    }
    match e.kind() {
        io::ErrorKind::PermissionDenied => IoKind::Permission,
        io::ErrorKind::NotFound => IoKind::NotFound,
        io::ErrorKind::StorageFull => IoKind::NoSpace,
        io::ErrorKind::ReadOnlyFilesystem => IoKind::ReadOnly,
        _ => IoKind::Other,
    }
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            OpError::NoInput => tr!("Adicione pelo menos um arquivo para continuar.").to_string(),
            OpError::MissingTool(t) => tr!(
                "Esta ferramenta precisa de “{program}”, que não foi encontrado no sistema. \
                 Veja em Menu → Dependências como instalá-lo.",
                program = t.label()
            ),
            OpError::InvalidPdf { file, .. } => tr!(
                "“{file}” não é um PDF válido ou está danificado demais para ser processado.",
                file = human::file_name(file)
            ),
            OpError::PasswordProtected(file) => tr!(
                "“{file}” está protegido por senha. Adicione-o de novo à ferramenta e digite a senha \
                 quando ela for pedida, ou use “Remover senha” para gravar uma cópia sem senha.",
                file = human::file_name(file)
            ),
            OpError::WrongPassword(file) => tr!(
                "A senha informada não abre “{file}”. Confira maiúsculas, minúsculas e acentos.",
                file = human::file_name(file)
            ),
            OpError::OwnerPasswordRequired(file) => tr!(
                "A senha informada abre “{file}”, mas o autor restringiu impressão, cópia ou edição. \
                 Para trabalhar nele ou remover a proteção, é preciso a senha de permissões (do proprietário).",
                file = human::file_name(file)
            ),
            OpError::NotEncrypted(file) => {
                tr!("“{file}” não está protegido por senha; não há o que remover.", file = human::file_name(file))
            }
            OpError::UnsupportedImage { file, .. } => tr!(
                "Não foi possível ler “{file}” como imagem. Formatos aceitos: JPEG, PNG, BMP, GIF, WebP e TIFF.",
                file = human::file_name(file)
            ),
            OpError::InvalidRange(msg) => msg.clone(),
            OpError::PageOutOfRange { page, total } => trn!(
                *total as usize,
                "A página {page} não existe: o documento tem {total} página.",
                "A página {page} não existe: o documento tem {total} páginas.",
                page,
                total
            ),
            OpError::EmptySelection => tr!("Nenhuma página foi selecionada.").to_string(),
            OpError::Io { context, source } => {
                let hint = match io_kind(source) {
                    IoKind::NoSpace => tr!("Libere espaço no disco ou escolha outro destino."),
                    IoKind::Permission => tr!("Escolha uma pasta em que você tenha permissão de escrita."),
                    IoKind::ReadOnly => tr!("O destino está montado como somente leitura. Escolha outra pasta."),
                    IoKind::NotFound => tr!("Verifique se o arquivo ainda existe e não foi movido."),
                    IoKind::Other => tr!("Verifique o arquivo e o destino e tente novamente."),
                };
                tr!("Falha ao {context}. {hint}", context, hint)
            }
            OpError::ToolFailed { tool, .. } => tr!(
                "{program} não conseguiu processar o documento. O arquivo pode estar danificado ou usar \
                 um recurso não suportado. Veja os detalhes técnicos abaixo.",
                program = tool.label()
            ),
            OpError::OutputInvalid(msg) => {
                tr!("O arquivo gerado não passou na verificação e foi descartado: {reason}", reason = msg)
            }
            OpError::MissingOcrLanguage { language, install } => tr!(
                "O reconhecimento em {language} precisa dos dados desse idioma para o Tesseract, que não estão \
                 instalados. Instale com: {install}",
                language,
                install
            ),
            OpError::Cancelled => tr!("A operação foi cancelada. Nenhum arquivo foi criado.").to_string(),
        };
        f.write_str(&message)
    }
}

impl std::error::Error for OpError {}
