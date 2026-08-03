grammar T;
s @after {System.out.println($ctx.toStringTree(this));} : a ;
a : a ID
  ;
ID : 'a'..'z'+ ;
WS : (' '|'\n') -> skip ;
