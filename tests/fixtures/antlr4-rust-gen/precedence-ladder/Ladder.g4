grammar Ladder;

start
    : expr EOF
    ;

starStart
    : sum EOF
    ;

sum
    : product ('+' product)*
    ;

product
    : primary ('*' primary)*
    ;

primary
    : INT
    ;

directStart
    : directEntry EOF
    ;

directEntry
    : direct
    ;

direct
    : right
    | direct '^' right
    ;

right
    : primary ('~' right)?
    ;

expr
    : e=conditionalOr (question='?' thenBranch=conditionalOr ':' elseBranch=expr)?
    ;

conditionalOr
    : e=conditionalAnd (operators+='||' operands+=conditionalAnd)*
    ;

conditionalAnd
    : e=relation (operators+='&&' operands+=relation)*
    ;

relation
    : calc
    | relation operator=('<' | '<=' | '>=' | '>' | '==' | '!=') relation
    ;

calc
    : unary
    | calc operator=('*' | '/' | '%') calc
    | calc operator=('+' | '-') calc
    ;

unary
    : atom                  # AtomExpression
    | (operators+='!')+ atom # LogicalNot
    | (operators+='-')+ atom # Negate
    ;

atom
    : INT
    | '(' expr ')'
    ;

INT : [0-9]+;
WS : [ \t\r\n]+ -> skip;
