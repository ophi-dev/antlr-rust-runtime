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

// The optional labeled PLUS is followed by an unlabeled PLUS that would slide
// into `.nth(0)` whenever the labeled one is absent — the accessor must be
// omitted (`shadowed_when_absent` in context_label_selector).
shadowed
    : lead = PLUS? PLUS unary
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
