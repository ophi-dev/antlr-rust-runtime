grammar LiteralAliasDifferingOccurrence;
r
@after { println!("{}", $x.text); }
  : B x='a' | C A x=A ;
A:'a'; B:'b'; C:'c';
