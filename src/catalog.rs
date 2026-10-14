//! Catálogo de ferramentas exibido na interface. Só entram aqui ferramentas
//! com implementação real; o restante do roteiro está em FEATURE_MATRIX.md.

use crate::engines::Tool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolId {
    Merge,
    Split,
    Extract,
    Remove,
    Rotate,
    Organize,
    ImagesToPdf,
    PdfToImages,
    Compress,
    NumberPages,
    Watermark,
    Metadata,
    Protect,
    Unlock,
    PdfToText,
    OfficeToPdf,
    Annotate,
    Ocr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Organize,
    Edit,
    Convert,
    Compress,
    Security,
}

impl Category {
    pub const ALL: [Category; 5] =
        [Category::Organize, Category::Edit, Category::Convert, Category::Compress, Category::Security];

    pub fn title(self) -> &'static str {
        match self {
            Category::Organize => "Organizar",
            Category::Edit => "Editar",
            Category::Convert => "Converter",
            Category::Compress => "Comprimir",
            Category::Security => "Segurança",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepts {
    Pdf,
    Images,
    /// Documentos, planilhas e apresentações (LibreOffice).
    Office,
}

pub struct ToolInfo {
    pub id: ToolId,
    pub key: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub icon: &'static str,
    pub category: Category,
    pub accepts: Accepts,
    pub multiple: bool,
    pub min_files: usize,
    pub needs: &'static [Tool],
    /// Palavras extras para a busca.
    pub keywords: &'static str,
}

pub const TOOLS: &[ToolInfo] = &[
    ToolInfo {
        id: ToolId::Organize,
        key: "organizar",
        title: "Organizar páginas",
        summary: "Reordene arrastando, gire, duplique, exclua e insira páginas em branco vendo as miniaturas",
        icon: "pdftoolbox-organizar-symbolic",
        category: Category::Organize,
        accepts: Accepts::Pdf,
        multiple: false,
        min_files: 1,
        needs: &[Tool::Qpdf, Tool::Pdftoppm],
        keywords: "reordenar ordem miniaturas mover duplicar excluir girar",
    },
    ToolInfo {
        id: ToolId::Merge,
        key: "mesclar",
        title: "Mesclar PDFs",
        summary: "Junte vários PDFs em um único documento, na ordem que escolher",
        icon: "pdftoolbox-mesclar-symbolic",
        category: Category::Organize,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 2,
        needs: &[Tool::Qpdf],
        keywords: "juntar unir combinar",
    },
    ToolInfo {
        id: ToolId::Split,
        key: "dividir",
        title: "Dividir PDF",
        summary: "Separe por intervalos, em blocos de N páginas ou uma página por arquivo",
        icon: "pdftoolbox-dividir-symbolic",
        category: Category::Organize,
        accepts: Accepts::Pdf,
        multiple: false,
        min_files: 1,
        needs: &[Tool::Qpdf],
        keywords: "separar partes intervalos",
    },
    ToolInfo {
        id: ToolId::Extract,
        key: "extrair",
        title: "Extrair páginas",
        summary: "Crie um novo PDF só com as páginas escolhidas",
        icon: "pdftoolbox-extrair-symbolic",
        category: Category::Organize,
        accepts: Accepts::Pdf,
        multiple: false,
        min_files: 1,
        needs: &[Tool::Qpdf],
        keywords: "selecionar copiar páginas",
    },
    ToolInfo {
        id: ToolId::Remove,
        key: "remover",
        title: "Remover páginas",
        summary: "Exclua páginas específicas e salve o restante",
        icon: "pdftoolbox-remover-symbolic",
        category: Category::Organize,
        accepts: Accepts::Pdf,
        multiple: false,
        min_files: 1,
        needs: &[Tool::Qpdf],
        keywords: "excluir apagar deletar",
    },
    ToolInfo {
        id: ToolId::Rotate,
        key: "girar",
        title: "Girar PDF",
        summary: "Gire todas as páginas ou só algumas, em um ou vários arquivos",
        icon: "pdftoolbox-girar-symbolic",
        category: Category::Organize,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf],
        keywords: "rotacionar virar orientação paisagem retrato",
    },
    ToolInfo {
        id: ToolId::ImagesToPdf,
        key: "imagens-para-pdf",
        title: "Imagens para PDF",
        summary: "JPEG, PNG, WebP, GIF, BMP e TIFF em PDF, sem recomprimir JPEGs",
        icon: "pdftoolbox-imagens-para-pdf-symbolic",
        category: Category::Convert,
        accepts: Accepts::Images,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf],
        keywords: "jpg jpeg png webp gif bmp tiff foto converter",
    },
    ToolInfo {
        id: ToolId::PdfToImages,
        key: "pdf-para-imagens",
        title: "PDF para imagens",
        summary: "Converta cada página em PNG, JPEG ou TIFF na resolução desejada",
        icon: "pdftoolbox-pdf-para-imagens-symbolic",
        category: Category::Convert,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf, Tool::Pdftoppm],
        keywords: "png jpg jpeg tiff exportar converter imagem",
    },
    ToolInfo {
        id: ToolId::Compress,
        key: "comprimir",
        title: "Comprimir PDF",
        summary: "Reduza o tamanho sem perda ou reamostrando imagens",
        icon: "pdftoolbox-comprimir-symbolic",
        category: Category::Compress,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf],
        keywords: "reduzir diminuir tamanho otimizar compactar",
    },
    ToolInfo {
        id: ToolId::NumberPages,
        key: "numerar",
        title: "Numerar páginas",
        summary: "Escreva o número em cada página, na posição e no formato que escolher",
        icon: "pdftoolbox-numerar-symbolic",
        category: Category::Edit,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf, Tool::Pdfinfo],
        keywords: "numeração número paginação rodapé cabeçalho contar",
    },
    ToolInfo {
        id: ToolId::Watermark,
        key: "marca-dagua",
        title: "Marca d’água",
        summary: "Escreva um texto semitransparente sobre as páginas, como “CONFIDENCIAL”",
        icon: "pdftoolbox-marca-dagua-symbolic",
        category: Category::Edit,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf, Tool::Pdfinfo],
        keywords: "marca dagua carimbo confidencial rascunho texto sobreposto",
    },
    ToolInfo {
        id: ToolId::Annotate,
        key: "anotar",
        title: "Anotar e assinar",
        summary: "Escreva textos, destaque trechos e coloque a imagem da sua assinatura nas páginas",
        icon: "pdftoolbox-anotar-symbolic",
        category: Category::Edit,
        accepts: Accepts::Pdf,
        multiple: false,
        min_files: 1,
        needs: &[Tool::Qpdf, Tool::Pdfinfo, Tool::Pdftoppm],
        keywords: "assinatura assinar rubrica anotação comentário escrever texto destacar marca-texto \
                   carimbo preencher formulário imagem logotipo moldura",
    },
    ToolInfo {
        id: ToolId::Metadata,
        key: "metadados",
        title: "Editar metadados",
        summary: "Veja e altere título, autor, assunto e palavras-chave, ou apague tudo",
        icon: "pdftoolbox-metadados-symbolic",
        category: Category::Edit,
        accepts: Accepts::Pdf,
        multiple: false,
        min_files: 1,
        needs: &[Tool::Qpdf, Tool::Pdfinfo],
        keywords: "propriedades título autor assunto palavras-chave informações limpar privacidade",
    },
    ToolInfo {
        id: ToolId::PdfToText,
        key: "pdf-para-texto",
        title: "PDF para texto",
        summary: "Extraia o texto para um arquivo .txt, reconhecendo (OCR) as páginas digitalizadas",
        icon: "pdftoolbox-pdf-para-texto-symbolic",
        category: Category::Convert,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf, Tool::Pdftotext],
        keywords: "txt extrair texto copiar conteúdo ocr digitalizado",
    },
    ToolInfo {
        id: ToolId::Ocr,
        key: "ocr",
        title: "OCR — reconhecer texto",
        summary: "Torne pesquisável e copiável o texto de documentos digitalizados, mesmo tortos ou deitados",
        icon: "pdftoolbox-ocr-symbolic",
        category: Category::Convert,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf, Tool::Pdftoppm, Tool::Pdftotext, Tool::Tesseract],
        keywords: "ocr reconhecimento texto digitalização digitalizado escaneado scanner foto pesquisável busca \
                   tesseract copiar",
    },
    ToolInfo {
        id: ToolId::OfficeToPdf,
        key: "office-para-pdf",
        title: "Office para PDF",
        summary: "Converta documentos, planilhas e apresentações (Word, Excel, PowerPoint, LibreOffice)",
        icon: "pdftoolbox-office-para-pdf-symbolic",
        category: Category::Convert,
        accepts: Accepts::Office,
        multiple: true,
        min_files: 1,
        needs: &[Tool::LibreOffice, Tool::Qpdf],
        keywords: "word excel powerpoint docx xlsx pptx odt ods odp doc xls ppt rtf csv txt documento planilha apresentação",
    },
    ToolInfo {
        id: ToolId::Protect,
        key: "proteger",
        title: "Proteger com senha",
        summary: "Exija senha para abrir o PDF (AES-256) e, se quiser, restrinja impressão e cópia",
        icon: "pdftoolbox-proteger-symbolic",
        category: Category::Security,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf],
        keywords: "senha criptografar proteger bloquear segurança cadeado",
    },
    ToolInfo {
        id: ToolId::Unlock,
        key: "remover-senha",
        title: "Remover senha",
        summary: "Salve uma cópia sem senha de um PDF cuja senha você conhece",
        icon: "pdftoolbox-remover-senha-symbolic",
        category: Category::Security,
        accepts: Accepts::Pdf,
        multiple: true,
        min_files: 1,
        needs: &[Tool::Qpdf],
        keywords: "desbloquear senha descriptografar destravar tirar proteção",
    },
];

pub fn info(id: ToolId) -> &'static ToolInfo {
    TOOLS.iter().find(|t| t.id == id).expect("ferramenta no catálogo")
}

pub fn by_key(key: &str) -> Option<&'static ToolInfo> {
    TOOLS.iter().find(|t| t.key == key)
}

/// Remove acentos e caixa para a busca.
pub fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

pub fn matches(tool: &ToolInfo, query: &str) -> bool {
    let haystack = normalize(&format!("{} {} {}", tool.title, tool.summary, tool.keywords));
    normalize(query).split_whitespace().all(|w| haystack.contains(w))
}

pub const PDF_EXTENSIONS: &[&str] = &["pdf"];
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff"];
pub use crate::operations::OFFICE_EXTENSIONS;

pub fn extension_ok(accepts: Accepts, path: &std::path::Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match accepts {
        Accepts::Pdf => PDF_EXTENSIONS.contains(&ext.as_str()),
        Accepts::Images => IMAGE_EXTENSIONS.contains(&ext.as_str()),
        Accepts::Office => OFFICE_EXTENSIONS.contains(&ext.as_str()),
    }
}
