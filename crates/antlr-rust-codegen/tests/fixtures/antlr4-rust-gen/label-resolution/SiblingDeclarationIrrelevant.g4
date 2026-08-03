grammar SiblingDeclarationIrrelevant;
r : (x=A { println!("{}", $x.text); } | x=A+ B) EOF ;
A:'a'; B:'b';
