grammar NestedChoiceSatisfiability;
r
@after { println!("{}", $x.text); }
  : q x=q | ((q|b)|(q|c)) ;
q : A ;
b : B ;
c : C ;
A:'a'; B:'b'; C:'c';
