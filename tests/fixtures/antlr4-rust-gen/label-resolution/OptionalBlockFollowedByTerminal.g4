grammar OptionalBlockFollowedByTerminal;
r : x=(A | B)? C { println!("{}", $x.text); } EOF ;
A:'a'; B:'b'; C:'c';
