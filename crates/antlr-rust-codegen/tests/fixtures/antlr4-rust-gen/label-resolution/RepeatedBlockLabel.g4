grammar RepeatedBlockLabel;
r : x=(A | B)+ { println!("{}", $x.text); } EOF ;
A:'a'; B:'b';
