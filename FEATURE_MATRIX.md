# Matriz de recursos

Estados: **Implementado e testado** (operação real + fluxo de interface +
testes automatizados que conferem o resultado) · **Implementado, não validado**
· **Parcialmente suportado** · **Em desenvolvimento** · **Não iniciado** ·
**Indisponível por limitação técnica**.

“Testes” refere-se a `tests/organize.rs` e `tests/convert.rs`. “UI verificada”
significa que o fluxo foi executado na interface real (Wayland) e o arquivo
gerado foi conferido.

## 4.1 Organizar

| Recurso | Estado | Motor | Observações |
|---|---|---|---|
| Mesclar vários PDFs | Implementado e testado | qpdf | Ordem por setas; duplicatas permitidas; UI verificada |
| Dividir por intervalos | Implementado e testado | qpdf | Também “a cada N” e “uma por página”; UI verificada |
| Extrair páginas | Implementado e testado | qpdf | Ordem digitada; opção um-arquivo-por-página; UI verificada |
| Um arquivo por página | Implementado e testado | qpdf | Mais de 3 saídas vão para uma subpasta |
| Remover páginas | Implementado e testado | qpdf | UI verificada |
| Reordenar arrastando miniaturas | Implementado, não validado | qpdf + Poppler | Operação testada; arrastar com mouse não foi automatizado (teclado Ctrl+←/→ verificado) |
| Girar páginas (individual/conjunto) | Implementado e testado | qpdf | Ferramenta Girar (lote) e organizador; UI verificada |
| Duplicar páginas | Implementado e testado | qpdf | Organizador; UI verificada |
| Inserir páginas em branco | Não iniciado | — | O gerador interno já cria páginas; falta a UI |
| Inserir páginas de outro PDF | Parcialmente suportado | qpdf | Possível via Mesclar + Organizar; sem fluxo dedicado |
| Misturar páginas de dois documentos | Não iniciado | — | |
| Recortar margens | Não iniciado | — | |
| Miniaturas antes de confirmar | Implementado e testado | Poppler | Organizador; carregamento em blocos de 12 |
| Contagem de páginas e ordem final | Implementado e testado | — | Rótulo “posição · pág. original” |
| Salvar em novo arquivo | Implementado e testado | — | Nunca sobrescreve |

## 4.2 Comprimir e otimizar

| Recurso | Estado | Motor | Observações |
|---|---|---|---|
| Níveis de intensidade | Implementado e testado | qpdf, Ghostscript | Sem perda / Recomendada (150 ppp) / Máxima (96 ppp) |
| Compressão de imagens | Implementado e testado | Ghostscript | JPEG via pdfwrite |
| Redução de resolução configurável | Parcialmente suportado | Ghostscript | Dois níveis fixos; sem campo livre de ppp |
| Recursos duplicados | Implementado, não validado | Ghostscript | `-dDetectDuplicateImages`; sem teste específico |
| Otimização estrutural | Implementado e testado | qpdf | Object streams, recompressão Flate nível 9 |
| Comparar tamanhos / economia | Implementado e testado | — | Bytes e %; não grava se a economia < 1% e explica |
| Qualidade máxima | Implementado e testado | qpdf | Nível “Sem perda” |

## 4.3 Converter

| Recurso | Estado | Motor | Observações |
|---|---|---|---|
| Imagens → PDF (JPEG, PNG, BMP, GIF, WebP, TIFF) | Implementado e testado | próprio | JPEG sem recompressão, EXIF, alfa; UI verificada |
| PDF → PNG / JPEG / TIFF | Implementado e testado | Poppler | 36–1200 ppp, até 4 processos; UI verificada |
| DOCX/XLSX/PPTX/CSV/TXT/HTML → PDF | Não iniciado | LibreOffice (candidato) | Fase 2; LibreOffice 26.8 detectado nesta máquina |
| PDF → DOCX/XLSX/PPTX/HTML/Markdown | Não iniciado | — | Fase 3; fidelidade limitada por natureza |
| PDF → TXT / extração de texto | Não iniciado | Poppler (candidato) | Fase 2 |
| Extração de imagens/tabelas | Não iniciado | — | |
| PDF/A | Não iniciado | — | Exige validador (veraPDF) — Fase 3 |

## 4.4–4.11

| Área | Estado | Observações |
|---|---|---|
| Editar e anotar (texto, imagens, formas, destaque, assinatura visual) | Não iniciado | Fase 2 |
| Marca d’água, número de página, cabeçalho/rodapé | Não iniciado | Fase 2 |
| OCR (Tesseract, por+eng) | Não iniciado | Fase 2; Tesseract não instalado nesta máquina |
| Senha: proteger / remover com senha | Não iniciado | Fase 2 (qpdf suporta); detecção de PDF protegido já existe |
| Metadados (ver/editar/remover) | Não iniciado | Fase 2 |
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
| Cancelamento | Implementado e testado | Real (mata o processo); lote mantém o que já foi gravado e informa |
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
| Cartões dos arquivos escolhidos (1ª página, páginas, tamanho, problema) | Implementado e testado | UI verificada com PDFs, imagens e mistura |
| Sugestão de ferramenta e motivo das indisponíveis | Implementado e testado | UI verificada |
| Pré-visualização de Girar/Extrair/Remover/Dividir | Implementado e testado | Plano com testes automatizados; UI verificada; resultado de Girar e Remover conferido com `pdfinfo`/`qpdf` |
| Marcar páginas clicando na pré-visualização | Implementado, não validado | Lógica testada (`toggle_page`); clique com mouse não foi automatizado |
| Pré-visualização de Imagens para PDF | Implementado e testado | Usa a mesma função `place` da conversão (testada) |
| Ícones próprios recoloríveis | Implementado e testado | Verificados com o tema Sweet/Candy |
| Estados: vazio, processando, sucesso, aviso, falha, cancelado, inválido, senha, dependência ausente, sem espaço, sem permissão | Implementado e testado | Mensagens por tipo de erro; sem espaço testado só por classificação de `ENOSPC` |
| Atalhos de teclado | Implementado e testado | Ver README |
| Janela estreita (≥ 360 px) | Implementado e testado | Verificado a 470 px |
