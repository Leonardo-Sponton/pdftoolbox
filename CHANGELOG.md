# Changelog

## Não lançado

### Corrigido
- **“Sem texto selecionável” só aparece quando é verdade:** o aviso no cartão
  do arquivo (e a sugestão de OCR na tela inicial) olhava só a primeira página,
  então um livro com capa em imagem parecia digitalizado. Agora conta todas as
  páginas: “sem texto selecionável” quando nenhuma tem texto, “texto
  selecionável em só X de N páginas” quando a maioria não tem.
- **OCR — contagem com “Pular páginas que já têm texto” desligado:** a
  descrição continuava dizendo que só as páginas sem texto seriam
  reconhecidas; agora diz que todas serão.
- **PDF para texto — prévia:** mostra a primeira página que tem texto (não
  mais sempre a página 1) e, quando nenhuma tem, indica a ferramenta de OCR em
  vez de dizer que ela “chega numa próxima versão”.

## 0.4.0 — 2026-10-14

### Adicionado
- **Organizar — inserir página em branco** (Ctrl+B ou Insert): entra depois da
  página selecionada, com o tamanho e a orientação da página anterior.
- **Ampliar página:** espaço abre a página em tamanho grande no Organizar e nas
  pré-visualizações (Girar, Extrair, Remover, Dividir, Numerar, Marca d’água);
  também clique duplo no Organizar e botão direito nas pré-visualizações.
  ← e → passam pelas páginas; mostra a rotação planejada.
- **Comprimir — nível “Personalizada”:** resolução das imagens de 36 a 600 ppp.
- **Comprimir — tamanho estimado** antes de executar, calculado em segundo
  plano (documento inteiro até 8 páginas; amostra de 8 páginas nos maiores).

### Alterado
- **Organizar pergunta antes de descartar** alterações não salvas (Esc,
  voltar, abrir outro arquivo); fechar a janela também pergunta quando há
  alterações não salvas no Organizar ou no Anotar.
- Ao sair, as pastas temporárias das tarefas interrompidas são apagadas na
  hora (antes, só na abertura seguinte).

### Corrigido
- Pré-visualização de documentos com até 6 páginas cortava as miniaturas pela
  metade.

## 0.3.0 — 2026-10-14

Fase 2 completa: nove ferramentas novas — numerar, marca d’água, metadados,
proteger e remover senha, PDF para texto, Office para PDF, anotar e assinar,
e OCR — todas com operação real, testes que conferem o conteúdo do resultado e
fluxo verificado na interface.

### Adicionado
- **Numerar páginas:** 6 posições, formatos “1”, “1 / 10” e “Página 1 de 10”,
  número inicial, tamanho do texto e escolha das páginas (“2-” pula a capa).
  Cada miniatura mostra o número que vai receber.
- **Marca d’água:** texto na diagonal ou na horizontal, cor, opacidade,
  tamanho e páginas.
- **“Como vai ficar”:** em Numerar e Marca d’água, a primeira página afetada é
  renderizada com o resultado real (mesmo código da operação), atualizada
  enquanto se digita.
- **Editar metadados:** título, autor, assunto e palavras-chave, preenchidos com
  os valores atuais; mostra programa de criação, produtor e datas; opção de
  apagar todos os metadados (Info e XMP).
- **Proteger com senha:** AES-256, confirmação da senha, aviso de senha curta e
  restrições opcionais (imprimir, copiar, editar) com senha de permissões.
- **Remover senha:** para documentos cuja senha se conhece; restrições do autor
  só saem com a senha de permissões.
- **PDF para texto:** `.txt` em UTF-8, na ordem de leitura ou mantendo o layout,
  com prévia da página 1; digitalizações são identificadas em vez de gerar
  arquivo vazio.
- **Office para PDF:** Word, Excel, PowerPoint, LibreOffice, RTF, CSV e TXT, via
  LibreOffice, isolado da rede quando o sistema permite.
- Categorias **Editar** e **Segurança** na tela inicial; ícones novos.
- Tela inicial aceita documentos do Office (cartão com ícone) e mostra
  “🔒 Protegido por senha” com um cadeado na miniatura; se houver PDF
  protegido, a sugestão é “Remover senha”.
- **Anotar e assinar:** escreva textos (várias linhas, tamanho, negrito, cor),
  a data de hoje, destaques que não escondem o texto, molduras e a imagem da
  sua assinatura (com remoção do papel claro em volta). A página aparece
  grande; arraste para mover, puxe o canto para redimensionar, ou use o
  teclado (setas, Shift+setas, + e −, Delete, Tab, Page Up/Down, Ctrl+Z).
  Opção de repetir em todas as páginas (rubrica). Gravado como camada
  sobreposta, sem rasterizar o documento.
- Ao sair (Voltar, Esc, abrir outro PDF) com anotações não salvas, o
  programa pergunta antes de descartar.
- **OCR — reconhecer texto** (Tesseract, português e inglês): torna
  pesquisável e copiável o texto de digitalizações. As imagens não são
  alteradas; uma camada de texto invisível é adicionada por cima. Pula
  páginas que já têm texto, endireita páginas deitadas ou de cabeça para baixo,
  mostra antes quantas páginas serão reconhecidas e avisa que o resultado pode
  ter erros.
- PDFs sem texto selecionável aparecem como tal na tela inicial e nas listas de
  arquivos; se todos forem digitalizações, a sugestão é o OCR.

### Alterado
- “PDF para texto” indica o OCR quando o arquivo é uma digitalização.
- PDFs protegidos nas demais ferramentas: a linha do arquivo e a
  pré-visualização explicam o motivo e indicam “Remover senha”.
- Avisos de formulário distinguem “falta preencher” (discreto) de “valor
  errado” (vermelho).
- Ao executar, o foco do teclado vai para **Cancelar** e volta ao botão
  principal no fim (antes ia para o botão Voltar).
- Cancelar encerra o grupo de processos inteiro do motor; fechar o programa
  (inclusive por SIGTERM/SIGINT) encerra motores em execução.
- Texto do destino: “nome (2)” em vez de “nome (2).pdf”, pois nem toda saída é PDF.

### Corrigido
- Organizador: a miniatura de uma página em paisagem girada invadia o número
  abaixo dela; o selo “↻ 90°” e o destaque da seleção não usavam a cor de
  acento da paleta (o selo ficava quase invisível no tema escuro).
- Metadados gravados com fuso UTC apareciam com a hora errada; agora usam o
  fuso local, como os demais programas.

### Documentação
- README em inglês, com capturas de tela nos temas claro e escuro; arquivo
  `LICENSE` (GPL-3.0); publicação no GitHub.

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
