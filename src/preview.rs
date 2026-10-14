//! Planos de pré-visualização: o que cada ferramenta fará com cada página,
//! calculado com as mesmas regras das operações, sem tocar no arquivo. A
//! interface usa o plano para marcar as miniaturas e escrever o resumo.

use crate::error::{OpError, OpResult};
use crate::operations::SplitMode;
use crate::pages;
use crate::stamp::{self, NumberFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkKind {
    /// A página não é afetada (permanece como está).
    Normal,
    /// A página é afetada (girada, extraída).
    Selected,
    /// A página sai do resultado.
    Removed,
    /// A página fica de fora de todos os arquivos gerados.
    Excluded,
    /// A página vai para o arquivo gerado de índice N (0, 1, …).
    Group(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageMark {
    pub kind: MarkKind,
    /// Texto curto sobre a miniatura (ex.: “1º”, “Arq. 2”, “↻ 90°”).
    pub badge: Option<String>,
    /// Rotação adicional a mostrar, em graus.
    pub rotation: i32,
}

impl PageMark {
    fn normal() -> Self {
        PageMark { kind: MarkKind::Normal, badge: None, rotation: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Uma marca por página; o índice 0 é a página 1.
    pub marks: Vec<PageMark>,
    pub summary: String,
}

impl Plan {
    /// Primeira página (1…) com marca diferente de `Normal`.
    pub fn first_marked(&self) -> Option<u32> {
        self.marks.iter().position(|m| m.kind != MarkKind::Normal).map(|i| i as u32 + 1)
    }

    /// Páginas (1…) marcadas neste plano e não marcadas em `before`
    /// (índice 0 = página 1). Usado para decidir se vale rolar a grade.
    pub fn newly_marked(&self, before: &[bool]) -> Vec<u32> {
        self.marks
            .iter()
            .enumerate()
            .filter(|(i, m)| m.kind != MarkKind::Normal && !before.get(*i).copied().unwrap_or(false))
            .map(|(i, _)| i as u32 + 1)
            .collect()
    }

    /// Marcação por página, para comparar com o próximo plano.
    pub fn marked(&self) -> Vec<bool> {
        self.marks.iter().map(|m| m.kind != MarkKind::Normal).collect()
    }
}

pub fn angle_text(angle: i32) -> &'static str {
    match angle.rem_euclid(360) {
        90 => tr!("90° no sentido horário"),
        180 => "180°",
        270 => tr!("90° no sentido anti-horário"),
        _ => "0°",
    }
}

fn dedup(list: &[u32]) -> Vec<u32> {
    let mut seen = Vec::new();
    for &p in list {
        if !seen.contains(&p) {
            seen.push(p);
        }
    }
    seen
}

/// Girar: `spec` vazio significa todas as páginas.
pub fn rotate(spec: &str, angle: i32, total: u32) -> OpResult<Plan> {
    let selected = if spec.trim().is_empty() { (1..=total).collect() } else { dedup(&pages::parse_list(spec, total)?) };
    let mut marks = vec![PageMark::normal(); total as usize];
    let symbol = if angle.rem_euclid(360) == 270 { "↺" } else { "↻" };
    for &p in &selected {
        marks[p as usize - 1] = PageMark {
            kind: MarkKind::Selected,
            badge: Some(format!("{symbol} {}°", if angle.rem_euclid(360) == 270 { 90 } else { angle.rem_euclid(360) })),
            rotation: angle.rem_euclid(360),
        };
    }
    let n = selected.len();
    let how = angle_text(angle);
    let summary = if total == 1 {
        tr!("A única página será girada {how}.", how)
    } else if n as u32 == total {
        tr!("Todas as {total} páginas serão giradas {how}.", total, how)
    } else {
        trn!(
            n,
            "{n} de {total} páginas será girada {how}; as demais ficam como estão.",
            "{n} de {total} páginas serão giradas {how}; as demais ficam como estão.",
            n,
            total,
            how
        )
    };
    Ok(Plan { marks, summary })
}

/// Páginas escolhidas para um carimbo: `spec` vazio significa todas.
fn stamped(spec: &str, total: u32) -> OpResult<Vec<u32>> {
    if spec.trim().is_empty() { Ok((1..=total).collect()) } else { Ok(pages::parse_list(spec, total)?) }
}

/// Numerar: cada página escolhida mostra, no selo, o número que vai receber.
/// Como na operação, a contagem segue a ordem das páginas (sem repetições).
pub fn numbers(spec: &str, format: NumberFormat, first_number: u32, total: u32) -> OpResult<Plan> {
    let mut chosen = dedup(&stamped(spec, total)?);
    chosen.sort_unstable();
    let last = first_number + chosen.len().saturating_sub(1) as u32;
    let mut marks = vec![PageMark::normal(); total as usize];
    for (k, &p) in chosen.iter().enumerate() {
        let n = first_number + k as u32;
        marks[p as usize - 1] =
            PageMark { kind: MarkKind::Selected, badge: Some(stamp::number_text(format, n, last)), rotation: 0 };
    }
    let n = chosen.len();
    let summary = match (n as u32 == total, n) {
        (true, 1) => tr!("A única página receberá o número {first_number}.", first_number),
        (true, _) => {
            tr!("Todas as {total} páginas serão numeradas, de {first_number} a {last}.", total, first_number, last)
        }
        (false, 1) => tr!(
            "1 de {total} páginas receberá o número {first_number}; as demais ficam sem número.",
            total,
            first_number
        ),
        (false, _) => tr!(
            "{n} de {total} páginas serão numeradas, de {first_number} a {last}; as demais ficam sem número.",
            n,
            total,
            first_number,
            last
        ),
    };
    Ok(Plan { marks, summary })
}

/// Marca d’água: páginas que recebem o texto.
pub fn watermark(spec: &str, total: u32) -> OpResult<Plan> {
    let chosen = dedup(&stamped(spec, total)?);
    let mut marks = vec![PageMark::normal(); total as usize];
    for &p in &chosen {
        marks[p as usize - 1] = PageMark { kind: MarkKind::Selected, badge: Some(tr!("Marca").into()), rotation: 0 };
    }
    let n = chosen.len();
    let summary = match (n as u32 == total, n) {
        (true, 1) => tr!("A única página receberá a marca d’água.").to_string(),
        (true, _) => tr!("Todas as {total} páginas receberão a marca d’água.", total),
        (false, _) => trn!(
            n,
            "{n} de {total} páginas receberá a marca d’água.",
            "{n} de {total} páginas receberão a marca d’água.",
            n,
            total
        ),
    };
    Ok(Plan { marks, summary })
}

/// Extrair: as páginas saem na ordem digitada (repetições são permitidas).
pub fn extract(spec: &str, separate: bool, total: u32) -> OpResult<Plan> {
    let list = pages::parse_list(spec, total)?;
    let mut marks = vec![PageMark::normal(); total as usize];
    let mut positions: Vec<Vec<usize>> = vec![Vec::new(); total as usize];
    for (i, &p) in list.iter().enumerate() {
        positions[p as usize - 1].push(i + 1);
    }
    let in_order = list.windows(2).all(|w| w[0] < w[1]);
    for (i, pos) in positions.iter().enumerate() {
        if pos.is_empty() {
            continue;
        }
        // A posição só é informativa quando a ordem difere da original.
        let badge = (!in_order).then(|| pos.iter().map(|n| tr!("{n}º", n)).collect::<Vec<_>>().join(", "));
        marks[i] = PageMark { kind: MarkKind::Selected, badge, rotation: 0 };
    }
    let n = list.len();
    let summary = if separate && n == 1 {
        tr!("Será criado 1 PDF com a página escolhida.").to_string()
    } else if separate {
        tr!("Serão criados {n} PDFs, um por página.", n)
    } else if in_order {
        trn!(n, "{n} página será copiada para um novo PDF.", "{n} páginas serão copiadas para um novo PDF.", n)
    } else {
        trn!(
            n,
            "{n} página será copiada para um novo PDF, na ordem digitada (veja os números sobre as páginas).",
            "{n} páginas serão copiadas para um novo PDF, na ordem digitada (veja os números sobre as páginas).",
            n
        )
    };
    Ok(Plan { marks, summary })
}

/// Remover: o restante é gravado num novo PDF.
pub fn remove(spec: &str, total: u32) -> OpResult<Plan> {
    let removed = pages::parse_list(spec, total)?;
    let keep = pages::complement(&removed, total);
    if keep.is_empty() {
        return Err(OpError::InvalidRange(tr!("Não é possível remover todas as páginas do documento.").into()));
    }
    let mut marks = vec![PageMark::normal(); total as usize];
    for &p in &removed {
        marks[p as usize - 1] = PageMark { kind: MarkKind::Removed, badge: Some(tr!("Removida").into()), rotation: 0 };
    }
    let gone = total as usize - keep.len();
    let left = keep.len();
    let summary = format!(
        "{} {}",
        trn!(gone, "{gone} página será removida;", "{gone} páginas serão removidas;", gone),
        trn!(left, "o novo PDF terá {left} página.", "o novo PDF terá {left} páginas.", left)
    );
    Ok(Plan { marks, summary })
}

/// Dividir: cada grupo vira um arquivo; a cor da miniatura indica o arquivo.
pub fn split(mode: &SplitMode, total: u32) -> OpResult<Plan> {
    let groups = match mode {
        SplitMode::Ranges(spec) => pages::parse_groups(spec, total)?,
        SplitMode::Every(n) => pages::chunks(total, *n),
        SplitMode::EachPage => pages::chunks(total, 1),
    };
    let mut member: Vec<Vec<usize>> = vec![Vec::new(); total as usize];
    for (g, pages) in groups.iter().enumerate() {
        for &p in pages {
            if !member[p as usize - 1].contains(&g) {
                member[p as usize - 1].push(g);
            }
        }
    }
    let marks: Vec<PageMark> = member
        .iter()
        .map(|gs| match gs.as_slice() {
            [] => PageMark { kind: MarkKind::Excluded, badge: Some(tr!("Fora").into()), rotation: 0 },
            [g, ..] => PageMark {
                kind: MarkKind::Group(*g),
                badge: Some(tr!(
                    "Arq. {files}",
                    files = gs.iter().map(|g| (g + 1).to_string()).collect::<Vec<_>>().join(", ")
                )),
                rotation: 0,
            },
        })
        .collect();
    let excluded = marks.iter().filter(|m| m.kind == MarkKind::Excluded).count();
    let count = groups.len();
    let mut summary = trn!(count, "Será criado {count} arquivo.", "Serão criados {count} arquivos.", count);
    if excluded > 0 {
        summary.push(' ');
        summary.push_str(&trn!(
            excluded,
            "{excluded} página não está em nenhum intervalo e fica de fora.",
            "{excluded} páginas não estão em nenhum intervalo e ficam de fora.",
            excluded
        ));
    }
    Ok(Plan { marks, summary })
}

/// Marca ou desmarca `page` numa seleção digitada, devolvendo o novo texto.
/// Seleções que preservam a ordem (`ordered`) acrescentam a página no fim;
/// as demais são reescritas em ordem crescente. Uma seleção inválida é
/// substituída pela página clicada.
pub fn toggle_page(spec: &str, page: u32, total: u32, ordered: bool) -> String {
    let mut list = if spec.trim().is_empty() { Vec::new() } else { pages::parse_list(spec, total).unwrap_or_default() };
    if list.contains(&page) {
        list.retain(|&p| p != page);
    } else {
        list.push(page);
    }
    if !ordered {
        list.sort_unstable();
        list.dedup();
    }
    pages::compact_list(&list).replace(',', ", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_plan_shows_the_number_each_page_gets() {
        let plan = numbers("4, 2-3", NumberFormat::NumberOfTotal, 1, 5).unwrap();
        assert_eq!(plan.marks[0].kind, MarkKind::Normal);
        assert_eq!(plan.marks[1].badge.as_deref(), Some("1 / 3"));
        assert_eq!(plan.marks[3].badge.as_deref(), Some("3 / 3"));
        assert!(plan.summary.starts_with("3 de 5 páginas serão numeradas, de 1 a 3"));
        let all = numbers("", NumberFormat::Number, 10, 2).unwrap();
        assert_eq!(all.marks[1].badge.as_deref(), Some("11"));
        assert!(numbers("9", NumberFormat::Number, 1, 2).is_err());
    }

    #[test]
    fn watermark_plan_marks_chosen_pages() {
        let plan = watermark("2", 3).unwrap();
        assert_eq!(plan.marks.iter().filter(|m| m.kind == MarkKind::Selected).count(), 1);
        assert_eq!(watermark("", 3).unwrap().summary, "Todas as 3 páginas receberão a marca d’água.");
    }

    #[test]
    fn rotate_marks_selected_pages() {
        let plan = rotate("2, 4-5", 90, 6).unwrap();
        let rotated: Vec<u32> = (1..=6).filter(|&p| plan.marks[p as usize - 1].rotation == 90).collect();
        assert_eq!(rotated, vec![2, 4, 5]);
        assert_eq!(plan.first_marked(), Some(2));
        assert!(plan.summary.starts_with("3 de 6 páginas serão giradas 90° no sentido horário"));
        let all = rotate("  ", 270, 3).unwrap();
        assert!(all.marks.iter().all(|m| m.rotation == 270 && m.badge.as_deref() == Some("↺ 90°")));
        assert_eq!(all.summary, "Todas as 3 páginas serão giradas 90° no sentido anti-horário.");
        assert_eq!(rotate("", 180, 1).unwrap().summary, "A única página será girada 180°.");
        assert!(rotate("9", 90, 3).is_err());
    }

    #[test]
    fn extract_shows_order_only_when_changed() {
        let plan = extract("1-3", false, 5).unwrap();
        assert!(plan.marks[..3].iter().all(|m| m.kind == MarkKind::Selected && m.badge.is_none()));
        assert_eq!(plan.summary, "3 páginas serão copiadas para um novo PDF.");
        let plan = extract("3, 1, 3", false, 5).unwrap();
        assert_eq!(plan.marks[2].badge.as_deref(), Some("1º, 3º"));
        assert_eq!(plan.marks[0].badge.as_deref(), Some("2º"));
        assert_eq!(extract("2", true, 5).unwrap().summary, "Será criado 1 PDF com a página escolhida.");
        assert_eq!(extract("2-4", true, 5).unwrap().summary, "Serão criados 3 PDFs, um por página.");
    }

    #[test]
    fn remove_refuses_everything() {
        let plan = remove("2", 3).unwrap();
        assert_eq!(plan.marks[1].kind, MarkKind::Removed);
        assert_eq!(plan.summary, "1 página será removida; o novo PDF terá 2 páginas.");
        assert!(remove("1-3", 3).is_err());
    }

    #[test]
    fn split_colors_groups_and_flags_excluded() {
        let plan = split(&SplitMode::Ranges("1-2, 4".into()), 5).unwrap();
        assert_eq!(plan.marks[0].kind, MarkKind::Group(0));
        assert_eq!(plan.marks[3].kind, MarkKind::Group(1));
        assert_eq!(plan.marks[2].kind, MarkKind::Excluded);
        assert_eq!(plan.summary, "Serão criados 2 arquivos. 2 páginas não estão em nenhum intervalo e ficam de fora.");
        let plan = split(&SplitMode::Every(2), 5).unwrap();
        assert_eq!(plan.marks[4].kind, MarkKind::Group(2));
        let plan = split(&SplitMode::Ranges("1-3, 2".into()), 3).unwrap();
        assert_eq!(plan.marks[1].badge.as_deref(), Some("Arq. 1, 2"));
    }

    #[test]
    fn newly_marked_ignores_pages_already_marked() {
        let before = rotate("2, 5", 90, 9).unwrap().marked();
        assert_eq!(rotate("2, 5, 8", 90, 9).unwrap().newly_marked(&before), vec![8]);
        assert!(rotate("2", 90, 9).unwrap().newly_marked(&before).is_empty());
        assert_eq!(rotate("1", 90, 3).unwrap().newly_marked(&[]), vec![1]);
    }

    #[test]
    fn toggling_pages_rewrites_the_spec() {
        assert_eq!(toggle_page("", 3, 9, false), "3");
        assert_eq!(toggle_page("1-3", 2, 9, false), "1, 3");
        assert_eq!(toggle_page("1, 3", 2, 9, false), "1-3");
        assert_eq!(toggle_page("5, 1", 2, 9, true), "5, 1-2");
        assert_eq!(toggle_page("abc", 4, 9, false), "4");
    }
}
