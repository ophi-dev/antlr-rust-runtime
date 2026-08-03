grammar ActionOnlyBranch;
r : x=A? (A | { println!("{}", $x.text); }) EOF ;
A:'a';
