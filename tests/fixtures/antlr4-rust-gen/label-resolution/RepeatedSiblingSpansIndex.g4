grammar RepeatedSiblingSpansIndex;
r
@after { println!("{}", $x.text); }
  : C x=(A | B) | (D | E)+ ;
A:'a'; B:'b'; C:'c'; D:'d'; E:'e';
