grammar L;

expr
    : left=A '+' right=A        #primary
    | left=expr '-' right=expr  #sub
    ;

A: 'a';
B: 'b';
C: 'c';
