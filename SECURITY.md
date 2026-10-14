# Segurança e privacidade

## Garantias

- **Processamento 100% local.** O código não abre sockets, não usa
  bibliotecas de rede e não chama serviços externos. Não há telemetria,
  verificação de atualizações, contas ou anúncios. (Verificável: nenhuma
  dependência de rede no `Cargo.lock`; os motores externos são executados sem
  argumentos de rede.)
- **Arquivos são dados, não código.** Nenhum script, anexo ou ação incorporada
  ao PDF é executado. O Ghostscript roda com `-dSAFER`; qpdf e Poppler não
  executam conteúdo. Na conversão de documentos do Office, o LibreOffice usa um
  perfil novo e descartável por tarefa, cujo nível de segurança padrão (alto)
  bloqueia macros não assinadas. O Tesseract (OCR) só recebe imagens
  renderizadas pelo Poppler, nunca o PDF.
- **LibreOffice sem rede.** Um documento pode citar imagens ou vínculos
  externos. Quando o sistema permite namespaces sem privilégios (padrão na
  maioria das distribuições), o LibreOffice roda dentro de
  `unshare --user --map-current-user --net`, sem nenhuma interface de rede
  (verificado: o processo fica num namespace de rede próprio). Se o sistema
  não permitir, a ferramenta avisa antes (na descrição) e depois (no resultado).
- **Sem shell.** Programas externos são executados com argumentos separados
  (`std::process::Command`), nunca concatenando nomes de arquivo em uma linha
  de comando. Caminhos são sempre absolutos, então nomes iniciados por “-” não
  viram opções (há teste para isso).
- **O original nunca é sobrescrito.** A gravação usa `link()`, que falha se o
  destino existir; em sistemas sem hard links (FAT/exFAT) usa verificação +
  `rename`, com uma pequena janela de corrida documentada aqui.
- **Gravação atômica e verificada.** O resultado é gerado em pasta temporária,
  validado (número de páginas; `qpdf --check` nos PDFs gerados internamente),
  sincronizado em disco (`fsync`) e só então exposto com o nome final.
- **Temporários isolados.** Cada tarefa usa `~/.cache/pdftoolbox/tarefas/<id>`
  com permissão 0700, apagada ao terminar, falhar ou ser cancelada. Pastas de
  processos encerrados de forma anormal são removidas na próxima inicialização.
- **Senhas.** Ficam só na memória do processo, num tipo que não aparece em
  `Debug` e é zerado ao ser descartado. São entregues ao qpdf por arquivos
  0600 dentro da pasta temporária da tarefa (nunca como argumento de linha de
  comando, que seria visível em `ps`), apagados ao fim. Não são guardadas nas
  preferências nem nos detalhes de erro.
- **Sem contorno de senhas.** “Remover senha” exige a senha do documento. Se o
  autor restringiu impressão, cópia ou edição, só a senha de permissões
  (proprietário) remove as restrições — a senha de abertura não basta. Não há
  tentativa de adivinhar ou quebrar senhas.
- **PDFs com senha nas demais ferramentas.** Com a senha digitada, a
  ferramenta trabalha numa cópia desbloqueada guardada em
  `$XDG_RUNTIME_DIR/pdftoolbox/desbloqueados/<id>` (pasta 0700; em quase todas
  as distribuições, um tmpfs só do usuário, então o conteúdo sem
  criptografia não vai para o disco). Sem `$XDG_RUNTIME_DIR`, a cópia fica em
  `~/.cache/pdftoolbox/desbloqueados`. As cópias duram a sessão e são
  apagadas ao sair (ou na próxima abertura, depois de uma queda). A regra das
  restrições vale aqui também: a senha de abertura não basta para trabalhar
  num documento com restrições do autor. O resultado é gravado sem senha, e a
  interface avisa isso na lista de arquivos e no resultado.
- **Processos encerrados de verdade.** Cancelar ou fechar o programa encerra o
  grupo de processos de cada motor, incluindo processos filhos.
- **Logs.** O programa não grava logs em disco. Os detalhes técnicos mostrados
  em falhas contêm só a saída de erro do motor (que pode incluir o nome do
  arquivo), nunca o conteúdo do documento.
- **Preferências** (`~/.config/pdftoolbox/config`) guardam apenas tema,
  ferramentas recentes, a última pasta escolhida e os idiomas do OCR —
  nenhum nome de documento nem senha.

## Limites de recursos

- No máximo 2 tarefas simultâneas; renderização com no máximo 4 processos.
- O conteúdo dos PDFs nunca é carregado inteiro na memória pela interface;
  imagens para PDF são lidas uma por vez.
- Um PDF malicioso que trave ou derrube um motor afeta só aquele processo; o
  usuário pode cancelar a tarefa a qualquer momento.

## Limitações conhecidas

- Não há sandbox adicional (seccomp/bubblewrap) em torno dos motores; o
  isolamento é o de processo comum do usuário. Planejado para a distribuição
  em Flatpak.
- Documentos com senha de abertura só são aceitos com a senha (em “Remover
  senha” ou pelo diálogo das demais ferramentas). O programa não tenta
  contornar senhas e nunca o fará.
- PDFs que só têm restrições do autor (abrem sem senha) ainda são aceitos
  pelas ferramentas estruturais da Fase 1 (o qpdf preserva a criptografia na
  saída); Editar metadados e Proteger os recusam.
- Redação definitiva e assinatura digital ainda não existem; quando existirem,
  serão testadas quanto à recuperação do conteúdo removido.

## Relatar um problema

Abra uma issue sem anexar documentos pessoais. Use o botão “Copiar” do diálogo
Sobre → Informações de depuração para enviar versões dos componentes.
