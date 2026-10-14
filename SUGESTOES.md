# Sugestões futuras

Anotações de 2026-10-14 (atualizadas na versão 0.5.0). A situação oficial de cada
recurso continua em [FEATURE_MATRIX.md](FEATURE_MATRIX.md); aqui ficam as ideias
e os próximos passos recomendados, em ordem de prioridade.

## Fase 2 — o que falta (0.3.0 entregou senha, metadados, numeração, marca d’água, texto e Office)

1. ~~OCR~~ — feito, com ~~correção de inclinação~~, ~~escolha entre os
   idiomas instalados~~ e ~~OCR direto no “PDF para texto”~~. Próximo passo:
   camada de texto que acompanhe a inclinação sem quebrar a extração (hoje,
   em páginas muito tortas, a seleção destacada pode sair um pouco
   deslocada nas pontas da linha; busca e cópia funcionam).
2. ~~Anotar e assinatura visual~~ — feito (“Anotar e assinar”), com ~~zoom na
   página~~; ~~arrastar com o mouse~~ verificado pelo usuário. Pendência:
   desfazer agrupando vários toques de seta num passo.
3. ~~Abrir PDFs com senha direto nas ferramentas~~ — feito (diálogo de senha e
   cópia temporária desbloqueada). Ideia: oferecer “manter a mesma senha no
   resultado”.
4. **Lote e fluxos** — já há lote por ferramenta; falta encadear (ex.:
   numerar → marca d’água → proteger) num fluxo salvo.

Decisões pendentes:
- **PDFs só com restrições do autor** (abrem sem senha, mas proíbem editar):
  as ferramentas estruturais da Fase 1 ainda os aceitam (o qpdf mantém a
  criptografia na saída); Metadados e Proteger recusam. Decidir se Girar,
  Numerar etc. devem recusar também quando “modificar” não é permitido.
- **Marca d’água com outros alfabetos e símbolos**: hoje só WinAnsi (fontes
  padrão). Para Unicode completo, embutir uma fonte livre (DejaVu/Noto) como
  subconjunto — aumenta a complexidade do gerador de PDF.
- **Marca d’água de imagem** (logotipo) e **cabeçalho/rodapé livre** com
  campos como data e nome do arquivo.
- **HTML → PDF** pelo LibreOffice foi deixado de fora: páginas web citam
  recursos externos e a fidelidade é baixa; reavaliar com o isolamento de rede.

## Melhorias na versão atual

- ~~Inserir página em branco~~, ~~resolução livre na compressão~~,
  ~~comprimir com estimativa~~, ~~prévia ampliada da página~~ e ~~confirmar ao
  fechar~~ — feitos na 0.4.0.
- ~~Ícones próprios~~ e ~~sugestão de ferramenta na tela inicial~~ — feitos na 0.2.0.
- **Estimativa de compressão mais precisa** em PDFs cujas páginas compartilham
  muitos recursos (a amostra copia fontes e imagens comuns para cada página);
  hoje o erro medido foi pequeno nos testes, mas não há garantia nesses casos.
- ~~Ampliar com zoom real~~ e ~~zoom no Anotar~~ — feitos.
- **Organizador com o mesmo visual de “papel”** das novas pré-visualizações
  (`PageThumb`), e girar desenhado de verdade em vez de transformação CSS.
- **Cache de miniaturas em disco** entre aberturas (hoje o cache é só em
  memória, durante a sessão) — avaliar privacidade: ficaria em `~/.cache`.
- **Miniaturas mais rápidas de fotos grandes** — decodificar JPEG em escala
  reduzida em vez de abrir a imagem inteira.
- **Teste automatizado da fila** (limite de 2 tarefas), que ainda não foi
  validada. (~~Arrastar com o mouse~~ no Organizar e no Anotar: verificado
  pelo usuário.)

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
- ~~Repositório git e publicação no GitHub~~ — feito: github.com/Leonardo-Sponton/pdftoolbox.

## Fase 3 e além

Redação definitiva (com teste de que o texto não é recuperável), formulários,
PDF/A com validação (veraPDF), comparação de documentos, recuperação de PDFs
danificados, leitor com busca e impressão, assinatura digital criptográfica,
fluxos de trabalho encadeados e, por último, IA local opcional (llama.cpp).
