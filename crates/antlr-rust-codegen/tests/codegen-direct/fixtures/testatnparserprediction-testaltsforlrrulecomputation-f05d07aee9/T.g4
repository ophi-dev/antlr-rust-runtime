grammar T;
e : e '*' e
  | INT
  | e '+' e
  | ID
  ;
ID : [a-z]+ ;
INT : [0-9]+ ;
WS : [ \r\t\n]+ ;