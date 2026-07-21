lexer grammar LexerBasic;

A : 'a' ;
WORD : [a-z]+ ;
WS : [ \t\r\n]+ -> skip ;
