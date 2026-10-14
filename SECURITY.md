# Segurança e privacidade

## Garantias

- **Processamento 100% local.** O código não abre sockets, não usa
  bibliotecas de rede e não chama serviços externos. Não há telemetria,
  verificação de atualizações, contas ou anúncios. (Verificável: nenhuma
  dependência de rede no `Cargo.lock`; os motores externos são executados sem
  argumentos de rede.)
- **Arquivos são dados, não código.** Nenhum script, anexo ou ação incorporada
  ao PDF é executado. O Ghostscript roda com `-dSAFER`; qpdf e Poppler não
  executam conteúdo.
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
- **Logs.** O programa não grava logs em disco. Os detalhes técnicos mostrados
  em falhas contêm só a saída de erro do motor (que pode incluir o nome do
  arquivo), nunca o conteúdo do documento.
- **Preferências** (`~/.config/pdftoolbox/config`) guardam apenas tema,
  ferramentas recentes e a última pasta escolhida — nenhum nome de documento.

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
- Documentos com senha de abertura são recusados; o programa não tenta
  contornar senhas e nunca o fará.
- Redação definitiva e assinatura digital ainda não existem; quando existirem,
  serão testadas quanto à recuperação do conteúdo removido.

## Relatar um problema

Abra uma issue sem anexar documentos pessoais. Use o botão “Copiar” do diálogo
Sobre → Informações de depuração para enviar versões dos componentes.
