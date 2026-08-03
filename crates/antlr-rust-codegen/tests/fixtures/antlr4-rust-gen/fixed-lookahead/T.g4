grammar T;

// The `a` decision mirrors Thrift's `namespace_` shape: alternatives 1 and 2
// share their first token and separate only at the second, so the decision
// is fixed-LL(2) but not LL(1). The `a*` loop stays LL(1).
s: a* EOF;
a: 'ns' '*' ID | 'ns' ID ID | 'x' ID;

ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
