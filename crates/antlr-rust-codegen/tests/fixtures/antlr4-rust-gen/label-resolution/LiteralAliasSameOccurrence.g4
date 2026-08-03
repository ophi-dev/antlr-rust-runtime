grammar LiteralAliasSameOccurrence;
r
@after { println!("{}", $x.text); }
  : x=A | x='a' ;
A : 'a';
