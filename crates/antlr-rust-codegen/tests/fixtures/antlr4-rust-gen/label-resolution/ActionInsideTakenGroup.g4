grammar ActionInsideTakenGroup;
r : (A x=A { println!("{}", $x.text); })? EOF ;
A:'a';
