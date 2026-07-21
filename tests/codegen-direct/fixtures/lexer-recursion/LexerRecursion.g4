lexer grammar LexerRecursion;

NESTED : LEVEL_A ;
fragment LEVEL_A : '{' (LEVEL_B | TEXT)* '}' ;
fragment LEVEL_B : LEVEL_A ;
fragment TEXT : ~[{}]+ ;
PLAIN : [a-z]+ ;
