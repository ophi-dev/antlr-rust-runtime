grammar T;
random : 'blort';
e : '--' e
  | e '*' e
  | e '+' e
  | e '--'
  | ID
  ;
ID : [a-z]+ ;
INT : [0-9]+ ;
WS : [ \r\t\n]+ ;