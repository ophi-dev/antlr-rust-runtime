parser grammar T;

tokens {
    NEW,
    OPEN_PARENS,
    CLOSE_PARENS
}

primary_expression_start
    : NEW object_creation_expression # objectCreationExpression
    | OPEN_PARENS object_creation_expression CLOSE_PARENS # parenthesized
    ;

object_creation_expression
    : OPEN_PARENS CLOSE_PARENS
    ;
