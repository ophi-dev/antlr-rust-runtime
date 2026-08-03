grammar T;

start
    : required = requiredRule
      optional = optionalRule?
      bang = BANG
      COLON
      question = QUESTION?
      items += atom+
      EOF
    ;

requiredRule
    : ID                 # Bare
    | LPAREN ID RPAREN   # Wrapped
    ;

optionalRule
    : LBRACK ID RBRACK
    ;

atom
    : ID
    ;

BANG
    : '!'
    ;

COLON
    : ':'
    ;

QUESTION
    : '?'
    ;

LPAREN
    : '('
    ;

RPAREN
    : ')'
    ;

LBRACK
    : '['
    ;

RBRACK
    : ']'
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
