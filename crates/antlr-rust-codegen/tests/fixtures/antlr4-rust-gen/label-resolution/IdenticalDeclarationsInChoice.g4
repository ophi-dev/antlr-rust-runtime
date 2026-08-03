grammar IdenticalDeclarationsInChoice;
r : (x=A | x=A) { println!("{}", $x.text); } EOF ;
A:'a';
