parser grammar Labels;

tokens {
    INT,
    PLUS,
    MINUS
}

expr
    : expr PLUS expr  # Binary
    | expr MINUS expr # Binary
    | INT             # Atom
    ;
