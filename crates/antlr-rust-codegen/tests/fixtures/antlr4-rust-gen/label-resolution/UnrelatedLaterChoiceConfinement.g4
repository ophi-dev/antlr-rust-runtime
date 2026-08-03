grammar UnrelatedLaterChoiceConfinement;
r : ({false}? x=A | A) (B { println!("{}", $x.text); } | C) EOF ;
A:'a'; B:'b'; C:'c';
