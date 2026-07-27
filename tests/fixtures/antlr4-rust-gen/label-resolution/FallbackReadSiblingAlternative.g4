grammar FallbackReadSiblingAlternative;
r
@after { println!("{}", $x.text); }
  : A? ((x=(B|C))) | D ;
A:'a'; B:'b'; C:'c'; D:'d';
