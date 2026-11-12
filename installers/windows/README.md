# Vortal: Windows controla Linux/Wayland

Este MVP envia mouse, teclado, texto e imagens copiadas, além de áudio do Linux, para uma sessão Windows/Linux autenticada por certificados. Após conectar, escolha o monitor e a borda na página **Portais**. Arraste o cursor para fora dessa borda para entrar no Linux; arraste de volta pela borda correspondente ou use o atalho configurado para retornar. A captura auxiliar se encerra se perder seus batimentos por 300 ms.

## Preparar o Linux

Compile a CLI e o daemon com Rust 1.85 ou superior:

```sh
cargo build -p vortal-cli -p vortal-daemon --release --locked
mkdir -p "$HOME/.local/share/vortal"
target/release/vortal identity linux --dir "$HOME/.local/share/vortal/identity"
```

Para parear por ID, instale o app Linux com `./scripts/install.sh`, abra **Configurações** e veja o ID de duas letras e seis números. O receptor escuta TCP 24801 mesmo antes de haver certificado do Windows. O Windows e o Linux devem estar na mesma rede Tailscale. No Linux, clique em **Permitir** ao receber o pedido. A troca automática usa apenas `cert.der`; `key.der` nunca sai do dispositivo.

Para pareamento manual, copie **somente** `$HOME/.local/share/vortal/identity/cert.der` para o Windows. Nunca copie `key.der`. Depois de criar a identidade no app Windows, copie o certificado público indicado pelo app para o Linux como `$HOME/.local/share/vortal/peer.der`.

Inicie o receptor no Linux, dentro da sessão Wayland do usuário:

```sh
target/release/vortald --input --clipboard --audio --bind 0.0.0.0:24800 --identity "$HOME/.local/share/vortal/identity" --peer-cert "$HOME/.local/share/vortal/peer.der"
```

Libere **UDP 24800** e **TCP 24801** na interface Tailscale do Linux. O injetor usa os protocolos de entrada virtual do compositor; foi inicializado e testado localmente no Hyprland. O processo encerra com uma mensagem se o compositor não oferecer esse acesso. Texto copiado requer `wl-copy` e `wl-paste`; áudio requer `parec` e o monitor do dispositivo de saída PipeWire/PulseAudio.

## Conectar no Windows

1. Para parear por ID, digite o ID mostrado no Linux e clique em **Solicitar conexão**. No Linux, confira o pedido e clique em **Permitir**. O app troca os certificados públicos e conecta. Para pareamento manual, siga os passos abaixo.
2. Clique em **Criar identidade**. Copie o arquivo `cert.der` no caminho mostrado para o Linux como `peer.der`.
3. Clique em **Selecionar certificado Linux** e escolha o `cert.der` recebido do Linux.
4. Adicione o IP do Linux, incluindo o IP Tailscale se for essa a rede usada.
5. Com `vortald --input` em execução no Linux, clique em **Conectar ao Linux**. O painel só mostra **Conectado** após o handshake autenticado.
6. Abra **Portais**, escolha o monitor Windows e a borda de saída. Arraste o cursor até essa borda para controlar o Linux. No Hyprland, cruze a borda correspondente ou use o atalho configurado para voltar.
7. Abra **Configurações** para ajustar mouse, rolagem, teclado, inicialização, conexão automática, área de transferência e áudio. Para compartilhar texto e áudio, ative as opções nos dois computadores.
8. A área de captura na tela inicial continua disponível como alternativa manual. Clique em **Desconectar** para encerrar a sessão.

Fechar a janela mantém o Vortal em segundo plano. Use o ícone na bandeja do Windows para reabrir ou sair do app.

O portal automático está disponível no app Windows x64. A captura nativa encaminha a tecla Windows como Super e não abre uma janela sobre programas em tela cheia. O compartilhamento da área de transferência aceita texto UTF-8 até 16 KiB; arquivos e imagens ainda não são transferidos. O áudio do Linux é enviado em PCM estéreo a 48 kHz por um fluxo QUIC separado do controle de entrada.

## Gerar o app Windows

No Windows x64, instale Rust 1.85+, Visual Studio C++ Build Tools e NSIS; depois execute `scripts\build-windows.bat`. O instalador Rust fica em `dist\windows-native\Vortal-Setup-Rust-x64.exe`. A janela usa WebView2 e requer o WebView2 Runtime. O Vortal inicia na bandeja ao entrar no Windows; essa opção pode ser desativada em Configurações. A instalação fica em `VortalRust` para preservar a versão Electron anterior.

Em Linux, `cargo xwin build --target x86_64-pc-windows-msvc -p vortal-desktop -p vortal-cli -p vortal-daemon -p vortal-input-windows --release --locked` gera os binários Windows. Em seguida, `makensis installers/windows/Vortal.nsi` gera o instalador nativo.
