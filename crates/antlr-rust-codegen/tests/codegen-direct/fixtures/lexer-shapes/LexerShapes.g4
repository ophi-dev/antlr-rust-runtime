lexer grammar LexerShapes;

tokens { RETYPED }
channels { COMMENTS }

A : 'ab' ;
RANGE : '0'..'9' ;
SET : 'x' | 'y' | 'z' ;
NOT : ~[q] ;
WILDCARD : . ;
CALL : DIGIT+ ;
fragment DIGIT : [0-9] ;
OPTIONAL : 'o'? ;
STAR : 's'*? ;
ACTION : 'c' {custom();} ;
PREDICATE : {ready()}? 'p' ;
COMMENT : '#' ~[\r\n]* -> channel(COMMENTS) ;
OPEN : '<' -> pushMode(TAG) ;
WS : [ \t]+ -> skip ;
EOF_TOKEN : EOF ;

mode TAG;

CLOSE : '>' -> popMode ;
NAME : [a-z]+ -> type(RETYPED) ;
CONTINUE : '&' -> more ;
RESET : '@' -> mode(DEFAULT_MODE) ;
