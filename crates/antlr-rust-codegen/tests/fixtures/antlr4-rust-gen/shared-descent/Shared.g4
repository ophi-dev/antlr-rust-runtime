grammar Shared;

start
    : expression EOF
    ;

expression
    : primary (PLUS primary)*
    ;

leftStart
    : leftExpression EOF
    ;

leftExpression
    : leftExpression PLUS leftExpression
    | primary
    ;

primary
    : identifier STRING
    | processingMode? qualifiedName LPAREN STAR RPAREN
    | processingMode? qualifiedName LPAREN expression? RPAREN
    | identifier OVER
    | identifier ARROW expression
    | identifier
    ;

processingMode
    : DISTINCT
    ;

qualifiedName
    : identifier (DOT identifier)*
    ;

identifier
    : ID
    | KW
    ;

prefixed
    : AT identifier COLON EOF
    | AT identifier SEMI EOF
    ;

wrapped
    : wrapper identifier COLON EOF
    | wrapper identifier SEMI EOF
    ;

wrapper
    : HASH?
    ;

failed
    : pair COLON EOF
    | pair SEMI EOF
    ;

pair
    : identifier BANG
    ;

semantic
    : {true}? identifier COLON EOF
    | {true}? identifier SEMI EOF
    ;

PLUS: '+';
LPAREN: '(';
RPAREN: ')';
STAR: '*';
OVER: 'over';
ARROW: '->';
DISTINCT: 'distinct';
DOT: '.';
AT: '@';
COLON: ':';
SEMI: ';';
HASH: '#';
BANG: '!';
QUESTION: '?';
KW: 'kw';
STRING: '\'' (~['\r\n])* '\'';
ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
