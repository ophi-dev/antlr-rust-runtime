grammar T;
s : e ';' ;
e : e '*' x=e
  | e '+' e
  | e '.' y=ID
  | '-' e
  | ID
  ;
ID : [a-z]+ ;
