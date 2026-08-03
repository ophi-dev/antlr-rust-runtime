grammar T;
statement
locals[Scope scope]
    : expressionA[$scope] ';'
    ;
expressionA[Scope scope]
    : atom[$scope]
    | expressionA[$scope] '[' expressionA[$scope] ']'
    ;
atom[Scope scope]
    : 'dummy'
    ;
