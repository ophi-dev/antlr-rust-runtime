grammar InitScalarRuleLabel;
r
@init { let _ = $x.ctx; }
  : x=q EOF ;
q : A ;
A:'a';
