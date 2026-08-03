grammar MandatoryInnerGroupClosed;
r : ((q) x=q { println!("{}", $x.text); })? EOF ;
q : A ;
A:'a';
