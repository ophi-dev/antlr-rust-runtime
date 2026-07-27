grammar SiblingUnlabeledSameTarget;
r : (B | C x=A? A { println!("{}", $x.text); }) EOF ;
A:'a'; B:'b'; C:'c';
