grammar T;

start
    : child[1 + 2] EOF
    ;

child[int value]
    : ID
    ;

ID: 'x';
