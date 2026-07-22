lexer grammar A;
X1 : 'x' -> more    // ok
   ;
Y1 : 'x' {more();}  // ok
   ;
fragment
X2 : 'x' -> more    // warning 158
   ;
fragment
Y2 : 'x' {more();}  // warning 158
   ;
