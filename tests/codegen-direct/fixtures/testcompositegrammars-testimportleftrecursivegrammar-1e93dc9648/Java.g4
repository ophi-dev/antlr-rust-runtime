grammar Java;
e : '(' e ')'
  | e '=' e
  | ID
  ;
ID : [a-z]+ ;
