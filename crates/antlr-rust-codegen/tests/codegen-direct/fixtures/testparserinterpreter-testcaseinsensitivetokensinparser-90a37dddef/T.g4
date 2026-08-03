parser grammar T;
options { tokenVocab=L; caseInsensitive = true; }
e
    : ID
    | 'not' e
    | e 'and' e
    | 'new' ID '(' e ')'
    ;