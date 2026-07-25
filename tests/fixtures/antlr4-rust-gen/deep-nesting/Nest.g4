// Mirrors the failure shape from issue #193: an expression grammar whose rule
// chain multiplies input nesting into native call depth (CEL walks nine rules
// per `[`). Deeply nested input must parse without exhausting the native
// stack. `expr` is left-recursive so operator expansions count toward the
// depth cap (issue #198). The chain does NOT make any rule ATN-preferred —
// every decision here has an LL(1) fast path, so the classifier's
// decision-cost gate never fires; the cap-overrides-ATN-preference dispatch
// guard is pinned by the generator unit test
// `renders_atn_preferred_dispatch_only_for_generated_only_mode` instead.
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
