lexer grammar L;
I : '0'..'9'+ {outStream.println("I");} ;
HASH : '#' -> type(100), skip, more  ;