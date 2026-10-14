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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Organize,
    Convert,
    Compress,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::Organize, Category::Convert, Category::Compress];

    pub fn title(self) -> &'static str {
        match self {
            Category::Organize => "Organizar",
            Category::Convert => "Converter",
            Category::Compress => "Comprimir",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepts {
    Pdf,
    Images,
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
        summary: "Reordene arrastando, gire, duplique e exclua páginas vendo as miniaturas",
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

pub fn extension_ok(accepts: Accepts, path: &std::path::Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match accepts {
        Accepts::Pdf => PDF_EXTENSIONS.contains(&ext.as_str()),
        Accepts::Images => IMAGE_EXTENSIONS.contains(&ext.as_str()),
    }
}
