lexer grammar L;
I : [0-9] {outStream.println("x");} [0-9]+ {outStream.println("y");} ;
WS : (' '|'\n') -> skip ;