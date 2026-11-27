# ia-get demo tape. Record with `just record`.

Output .demo.mp4

Set Shell "/bin/sh"
Set Columns 84
Set Rows 24
Set FontFamily "Fira Code"
Set FontSize 17
Set Padding 16
Set Framerate 20
Set TypingSpeed 40ms
Set CursorBlink false
Set Theme { "name": "ia-get-espresso", "background": "#241A14", "foreground": "#F5EFE6", "cursor": "#F2C96B", "selectionBackground": "#4A3728", "black": "#241A14", "brightBlack": "#8C7B6B", "red": "#E06C65", "brightRed": "#F08A84", "green": "#98C379", "brightGreen": "#B5DB96", "yellow": "#F2C96B", "brightYellow": "#F7DA90", "blue": "#6FA8DC", "brightBlue": "#8CC0EE", "magenta": "#D19FE8", "brightMagenta": "#E2BDF2", "cyan": "#6FC2B8", "brightCyan": "#8FD8CF", "white": "#E6DDD0", "brightWhite": "#FFFDF9" }

Hide
Type 'export HOME="${IA_GET_DEMO_DIR:-$HOME}" && mkdir -p "$HOME/deftributetozzap64" && cd "$HOME/deftributetozzap64" && clear'
Enter
Sleep 200ms
Show

Sleep 400ms
Type "ia-get --list https://archive.org/details/deftributetozzap64"
Enter
Wait+Screen@30s /[0-9]+ files/
Wait
Sleep 1.5s

Type "ia-get -f '!*.zip' -f '!*.pdf' https://archive.org/details/deftributetozzap64"
Enter
Wait+Screen@45s /✓ [0-9]+ ok/
Wait
Sleep 1.5s

Type "ia-get --check -f '*djvu.*' https://archive.org/details/deftributetozzap64"
Enter
Wait+Screen@30s /✓ verified/
Wait
Sleep 1s
Screenshot pages/demo-still.png
Sleep 2.5s
