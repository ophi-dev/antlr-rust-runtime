grammar ActionInCollapsibleChoice;
r : x=A? (A | B { println!("{}", $x.text); }) EOF ;
A:'a'; B:'b';
