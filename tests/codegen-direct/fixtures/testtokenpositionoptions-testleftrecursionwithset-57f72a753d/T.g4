grammar T;
s : e ';' ;
e : e op=('*'|'/') e
  | e '+' e
  | e '.' ID
  | '-' e
  | ID
  ;
ID : [a-z]+ ;
