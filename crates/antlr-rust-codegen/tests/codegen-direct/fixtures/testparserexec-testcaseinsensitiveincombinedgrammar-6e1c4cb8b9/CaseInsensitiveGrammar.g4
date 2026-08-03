grammar CaseInsensitiveGrammar;
options { caseInsensitive = true; }
e
    : ID
    | 'not' e
    | e 'and' e
    | 'new' ID '(' e ')'
    ;
ID: [a-z_][a-z_0-9]*;
WS: [ \t\n\r]+ -> skip;