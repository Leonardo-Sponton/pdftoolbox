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

    pub fn io_path(action: &str, path: &Path, source: io::Error) -> Self {
        Self::io(format!("{action} “{}”", human::file_name(path)), source)
    }

    pub fn title(&self) -> &'static str {
        match self {
            OpError::NoInput => "Nenhum arquivo selecionado",
            OpError::MissingTool(_) | OpError::MissingOcrLanguage { .. } => "Dependência ausente",
            OpError::InvalidPdf { .. } => "Documento inválido",
            OpError::PasswordProtected(_) => "Documento protegido por senha",
            OpError::WrongPassword(_) => "Senha incorreta",
            OpError::OwnerPasswordRequired(_) => "É preciso a senha de permissões",
            OpError::NotEncrypted(_) => "O documento não tem senha",
            OpError::UnsupportedImage { .. } => "Imagem não suportada",
            OpError::InvalidRange(_) | OpError::PageOutOfRange { .. } | OpError::EmptySelection => {
                "Seleção de páginas inválida"
            }
            OpError::Io { source, .. } => match io_kind(source) {
                IoKind::NoSpace => "Espaço em disco insuficiente",
                IoKind::Permission => "Permissão insuficiente",
                IoKind::ReadOnly => "Destino somente leitura",
                IoKind::NotFound => "Arquivo não encontrado",
                IoKind::Other => "Erro de leitura ou gravação",
            },
            OpError::ToolFailed { .. } => "O motor de processamento falhou",
            OpError::OutputInvalid(_) => "O resultado não passou na verificação",
            OpError::Cancelled => "Operação cancelada",
        }
    }

    /// Detalhe técnico para a seção "Detalhes" e para o log.
    pub fn technical(&self) -> String {
        match self {
            OpError::Io { context, source } => format!("{context}: {source}"),
            OpError::ToolFailed { tool, code, stderr } => {
                let code = code.map_or("sinal".to_string(), |c| c.to_string());
                format!("{} saiu com código {code}\n{}", tool.program(), stderr.trim())
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
        match self {
            OpError::NoInput => write!(f, "Adicione pelo menos um arquivo para continuar."),
            OpError::MissingTool(t) => write!(
                f,
                "Esta ferramenta precisa de “{}”, que não foi encontrado no sistema. \
                 Veja em Menu → Dependências como instalá-lo.",
                t.label()
            ),
            OpError::InvalidPdf { file, .. } => {
                write!(
                    f,
                    "“{}” não é um PDF válido ou está danificado demais para ser processado.",
                    human::file_name(file)
                )
            }
            OpError::PasswordProtected(file) => write!(
                f,
                "“{}” está protegido por senha. Adicione-o de novo à ferramenta e digite a senha \
                 quando ela for pedida, ou use “Remover senha” para gravar uma cópia sem senha.",
                human::file_name(file)
            ),
            OpError::WrongPassword(file) => write!(
                f,
                "A senha informada não abre “{}”. Confira maiúsculas, minúsculas e acentos.",
                human::file_name(file)
            ),
            OpError::OwnerPasswordRequired(file) => write!(
                f,
                "A senha informada abre “{}”, mas o autor restringiu impressão, cópia ou edição. \
                 Para trabalhar nele ou remover a proteção, é preciso a senha de permissões (do proprietário).",
                human::file_name(file)
            ),
            OpError::NotEncrypted(file) => {
                write!(f, "“{}” não está protegido por senha; não há o que remover.", human::file_name(file))
            }
            OpError::UnsupportedImage { file, .. } => write!(
                f,
                "Não foi possível ler “{}” como imagem. Formatos aceitos: JPEG, PNG, BMP, GIF, WebP e TIFF.",
                human::file_name(file)
            ),
            OpError::InvalidRange(msg) => write!(f, "{msg}"),
            OpError::PageOutOfRange { page, total } => write!(
                f,
                "A página {page} não existe: o documento tem {total} página{}.",
                if *total == 1 { "" } else { "s" }
            ),
            OpError::EmptySelection => write!(f, "Nenhuma página foi selecionada."),
            OpError::Io { context, source } => {
                let hint = match io_kind(source) {
                    IoKind::NoSpace => "Libere espaço no disco ou escolha outro destino.",
                    IoKind::Permission => "Escolha uma pasta em que você tenha permissão de escrita.",
                    IoKind::ReadOnly => "O destino está montado como somente leitura. Escolha outra pasta.",
                    IoKind::NotFound => "Verifique se o arquivo ainda existe e não foi movido.",
                    IoKind::Other => "Verifique o arquivo e o destino e tente novamente.",
                };
                write!(f, "Falha ao {context}. {hint}")
            }
            OpError::ToolFailed { tool, .. } => write!(
                f,
                "{} não conseguiu processar o documento. O arquivo pode estar danificado ou usar \
                 um recurso não suportado. Veja os detalhes técnicos abaixo.",
                tool.label()
            ),
            OpError::OutputInvalid(msg) => {
                write!(f, "O arquivo gerado não passou na verificação e foi descartado: {msg}")
            }
            OpError::MissingOcrLanguage { language, install } => write!(
                f,
                "O reconhecimento em {language} precisa dos dados desse idioma para o Tesseract, que não estão \
                 instalados. Instale com: {install}"
            ),
            OpError::Cancelled => write!(f, "A operação foi cancelada. Nenhum arquivo foi criado."),
        }
    }
}

impl std::error::Error for OpError {}
