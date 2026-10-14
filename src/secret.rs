//! Senhas em memória: nunca aparecem em `Debug` (logs, relatórios de erro) e
//! são apagadas da memória quando descartadas.

use std::fmt;

#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // SAFETY: zeros são UTF-8 válido; a string é descartada em seguida.
        unsafe { self.0.as_mut_vec().fill(0) };
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn debug_never_shows_the_password() {
        let s = Secret::new("minha senha");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(s.expose(), "minha senha");
    }
}
