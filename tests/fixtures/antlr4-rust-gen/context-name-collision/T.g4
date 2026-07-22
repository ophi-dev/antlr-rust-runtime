parser grammar T;

tokens {
    NEW,
    OPEN_PARENS,
    CLOSE_PARENS
}

primary_expression_start
    : NEW object_creation_expression # objectCreationExpression
    ;

object_creation_expression
    : OPEN_PARENS CLOSE_PARENS
    ;
