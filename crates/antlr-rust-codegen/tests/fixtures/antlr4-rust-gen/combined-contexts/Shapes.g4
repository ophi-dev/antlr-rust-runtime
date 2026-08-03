grammar Shapes;

start
    : first = atom # Single
    | rest += atom (COMMA rest += atom)* # Many
    ;

latest
    : value = atom+
    ;

atom
    : ID
    ;

COMMA
    : ','
    ;

ID
    : [a-z]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
