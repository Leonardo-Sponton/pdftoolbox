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
│   ├── error.rs      OpError: mensagem ao usuário (montada no idioma atual ao
│   │                 ser exibida) + detalhe técnico
│   ├── human.rs      textos para pessoas: nome de arquivo, tamanho, separador
│   │                 decimal do idioma
│   ├── i18n/         idioma da interface (português e inglês)
│   │   ├── mod.rs    idioma atual, preferência, `tr!`/`trn!`/`n_`
│   │   ├── locale.rs idioma do sistema (LANGUAGE, LC_ALL, LC_MESSAGES, LANG,
│   │   │             /etc/locale.conf)
│   │   ├── en.rs     catálogo inglês (texto em português → tradução)
│   │   └── tests.rs  integridade do catálogo contra o código-fonte
│   ├── jobs.rs       CancelToken, JobCtx (progresso), fila com limite de 2 tarefas
│   ├── operations/   Request → Outcome; validação e gravação dos resultados.
│   │                 mod.rs tem o que as operações compartilham: lote, nomes
│   │                 de saída, verificação e `parallel_map` (páginas em paralelo)
│   ├── pages.rs      interpretação de seleções de páginas (“1-3, 5, 8-”)
│   ├── pdfwriter/    gerador mínimo de PDF
│   │   ├── mod.rs    escrita de objetos, páginas e tabela; fixtures de texto
│   │   ├── images.rs imagens → PDF e `place`, a regra de posicionamento usada
│   │   │             também pela pré-visualização
│   │   ├── overlay.rs  camadas sobrepostas (carimbos, anotações, assinaturas)
│   │   └── invisible.rs  camada de texto invisível do OCR
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
    ├── tool_page/    página genérica: arquivos → opções → destino → executar → resultado
    │   ├── mod.rs    estrutura da página, lista de arquivos, pré-visualizações
    │   ├── options.rs  linhas de opções de cada ferramenta
    │   ├── form.rs   leitura do formulário: Request, plano, validação, rótulos
    │   ├── background.rs  campos em segundo plano (idiomas do OCR, metadados,
    │   │             estimativa de compressão)
    │   ├── run.rs    execução, progresso e resultado
    │   └── texts.rs  textos que dependem só de dados (com testes)
    ├── editor.rs     partes comuns de Organizar e Anotar: cabeçalho, telas de
    │                 escolha/carregamento/erro, descarte, desfazer, gravação
    ├── organizer.rs  grade de miniaturas com arrastar/soltar
    ├── annotator/    Anotar e assinar
    │   ├── mod.rs    estado, documento, anotações e gravação
    │   ├── canvas.rs página grande (cairo): desenho, mouse, teclado, zoom
    │   └── panel.rs  painel lateral: adicionar, propriedades, lista
    ├── page_preview.rs  grade de páginas marcadas pelo plano; folhas de Imagens para PDF;
    │                 “Como vai ficar” (página real com o carimbo); prévia de texto
    ├── page_zoom.rs  página ampliada; gestos e controles de zoom compartilhados
    ├── page_thumb.rs widget de miniatura com o formato (e a rotação) da página
    ├── thumbs.rs     miniaturas e dados dos arquivos, em segundo plano, com cache em memória
    ├── theme.rs      paleta (MyNotes) gerada em CSS para libadwaita 1.5 e 1.6+
    ├── tasks.rs      lista de tarefas da sessão
    ├── unlock.rs     PDFs com senha: diálogo e cópias desbloqueadas da sessão
    ├── dialogs.rs    dependências, sobre
    ├── settings.rs   preferências (tema, idioma, recentes, última pasta,
    │                 idiomas do OCR); linhas desconhecidas são preservadas
    └── widgets.rs    seletores de arquivo, atalhos, utilitários
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
(`jobs::slots`, intrínseco à função), o `thread_local` que guarda o estado da
janela única (GtkApplication é de instância única) e o idioma atual
(`i18n::CURRENT`, um atômico lido também pelas threads das tarefas).

**Idioma da interface: catálogo próprio, com o português como chave.** As
mensagens continuam escritas em português no código, dentro de `tr!`,
`trn!` (singular/plural) ou `n_` (tabelas constantes), e esse texto é a chave
do catálogo inglês `i18n/en.rs`, como no gettext — mas sem `.po`/`.mo`, sem
`setlocale` e sem dependência nova: o catálogo vai embutido no binário e o
idioma pode mudar com o programa aberto. Os marcadores são nomeados
(`{file}`), para a tradução poder mudar a ordem das palavras; frases com
plural ou gênero diferente são mensagens inteiras separadas, nunca pedaços
concatenados. Mensagem sem tradução aparece em português. Os testes de
`i18n/tests.rs` leem o código-fonte e conferem: toda mensagem marcada tem
tradução, toda tradução é usada, não há chaves repetidas, os marcadores são
os mesmos dos dois lados e não sobrou texto acentuado sem marcação.

O idioma vem da preferência gravada (`idioma=pt|en`; sem ela, “seguir o
sistema”). O do sistema segue as regras do gettext — `LANGUAGE` (lista,
ignorada com locale C), depois `LC_ALL`, `LC_MESSAGES`, `LANG` e, sem nenhuma
delas, `/etc/locale.conf` —; idioma não suportado cai no português. Quando o
idioma escolhido difere do sistema, `main` ajusta `LANGUAGE` antes de o GTK
iniciar, para os textos do próprio GTK/libadwaita acompanharem; os
programas externos recebem o valor original (`engines::run_command`). Trocar
o idioma refaz o menu, a tela inicial (mantendo os arquivos escolhidos) e a
lista de tarefas; páginas de ferramentas já abertas ficam como estão até
serem reabertas, para não perder opções nem alterações. O idioma não muda
nenhum arquivo: nomes gerados e o texto gravado pela numeração continuam em
português (`tests/i18n.rs` compara as saídas nos dois idiomas).

**Interface sem subclasses GObject.** Cada página é uma struct Rust em `Rc`
presa ao tempo de vida do widget (`widgets::keep_alive`), com referências
fracas nos callbacks para evitar ciclos (`glib::clone!` com `#[weak]` nos
sinais; `Rc::downgrade` explícito nas tarefas assíncronas, que só reabrem a
referência depois do `await`). A página também nunca guarda uma referência
forte ao próprio `NavigationPage`. O organizador reaproveita as células do `GtkFlowBox` em
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
4. Adicione as opções em `ui/tool_page/`: a variante de `Options` e as
   linhas em `options.rs` (`Options::build`, `connect_changes`, `entries`) e
   a leitura em `form.rs` (`request`, `single_output`, `action_label` e, se
   houver pré-visualização, `plan`).
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
