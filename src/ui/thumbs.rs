//! Miniaturas compartilhadas (tela inicial, listas de arquivos e
//! pré-visualizações). A renderização roda fora da thread da interface; o
//! resultado fica num cache em memória, por arquivo e data de modificação,
//! e nunca é gravado em disco fora da pasta temporária da tarefa.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gtk::prelude::Cast;
use gtk::{gdk, glib};

use pdftoolbox::engines::{poppler, qpdf};
use pdftoolbox::error::display_name;
use pdftoolbox::jobs::CancelToken;
use pdftoolbox::operations::human_size;
use pdftoolbox::storage::JobDir;
use pdftoolbox::{OpError, pdfwriter};

use super::page_thumb::PageThumb;
use super::widgets;

/// Lado maior das miniaturas renderizadas, em pixels.
pub const SIZE: u32 = 180;
/// Acima disso, o cache é esvaziado antes de crescer mais.
const MAX_ENTRIES: usize = 1500;

type Key = (PathBuf, Option<SystemTime>, u32);

thread_local! {
    static CACHE: RefCell<HashMap<Key, gdk::Texture>> = RefCell::new(HashMap::new());
}

fn key(path: &Path, page: u32) -> Key {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    (path.to_path_buf(), mtime, page)
}

pub fn cached(path: &Path, page: u32) -> Option<gdk::Texture> {
    let k = key(path, page);
    CACHE.with(|c| c.borrow().get(&k).cloned())
}

fn store(path: &Path, page: u32, texture: &gdk::Texture) {
    let k = key(path, page);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= MAX_ENTRIES {
            c.clear();
        }
        c.insert(k, texture.clone());
    });
}

/// Pixels RGBA prontos para virar textura (podem atravessar threads).
struct Pixels {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Pixels {
    fn from_image(img: image::DynamicImage) -> Pixels {
        let rgba = img.into_rgba8();
        Pixels { width: rgba.width(), height: rgba.height(), data: rgba.into_raw() }
    }

    fn texture(self) -> gdk::Texture {
        gdk::MemoryTexture::new(
            self.width as i32,
            self.height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(self.data),
            self.width as usize * 4,
        )
        .upcast()
    }
}

/// Renderiza as páginas `first..=last` de um PDF numa única chamada do pdftoppm.
fn render_pdf(path: &Path, first: u32, last: u32, cancel: &CancelToken) -> Vec<(u32, Pixels)> {
    let Ok(dir) = JobDir::new() else { return Vec::new() };
    let Ok(files) = poppler::thumbnails(path, first, last, SIZE, dir.path(), cancel) else { return Vec::new() };
    (first..=last)
        .zip(files)
        .filter_map(|(page, file)| image::open(&file).ok().map(|img| (page, Pixels::from_image(img))))
        .collect()
}

/// Lê uma imagem, aplica a orientação EXIF e reduz para o tamanho da miniatura.
fn render_image(path: &Path) -> Option<Pixels> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?.thumbnail(SIZE, SIZE);
    img.apply_orientation(orientation);
    Some(Pixels::from_image(img))
}

/// Miniaturas das páginas `first..=last`, usando o cache quando possível.
pub async fn pdf_pages(path: &Path, first: u32, last: u32, cancel: &CancelToken) -> Vec<(u32, gdk::Texture)> {
    let mut have: Vec<(u32, gdk::Texture)> = Vec::new();
    let mut missing = Vec::new();
    for page in first..=last {
        match cached(path, page) {
            Some(t) => have.push((page, t)),
            None => missing.push(page),
        }
    }
    if let (Some(&a), Some(&b)) = (missing.first(), missing.last()) {
        let (p, c) = (path.to_path_buf(), cancel.clone());
        let rendered = widgets::blocking(move || render_pdf(&p, a, b, &c)).await;
        for (page, pixels) in rendered {
            let texture = pixels.texture();
            store(path, page, &texture);
            if missing.contains(&page) {
                have.push((page, texture));
            }
        }
    }
    have.sort_by_key(|(p, _)| *p);
    have
}

/// Primeira página de um PDF ou a própria imagem, já reduzida.
/// Documentos do Office não têm miniatura (seria preciso convertê-los).
pub async fn first_page(path: &Path) -> Option<gdk::Texture> {
    if let Some(t) = cached(path, 1) {
        return Some(t);
    }
    if is_office(path) {
        return None;
    }
    if pdftoolbox::catalog::extension_ok(pdftoolbox::catalog::Accepts::Pdf, path) {
        return pdf_pages(path, 1, 1, &CancelToken::new()).await.into_iter().next().map(|(_, t)| t);
    }
    let p = path.to_path_buf();
    let texture = widgets::blocking(move || render_image(&p)).await?.texture();
    store(path, 1, &texture);
    Some(texture)
}

/// Informações de um arquivo para exibir ao usuário.
#[derive(Debug, Clone, Default)]
pub struct FileFacts {
    pub size: Option<u64>,
    pub pages: Option<u32>,
    /// Dimensões exibidas (imagens: pixels; PDFs: pontos da 1ª página).
    pub dims: Option<(f32, f32)>,
    /// Problema que impede o uso do arquivo, já em linguagem do usuário.
    pub problem: Option<String>,
    /// PDF que exige senha para abrir (só “Remover senha” o aceita).
    pub locked: bool,
    /// PDFs: páginas com texto selecionável e total (`None` se não deu para ler).
    pub text_pages: Option<(u32, u32)>,
}

impl FileFacts {
    /// Mais da metade das páginas sem texto selecionável: provável
    /// digitalização, que só o OCR torna pesquisável. Uma capa ou ilustração
    /// sem texto num livro digital não conta.
    pub fn mostly_scanned(&self) -> bool {
        self.text_pages.is_some_and(|(with, total)| total > 0 && with * 2 < total)
    }

    /// Linha curta: “308 páginas · 6,5 MB” ou “1920 × 1080 px · 2 MB”.
    pub fn line(&self, is_pdf: bool) -> String {
        let size = self.size.map(human_size);
        if self.locked {
            return match size {
                Some(s) => format!("🔒 Protegido por senha · {s}"),
                None => "🔒 Protegido por senha".into(),
            };
        }
        if let Some(problem) = &self.problem {
            return match size {
                Some(s) => format!("⚠ {problem} · {s}"),
                None => format!("⚠ {problem}"),
            };
        }
        let mut parts = Vec::new();
        if is_pdf {
            if let Some(n) = self.pages {
                parts.push(format!("{n} página{}", if n == 1 { "" } else { "s" }));
            }
        } else if let Some((w, h)) = self.dims {
            parts.push(format!("{} × {} px", w as u32, h as u32));
        }
        parts.extend(size);
        match self.text_pages {
            Some((0, total)) if total > 0 => parts.push("sem texto selecionável".into()),
            Some((with, total)) if self.mostly_scanned() => {
                parts.push(format!("texto selecionável em só {with} de {total} páginas"))
            }
            _ => {}
        }
        parts.join(" · ")
    }
}

/// Lê tamanho, páginas e dimensões (em segundo plano).
pub fn facts_blocking(path: &Path) -> FileFacts {
    let mut facts = FileFacts::default();
    match std::fs::metadata(path) {
        Ok(m) => facts.size = Some(m.len()),
        Err(_) => {
            facts.problem = Some("Arquivo não encontrado ou sem permissão de leitura".into());
            return facts;
        }
    }
    let cancel = CancelToken::new();
    if pdftoolbox::catalog::extension_ok(pdftoolbox::catalog::Accepts::Pdf, path) {
        match qpdf::page_count(path, &cancel) {
            Ok(n) => facts.pages = Some(n),
            Err(OpError::PasswordProtected(_)) => facts.locked = true,
            Err(OpError::MissingTool(_)) => {}
            Err(_) => facts.problem = Some("Não parece ser um PDF válido".into()),
        }
        if facts.problem.is_none() && !facts.locked {
            facts.dims = poppler::page_size(path, 1, &cancel).ok();
            facts.text_pages = pdftoolbox::operations::text_pages(path, &cancel).ok();
        }
    } else if is_office(path) {
        // Só o tamanho: ler páginas exigiria converter o documento.
    } else {
        match pdfwriter::display_dimensions(path) {
            Ok((w, h)) => facts.dims = Some((w as f32, h as f32)),
            Err(_) => facts.problem = Some("Imagem ilegível ou formato não suportado".into()),
        }
    }
    facts
}

pub async fn facts(path: &Path) -> FileFacts {
    let p = path.to_path_buf();
    widgets::blocking(move || facts_blocking(&p)).await
}

/// Miniatura da primeira página (ou da imagem) que carrega sozinha.
pub fn picture(path: &Path, max_w: i32, max_h: i32) -> PageThumb {
    use gtk::prelude::*;
    let thumb = PageThumb::new(max_w, max_h);
    thumb.update_property(&[gtk::accessible::Property::Label(&format!("Miniatura de {}", display_name(path)))]);
    if let Some(t) = cached(path, 1) {
        thumb.set_texture(Some(&t));
        return thumb;
    }
    let weak = thumb.downgrade();
    let path = path.to_path_buf();
    glib::spawn_future_local(async move {
        let texture = first_page(&path).await;
        let Some(thumb) = weak.upgrade() else { return };
        match texture {
            Some(texture) => thumb.set_texture(Some(&texture)),
            // Sem miniatura: se for por senha, um cadeado explica o porquê.
            None if pdftoolbox::catalog::extension_ok(pdftoolbox::catalog::Accepts::Pdf, &path) => {
                let p = path.clone();
                let kind = widgets::blocking(move || qpdf::encryption(&p, &CancelToken::new()).ok()).await;
                if kind == Some(qpdf::Encryption::Password) {
                    thumb.add_css_class("miniatura-bloqueada");
                    thumb.update_property(&[gtk::accessible::Property::Label(&format!(
                        "{} está protegido por senha",
                        display_name(&path)
                    ))]);
                }
            }
            None => {}
        }
    });
    thumb
}

pub fn is_office(path: &Path) -> bool {
    pdftoolbox::catalog::extension_ok(pdftoolbox::catalog::Accepts::Office, path)
}

/// Página `page` de `path` com a camada de carimbo aplicada, renderizada com
/// lado maior `size` px. Usa as mesmas funções da operação (qpdf + gerador da
/// camada), então a prévia mostra exatamente o resultado.
/// `layer` recebe o tamanho exibido da página e devolve a camada dela.
pub async fn stamped_page(
    path: &Path,
    page: u32,
    layer: impl FnOnce((f32, f32)) -> pdfwriter::OverlayPage + Send + 'static,
    size: u32,
) -> Option<gdk::Texture> {
    let p = path.to_path_buf();
    let pixels = widgets::blocking(move || -> Option<Pixels> {
        let cancel = CancelToken::new();
        let layer = layer(poppler::page_size(&p, page, &cancel).ok()?);
        let dir = JobDir::new().ok()?;
        let single = dir.file("pagina.pdf");
        qpdf::assemble(&[(p.as_path(), page)], &single, &cancel).ok()?;
        let layer_pdf = dir.file("camada.pdf");
        pdfwriter::overlay_pdf(&layer_pdf, &[layer]).ok()?;
        let stamped = dir.file("carimbado.pdf");
        qpdf::overlay(&single, &layer_pdf, &stamped, &cancel).ok()?;
        let files = poppler::thumbnails(&stamped, 1, 1, size, dir.path(), &cancel).ok()?;
        image::open(files.first()?).ok().map(Pixels::from_image)
    })
    .await?;
    Some(pixels.texture())
}

/// Página `page` renderizada grande (lado maior = `size` px), para o
/// visualizador ampliado. Não entra no cache das miniaturas.
pub async fn large_page(path: &Path, page: u32, size: u32) -> Option<gdk::Texture> {
    let p = path.to_path_buf();
    let pixels = widgets::blocking(move || -> Option<Pixels> {
        let dir = JobDir::new().ok()?;
        let files = poppler::thumbnails(&p, page, page, size, dir.path(), &CancelToken::new()).ok()?;
        image::open(files.first()?).ok().map(Pixels::from_image)
    })
    .await?;
    Some(pixels.texture())
}

/// Folha branca `w` × `h` px (páginas em branco do organizador).
pub fn blank_texture(w: u32, h: u32) -> gdk::Texture {
    let (w, h) = (w.max(1), h.max(1));
    Pixels { width: w, height: h, data: vec![255; (w * h * 4) as usize] }.texture()
}

/// Primeira página com texto selecionável e o texto dela (pré-visualização da
/// extração). `Some(None)`: nenhuma página tem texto; `None`: não deu para ler.
pub async fn first_text_page(path: &Path, layout: bool) -> Option<Option<(u32, String)>> {
    let p = path.to_path_buf();
    widgets::blocking(move || {
        let cancel = CancelToken::new();
        let flags = pdftoolbox::operations::pages_with_text(&p, &cancel).ok()?;
        let Some(i) = flags.iter().position(|&t| t) else { return Some(None) };
        let page = i as u32 + 1;
        poppler::page_text(&p, page, layout, &cancel).ok().map(|t| Some((page, t)))
    })
    .await
}
