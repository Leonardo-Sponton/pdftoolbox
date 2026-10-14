# Changelog

## 0.2.0 — 2026-10-14

Informação visual para decidir antes de executar (heurísticas de Nielsen:
visibilidade do estado, prevenção de erros, reconhecimento em vez de memória).

### Adicionado
- **Tela inicial:** os arquivos escolhidos aparecem como cartões com a
  primeira página (ou a imagem), nome, páginas/dimensões e tamanho; cada um pode
  ser tirado da lista. Arquivos com problema (senha, inválido) ficam marcados
  em vermelho. Novos arquivos se somam aos já escolhidos.
- **Sugestão de ferramenta:** 1 PDF → Organizar; vários PDFs → Mesclar;
  imagens → Imagens para PDF. A sugestão aparece como botão ao lado dos
  arquivos e como selo “Sugerida” no cartão. Ferramentas que não aceitam os
  arquivos dizem por quê (“Não aceita imagens — só PDFs”).
- **Pré-visualização ao vivo** em Girar, Extrair, Remover e Dividir: grade de
  miniaturas que mostra as páginas giradas de verdade, as extraídas (com a
  ordem quando difere da original), as removidas (riscadas) e, ao dividir, a
  cor de cada arquivo gerado e as páginas que ficam de fora. Um resumo diz o que
  vai acontecer (“3 de 308 páginas serão giradas…”). Clicar numa página a marca
  ou desmarca no campo.
- **Validação enquanto digita:** seleção inválida deixa o campo vermelho, mostra
  o motivo e desativa o botão. O botão diz a ação exata (“Remover 4 páginas”,
  “Dividir em 3 arquivos”).
- **Imagens para PDF:** folhas de exemplo mostram onde cada imagem ficará, com
  a mesma regra de posicionamento da conversão (página deitada para imagens
  largas, margem).
- **PDF para imagens:** informa o tamanho em pixels da primeira página na
  resolução escolhida. **Mesclar:** informa o total de páginas do resultado.
- Miniaturas em todas as listas de arquivos; número da posição em Mesclar e
  Imagens para PDF. Após executar, a página rola até o resultado.
- **Paleta do MyNotes** (azul-marinho, lilás e acento roxo) no modo escuro e
  uma variante clara com os mesmos tons. Funciona com libadwaita 1.5 (cores
  nomeadas) e 1.6+ (variáveis CSS).
- **Ícones simbólicos próprios** para as ferramentas, embutidos no binário via
  GResource: recebem a cor de destaque em qualquer tema de ícones.
- 7 testes novos (planos de pré-visualização, posicionamento de imagens,
  leitura do tamanho de página).

### Alterado
- README reescrito em inglês, com capturas de tela nos temas claro e escuro;
  arquivo `LICENSE` (GPL-3.0) adicionado para publicação no GitHub.
- **Novo nome: PDFToolbox** (antes “Oficina PDF”). Mudam também o binário
  (`pdftoolbox`), o identificador (`io.github.leonardo_sponton.PDFToolbox`) e as
  pastas `~/.config/pdftoolbox` e `~/.cache/pdftoolbox`. As entradas antigas
  deste changelog foram atualizadas para o nome novo.
- Miniaturas da pré-visualização acessíveis pelo teclado (Tab, setas, Enter).

### Corrigido
- “2 imagemns prontas” → “2 imagens prontas”.
- A grade da pré-visualização voltava para a primeira linha ao clicar numa
  página (Girar, Remover, Extrair): o campo é reescrito em duas etapas pelo GTK
  e o estado intermediário vazio disparava a rolagem automática. Agora cliques
  nunca rolam a grade, e ao digitar ela só rola se uma página recém-marcada
  estiver fora de vista.
- Organizador: após girar/duplicar com várias páginas selecionadas (ex.: Ctrl+A,
  Ctrl+R) a grade voltava ao início; o foco agora fica na página atual.
- Organizador: a miniatura de uma página em paisagem girada invadia o número
  abaixo dela; o selo “↻ 90°” e o destaque da seleção não usavam a cor de
  acento da paleta (o selo ficava quase invisível no tema escuro).

## 0.1.0 — 2026-10-14

Primeira versão utilizável (Fase 1).

### Adicionado
- Interface GTK4/libadwaita: tela inicial com área de soltar arquivos, busca de
  ferramentas, recentes e categorias; tema claro/escuro/sistema; lista de
  tarefas; diagnóstico de dependências; abertura de arquivos pelo gerenciador.
- Ferramentas: Organizar páginas (miniaturas, arrastar, girar, duplicar,
  excluir, desfazer), Mesclar, Dividir, Extrair, Remover, Girar, Imagens para
  PDF, PDF para imagens, Comprimir.
- Gravação segura: temporários por tarefa, verificação do resultado, gravação
  atômica sem sobrescrever, sufixo numérico em colisões.
- Processamento em segundo plano com progresso, cancelamento real e fila.
- 41 testes automatizados; benchmark reproduzível.
- Instalação por usuário (`install.sh`) e metadados de `.deb` (cargo-deb 3.8).
  O `.deb` foi gerado e inspecionado (estrutura e `Depends` corretos), mas
  **ainda não foi instalado em um Debian/Ubuntu real**. O binário gerado no
  Arch exige glibc ≥ 2.39 (Ubuntu 24.04+, Debian 13+).

### Corrigido durante o desenvolvimento
- Nomes de saída longos (extração de muitas páginas) ultrapassavam 255 bytes.
- Falha de segmentação no GTK ao navegar pelo teclado após recriar a grade do
  organizador; a grade agora reaproveita as células.
