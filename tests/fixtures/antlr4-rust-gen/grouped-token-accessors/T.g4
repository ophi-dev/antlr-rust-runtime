grammar T;

root
    : expression operatorSequence EOF
    ;

expression
    : identifier
    | expression bop = ('<=' | '>=' | '==' | '!=' | '=' | '+=') expression
    ;

operatorSequence
    : ('<=' | '>=' | '==' | '!=' | '=' | '+=')+
    ;

identifier
    : IDENTIFIER
    ;

LE
    : '<='
    ;

GE
    : '>='
    ;

EQUAL
    : '=='
    ;

NOTEQUAL
    : '!='
    ;

ASSIGN
    : '='
    ;

ADD_ASSIGN
    : '+='
    ;

IDENTIFIER
    : [a-z]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
