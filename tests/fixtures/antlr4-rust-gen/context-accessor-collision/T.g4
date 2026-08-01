parser grammar T;

tokens {
    ID
}

start
    : directTerminals labeled EOF
    ;

directTerminals
    : ID
    ;

labeled
    : direct_terminals=ID
    ;
