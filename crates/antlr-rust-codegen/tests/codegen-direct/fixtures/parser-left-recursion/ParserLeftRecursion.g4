parser grammar ParserLeftRecursion;

tokens { INT, STAR, PLUS }

start : expression EOF ;

expression
    : INT
    | expression STAR expression
    | expression PLUS expression
    ;
