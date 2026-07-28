// A return value named after a cycle satellite: the mutual-left-recursion
// pass would delete rule `s`, hiding the symbol conflict from validation.
// Symbol checks must therefore run against the authored grammar.
grammar ReturnsClash;
e returns [i32 s] : s | ID ;
s : e '+' ID ;
ID : [a-z]+ ;
WS : [ \t]+ -> skip ;
