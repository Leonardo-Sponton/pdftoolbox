# Sugestões futuras

Anotações de 2026-10-14, ao fim da versão 0.1.0. A situação oficial de cada
recurso continua em [FEATURE_MATRIX.md](FEATURE_MATRIX.md); aqui ficam as ideias
e os próximos passos recomendados, em ordem de prioridade.

## Próxima fase (2) — maior valor, menor risco

1. **PDFs com senha** — hoje são só detectados. Pedir a senha num diálogo e
   passá-la ao qpdf com `--password-file` (nunca na linha de comando, que fica
   visível em `/proc`). Na mesma leva: **proteger com senha** e **remover
   proteção** (qpdf `--encrypt` / `--decrypt`), com teste que confirma que o
   arquivo exige a senha configurada.
2. **Metadados** — ver, editar e remover título/autor/datas (qpdf JSON).
3. **Número de página e marca d’água** — gerar a sobreposição com o
   `pdfwriter` próprio e aplicar com `qpdf --overlay`, sem rasterizar.
4. **Extrair texto** (PDF → TXT) com `pdftotext`, já instalado.
5. **Documentos do escritório → PDF** com LibreOffice headless (26.8 já
   instalado). Usar um perfil temporário (`-env:UserInstallation=`) por
   tarefa para não travar se o LibreOffice do usuário estiver aberto.
6. **OCR** com Tesseract (`sudo pacman -S tesseract tesseract-data-por
   tesseract-data-eng`), progresso por página e aviso de possíveis erros.

## Melhorias na versão atual

- **Inserir página em branco** no organizador (o gerador interno já cria
  páginas; falta o botão e o tamanho baseado na página vizinha).
- **Resolução livre na compressão** (campo de ppp, além dos dois níveis fixos).
- ~~Ícones próprios~~ e ~~sugestão de ferramenta na tela inicial~~ — feitos na 0.2.0.
- **Organizador com o mesmo visual de “papel”** das novas pré-visualizações
  (`PageThumb`), e girar desenhado de verdade em vez de transformação CSS.
- **Comprimir com estimativa** — comprimir uma amostra de páginas em segundo
  plano para mostrar “≈ X MB” antes de executar.
- **Prévia da página escolhida ampliada** (clique duplo ou espaço abre a página
  grande), útil para conferir detalhes antes de remover.
- **Cache de miniaturas em disco** entre aberturas (hoje o cache é só em
  memória, durante a sessão) — avaliar privacidade: ficaria em `~/.cache`.
- **Miniaturas mais rápidas de fotos grandes** — decodificar JPEG em escala
  reduzida em vez de abrir a imagem inteira.
- **Teste automatizado da fila** (limite de 2 tarefas) e do arrastar com mouse
  no organizador, que ainda não foram validados.

## Instalação e distribuição

- **Cuidado com aplicativos padrão:** em sistemas sem padrões explícitos no
  `~/.config/mimeapps.list`, instalar o `.desktop` com `MimeType=` fez o
  PDFToolbox virar o padrão para PDF e imagens (corrigido nesta máquina
  definindo Papers e eog). Opções: o `install.sh` avisar sobre isso, ou
  declarar `MimeType` apenas para “Abrir com” e documentar.
- **Validar o `.deb` num Ubuntu 24.04 real** (ou contêiner: `docker run
  ubuntu:24.04`, `apt install ./pdftoolbox_*.deb`, conferir `ldd`).
- **Flatpak** — daria sandbox real aos motores, mas exige embutir qpdf e
  Poppler (obrigações de licença em LICENSES.md).
- **Repositório git** e publicação no GitHub, como o MyNotes.

## Fase 3 e além

Redação definitiva (com teste de que o texto não é recuperável), formulários,
PDF/A com validação (veraPDF), comparação de documentos, recuperação de PDFs
danificados, leitor com busca e impressão, assinatura digital criptográfica,
fluxos de trabalho encadeados e, por último, IA local opcional (llama.cpp).
