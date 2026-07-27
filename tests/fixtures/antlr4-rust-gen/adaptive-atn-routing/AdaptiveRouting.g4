grammar AdaptiveRouting;

start
    : wrapper EOF
    ;

wrapper
    : (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      expression
    ;

expression
    : (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      (ID | ID ID)?
      ID
    | expression STAR expression
    | expression SLASH expression
    | expression PLUS expression
    | expression MINUS expression
    | expression AMP expression
    | expression CARET expression
    | expression PIPE expression
    | expression QUESTION expression
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

AMP
    : '&'
    ;

CARET
    : '^'
    ;

PIPE
    : '|'
    ;

QUESTION
    : '?'
    ;

ID
    : [a-z]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
