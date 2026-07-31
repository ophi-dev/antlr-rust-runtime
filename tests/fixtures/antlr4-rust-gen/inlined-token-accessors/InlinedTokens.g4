grammar InlinedTokens;

start
    : expr EOF
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

ID
    : [a-z]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
