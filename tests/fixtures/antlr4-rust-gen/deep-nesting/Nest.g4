// Mirrors the failure shape from issue #193: an expression grammar whose rule
// chain multiplies input nesting into native call depth (CEL walks nine rules
// per `[`). Deeply nested input must parse without exhausting the native
// stack.
grammar Nest;

s
    : expr EOF
    ;

expr
    : disjunction
    ;

disjunction
    : conjunction ('||' conjunction)*
    ;

conjunction
    : relation ('&&' relation)*
    ;

relation
    : unary
    ;

unary
    : '!' unary
    | primary
    ;

primary
    : '[' expr ']'
    | A
    ;

A
    : 'a'
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
