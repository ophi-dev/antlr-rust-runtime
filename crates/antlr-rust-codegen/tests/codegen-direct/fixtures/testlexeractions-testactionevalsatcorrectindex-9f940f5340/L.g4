lexer grammar L;
I : [0-9] {outStream.println("2nd char: "+(char)_input.LA(1));} [0-9]+ ;
WS : (' '|'\n') -> skip ;