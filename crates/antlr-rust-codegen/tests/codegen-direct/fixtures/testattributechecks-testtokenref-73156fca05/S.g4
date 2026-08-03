parser grammar S;
tokens{ID}
a : x=ID {Token t = $x; t = $ID;} ;
