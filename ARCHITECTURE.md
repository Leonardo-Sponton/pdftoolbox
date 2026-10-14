# Arquitetura

## Visão geral

```
src/
├── lib.rs            núcleo (sem GTK) — testável isoladamente
│   ├── catalog.rs    catálogo de ferramentas exibido na interface (só as reais)
│   ├── engines/      adaptadores de executáveis externos
│   │   ├── mod.rs    Tool, detecção no PATH, versão, execução com cancelamento
│   │   ├── qpdf.rs   montar páginas, girar, otimizar, verificar, contar páginas
│   │   ├── poppler.rs  renderizar página, miniaturas
│   │   └── ghostscript.rs  reamostragem de imagens (pdfwrite, -dSAFER)
│   ├── error.rs      OpError: mensagem ao usuário (pt-BR) + detalhe técnico
│   ├── jobs.rs       CancelToken, JobCtx (progresso), fila com limite de 2 tarefas
│   ├── operations/   Request → Outcome; validação e gravação dos resultados
│   ├── pages.rs      interpretação de seleções de páginas (“1-3, 5, 8-”)
│   ├── pdfwriter.rs  gerador mínimo de PDF (imagens → PDF, fixtures) e `place`,
│   │                 a regra de posicionamento usada também pela pré-visualização
│   ├── preview.rs    planos de pré-visualização: o que cada ferramenta fará com
│   │                 cada página, com as mesmas regras das operações
│   └── storage.rs    pasta temporária por tarefa, gravação atômica sem sobrescrever
├── main.rs           inicialização do GtkApplication
build.rs              compila data/resources (ícones simbólicos) em GResource
└── ui/               interface libadwaita — nenhum processamento aqui
    ├── mod.rs        janela, navegação, ações, atalhos
    ├── home.rs       tela inicial: soltar arquivos, busca, recentes, categorias
    ├── tool_page.rs  página genérica: arquivos → opções → destino → executar → resultado
    ├── organizer.rs  grade de miniaturas com arrastar/soltar
    ├── page_preview.rs  grade de páginas marcadas pelo plano; folhas de Imagens para PDF
    ├── page_thumb.rs widget de miniatura com o formato (e a rotação) da página
    ├── thumbs.rs     miniaturas e dados dos arquivos, em segundo plano, com cache em memória
    ├── theme.rs      paleta (MyNotes) gerada em CSS para libadwaita 1.5 e 1.6+
    ├── tasks.rs      lista de tarefas da sessão
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
