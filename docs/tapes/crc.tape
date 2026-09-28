Output build/tapes/laxity-crc.gif
Set Height 800
Source docs/tapes/common.tape

Type "laxity tui --file build/tapes/crc-corruption.bin"
Enter
Wait+Screen /replay complete/
Wait+Screen /1 crc rejects/
Sleep 8s
Type "q"
