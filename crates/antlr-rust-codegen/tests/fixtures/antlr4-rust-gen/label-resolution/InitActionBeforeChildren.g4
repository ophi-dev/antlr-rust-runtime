grammar InitActionBeforeChildren;
r
@init { let _: Vec<_> = $xs.collect(); }
  : xs+=A A ;
A:'a';
