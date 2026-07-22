parser grammar ParserLeftRecursionFull;

tokens { INT, MINUS, POWER, QUESTION, COLON, PLUS, BANG }

start : expression EOF ;

expression returns [int value]
    : MINUS expression
    | INT
    | <assoc=right> left=expression POWER right=expression
    | <assoc=right> expression QUESTION expression COLON expression
    | expression PLUS {notify();} expression
    | expression BANG
    ;
