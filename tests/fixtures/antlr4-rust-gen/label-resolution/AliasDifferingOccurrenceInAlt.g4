grammar AliasDifferingOccurrenceInAlt;
r
@after { println!("{}", $x.text); }
  : A x='a' B | A x=A C ;
A:'a'; B:'b'; C:'c';
