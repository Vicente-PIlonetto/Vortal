# Wireframe — primeira entrega

## Objetivo

Desenhar um wireframe navegável da experiência inicial do Vortal para controlar um computador Linux a partir do Windows. A pessoa deve conseguir entender como encontrar e autorizar o Linux, configurar a borda do monitor, conectar e recuperar o controle local. Esta entrega é de interface; não exige implementar o transporte, o daemon ou a captura de entrada.

## Referências do projeto

- `README.md`: produto, estado atual e forma de conexão.
- `docs/design-system.md`: regras visuais e de acessibilidade.
- `crates/vortal-ui/ui/index.html`: estrutura e elementos da interface existente.
- `crates/vortal-ui/ui/tokens.css` e `app.css`: tokens e layout atuais.
- `crates/vortal-ui/ui/locales/pt-BR.json` e `en.json`: textos existentes.
- `installers/windows/README.md`: fluxo experimental Windows → Linux.

O wireframe pode reorganizar a interface atual para tornar o fluxo mais claro. Não apresente uma função futura como se já estivesse pronta: marque ações ainda não implementadas como conceito de interface.

## Escopo das telas

1. **Início / Dispositivos**: estado vazio, identificação do computador, campo para ID ou IP do Linux, ação de solicitar conexão, lista com um Linux encontrado e estado da conexão. Mostre o próximo passo mais evidente.
2. **Pedido de conexão no Linux**: ID e nome do solicitante, identificação de segurança resumida, ações Permitir e Recusar, e estados de espera, aprovação, recusa e expiração.
3. **Portais**: seleção do monitor Windows e da borda de saída, prévia da posição Windows/Linux e indicação de como voltar pelo lado correspondente.
4. **Controle / Configurações**: conexão e desconexão, atalho de retorno ao Windows, captura de teclado, sensibilidade do mouse e da rolagem. Área de transferência e áudio podem aparecer como opções secundárias, com estado explícito.

## Fluxos e estados obrigatórios

- Primeiro uso: abrir o Vortal → informar ID ou IP do Linux → solicitar conexão → permitir no Linux → escolher monitor e borda → conectar → atravessar a borda.
- Retorno seguro: atravessar a borda de volta, usar o atalho configurado ou desconectar. A perda de conexão deve mostrar que mouse e teclado locais foram liberados; a interface não deve sugerir bloqueio persistente.
- Erros: ID/IP inválido, Linux não encontrado, pedido recusado ou expirado, conexão perdida e falha ao capturar entrada.
- Variações: sem dispositivos, aguardando permissão, conectado e desconectado.

## Regras de desenho

- Priorize desktop Windows. Inclua uma adaptação compacta para largura reduzida das telas Início e Portais.
- Use layout de tiles do Windows Modern UI, tema escuro, escala de cinza, grade de 8 px e superfícies planas. Sem sombras, gradientes, transparência decorativa ou cantos arredondados.
- Mantenha títulos e ações curtos. Prepare a cópia em `pt-BR` e `en`; use os arquivos de localização existentes como ponto de partida.
- Indique ordem de foco, foco visível, operação por teclado e contraste legível. Para interações animadas, use apenas transformação ou opacidade e respeite movimento reduzido.
- Diferencie visualmente ação principal, ação secundária, estado, erro e controle desabilitado sem depender apenas de cor.

## Entregáveis e revisão

- Arquivo editável do wireframe e exportação em PDF ou PNG, com as quatro telas, estados principais e adaptação compacta solicitada.
- Protótipo clicável ou mapa de navegação que conecte o primeiro uso, a recusa e o retorno seguro.
- Anotações curtas sobre comportamento de cada ação, inclusive o que ocorre quando a conexão cai.
- Lista de decisões ou dúvidas de produto que não estejam resolvidas pelo material do projeto.

Considere a primeira entrega pronta quando outra pessoa conseguir percorrer o fluxo principal e os caminhos de falha sem explicação verbal, identificar sempre como voltar ao controle local e conferir os requisitos visuais acima.
