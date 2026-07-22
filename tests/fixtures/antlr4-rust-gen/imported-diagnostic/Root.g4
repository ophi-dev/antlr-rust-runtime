grammar Root;

import Delegate;

start
    : delegated EOF
    ;

ID
    : [a-z]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
