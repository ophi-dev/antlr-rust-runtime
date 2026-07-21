grammar T;
s : e ';' ;
e : e '*' e
  | e '+' e
  | e '.' ID
  | '-' e
  | ID
  ;
ID : [a-z]+ ;
