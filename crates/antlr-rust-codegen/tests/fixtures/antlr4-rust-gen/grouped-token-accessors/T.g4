grammar T;

root
    : expression operatorSequence eofChoice
    ;

expression
    : identifier
    | expression bop = ('<=' | '>=' | '==' | '!=' | '=' | '+=') expression
    ;

operatorSequence
    : ('<=' | '>=' | '==' | '!=' | '=' | '+=')+
    ;

eofChoice
    : (LE | EOF)
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
