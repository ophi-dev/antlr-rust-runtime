grammar NestedChoiceInsideConfinedBranch;
r : ((a=A | b=B) x=(C | D) { println!("{}", $x.text); } | E) EOF ;
A:'a'; B:'b'; C:'c'; D:'d'; E:'e';
