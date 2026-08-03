grammar ReassignedAfterAction;
r : x=A { println!("{}", $x.text); } x=A EOF ;
A:'a';
