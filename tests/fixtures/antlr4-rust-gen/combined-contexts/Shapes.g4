grammar Shapes;

start
    : first = atom # Single
    | rest += atom+ # Many
    ;

atom
    : ID
    ;

ID
    : [a-z]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
