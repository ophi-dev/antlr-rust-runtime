grammar T;
a : A {System.out.println($A.text);} ;
A : ~('a'|'aa') ;
B : 'b' ;
