grammar Reachability;

start
    : live EOF
    ;

script
    : live end
    ;

alternate
    : EOF
    | ID
    ;

end
    : EOF
    ;

manual
    : live
    ;

live
    : ID
    ;

deadRoot
    : ID deadHelper?
    ;

deadHelper
    : ID deadRoot?
    ;

fragment LETTER
    : [a-z]
    ;

ID
    : LETTER+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
