lexer grammar L;
I : '0'..'9'+ {outStream.println("I");} -> skip ;
WS : (' '|'\n') -> skip ;