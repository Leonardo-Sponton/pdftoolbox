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
| Poppler (pdftoppm, pdfinfo, pdftotext) | 26.08.0 | GPL-2.0-only / GPL-3.0-or-later | Renderização, miniaturas, metadados e extração de texto | Se embutido: oferecer o código-fonte correspondente |
| Ghostscript | 10.08.0 (opcional) | AGPL-3.0-or-later | Compressão com reamostragem | Se embutido: oferecer o código-fonte; a AGPL cobre uso em rede, irrelevante aqui (uso local). Alternativa comercial da Artifex existe |
| LibreOffice | 26.8.1 (opcional) | MPL-2.0 | Office → PDF (`soffice --headless`) | Nenhuma se for dependência; se embutido, disponibilizar o código-fonte dos arquivos MPL |
| Tesseract (+ dados por/eng/osd) | 5.5.3 (opcional) | Apache-2.0 | OCR (`tesseract … tsv`, detecção de orientação) | Nenhuma se for dependência; se embutido, incluir LICENSE e NOTICE. A camada de texto gerada embute a fonte “sem desenho” do próprio Tesseract (`pdf.ttf`, 572 bytes, Apache-2.0, lida da instalação do sistema) — mesma fonte que o Tesseract embute nos PDFs que ele gera |
| util-linux (`unshare`) | 2.42.4 (opcional) | GPL-2.0-or-later | Isolar o LibreOffice da rede | Nenhuma (programa do sistema) |
| GTK 4 / libadwaita | 4.24.1 / 1.10.0 | LGPL-2.1-or-later | Interface (ligação dinâmica) | Ligação dinâmica: nenhuma obrigação extra; se embutido, permitir substituição da biblioteca |

**Avaliado e não adotado:** MuPDF (AGPL-3.0 ou licença comercial) — redundante
com Poppler + qpdf nesta fase e não instalado no ambiente.

**Atenção para o futuro:** se o Ghostscript for distribuído junto (Flatpak), o
pacote inteiro precisa permitir a obtenção do código-fonte do Ghostscript. O
mesmo vale para Poppler. Para um `.deb`, eles são apenas dependências
(`Depends`/`Recommends`), sem obrigação adicional.

## Recursos próprios

Os ícones em `data/resources/icons` foram desenhados para o projeto e seguem a
licença do PDFToolbox (GPL-3.0-or-later). As larguras de caracteres em
`src/stamp.rs` foram extraídas das fontes Nimbus Sans (URW, AGPL com exceção
de fontes); larguras de glifos são dados métricos, as mesmas da Helvetica
padrão do PDF, e nenhuma fonte é embutida. A paleta de cores vem do MyNotes, do
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
| libc | 0.2.190 | MIT OR Apache-2.0 | Encerrar grupos de processos, fuso horário local, sinais |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | Ler e montar o JSON do qpdf (metadados) |

Árvore completa (109 crates de terceiros, `cargo metadata`, 2026-10-14, v0.5.0):
todas permissivas — MIT OR Apache-2.0 e equivalentes (59), MIT (33), variantes
com Zlib/BSD/Unlicense/0BSD (15), Apache-2.0 WITH LLVM-exception (1) e uma com
componente Unicode-3.0.
Nenhuma copyleft. Todas são compatíveis com GPL-3.0-or-later; as licenças MIT
exigem manter os avisos de copyright, o que o pacote deve fazer incluindo
os arquivos de licença (o `cargo deb`/`cargo about` pode gerá-los).

Para regenerar a lista:

```sh
cargo metadata --format-version 1 | jq -r '.packages[] | "\(.name) \(.version) \(.license)"'
```
