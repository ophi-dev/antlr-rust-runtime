grammar RecoveredDeletedTokenIndex;
r : D A x=(B | C) { println!("{}", $x.text); } EOF ;
A:'a'; B:'b'; C:'c'; D:'d'; X:'x';
