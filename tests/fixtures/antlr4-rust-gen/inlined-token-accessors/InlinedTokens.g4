grammar InlinedTokens;

start
    : expr EOF
    ;

recovered
    : ID '=' ID EOF
    ;

collision
    : DIRECT DIRECT ID EOF
    ;

expr
    : assign
    | binop
    | prim
    ;

assign
    : expr '=' expr
    ;

binop
    : expr ('+' | '-') expr
    ;

prim
    : ID
    ;

DIRECT
    : 'direct'
    ;

ID
    : [a-z]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
