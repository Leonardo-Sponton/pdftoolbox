//! Numeração de páginas, marca d’água e metadados. Os carimbos são uma camada
//! sobreposta pelo qpdf: o conteúdo original não é rasterizado nem reescrito.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{Destination, Outcome, batch, batch_output_name, output_name, verify_pdf};
use crate::annotate::{self, Annotation};
use crate::engines::{poppler, qpdf};
use crate::error::{OpError, OpResult};
use crate::jobs::JobCtx;
use crate::pages;
use crate::pdfwriter::{self, OverlayPage};
use crate::stamp::{self, NumberStyle, WatermarkStyle};
use crate::storage::{self, JobDir, stem};

/// Campos editáveis do dicionário de informações. Texto vazio remove o campo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocFields {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataChange {
    Set(DocFields),
    /// Remove o dicionário de informações e os metadados XMP.
    RemoveAll,
}

/// Páginas escolhidas (`None` ou vazio = todas), em ordem e sem repetição:
/// a numeração segue a ordem do documento, não a da digitação.
fn chosen_pages(spec: Option<&str>, total: u32) -> OpResult<Vec<u32>> {
    let mut list = match spec.map(str::trim).filter(|s| !s.is_empty()) {
        Some(spec) => pages::parse_list(spec, total)?,
        None => (1..=total).collect(),
    };
    list.sort_unstable();
    list.dedup();
    Ok(list)
}

fn apply_layer(
    inputs: &[PathBuf],
    spec: Option<&str>,
    suffix: &str,
    // Progresso, já traduzido, com `{file}` para o nome do arquivo.
    action: &str,
    dest: &Destination,
    ctx: &JobCtx,
    layer: impl Fn(&[(f32, f32)], &[u32]) -> Vec<OverlayPage>,
) -> OpResult<Outcome> {
    let n = inputs.len();
    batch(inputs, |i, input, outcome| {
        ctx.report(i as f64 / n as f64, &crate::i18n::fill(action, &[("file", &stem(input))]));
        let total = qpdf::page_count(input, &ctx.cancel)?;
        let chosen = chosen_pages(spec, total)?;
        let sizes = poppler::page_sizes(input, total, &ctx.cancel)?;
        let job = JobDir::new()?;
        let layer_pdf = job.file("camada.pdf");
        pdfwriter::overlay_pdf(&layer_pdf, &layer(&sizes, &chosen))
            .map_err(|e| OpError::io(tr!("gravar a camada temporária"), e))?;
        let tmp = job.file("carimbado.pdf");
        qpdf::overlay(input, &layer_pdf, &tmp, &ctx.cancel)?;
        verify_pdf(&tmp, total, ctx)?;
        let base = format!("{} - {suffix}", stem(input));
        let name = batch_output_name(dest, n, &base, "pdf");
        outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
        Ok(())
    })
}

pub fn number_pages(
    inputs: &[PathBuf],
    spec: Option<&str>,
    style: &NumberStyle,
    dest: &Destination,
    ctx: &JobCtx,
) -> OpResult<Outcome> {
    apply_layer(inputs, spec, "numerado", tr!("Numerando {file}"), dest, ctx, |sizes, chosen| {
        stamp::number_pages(sizes, chosen, style)
    })
}

pub fn watermark(
    inputs: &[PathBuf],
    spec: Option<&str>,
    style: &WatermarkStyle,
    dest: &Destination,
    ctx: &JobCtx,
) -> OpResult<Outcome> {
    let text = style.text.trim();
    if text.is_empty() {
        return Err(OpError::InvalidRange(tr!("Digite o texto da marca d’água.").into()));
    }
    let (_, missing) = stamp::encode(text);
    if !missing.is_empty() {
        return Err(OpError::InvalidRange(unsupported_chars(&missing)));
    }
    apply_layer(
        inputs,
        spec,
        "marca d’água",
        tr!("Aplicando marca d’água em {file}"),
        dest,
        ctx,
        |sizes, chosen| stamp::watermark(sizes, chosen, style),
    )
}

/// Anotações e assinatura visual sobre um PDF.
pub fn annotate(input: &Path, items: &[Annotation], dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    let total = qpdf::page_count(input, &ctx.cancel)?;
    annotate::validate(items, total)?;
    ctx.report(0.1, tr!("Lendo o tamanho das páginas"));
    let sizes = poppler::page_sizes(input, total, &ctx.cancel)?;
    ctx.report(0.3, tr!("Preparando imagens"));
    let sources = annotate::image_sources(items);
    let images = annotate::load_images(&sources)?;
    ctx.cancel.check()?;
    let job = JobDir::new()?;
    let layer_pdf = job.file("camada.pdf");
    pdfwriter::overlay_pdf_with_images(&layer_pdf, &annotate::layer(items, &sizes, &sources), &images)
        .map_err(|e| OpError::io(tr!("gravar a camada temporária"), e))?;
    drop(images);
    ctx.report(0.6, tr!("Aplicando anotações"));
    let tmp = job.file("anotado.pdf");
    qpdf::overlay(input, &layer_pdf, &tmp, &ctx.cancel)?;
    verify_pdf(&tmp, total, ctx)?;
    let name = output_name(dest, &format!("{} - anotado", stem(input)), "pdf");
    let mut outcome = Outcome::default();
    outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
    if items.iter().any(|a| matches!(a.kind, annotate::Kind::Image { .. })) {
        outcome.notes.push(
            tr!("A assinatura foi inserida como imagem: serve como assinatura visual, sem validade de assinatura \
                 digital.")
            .into(),
        );
    }
    Ok(outcome)
}

/// Mensagem para caracteres fora da codificação das fontes padrão.
pub fn unsupported_chars(missing: &[char]) -> String {
    let list: Vec<String> = missing.iter().map(|c| format!("“{c}”")).collect();
    tr!(
        "Estes caracteres não existem nas fontes padrão do PDF usadas nos textos: {chars}. \
         Letras acentuadas do português funcionam; símbolos e outros alfabetos ainda não.",
        chars = list.join(" ")
    )
}

/// Data atual no fuso local, no formato de data do PDF.
fn pdf_date_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    pdf_date(secs, local_offset(secs))
}

/// Deslocamento do fuso local em segundos (ex.: −10800 para UTC−3).
fn local_offset(secs: u64) -> i64 {
    let t = secs as libc::time_t;
    // SAFETY: `localtime_r` só escreve em `tm`, que é nosso.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    if ok { tm.tm_gmtoff } else { 0 }
}

/// Segundos desde 1970 + deslocamento → `D:AAAAMMDDhhmmss-03'00'` (algoritmo
/// de dias civis; `Z` quando o fuso é UTC).
fn pdf_date(secs: u64, offset: i64) -> String {
    let local = secs as i64 + offset;
    let days = local.div_euclid(86_400);
    let rem = local.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let zone = if offset == 0 {
        "Z".to_string()
    } else {
        let sign = if offset < 0 { '-' } else { '+' };
        let m = offset.abs() / 60;
        format!("{sign}{:02}'{:02}'", m / 60, m % 60)
    };
    format!("D:{year:04}{month:02}{day:02}{:02}{:02}{:02}{zone}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Chaves do trailer que pertencem ao documento; as da tabela de
/// referências o qpdf refaz. Sem /Size, o qpdf grava uma tabela de
/// referências inválida.
fn document_keys(trailer: &Value) -> Map<String, Value> {
    trailer
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(k, _)| matches!(k.as_str(), "/Root" | "/ID" | "/Size"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Atualização no formato JSON v2 do qpdf com `objects` (trailer e objetos).
fn qpdf_update(max_object: u64, objects: Map<String, Value>) -> Value {
    json!({
        "qpdf": [
            {
                "jsonversion": 2,
                "pushedinheritedpageresources": false,
                "calledgetallpages": false,
                "maxobjectid": max_object
            },
            objects
        ]
    })
}

/// Monta a atualização JSON do qpdf: um dicionário de informações novo
/// (cópia do atual com os campos alterados) e o trailer apontando para ele.
/// Editar o objeto existente não serve: quando ele está dentro de um fluxo de
/// objetos (comum), o qpdf ignora a atualização.
fn info_update(
    trailer: &Value,
    max_object: u64,
    current: Option<&Map<String, Value>>,
    fields: &DocFields,
    now: &str,
) -> Value {
    let mut info = current.cloned().unwrap_or_default();
    for (key, value) in [
        ("/Title", &fields.title),
        ("/Author", &fields.author),
        ("/Subject", &fields.subject),
        ("/Keywords", &fields.keywords),
    ] {
        let value = value.trim();
        if value.is_empty() {
            info.remove(key);
        } else {
            info.insert(key.into(), Value::String(format!("u:{value}")));
        }
    }
    info.insert("/ModDate".into(), Value::String(format!("u:{now}")));
    let id = format!("{} 0 R", max_object + 1);
    let mut t = document_keys(trailer);
    t.insert("/Info".into(), Value::String(id.clone()));
    t.insert("/Size".into(), json!(max_object + 2));
    let mut objects = Map::new();
    objects.insert("trailer".into(), json!({ "value": t }));
    objects.insert(format!("obj:{id}"), json!({ "value": info }));
    qpdf_update(max_object, objects)
}

/// Atualização que regrava o trailer sem a chave /Info.
fn trailer_without_info(trailer: &Value, max_object: u64) -> Value {
    let mut objects = Map::new();
    objects.insert("trailer".into(), json!({ "value": document_keys(trailer) }));
    qpdf_update(max_object, objects)
}

/// Aplica a atualização JSON `update` a `input`, gravando em `out`.
fn apply_update(input: &Path, update: &Value, job: &JobDir, out: &Path, ctx: &JobCtx) -> OpResult<()> {
    let json_path = job.file("atualizacao.json");
    let bytes = serde_json::to_vec(update)
        .map_err(|e| OpError::OutputInvalid(tr!("não foi possível montar a atualização: {error}", error = e)))?;
    std::fs::write(&json_path, bytes).map_err(|e| OpError::io(tr!("gravar arquivo temporário"), e))?;
    qpdf::update_from_json(input, &json_path, out, &ctx.cancel)
}

pub fn metadata(input: &Path, change: &MetadataChange, dest: &Destination, ctx: &JobCtx) -> OpResult<Outcome> {
    let total = qpdf::page_count(input, &ctx.cancel)?;
    if qpdf::encryption(input, &ctx.cancel)? != qpdf::Encryption::None {
        return Err(OpError::InvalidRange(tr!(
            "“{file}” tem restrições definidas pelo autor. Remova-as antes com “Remover senha”, usando a senha \
             de permissões.",
            file = crate::human::file_name(input)
        )));
    }
    let job = JobDir::new()?;
    let tmp = job.file("metadados.pdf");
    let mut outcome = Outcome::default();
    let suffix = match change {
        MetadataChange::RemoveAll => {
            ctx.report(0.3, tr!("Removendo metadados"));
            let stripped = job.file("sem-xmp.pdf");
            qpdf::strip_metadata(input, &stripped, &ctx.cancel)?;
            // O --remove-info do qpdf mantém a data de modificação; sem o
            // dicionário de informações no trailer, não sobra nada.
            let (trailer, max) = qpdf::trailer(&stripped, &ctx.cancel)?;
            apply_update(&stripped, &trailer_without_info(&trailer, max), &job, &tmp, ctx)?;
            let left = poppler::doc_info(&tmp, &ctx.cancel)?;
            if left != poppler::DocInfo::default() {
                return Err(OpError::OutputInvalid(tr!("ainda restaram metadados no arquivo").into()));
            }
            "sem metadados"
        }
        MetadataChange::Set(fields) => {
            ctx.report(0.2, tr!("Lendo metadados"));
            let (trailer, max) = qpdf::trailer(input, &ctx.cancel)?;
            let current = match trailer.get("/Info").and_then(Value::as_str) {
                Some(id) => qpdf::dictionary(input, id, &ctx.cancel)?,
                None => None,
            };
            let update = info_update(&trailer, max, current.as_ref(), fields, &pdf_date_now());
            ctx.report(0.5, tr!("Gravando metadados"));
            apply_update(input, &update, &job, &tmp, ctx)?;
            let got = poppler::doc_info(&tmp, &ctx.cancel)?;
            if got.title != fields.title.trim() || got.author != fields.author.trim() {
                return Err(OpError::OutputInvalid(tr!("os metadados gravados não conferem com os informados").into()));
            }
            "metadados"
        }
    };
    verify_pdf(&tmp, total, ctx)?;
    let name = output_name(dest, &format!("{} - {suffix}", stem(input)), "pdf");
    outcome.push(storage::commit(&tmp, &dest.dir, &name)?);
    if *change == MetadataChange::RemoveAll {
        outcome.notes.push(tr!("Título, autor, assunto, palavras-chave, programas e datas foram removidos.").into());
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_dates_carry_the_local_offset() {
        assert_eq!(pdf_date(0, 0), "D:19700101000000Z");
        // 2026-10-14 12:30:05 UTC
        assert_eq!(pdf_date(1_791_981_005, 0), "D:20261014123005Z");
        assert_eq!(pdf_date(1_791_981_005, -3 * 3600), "D:20261014093005-03'00'");
        assert_eq!(pdf_date(1_791_981_005, 5 * 3600 + 1800), "D:20261014180005+05'30'");
        // Virada de dia para trás.
        assert_eq!(pdf_date(1_791_936_000, -3 * 3600), "D:20261013210000-03'00'");
        assert_eq!(pdf_date(951_782_400, 0), "D:20000229000000Z");
    }

    #[test]
    fn creates_info_when_document_has_none() {
        let trailer =
            json!({"/Root": "1 0 R", "/ID": ["b:00", "b:11"], "/Size": 21, "/Type": "/XRef", "/W": [1, 2, 1]});
        let fields = DocFields { title: "Relatório".into(), author: " Ana ".into(), ..Default::default() };
        let up = info_update(&trailer, 20, None, &fields, "D:20261014093000-03'00'");
        let objs = &up["qpdf"][1];
        let t = &objs["trailer"]["value"];
        assert_eq!(t["/Info"], "21 0 R");
        assert_eq!(t["/Root"], "1 0 R");
        assert_eq!(t["/Size"], 22);
        assert!(t.get("/W").is_none() && t.get("/Type").is_none());
        let info = &objs["obj:21 0 R"]["value"];
        assert_eq!(info["/Title"], "u:Relatório");
        assert_eq!(info["/Author"], "u:Ana");
        assert!(info.get("/Subject").is_none());
        assert_eq!(info["/ModDate"], "u:D:20261014093000-03'00'");
    }

    #[test]
    fn trailer_loses_only_the_info_key() {
        let trailer = json!({"/Root": "1 0 R", "/Info": "2 0 R", "/Size": 9, "/ID": ["b:00", "b:00"], "/W": [1, 2, 1]});
        let up = trailer_without_info(&trailer, 8);
        let t = &up["qpdf"][1]["trailer"]["value"];
        assert!(t.get("/Info").is_none() && t.get("/W").is_none());
        assert_eq!(t["/Root"], "1 0 R");
        assert_eq!(t["/Size"], 9);
    }

    #[test]
    fn copies_existing_info_into_a_new_object() {
        let mut current = Map::new();
        current.insert("/Title".into(), json!("u:Antigo"));
        current.insert("/Producer".into(), json!("u:Gerador X"));
        current.insert("/Keywords".into(), json!("u:a, b"));
        let fields = DocFields { title: "Novo".into(), ..Default::default() };
        let trailer = json!({"/Root": "1 0 R", "/Info": "4 0 R", "/Size": 10});
        let up = info_update(&trailer, 9, Some(&current), &fields, "D:1");
        let objs = &up["qpdf"][1];
        assert_eq!(objs["trailer"]["value"]["/Info"], "10 0 R");
        let info = &objs["obj:10 0 R"]["value"];
        assert_eq!(info["/Title"], "u:Novo");
        assert_eq!(info["/Producer"], "u:Gerador X");
        assert!(info.get("/Keywords").is_none(), "campo apagado na interface é removido");
    }
}
