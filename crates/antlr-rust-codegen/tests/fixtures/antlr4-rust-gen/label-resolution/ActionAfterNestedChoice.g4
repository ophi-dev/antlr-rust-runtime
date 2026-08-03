grammar ActionAfterNestedChoice;
r : (A | xs+=A) { let _: Vec<_> = $xs.collect(); } EOF ;
A:'a';
