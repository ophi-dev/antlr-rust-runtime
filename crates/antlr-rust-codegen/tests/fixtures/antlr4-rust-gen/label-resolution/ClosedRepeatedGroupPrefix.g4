grammar ClosedRepeatedGroupPrefix;
r : ((A B)+ x=A { println!("{}", $x.text); })? EOF ;
A : [ac]; B : 'b';
