//! Paleta de cores. O modo escuro usa a paleta do MyNotes (tema Retroma:
//! azul-marinho profundo, lilás e acento roxo); o modo claro é uma variante
//! com os mesmos tons, escurecidos para manter o contraste sobre fundo claro.
//!
//! A libadwaita 1.6+ usa variáveis CSS (`--window-bg-color`); a 1.5 (Ubuntu
//! 24.04) usa cores nomeadas (`@define-color`). Geramos a forma certa em
//! tempo de execução e recarregamos quando o modo claro/escuro muda.

pub struct Palette {
    pub fundo: &'static str,
    pub fundo_escuro: &'static str,
    pub fundo_header: &'static str,
    pub texto: &'static str,
    pub texto_forte: &'static str,
    pub fraco: &'static str,
    pub acento: &'static str,
    pub acento_hover: &'static str,
    pub acento_fg: &'static str,
    pub borda: &'static str,
    pub borda_botao: &'static str,
    pub botao_ativo: &'static str,
    pub h1: &'static str,
    pub h2: &'static str,
    pub h3: &'static str,
    pub ok: &'static str,
    pub erro: &'static str,
    pub pagina: &'static str,
    /// Cores dos arquivos gerados (dividir), na ordem roxo, ciano, laranja…
    pub grupos: [&'static str; 8],
}

/// Paleta do MyNotes (Obsidian Retroma, esquema escuro).
pub const ESCURO: Palette = Palette {
    fundo: "#010a29",
    fundo_escuro: "#000623",
    fundo_header: "#050017",
    texto: "#afa7d4",
    texto_forte: "#d4ccf5",
    fraco: "#8a83b0",
    acento: "#a67cff",
    acento_hover: "#b994ff",
    acento_fg: "#050017",
    borda: "#6452a0",
    borda_botao: "#4e5271",
    botao_ativo: "#280072",
    h1: "#e6958c",
    h2: "#daa069",
    h3: "#e0c07a",
    ok: "#8fd3a0",
    erro: "#f0857a",
    pagina: "#f4f2fb",
    grupos: ["#a67cff", "#7fd1d8", "#daa069", "#8fd3a0", "#f08fc0", "#6fa8ff", "#e0c07a", "#e6958c"],
};

/// Variante clara com os mesmos matizes (contraste ≥ 4,5:1 para texto).
pub const CLARO: Palette = Palette {
    fundo: "#fdfcff",
    fundo_escuro: "#f3f1fb",
    fundo_header: "#e9e5f7",
    texto: "#2b2652",
    texto_forte: "#17133a",
    fraco: "#5d5785",
    acento: "#6a3dd8",
    acento_hover: "#7b52e6",
    acento_fg: "#ffffff",
    borda: "#b3a8dc",
    borda_botao: "#c4bde0",
    botao_ativo: "#e2d8ff",
    h1: "#b5483d",
    h2: "#96570f",
    h3: "#8a6d12",
    ok: "#1f7a47",
    erro: "#c0392b",
    pagina: "#ffffff",
    grupos: ["#6a3dd8", "#11798a", "#a85d10", "#1f7a47", "#b0306f", "#2d62c4", "#8a6d12", "#b5483d"],
};

fn uses_variables() -> bool {
    (adw::major_version(), adw::minor_version()) >= (1, 6)
}

fn base(c: &Palette) -> String {
    let pairs = [
        ("window-bg-color", c.fundo_escuro),
        ("window-fg-color", c.texto),
        ("view-bg-color", c.fundo),
        ("view-fg-color", c.texto),
        ("headerbar-bg-color", c.fundo_header),
        ("headerbar-fg-color", c.texto_forte),
        ("headerbar-backdrop-color", c.fundo_escuro),
        ("sidebar-bg-color", c.fundo_escuro),
        ("sidebar-fg-color", c.texto),
        ("popover-bg-color", c.fundo_header),
        ("popover-fg-color", c.texto),
        ("dialog-bg-color", c.fundo),
        ("dialog-fg-color", c.texto),
        ("card-bg-color", c.fundo_header),
        ("card-fg-color", c.texto),
        ("accent-bg-color", c.acento),
        ("accent-color", c.acento),
        ("accent-fg-color", c.acento_fg),
        ("success-color", c.ok),
        ("warning-color", c.h3),
        ("error-color", c.erro),
        ("destructive-color", c.erro),
    ];
    if uses_variables() {
        let mut s = String::from(":root {\n");
        for (k, v) in pairs {
            s.push_str(&format!("  --{k}: {v};\n"));
        }
        s.push_str(&format!("  --border-color: alpha({}, 0.55);\n  --dim-opacity: 0.8;\n}}\n", c.borda));
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
.tool-card .tool-icon {{ color: {acento}; }}
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
        fundo = c.fundo,
        fundo_header = c.fundo_header,
        texto_forte = c.texto_forte,
        fraco = c.fraco,
        acento = c.acento,
        acento_hover = c.acento_hover,
        acento_fg = c.acento_fg,
        borda = c.borda,
        borda_botao = c.borda_botao,
        botao_ativo = c.botao_ativo,
        h1 = c.h1,
        h2 = c.h2,
        h3 = c.h3,
        ok = c.ok,
        erro = c.erro,
        pagina = c.pagina,
    ));
    for (i, color) in c.grupos.iter().enumerate() {
        s.push_str(&format!(
            ".pv-grupo-{i} {{ border-color: {color}; background: alpha({color}, 0.12); }}\n\
             .pv-grupo-{i} .pv-selo {{ background: {color}; color: {}; }}\n\
             .pv-grupo-{i} .pv-numero {{ color: {color}; font-weight: bold; }}\n",
            c.acento_fg
        ));
    }
    s
}

pub fn current() -> &'static Palette {
    if adw::StyleManager::default().is_dark() { &ESCURO } else { &CLARO }
}

/// Instala a paleta e a mantém sincronizada com o modo claro/escuro.
pub fn install(display: &gtk::gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css(current()));
    // Acima do tema (APPLICATION) para vencer as cores do Adwaita.
    gtk::style_context_add_provider_for_display(display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    adw::StyleManager::default().connect_dark_notify(move |_| provider.load_from_string(&css(current())));
}
