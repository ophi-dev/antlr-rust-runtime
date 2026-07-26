grammar T;

expr
    : relation EOF
    ;

relation
    : calc
    | relation op = ('<' | '<=' | '>=' | '>' | '==' | '!=' | 'in') relation
    ;

calc
    : unary
    | calc op = ('*' | '/' | '%') calc
    | calc op = ('+' | '-') calc
    ;

unary
    : IDENT
    | NUM
    ;

LESS
    : '<'
    ;

LESS_EQUALS
    : '<='
    ;

GREATER_EQUALS
    : '>='
    ;

GREATER
    : '>'
    ;

EQUALS
    : '=='
    ;

NOT_EQUALS
    : '!='
    ;

IN
    : 'in'
    ;

STAR
    : '*'
    ;

SLASH
    : '/'
    ;

PERCENT
    : '%'
    ;

PLUS
    : '+'
    ;

MINUS
    : '-'
    ;

IDENT
    : [a-zA-Z_] [a-zA-Z0-9_]*
    ;

NUM
    : [0-9]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
