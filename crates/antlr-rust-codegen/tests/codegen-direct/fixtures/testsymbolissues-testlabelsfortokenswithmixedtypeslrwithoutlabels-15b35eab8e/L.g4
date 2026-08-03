grammar L;

expr
    : left=A '+' right=A
    | left=expr '-' right=expr
    ;

A: 'a';
B: 'b';
C: 'c';
