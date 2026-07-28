lexer grammar LexerLeftRecursion;

A : A 'a' | 'x' ;
B : C 'b' | 'y' ;
C : B 'c' ;
