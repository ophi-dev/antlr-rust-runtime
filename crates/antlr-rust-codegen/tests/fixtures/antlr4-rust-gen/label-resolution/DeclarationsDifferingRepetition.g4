grammar DeclarationsDifferingRepetition;
r : (x=A B | x=A+ C) { println!("{}", $x.text); } ;
A:'a'; B:'b'; C:'c';
