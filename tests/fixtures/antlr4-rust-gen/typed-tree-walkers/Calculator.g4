grammar Calculator;

start
    : expression EOF
    ;

expression
    : left = expression STAR right = expression  # Multiply
    | left = expression SLASH right = expression # Multiply
    | left = expression PLUS right = expression  # Add
    | left = expression MINUS right = expression # Add
    | INT                                         # Number
    ;

STAR
    : '*'
    ;

SLASH
    : '/'
    ;

PLUS
    : '+'
    ;

MINUS
    : '-'
    ;

INT
    : [0-9]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
