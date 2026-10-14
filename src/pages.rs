//! Interpretação de seleções de páginas digitadas pelo usuário, como
//! `1-3, 5, 8-` (até o fim) ou `-4` (do início até a 4).

use crate::error::{OpError, OpResult};

/// Interpreta cada item separado por vírgula (ou ponto e vírgula) como um grupo.
/// Intervalos decrescentes (`5-3`) produzem páginas em ordem inversa.
pub fn parse_groups(spec: &str, total: u32) -> OpResult<Vec<Vec<u32>>> {
    let mut groups = Vec::new();
    for raw in spec.split([',', ';']) {
        let item: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        if item.is_empty() {
            continue;
        }
        let (start, end) = match item.split_once('-') {
            None => {
                let p = parse_page(&item, &item)?;
                (p, p)
            }
            Some((a, b)) => {
                let a = if a.is_empty() { 1 } else { parse_page(a, &item)? };
                let b = if b.is_empty() { total } else { parse_page(b, &item)? };
                (a, b)
            }
        };
        for p in [start, end] {
            if p > total {
                return Err(OpError::PageOutOfRange { page: p, total });
            }
        }
        let group: Vec<u32> = if start <= end { (start..=end).collect() } else { (end..=start).rev().collect() };
        groups.push(group);
    }
    if groups.is_empty() {
        return Err(OpError::EmptySelection);
    }
    Ok(groups)
}

/// Lista de páginas na ordem digitada.
pub fn parse_list(spec: &str, total: u32) -> OpResult<Vec<u32>> {
    Ok(parse_groups(spec, total)?.concat())
}

fn parse_page(s: &str, item: &str) -> OpResult<u32> {
    match s.parse::<u32>() {
        Ok(0) => Err(OpError::InvalidRange(tr!("A numeração de páginas começa em 1.").into())),
        Ok(n) => Ok(n),
        Err(_) => Err(OpError::InvalidRange(tr!(
            "“{item}” não é uma seleção de páginas válida. Use, por exemplo: 1-3, 5, 8-",
            item
        ))),
    }
}

/// Páginas de 1..=total que não estão em `selected`, em ordem.
pub fn complement(selected: &[u32], total: u32) -> Vec<u32> {
    (1..=total).filter(|p| !selected.contains(p)).collect()
}

/// Divide 1..=total em blocos de `size` páginas.
pub fn chunks(total: u32, size: u32) -> Vec<Vec<u32>> {
    let size = size.max(1);
    (1..=total).collect::<Vec<_>>().chunks(size as usize).map(<[u32]>::to_vec).collect()
}

/// Forma compacta para o qpdf, preservando a ordem: `[1,2,3,5,4]` → `1-3,5,4`.
pub fn compact_list(pages: &[u32]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            end += 1;
            i += 1;
        }
        parts.push(if start == end { start.to_string() } else { format!("{start}-{end}") });
        i += 1;
    }
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ranges_and_open_ends() {
        assert_eq!(parse_list("1-3, 5, 8-", 9).unwrap(), vec![1, 2, 3, 5, 8, 9]);
        assert_eq!(parse_list("-2", 9).unwrap(), vec![1, 2]);
        assert_eq!(parse_list("4-2", 9).unwrap(), vec![4, 3, 2]);
        assert_eq!(parse_groups("1-2;3", 3).unwrap(), vec![vec![1, 2], vec![3]]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(parse_list("0", 5), Err(OpError::InvalidRange(_))));
        assert!(matches!(parse_list("abc", 5), Err(OpError::InvalidRange(_))));
        assert!(matches!(parse_list("1-x", 5), Err(OpError::InvalidRange(_))));
        assert!(matches!(parse_list("2-7", 5), Err(OpError::PageOutOfRange { page: 7, total: 5 })));
        assert!(matches!(parse_list(" , ", 5), Err(OpError::EmptySelection)));
    }

    #[test]
    fn compacts_preserving_order() {
        assert_eq!(compact_list(&[1, 2, 3, 5, 4]), "1-3,5,4");
        assert_eq!(compact_list(&[7]), "7");
        assert_eq!(complement(&[2, 4], 5), vec![1, 3, 5]);
        assert_eq!(chunks(5, 2), vec![vec![1, 2], vec![3, 4], vec![5]]);
    }
}
