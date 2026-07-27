grammar ListAliasDeclarations;
r : (xs+=A B | xs+='a' C) { let _: Vec<_> = $xs.collect(); } ;
A:'a'; B:'b'; C:'c';
