grammar T;

differentTypes
    : X op = A
    | Y op = B
    ;

sameType
    : X Y op = A
    ;

missingToken
    : X op = A B
    ;

X
    : 'x'
    ;

Y
    : 'y'
    ;

A
    : 'a'
    ;

B
    : 'b'
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
