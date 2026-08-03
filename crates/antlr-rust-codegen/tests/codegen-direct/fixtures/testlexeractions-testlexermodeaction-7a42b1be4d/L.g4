lexer grammar L;
STRING_START : '"' -> mode(STRING_MODE), more ;
WS : (' '|'\n') -> skip ;
mode STRING_MODE;
STRING : '"' -> mode(DEFAULT_MODE) ;
ANY : . -> more ;
