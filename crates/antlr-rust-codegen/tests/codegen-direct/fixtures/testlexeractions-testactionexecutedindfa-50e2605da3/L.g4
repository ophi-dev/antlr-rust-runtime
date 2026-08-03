lexer grammar L;
I : '0'..'9'+ {outStream.println("I");} ;
WS : (' '|'\n') -> skip ;