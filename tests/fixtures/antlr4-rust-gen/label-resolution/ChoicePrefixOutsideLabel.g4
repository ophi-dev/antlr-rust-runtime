grammar ChoicePrefixOutsideLabel;
r : (A B | A C) x=A { println!("{}", $x.text); } ;
A:'a'; B:'b'; C:'c';
