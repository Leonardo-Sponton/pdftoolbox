# Arquitetura

## Visão geral

```
src/
├── lib.rs            núcleo (sem GTK) — testável isoladamente
│   ├── catalog.rs    catálogo de ferramentas exibido na interface (só as reais)
│   ├── deskew.rs     OCR: mede a inclinação das linhas (perfil de projeção) e
│   │                 endireita a imagem; leva pontos de volta à original
│   ├── engines/      adaptadores de executáveis externos
│   │   ├── mod.rs    Tool, detecção no PATH, versão, execução com cancelamento
│   │   ├── qpdf.rs   montar páginas, girar, otimizar, verificar, contar páginas,
│   │   │             sobrepor camadas, criptografia, metadados (JSON v2)
│   │   ├── poppler.rs  renderizar página, miniaturas, tamanhos, metadados, texto
│   │   ├── ghostscript.rs  reamostragem de imagens (pdfwrite, -dSAFER)
│   │   ├── libreoffice.rs  Office → PDF (perfil descartável, sem rede via unshare)
│   │   └── tesseract.rs    OCR: palavras e linhas (TSV) e orientação da página
│   ├── annotate.rs   anotações (texto, imagem, destaque, moldura): caixas em
│   │                 pontos e a camada de cada página
│   ├── error.rs      OpError: mensagem ao usuário (pt-BR) + detalhe técnico
│   ├── jobs.rs       CancelToken, JobCtx (progresso), fila com limite de 2 tarefas
│   ├── operations/   Request → Outcome; validação e gravação dos resultados
│   ├── pages.rs      interpretação de seleções de páginas (“1-3, 5, 8-”)
│   ├── pdfwriter.rs  gerador mínimo de PDF (imagens → PDF, fixtures) e `place`,
│   │                 a regra de posicionamento usada também pela pré-visualização
│   ├── preview.rs    planos de pré-visualização: o que cada ferramenta fará com
│   │                 cada página, com as mesmas regras das operações
│   ├── secret.rs     senhas que nunca aparecem em Debug e são zeradas ao descartar
│   ├── stamp.rs      numeração e marca d’água: texto WinAnsi, larguras da
│   │                 Helvetica, posição → camada (`pdfwriter::overlay_pdf`)
│   └── storage.rs    pasta temporária por tarefa, gravação atômica sem sobrescrever
├── main.rs           inicialização do GtkApplication
build.rs              compila data/resources (ícones simbólicos) em GResource
└── ui/               interface libadwaita — nenhum processamento aqui
    ├── mod.rs        janela, navegação, ações, atalhos
    ├── home.rs       tela inicial: soltar arquivos, busca, recentes, categorias
    ├── tool_page.rs  página genérica: arquivos → opções → destino → executar → resultado
    ├── organizer.rs  grade de miniaturas com arrastar/soltar
    ├── annotator.rs  Anotar e assinar: página grande (cairo), arrastar e teclado
    ├── page_preview.rs  grade de páginas marcadas pelo plano; folhas de Imagens para PDF;
    │                 “Como vai ficar” (página real com o carimbo); prévia de texto
    ├── page_thumb.rs widget de miniatura com o formato (e a rotação) da página
    ├── thumbs.rs     miniaturas e dados dos arquivos, em segundo plano, com cache em memória
    ├── theme.rs      paleta (MyNotes) gerada em CSS para libadwaita 1.5 e 1.6+
    ├── tasks.rs      lista de tarefas da sessão
    ├── unlock.rs     PDFs com senha: diálogo e cópias desbloqueadas da sessão
    ├── dialogs.rs    dependências, sobre
    ├── settings.rs   preferências (tema, recentes, última pasta)
    └── widgets.rs    seletores de arquivo, utilitários
```

## Fluxo de uma operação

1. A página da ferramenta monta uma `operations::Request` (enum com os
   parâmetros já tipados) e um `Destination` (pasta + nome opcional).
2. `jobs::spawn` executa `operations::execute` em uma thread de trabalho. No
   máximo duas tarefas rodam ao mesmo tempo; as demais aguardam na fila.
3. A operação lê as entradas, trabalha em um `storage::JobDir`
   (`~/.cache/pdftoolbox/tarefas/<pid>-…`, permissão 0700), **verifica** o
   resultado (abre com qpdf e confere o número de páginas; para PDFs gerados
   internamente também roda `qpdf --check`) e grava com `storage::commit`.
4. Progresso e resultado voltam à interface por um `async_channel`, consumido
   em uma `glib::spawn_future_local` — a interface nunca bloqueia.

## Decisões

**Motores externos como processos, não bibliotecas.** qpdf, Poppler e
Ghostscript são invocados com `std::process::Command` e argumentos separados
(nunca shell). Vantagens: isolamento de falhas (um PDF malicioso que derrube o
motor não derruba a interface), cancelamento real via `kill`, ausência de
problemas de compatibilidade entre versões de bindings, e fronteira de licença
clara (GPL/AGPL como programas separados). Custo: ~5–20 ms por processo,
irrelevante perto do trabalho de cada operação. Todos os caminhos passados são
absolutos, então um nome iniciado por “-” nunca é lido como opção.

**qpdf para tudo que é estrutural.** Não rasteriza; preserva texto, fontes,
vetores, metadados do documento primário. Rotação de páginas após reordenação
é feita em uma segunda passagem para que as posições se refiram ao documento
final.

**Gerador de PDF próprio para imagens → PDF.** O formato de saída é simples
(uma imagem por página) e assim JPEGs entram byte a byte (`DCTDecode`), a
orientação EXIF vira a matriz de transformação (sem recompressão) e alfa vira
`SMask`. Evita adicionar img2pdf (Python) ou uma biblioteca grande. O resultado
é validado pelo `qpdf --check`.

**Poppler em vez de MuPDF.** MuPDF é AGPL e não estava instalado; Poppler já
está em praticamente toda distribuição GNOME e cobre renderização e miniaturas.

**Sem estado global mutável**, exceto: o contador de vagas da fila
(`jobs::slots`, intrínseco à função) e o `thread_local` que guarda o estado da
janela única (GtkApplication é de instância única).

**Interface sem subclasses GObject.** Cada página é uma struct Rust em `Rc`
presa ao tempo de vida do widget (`connect_destroy`), com `Weak` nos callbacks
para evitar ciclos. O organizador reaproveita as células do `GtkFlowBox` em
vez de recriá-las: `remove_all()` seguido de navegação por teclado causava
falha de segmentação dentro do GTK 4.24 (cursor inválido).

## PDFs com senha

`operations::unlocked_copy` abre o PDF com a senha (arquivo 0600, apagado em
seguida), recusa a senha de abertura quando o autor restringiu ações (como
“Remover senha”) e grava uma cópia sem criptografia com o mesmo nome, numa
pasta 0700 em `storage::unlocked_root()` (`$XDG_RUNTIME_DIR`, em memória). Na
interface, `ui/unlock.rs` troca cada PDF com senha que entra numa ferramenta
pela cópia (`prepare`), guarda as cópias da sessão (reaproveitadas enquanto o
original não muda) e informa o original (`original`) para o destino padrão e
as dicas. As operações não sabem de nada disso: recebem um PDF comum.

## Zoom

A página ampliada (`page_zoom.rs`) e o Anotar põem a página numa
`ScrolledWindow`; o zoom muda o tamanho pedido pela página (o `PageThumb` ou
a área de desenho) e, depois do layout, as barras de rolagem são acertadas
para o ponto sob o ponteiro ficar parado. Quando a ampliação para de mudar,
a página é renderizada de novo com o lado que ela ocupa na tela (até 5000
px); só a página atual fica guardada em alta resolução. No Anotar, a
`Viewport` tem `scroll-to-focus` desligado: focar a área de desenho rolaria
para o canto da página.

## Como acrescentar uma ferramenta

1. Implemente a operação em `operations/` (função pura que recebe `JobCtx`,
   usa `JobDir`, verifica e chama `storage::commit`) e acrescente uma variante
   em `Request`.
2. Escreva testes em `tests/` conferindo o **conteúdo** do resultado.
3. Registre-a em `catalog::TOOLS` (com `needs` = motores exigidos).
4. Adicione as opções em `ui/tool_page.rs` (`Options` + `build_options` +
   `request`).
5. Atualize `FEATURE_MATRIX.md` e `CHANGELOG.md`.

## Pré-visualização

A interface nunca adivinha o resultado: `preview.rs` calcula o plano com as
mesmas funções que as operações usam (`pages::parse_list`, `parse_groups`,
`complement`, `chunks`) e devolve o mesmo erro que a operação devolveria. Os
testes de `preview.rs` fixam essa equivalência. As miniaturas vêm do
`pdftoppm` (PDFs) ou do crate `image` com orientação EXIF (imagens), sempre
fora da thread da interface, e ficam só em memória (cache por caminho, data de
modificação e página; esvaziado acima de 1500 itens). Os PNG intermediários do
`pdftoppm` vivem numa `JobDir` apagada ao fim de cada lote.

### Carimbos (numeração e marca d’água)

`stamp.rs` calcula, para cada página, o texto e sua posição a partir do tamanho
**exibido** (rotação aplicada, via `pdfinfo`). `pdfwriter::overlay_pdf` grava
uma camada com uma página por página do documento, usando as fontes padrão
Helvetica (sem embutir fonte) e transparência por `ExtGState`. O
`qpdf --overlay` encaixa cada página da camada na página correspondente,
respeitando a rotação, sem rasterizar nada. A prévia “Como vai ficar” roda
exatamente o mesmo caminho para uma página e a renderiza com o `pdftoppm`.

### OCR

`operations/ocr.rs` descobre pelo `pdftotext` quais páginas não têm texto,
renderiza cada uma em cinza (PGM, sem compressão — PNG custava 3,6 s por
página digitalizada com ruído) e pede ao Tesseract só as palavras com suas
caixas (TSV), até 4 páginas em paralelo, uma thread por processo. A camada de
texto invisível é gerada aqui (`pdfwriter::invisible_text_pdf`), não pelo
renderizador de PDF do Tesseract: ele inclina cada linha junto com a
digitalização, e o Poppler quebra linhas inclinadas (a 1,2° “declaro” virava
“decla” + “ro” e a busca falhava). Cada linha é escrita na horizontal, numa
linha de base só, com as palavras esticadas (`Tz`) sobre a posição delas na
imagem; a fonte é a `pdf.ttf` do Tesseract (sem desenho), com CID = UTF-16 e
ToUnicode identidade. O qpdf sobrepõe a camada só às páginas reconhecidas.

Inclinação: antes de ler, `deskew::analyze` projeta os pixels escuros em
ângulos de −15° a 15° e fica com o de faixas mais nítidas (as linhas de
texto). Se passa de 0,3°, a leitura é feita numa cópia endireitada (tela
ampliada, fundo branco) e o centro de cada palavra volta ao sistema da página
por `Rotation::to_original`. A camada continua horizontal por linha: texto
girado ou palavras em degraus fazem o Poppler embaralhar a ordem (testado),
então a linha usa a linha de base do meio dela. A mesma medição feita na
imagem girada 90° revela páginas deitadas (as faixas ficam nítidas só nela)
antes de qualquer leitura.

Páginas deitadas ou de cabeça para baixo: se a primeira leitura tem linhas na
vertical ou confiança baixa, a detecção de orientação (`--psm 0`) decide quanto
girar (com pouco texto, a direção das linhas ou a tentativa de 180°); a imagem
é endireitada, lida de novo e a melhor leitura vence. A camada é escrita no
sistema endireitado com uma matriz de volta ao da página e, com a opção
padrão, a página é girada sem perda (`/Rotate`).

### Anotações

`annotate.rs` guarda cada anotação com o canto superior esquerdo em fração da
página exibida e o tamanho em pontos (texto: calculado pelas larguras da
Helvetica). A camada ganhou formas (`ShapeMark`, com `/BM /Multiply` no
destaque) e imagens (`ImageMark`): cada imagem é gravada uma vez e citada por
todas as páginas; com “remover fundo branco”, ela é decodificada e o papel
claro vira canal alfa (`SMask`). Na interface, a prévia é desenhada com cairo
sobre a página renderizada, com a mesma fonte e as mesmas caixas, para
acompanhar o arrastar sem esperar o qpdf.

### Senhas e metadados

As senhas chegam à operação como `Secret` e vão ao qpdf por arquivos 0600 na
pasta da tarefa (`--password-file`, `@arquivo` com as opções de `--encrypt`):
nunca aparecem na lista de processos. “Remover senha” exige a senha e só
remove restrições do autor com a senha de permissões, conferida por
`qpdf --show-encryption`.

Os metadados são gravados com `qpdf --update-from-json`: um dicionário de
informações **novo** (cópia do atual com as alterações) e o trailer apontando
para ele. Editar o objeto existente não funciona quando ele está dentro de um
fluxo de objetos — o qpdf ignora a atualização sem erro —, e o trailer precisa
manter `/Size`, senão a tabela de referências sai inválida.

### Processos externos

Cada motor roda no próprio grupo de processos. Cancelar envia `SIGKILL` ao
grupo, o que alcança os filhos (o LibreOffice roda como `oosplash` →
`soffice.bin`). Os grupos ativos ficam registrados e são encerrados quando o
aplicativo sai, inclusive por SIGTERM/SIGINT.
