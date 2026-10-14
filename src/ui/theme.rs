//! Paleta de cores. O modo escuro usa a paleta do MyNotes (tema Retroma:
//! azul-marinho profundo, lilás e acento roxo); o modo claro é uma variante
//! com os mesmos tons, escurecidos para manter o contraste sobre fundo claro.
//!
//! A libadwaita 1.6+ usa variáveis CSS (`--window-bg-color`); a 1.5 (Ubuntu
//! 24.04) usa cores nomeadas (`@define-color`). Geramos a forma certa em
//! tempo de execução e recarregamos quando o modo claro/escuro muda.

pub struct Palette {
    pub background: &'static str,
    pub background_dark: &'static str,
    pub background_header: &'static str,
    pub text: &'static str,
    pub text_strong: &'static str,
    pub dim: &'static str,
    pub accent: &'static str,
    pub accent_hover: &'static str,
    pub accent_fg: &'static str,
    pub border: &'static str,
    pub button_border: &'static str,
    pub button_checked: &'static str,
    pub h1: &'static str,
    pub h2: &'static str,
    pub h3: &'static str,
    pub ok: &'static str,
    pub error: &'static str,
    pub page: &'static str,
    /// Cores dos arquivos gerados (dividir), na ordem roxo, ciano, laranja…
    pub groups: [&'static str; 8],
}

/// Paleta do MyNotes (Obsidian Retroma, esquema escuro).
pub const DARK: Palette = Palette {
    background: "#010a29",
    background_dark: "#000623",
    background_header: "#050017",
    text: "#afa7d4",
    text_strong: "#d4ccf5",
    dim: "#8a83b0",
    accent: "#a67cff",
    accent_hover: "#b994ff",
    accent_fg: "#050017",
    border: "#6452a0",
    button_border: "#4e5271",
    button_checked: "#280072",
    h1: "#e6958c",
    h2: "#daa069",
    h3: "#e0c07a",
    ok: "#8fd3a0",
    error: "#f0857a",
    page: "#f4f2fb",
    groups: ["#a67cff", "#7fd1d8", "#daa069", "#8fd3a0", "#f08fc0", "#6fa8ff", "#e0c07a", "#e6958c"],
};

/// Variante clara com os mesmos matizes (contraste ≥ 4,5:1 para texto).
pub const LIGHT: Palette = Palette {
    background: "#fdfcff",
    background_dark: "#f3f1fb",
    background_header: "#e9e5f7",
    text: "#2b2652",
    text_strong: "#17133a",
    dim: "#5d5785",
    accent: "#6a3dd8",
    accent_hover: "#7b52e6",
    accent_fg: "#ffffff",
    border: "#b3a8dc",
    button_border: "#c4bde0",
    button_checked: "#e2d8ff",
    h1: "#b5483d",
    h2: "#96570f",
    h3: "#8a6d12",
    ok: "#1f7a47",
    error: "#c0392b",
    page: "#ffffff",
    groups: ["#6a3dd8", "#11798a", "#a85d10", "#1f7a47", "#b0306f", "#2d62c4", "#8a6d12", "#b5483d"],
};

fn uses_variables() -> bool {
    (adw::major_version(), adw::minor_version()) >= (1, 6)
}

fn base(c: &Palette) -> String {
    let pairs = [
        ("window-bg-color", c.background_dark),
        ("window-fg-color", c.text),
        ("view-bg-color", c.background),
        ("view-fg-color", c.text),
        ("headerbar-bg-color", c.background_header),
        ("headerbar-fg-color", c.text_strong),
        ("headerbar-backdrop-color", c.background_dark),
        ("sidebar-bg-color", c.background_dark),
        ("sidebar-fg-color", c.text),
        ("popover-bg-color", c.background_header),
        ("popover-fg-color", c.text),
        ("dialog-bg-color", c.background),
        ("dialog-fg-color", c.text),
        ("card-bg-color", c.background_header),
        ("card-fg-color", c.text),
        ("accent-bg-color", c.accent),
        ("accent-color", c.accent),
        ("accent-fg-color", c.accent_fg),
        ("success-color", c.ok),
        ("warning-color", c.h3),
        ("error-color", c.error),
        ("destructive-color", c.error),
    ];
    if uses_variables() {
        let mut s = String::from(":root {\n");
        for (k, v) in pairs {
            s.push_str(&format!("  --{k}: {v};\n"));
        }
        s.push_str(&format!("  --border-color: alpha({}, 0.55);\n  --dim-opacity: 0.8;\n}}\n", c.border));
        s
    } else {
        pairs.iter().map(|(k, v)| format!("@define-color {} {v};\n", k.replace('-', "_"))).collect()
    }
}

/// CSS completo para a paleta `c`.
pub fn css(c: &Palette) -> String {
    let mut s = base(c);
    s.push_str(&format!(
        r#"
/* botões com contorno, como no MyNotes */
button {{ border-radius: 10px; }}
splitbutton.suggested-action > button, splitbutton.suggested-action > menubutton > button {{
  background: {acento}; color: {acento_fg}; font-weight: bold; }}
splitbutton.suggested-action > button:hover, splitbutton.suggested-action > menubutton > button:hover {{
  background: {acento_hover}; }}
:not(splitbutton) > button:not(.flat):not(.toggle):not(.suggested-action):not(.destructive-action):not(.card):not(.image-button):not(.pill) {{
  border: 1px solid alpha({borda_botao}, 0.9); background: {fundo_header}; box-shadow: none; }}
:not(splitbutton) > button:not(.flat):not(.toggle):not(.suggested-action):not(.destructive-action):not(.card):hover {{
  background: alpha({acento}, 0.12); border-color: alpha({acento}, 0.7); }}
button.flat:hover {{ background: alpha({acento}, 0.15); }}
button:checked {{ background: {botao_ativo}; color: {texto_forte}; box-shadow: inset 0 0 0 1px alpha({acento}, 0.8); }}
button.suggested-action {{ background: {acento}; color: {acento_fg}; font-weight: bold; }}
button.suggested-action:hover {{ background: {acento_hover}; }}
button.pill:not(.suggested-action) {{ border: 1px solid alpha({borda_botao}, 0.9); }}
headerbar {{ border-bottom: 1px solid alpha({borda}, 0.35); box-shadow: none; }}
entry:focus-within {{ outline-color: alpha({acento}, 0.7); }}

/* cartões e listas */
.card, list.boxed-list {{ background: {fundo_header}; border: 1px solid alpha({borda}, 0.45);
  box-shadow: none; border-radius: 12px; }}
list.boxed-list > row {{ background: none; }}
list.boxed-list > row:not(:last-child) {{ border-bottom: 1px solid alpha({borda}, 0.25); }}
.tool-card {{ padding: 14px; }}
button.card.tool-card:hover {{ background: alpha({acento}, 0.10); border-color: alpha({acento}, 0.7); }}
.tool-card .tool-icon, .icone-documento {{ color: {acento}; }}
.thumb-rotated {{ color: {acento}; }}
flowboxchild:selected .thumb {{ background: alpha({acento}, 0.22); box-shadow: inset 0 0 0 2px alpha({acento}, 0.8); }}
.tool-card.sugerida {{ border-color: alpha({acento}, 0.9); box-shadow: inset 0 0 0 1px alpha({acento}, 0.5); }}
.category-title {{ color: {h2}; }}
.titulo-pagina {{ color: {h1}; }}
.dim-label, .dica {{ color: {fraco}; opacity: 1; }}

/* área de soltar: folha com grade de pontos */
.drop-zone {{ border: 2px dashed alpha({borda}, 0.75); border-radius: 14px; background-color: {fundo};
  background-image: radial-gradient(circle closest-side, alpha({borda}, 0.28) 0, alpha({borda}, 0.28) 1px, transparent 1.6px);
  background-size: 24px 24px; }}
.drop-zone.drop-active, .drop-zone:drop(active) {{ border-color: {acento}; background-color: alpha({acento}, 0.12); }}

/* etiquetas */
.badge {{ border-radius: 99px; padding: 1px 9px; font-weight: bold; font-size: 0.8em; }}
.badge.acento {{ background: alpha({acento}, 0.16); color: {acento}; border: 1px solid alpha({acento}, 0.5); }}
.badge.ok {{ background: alpha({ok}, 0.14); color: {ok}; border: 1px solid alpha({ok}, 0.5); }}
.badge.aviso {{ background: alpha({h3}, 0.14); color: {h3}; border: 1px solid alpha({h3}, 0.5); }}
.badge.erro {{ background: alpha({erro}, 0.14); color: {erro}; border: 1px solid alpha({erro}, 0.5); }}
.texto-erro {{ color: {erro}; }}
.texto-ok {{ color: {ok}; }}
.texto-aviso {{ color: {h3}; }}
.texto-acento {{ color: {acento}; }}

/* miniaturas: a página aparece como papel sobre o fundo */
.miniatura {{ background: {pagina}; border-radius: 3px; box-shadow: 0 1px 3px alpha(black, 0.35); }}
.miniatura-vazia {{ background: alpha({borda}, 0.18); border-radius: 3px; }}
.miniatura-bloqueada {{ background-image: -gtk-icontheme("pdftoolbox-proteger-symbolic"); background-repeat: no-repeat;
  background-position: center; background-size: 45%; color: {acento}; -gtk-icon-palette: success {acento}; }}
.arquivo-card {{ padding: 10px; border-radius: 12px; background: alpha({fundo_header}, 0.85);
  border: 1px solid alpha({borda}, 0.5); }}
.arquivo-card.problema {{ border-color: alpha({erro}, 0.7); }}
.arquivo-card .nome {{ color: {texto_forte}; font-weight: bold; }}
button.tirar-arquivo {{ min-width: 26px; min-height: 26px; padding: 0; margin: 2px;
  background: {fundo_header}; color: {texto_forte}; border: 1px solid alpha({borda}, 0.8); }}
button.tirar-arquivo:hover {{ color: {erro}; border-color: {erro}; background: {fundo_header}; }}

/* pré-visualização das páginas */
.pv-celula {{ padding: 6px; border-radius: 10px; border: 2px solid transparent; }}
.pv-celula .pv-numero {{ color: {fraco}; font-size: 0.85em; }}
.pv-selecionada {{ border-color: {acento}; background: alpha({acento}, 0.12); }}
.pv-selecionada .pv-numero {{ color: {acento}; font-weight: bold; }}
.pv-removida {{ border-color: alpha({erro}, 0.8); background: alpha({erro}, 0.08); }}
.pv-removida .miniatura {{ opacity: 0.3; }}
.pv-removida .pv-numero {{ color: {erro}; text-decoration: line-through; }}
.pv-fora .miniatura {{ opacity: 0.3; }}
.pv-esmaecida .miniatura {{ opacity: 0.45; }}
.pv-selo {{ border-radius: 99px; padding: 0 7px; font-size: 0.75em; font-weight: bold;
  background: {acento}; color: {acento_fg}; margin: 4px; }}
.pv-removida .pv-selo {{ background: {erro}; color: {acento_fg}; }}
.pv-fora .pv-selo {{ background: alpha({fraco}, 0.9); color: {acento_fg}; }}
.pv-resumo {{ color: {texto_forte}; }}
.pv-resumo.texto-erro {{ color: {erro}; }}
.pv-legenda {{ color: {fraco}; font-size: 0.85em; }}
.folha-previa {{ background: {pagina}; border-radius: 2px; box-shadow: 0 1px 4px alpha(black, 0.4); }}
.margem-previa {{ border: 1px dashed alpha({acento}, 0.6); }}
"#,
        fundo = c.background,
        fundo_header = c.background_header,
        texto_forte = c.text_strong,
        fraco = c.dim,
        acento = c.accent,
        acento_hover = c.accent_hover,
        acento_fg = c.accent_fg,
        borda = c.border,
        borda_botao = c.button_border,
        botao_ativo = c.button_checked,
        h1 = c.h1,
        h2 = c.h2,
        h3 = c.h3,
        ok = c.ok,
        erro = c.error,
        pagina = c.page,
    ));
    for (i, color) in c.groups.iter().enumerate() {
        s.push_str(&format!(
            ".pv-grupo-{i} {{ border-color: {color}; background: alpha({color}, 0.12); }}\n\
             .pv-grupo-{i} .pv-selo {{ background: {color}; color: {}; }}\n\
             .pv-grupo-{i} .pv-numero {{ color: {color}; font-weight: bold; }}\n",
            c.accent_fg
        ));
    }
    s
}

pub fn current() -> &'static Palette {
    if adw::StyleManager::default().is_dark() { &DARK } else { &LIGHT }
}

/// Instala a paleta e a mantém sincronizada com o modo claro/escuro.
pub fn install(display: &gtk::gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css(current()));
    // Acima do tema (APPLICATION) para vencer as cores do Adwaita.
    gtk::style_context_add_provider_for_display(display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    adw::StyleManager::default().connect_dark_notify(move |_| provider.load_from_string(&css(current())));
}
