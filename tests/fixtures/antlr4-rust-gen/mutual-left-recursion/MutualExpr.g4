// Mutual (indirect) left recursion — the tractable shapes from issue #151,
// distilled from dotnet/roslyn's CSharp.Generated.g4 cycles. ANTLR 4.13.2
// rejects this grammar with error(119); our generator reduces each cycle to
// direct left recursion (see src/bin_support/grammar/mutual_recursion.rs) and
// produces a working precedence-climbing parser.
grammar MutualExpr;

// Hub-and-spoke expression cycle: every binary/postfix satellite's left corner
// is `expr`; each is referenced only by the hub, so all collapse into it.
expr
  : add_expr
  | mul_expr
  | call_expr
  | range_expr        // leading-optional recursion (C#'s `a? '..' b?`)
  | primary
  ;

add_expr  : expr '+' expr ;
mul_expr  : expr '*' expr ;
call_expr : expr '(' ')' ;
range_expr : expr? '..' expr? ;
primary : INT | name ;

// Two-rule name cycle: `name`/`qualified_name`, exactly the Roslyn `name`
// shape. qualified_name is hub-only and collapses away.
name : qualified_name | ID ;
qualified_name : name '.' ID ;

INT : [0-9]+ ;
ID  : [a-zA-Z_] [a-zA-Z0-9_]* ;
WS  : [ \t\r\n]+ -> skip ;
