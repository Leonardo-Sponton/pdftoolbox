# Matriz de recursos

Estados: **Implementado e testado** (operação real + fluxo de interface +
testes automatizados que conferem o resultado) · **Implementado, não validado**
· **Parcialmente suportado** · **Em desenvolvimento** · **Não iniciado** ·
**Indisponível por limitação técnica**.

“Testes” refere-se a `tests/organize.rs`, `tests/convert.rs` e `tests/phase2.rs`. “UI verificada”
significa que o fluxo foi executado na interface real (Wayland) e o arquivo
gerado foi conferido.

## 4.1 Organizar

| Recurso | Estado | Motor | Observações |
|---|---|---|---|
| Mesclar vários PDFs | Implementado e testado | qpdf | Ordem por setas ou arrastando as linhas com o mouse (também em Imagens→PDF); duplicatas permitidas; UI verificada (setas pelo teclado; arrastar com o mouse ainda não verificado — não automatizável aqui) |
| Dividir por intervalos | Implementado e testado | qpdf | Também “a cada N” e “uma por página”; UI verificada |
| Extrair páginas | Implementado e testado | qpdf | Ordem digitada; opção um-arquivo-por-página; UI verificada |
| Um arquivo por página | Implementado e testado | qpdf | Mais de 3 saídas vão para uma subpasta |
| Remover páginas | Implementado e testado | qpdf | UI verificada |
| Reordenar arrastando miniaturas | Implementado e testado | qpdf + Poppler | Operação testada; teclado (Ctrl+←/→) verificado na UI; arrastar com o mouse verificado pelo usuário numa sessão real (2026-10-14) |
| Girar páginas (individual/conjunto) | Implementado e testado | qpdf | Ferramenta Girar (lote) e organizador; UI verificada |
| Duplicar páginas | Implementado e testado | qpdf | Organizador; UI verificada |
| Inserir páginas em branco | Implementado e testado | qpdf + gerador próprio | Organizador (Ctrl+B ou Insert): entra depois da seleção, com o tamanho e a orientação da página anterior (ou da seguinte, se for a primeira), inclusive quando a vizinha foi girada no organizador. Testes conferem tamanhos, rotação e que a página sai vazia; UI verificada com documento retrato + paisagem |
| Inserir páginas de outro PDF | Parcialmente suportado | qpdf | Possível via Mesclar + Organizar; sem fluxo dedicado |
| Misturar páginas de dois documentos | Não iniciado | — | |
| Recortar margens | Não iniciado | — | |
| Miniaturas antes de confirmar | Implementado e testado | Poppler | Organizador; carregamento em blocos de 12 |
| Contagem de páginas e ordem final | Implementado e testado | — | Rótulo “posição · pág. original” |
| Salvar em novo arquivo | Implementado e testado | — | Nunca sobrescreve |
| Aviso ao sair sem salvar | Implementado e testado | — | Esc, Alt+←, “voltar”, “Abrir outro…”, soltar outro arquivo e fechar a janela perguntam antes de descartar; desfazer até o estado salvo libera a saída. UI verificada |

## 4.2 Comprimir e otimizar

| Recurso | Estado | Motor | Observações |
|---|---|---|---|
| Níveis de intensidade | Implementado e testado | qpdf, Ghostscript | Sem perda / Recomendada (150 ppp) / Máxima (96 ppp) / Personalizada |
| Compressão de imagens | Implementado e testado | Ghostscript | JPEG via pdfwrite |
| Redução de resolução configurável | Implementado e testado | Ghostscript | Nível “Personalizada”: 36 a 600 ppp. Teste confere que 50 ppp gera arquivo bem menor que 200 ppp; UI verificada |
| Estimativa de tamanho antes de comprimir | Implementado e testado | qpdf, Ghostscript | Calculada em segundo plano numa pasta temporária: documentos de até 8 páginas são comprimidos inteiros (valor exato); nos maiores, 8 páginas espalhadas formam a amostra. Testes: exata = resultado real; amostra a menos de 25% do real. Na UI, álbum de fotos de 16 páginas: previsto 2,1 MB / real 2,06 MB (150 ppp) e 399,5 KB / 400 KB (70 ppp). PDFs com recursos muito compartilhados entre páginas podem desviar mais |
| Recursos duplicados | Implementado, não validado | Ghostscript | `-dDetectDuplicateImages`; sem teste específico |
| Otimização estrutural | Implementado e testado | qpdf | Object streams, recompressão Flate nível 9 |
| Comparar tamanhos / economia | Implementado e testado | — | Bytes e %; não grava se a economia < 1% e explica |
| Qualidade máxima | Implementado e testado | qpdf | Nível “Sem perda” |

## 4.3 Converter

| Recurso | Estado | Motor | Observações |
|---|---|---|---|
| Imagens → PDF (JPEG, PNG, BMP, GIF, WebP, TIFF) | Implementado e testado | próprio | JPEG sem recompressão, EXIF, alfa; UI verificada |
| PDF → PNG / JPEG / TIFF | Implementado e testado | Poppler | 36–1200 ppp, até 4 processos; UI verificada |
| DOC/DOCX/ODT/RTF/TXT, XLS/XLSX/ODS/CSV, PPT/PPTX/ODP → PDF | Implementado e testado | LibreOffice | Perfil temporário por tarefa; sem rede via `unshare` (verificado: processo em namespace de rede próprio); macros bloqueadas pelo nível padrão; cancelamento encerra o LibreOffice; testes com CSV e TXT; UI verificada com DOCX e CSV. HTML não incluído |
| PDF → DOCX/XLSX/PPTX/HTML/Markdown | Não iniciado | — | Fase 3; fidelidade limitada por natureza |
| PDF → TXT / extração de texto | Implementado e testado | Poppler (pdftotext), Tesseract opcional | UTF-8; ordem de leitura ou layout; prévia da primeira página com texto; opção “Reconhecer texto das páginas digitalizadas (OCR)” (ligada quando o Tesseract está instalado): só as páginas sem texto são reconhecidas, numa cópia temporária, e só o .txt é gravado. Sem OCR, PDFs sem texto não geram arquivo vazio e são explicados. Teste: documento misto (digitada + digitalizada) gera um .txt com as duas, sem duplicar a digitada e sem PDF novo; UI verificada com digitalização torta |
| Extração de imagens/tabelas | Não iniciado | — | |
| PDF/A | Não iniciado | — | Exige validador (veraPDF) — Fase 3 |

## 4.4–4.11

| Área | Estado | Observações |
|---|---|---|
| Anotar e assinatura visual | Implementado e testado | Ferramenta “Anotar e assinar”: texto (várias linhas, tamanho, negrito, cor), data de hoje, imagem/assinatura (fundo claro removido opcionalmente), destaque (mistura “multiplicar”, o texto embaixo continua legível) e moldura; repetir em todas as páginas; camada sobreposta pelo qpdf (sem rasterizar). Testes conferem posição do texto (inclusive em página girada), cores renderizadas, remoção do fundo e destaque sem esconder o texto. UI verificada pelo teclado (adicionar, digitar, mover, redimensionar, desfazer, salvar; resultado idêntico à prévia); arrastar com o mouse verificado pelo usuário numa sessão real (2026-10-14). Zoom na página (Ctrl+roda, pinça, Ctrl+mais/menos, Ctrl+0; até 6× a página ajustada, renderizada de novo em resolução maior); aproximada, arrastar fora das anotações move a vista e mover pelo teclado faz a vista acompanhar — verificado pelo teclado; Ctrl+roda, pinça e arrastar a vista não foram automatizados. Assinatura é só imagem, sem validade criptográfica |
| Editar texto existente, formas livres, notas/comentários PDF | Não iniciado | |
| Número de página | Implementado e testado | 6 posições, 3 formatos, número inicial, tamanho, páginas escolhidas; camada sobreposta pelo qpdf (sem rasterizar); posição conferida em página girada e em paisagem; prévia real da página; UI verificada |
| Marca d’água de texto | Implementado e testado | Diagonal ou horizontal, cor, opacidade, tamanho, páginas; prévia real; caracteres fora do WinAnsi (ex.: ✓, CJK) são recusados com explicação; UI verificada |
| Marca d’água de imagem, cabeçalho/rodapé livre | Não iniciado | |
| OCR (Tesseract, idiomas instalados) | Implementado e testado | Idiomas: um interruptor por idioma instalado no Tesseract (lembra a última escolha; padrão português + inglês) e o comando para instalar outros. Só a camada de texto invisível é adicionada (imagens intactas, ~1,5–3 KB por página); pula páginas que já têm texto; detecta e endireita páginas deitadas/de cabeça para baixo; recusa resultado sem nenhum texto reconhecido; 4 páginas em paralelo (40 páginas em ~13 s nesta máquina); cancelamento encerra os processos. **Correção de inclinação**: o ângulo das linhas é medido na própria imagem (perfil de projeção, até 15°) e a leitura é feita numa cópia endireitada; a página do PDF não é alterada. A mesma medição reconhece páginas deitadas antes do Tesseract. Testes: texto reconhecido e posicionado sobre a palavra, páginas mistas, página girada, deitada e invertida, digitalização em branco, digitalizações tortas a −6° e 9° (≥95% das palavras, linhas inteiras e na ordem), deitada **e** torta (90° e 270° + 5°). Medido à parte com digitalização ruidosa em português (Noto Serif 10 pt, 300 ppp, ruído e desfoque): antes da correção, 63% das palavras a 2,5° e nenhuma a partir de 4°; com ela, 100% até 10°, 99% a 7°, 97% a 13°, na ordem de leitura. Limite conhecido: a camada é horizontal por linha (o Poppler quebra texto inclinado), então em páginas muito tortas a seleção destacada pode sair um pouco acima/abaixo das pontas da linha; busca e cópia não são afetadas. UI verificada (progresso, cancelar, resumo de páginas sem texto, sugestão na tela inicial, lista de idiomas) |
| Proteger com senha | Implementado e testado | AES-256 (qpdf); restrições opcionais de impressão/cópia/edição com senha de permissões distinta; senhas via arquivo 0600 (nunca na linha de comando); resultado verificado abrindo com a senha; UI verificada |
| Remover senha (conhecida) | Implementado e testado | Exige a senha; restrições do autor só saem com a senha de permissões; senha errada recusada; nenhum mecanismo de descoberta; UI verificada |
| Abrir PDF com senha nas demais ferramentas | Implementado e testado | Ao entrar um PDF que exige senha (em qualquer ferramenta, no Organizar e no Anotar), um diálogo pede a senha; a ferramenta trabalha numa cópia temporária desbloqueada em `$XDG_RUNTIME_DIR` (memória, 0700), apagada ao sair; o original não muda, o destino padrão é a pasta dele e a lista e o resultado avisam que o resultado sai sem senha. A senha de abertura não basta se o autor restringiu impressão/cópia/edição (como em “Remover senha”). Senha errada pede de novo; cancelar deixa o arquivo de fora com aviso. A mesma cópia é reaproveitada na sessão enquanto o original não muda. Testes: senha errada, arquivo sem senha, cópia com o mesmo nome e pasta 0700 sem arquivo de senha, ferramenta trabalhando na cópia, original intacto, restrições exigem a senha de permissões, cópia apagada ao descartar. UI verificada (Girar e Anotar, senha errada e certa, resultado sem senha) |
| Metadados (ver/editar/remover) | Implementado e testado | Título, autor, assunto, palavras-chave; mostra programa e datas; “apagar tudo” remove Info e XMP (inclusive a data de modificação); funciona com Info dentro de fluxo de objetos (teste dedicado); recusa PDFs criptografados; UI verificada |
| Redação definitiva | Não iniciado | Fase 3 |
| Formulários | Não iniciado | Fase 3 |
| Leitor, busca, impressão | Não iniciado | |
| Comparação de PDFs | Não iniciado | Fase 3 |
| Recuperação de PDFs danificados | Não iniciado | Fase 3 (qpdf reconstrói xref) |
| Assinatura digital criptográfica | Não iniciado | Fase 3 |
| IA local | Não iniciado | Fase 4, opcional |

## 4.12 Lote e 4.13 Fluxos

| Recurso | Estado | Observações |
|---|---|---|
| Vários arquivos / arrastar vários | Implementado e testado | Girar, Imagens→PDF, PDF→imagens, Comprimir |
| Fila e limite de concorrência | Implementado, não validado | Máx. 2 tarefas simultâneas; sem teste automatizado da fila |
| Progresso por arquivo e geral | Implementado e testado | Barra única com texto “arquivo: página X de Y” |
| Cancelamento | Implementado e testado | Real: encerra o grupo de processos (inclui os filhos do LibreOffice); lote mantém o que já foi gravado e informa. Ao fechar o app (ou SIGTERM/SIGINT), motores em execução são encerrados e as pastas temporárias das tarefas apagadas — verificado com o LibreOffice e com o OCR |
| Confirmar ao fechar | Implementado e testado | Com tarefa em andamento ou alterações não salvas (Organizar, Anotar), fechar a janela pergunta antes. UI verificada: “Fechar mesmo assim” durante um OCR não deixa resultado parcial, processos nem temporários |
| Falha isolada em lote + relatório | Implementado e testado | UI verificada |
| Retentativa | Parcialmente suportado | Botão “Tentar novamente” refaz a tarefa inteira |
| Diretório de saída configurável | Implementado e testado | Padrão: pasta do original |
| Nomes duplicados | Implementado e testado | Sufixo “ (2)”, sem sobrescrever |
| Lista de tarefas da sessão | Implementado, não validado | Ctrl+T |
| Fluxos encadeados reutilizáveis | Não iniciado | Fase 3 |

## Interface

| Recurso | Estado | Observações |
|---|---|---|
| Tela inicial com soltar arquivos, recentes, categorias, busca | Implementado e testado | UI verificada |
| Abrir pelo gerenciador de arquivos (`%U`) | Implementado e testado | Via linha de comando; .desktop validado |
| Tema claro/escuro/sistema | Implementado e testado | UI verificada; paleta do MyNotes no escuro e variante clara |
| Idioma: português e inglês (seguir o sistema ou fixar) | Implementado e testado | Detecção pelo locale (LANGUAGE, LC_ALL, LC_MESSAGES, LANG, /etc/locale.conf; não suportado → português), escolha no menu gravada em `idioma=` e aplicada na hora ao menu, à tela inicial, aos diálogos e mensagens; ferramentas já abertas mudam ao serem reabertas. Testes: resolução e detecção com ambiente simulado, persistência sem apagar outras preferências, catálogo conferido contra o código, saídas idênticas nos dois idiomas. UI verificada nas 18 ferramentas, diálogos e janela de 360 px nos dois idiomas (instância isolada, textos lidos por AT-SPI) |
| Cartões dos arquivos escolhidos (1ª página, páginas, tamanho, problema) | Implementado e testado | UI verificada com PDFs, imagens e mistura |
| Sugestão de ferramenta e motivo das indisponíveis | Implementado e testado | UI verificada |
| Pré-visualização de Girar/Extrair/Remover/Dividir | Implementado e testado | Plano com testes automatizados; UI verificada; resultado de Girar e Remover conferido com `pdfinfo`/`qpdf` |
| Marcar páginas clicando na pré-visualização | Implementado, não validado | Lógica testada (`toggle_page`); clique com mouse não foi automatizado |
| Ampliar uma página | Implementado e testado | Espaço (ou clique duplo/Enter no organizador; botão direito nas pré-visualizações) abre a página grande, com a rotação planejada; ← e → passam pelas páginas na ordem da grade. Zoom real: +, −, 0, botões, Ctrl+roda e pinça, até 6× a página ajustada, com rolagem e arrastar para mover; a página é renderizada de novo em resolução maior (até 5000 px). UI verificada pelo teclado (zoom e nitidez); clique duplo, botão direito, Ctrl+roda, pinça e arrastar não foram automatizados |
| Pré-visualização de Imagens para PDF | Implementado e testado | Usa a mesma função `place` da conversão (testada) |
| Ícones próprios recoloríveis | Implementado e testado | Verificados com o tema Sweet/Candy |
| Estados: vazio, processando, sucesso, aviso, falha, cancelado, inválido, senha, dependência ausente, sem espaço, sem permissão | Implementado e testado | Mensagens por tipo de erro; sem espaço testado só por classificação de `ENOSPC` |
| Atalhos de teclado | Implementado e testado | Ver README |
| Janela estreita (≥ 360 px) | Implementado e testado | Verificado a 470 px |
