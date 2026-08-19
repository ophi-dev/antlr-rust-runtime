grammar Inline;

start
    : stmt+ EOF
    ;

stmt
    : kw target SEMI
    | target kw* SEMI
    | marked SEMI
    | group SEMI
    ;

// Multi-use token-set candidate: inlined into both stmt references.
kw
    : 'select'
    | 'from'
    | 'where'
    ;

// Multi-use non-trivial rule: retained.
target
    : qualifier ID
    ;

// Single-use pure sequence candidate: inlined into target.
qualifier
    : AT AT?
    ;

// Single-use rule with an element label: declined.
marked
    : label=badge ID
    ;

// Token-set rule behind a labeled call site: declined all-or-nothing.
badge
    : STAR
    | PLUS
    ;

// Single-use pure sequence candidate: inlined into stmt.
group
    : LP target opt RP
    ;

// Nullable single-use rule: declined.
opt
    : COMMA?
    ;

ID : [a-z]+ ;
AT : '@' ;
SEMI : ';' ;
STAR : '*' ;
PLUS : '+' ;
LP : '(' ;
RP : ')' ;
COMMA : ',' ;
WS : [ \t\r\n]+ -> skip ;
