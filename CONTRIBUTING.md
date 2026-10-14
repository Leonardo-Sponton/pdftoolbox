# Contribuindo

## Ambiente

- Rust estável ≥ 1.85 (edição 2024). Testado com 1.99.
- GTK ≥ 4.12 e libadwaita ≥ 1.5 com cabeçalhos.
- qpdf, Poppler (pdftoppm, pdfinfo, pdftotext) e, para testar a compressão
  com perda, Ghostscript. Os testes de OCR precisam do Tesseract com `por` e
  `eng`; sem eles, são ignorados (com aviso).

```sh
# Debian/Ubuntu
sudo apt install cargo libgtk-4-dev libadwaita-1-dev qpdf poppler-utils ghostscript tesseract-ocr tesseract-ocr-por
# Arch
sudo pacman -S rust gtk4 libadwaita qpdf poppler ghostscript tesseract tesseract-data-por tesseract-data-eng
```

## Antes de enviar uma mudança

```sh
cargo fmt
cargo clippy --all-targets      # deve terminar sem avisos
cargo test                      # testes de unidade e integração
cargo build --release
```

Os testes geram suas próprias fixtures (PDFs de texto e imagens sintéticas) em
uma pasta temporária com espaço e Unicode no nome; nunca use documentos
pessoais. Os testes conferem o **conteúdo** (texto de cada página via
`pdftotext`, rotação e tamanho via `pdfinfo`, pixels renderizados), não apenas
a ausência de erro.

## Convenções de código

- `rustfmt.toml` define linhas de até 120 colunas; `clippy` sem avisos.
- Nada de `unwrap()`/`expect()` para contornar erros: as operações devolvem
  `OpResult`, e a interface mostra o erro. Os poucos `expect` que restam
  afirmam invariantes (catálogo completo, recursos embutidos, pipes pedidos).
- Textos para o usuário são escritos em português dentro de `tr!` (ou
  `trn!` quando dependem de quantidade; `n_` em tabelas constantes) e
  traduzidos em `src/i18n/en.rs`. Use marcadores nomeados
  (`tr!("Lendo {file}", file = nome)`) e frases inteiras — nada de montar
  frase com pedaços traduzidos separadamente nem de `if idioma == …`.
  `cargo test i18n` aponta mensagens sem tradução, traduções sem uso e
  marcadores divergentes. Nomes de arquivo, tamanhos e números com casas
  decimais passam por `pdftoolbox::human`. Não traduza nomes de arquivos
  gerados, o que é gravado no PDF nem detalhes técnicos/log.
- Para ver a interface em inglês sem mudar o sistema: escolha
  Menu → Idioma · Language → English, ou rode com `LANG=en_US.UTF-8`.
- Na interface, sinais usam `glib::clone!(#[weak] …)`; código que espera
  (`async`) guarda um `Weak` e só o reabre depois do `await`.
- Lógica sem GTK (textos, escolhas, cálculos) fica em funções puras, com
  testes de unidade no mesmo arquivo.

## Ferramentas de desenvolvimento

- `cargo run --example amostras -- <pasta>` gera PDFs de exemplo.
- `cargo run --release --example bench -- <pasta>` roda o benchmark do README.
- `PDFTOOLBOX_FERRAMENTA=<chave> pdftoolbox arquivo.pdf` abre uma ferramenta
  diretamente (chaves em `src/catalog.rs`, ex.: `mesclar`, `organizar`).
- `GTK_DEBUG=keybindings` mostra como os atalhos são tratados.

## Distribuição

- **Instalação por usuário:** `./install.sh` (ver README).
- **.deb:** `Cargo.toml` contém `[package.metadata.deb]` para o
  [cargo-deb](https://github.com/kornelski/cargo-deb):
  `cargo install cargo-deb && cargo deb` → `target/debian/*.deb`. Gere o
  pacote na distribuição mais antiga que pretende suportar (a glibc exigida é
  a da máquina que compilou). Gerado no Arch, exige glibc ≥ 2.39. Ainda não
  testado em instalação real no Debian/Ubuntu.
- **Flatpak / AppImage:** ainda não avaliados (exigem embutir qpdf e Poppler e
  cumprir as obrigações descritas em LICENSES.md).
