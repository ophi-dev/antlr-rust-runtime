grammar MixedModeLeadingTerminal;
r
@after { println!("{}", $x.text); }
  : B x=A | x='a' ;
A:'a'; B:'b';
