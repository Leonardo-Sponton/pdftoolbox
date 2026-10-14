# Auditoria de licenças

Licença do PDFToolbox: **GPL-3.0-or-later**. Escolhida por ser compatível com
todas as dependências abaixo e por permitir, no futuro, ligar diretamente à
libpoppler (GPL) sem mudar a licença.

## Motores externos (executados como programas separados)

Executar um programa como processo separado, comunicando-se por argumentos e
arquivos, é considerado agregação pela FSF: as licenças dos motores não se
estendem ao PDFToolbox. Nenhum motor é embutido no binário.

| Componente | Versão verificada | Licença | Uso | Obrigações na distribuição |
|---|---|---|---|---|
| qpdf | 12.4.2 | Apache-2.0 (Artistic-2.0 opcional) | Todas as operações estruturais | Nenhuma se for dependência do pacote; se embutido (Flatpak/AppImage), incluir LICENSE e NOTICE |
| Poppler (pdftoppm, pdfinfo) | 26.08.0 | GPL-2.0-only / GPL-3.0-or-later | Renderização e miniaturas | Se embutido: oferecer o código-fonte correspondente |
| Ghostscript | 10.08.0 (opcional) | AGPL-3.0-or-later | Compressão com reamostragem | Se embutido: oferecer o código-fonte; a AGPL cobre uso em rede, irrelevante aqui (uso local). Alternativa comercial da Artifex existe |
| GTK 4 / libadwaita | 4.24.1 / 1.10.0 | LGPL-2.1-or-later | Interface (ligação dinâmica) | Ligação dinâmica: nenhuma obrigação extra; se embutido, permitir substituição da biblioteca |

**Avaliado e não adotado:** MuPDF (AGPL-3.0 ou licença comercial) — redundante
com Poppler + qpdf nesta fase e não instalado no ambiente.

**Atenção para o futuro:** se o Ghostscript for distribuído junto (Flatpak), o
pacote inteiro precisa permitir a obtenção do código-fonte do Ghostscript. O
mesmo vale para Poppler. Para um `.deb`, eles são apenas dependências
(`Depends`/`Recommends`), sem obrigação adicional.

## Recursos próprios

Os ícones em `data/resources/icons` foram desenhados para o projeto e seguem a
licença do PDFToolbox (GPL-3.0-or-later). A paleta de cores vem do MyNotes, do
mesmo autor.

## Crates Rust (compiladas no binário)

Diretas:

| Crate | Versão | Licença | Uso |
|---|---|---|---|
| gtk4 | 0.11.5 | MIT | Bindings GTK |
| libadwaita | 0.9.2 | MIT | Bindings libadwaita |
| async-channel | 2.5.0 | Apache-2.0 OR MIT | Mensagens da thread de trabalho para a interface |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | Compressão Flate no gerador de PDF |
| image | 0.25.9 | MIT OR Apache-2.0 | Decodificação de imagens (PNG, JPEG, BMP, GIF, WebP, TIFF) |

Árvore completa (105 crates, `cargo metadata`, 2026-10-14): todas permissivas —
MIT OR Apache-2.0 (56), MIT (32), variantes com Zlib/BSD/Unlicense/0BSD (16),
Apache-2.0 WITH LLVM-exception (1), e uma com componente Unicode-3.0.
Nenhuma copyleft. Todas são compatíveis com GPL-3.0-or-later; as licenças MIT
exigem manter os avisos de copyright, o que o pacote deve fazer incluindo
os arquivos de licença (o `cargo deb`/`cargo about` pode gerá-los).

Para regenerar a lista:

```sh
cargo metadata --format-version 1 | jq -r '.packages[] | "\(.name) \(.version) \(.license)"'
```
