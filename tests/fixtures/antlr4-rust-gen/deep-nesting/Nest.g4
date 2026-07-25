// Mirrors the failure shape from issue #193: an expression grammar whose rule
// chain multiplies input nesting into native call depth (CEL walks nine rules
// per `[`). Deeply nested input must parse without exhausting the native
// stack. The chain is deliberately 8+ rules long so the ATN-preferred
// classifier fires (issue #198: the depth cap must hold on that path too),
// and `expr` is left-recursive so operator expansions count toward the cap.
grammar Nest;

s
    : expr EOF
    ;

expr
    : expr '+' expr
    | disjunction
    ;

disjunction
    : conjunction ('||' conjunction)*
    ;

conjunction
    : equality ('&&' equality)*
    ;

equality
    : relation ('==' relation)*
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
