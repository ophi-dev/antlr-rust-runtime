grammar T;
a : A {System.out.println($A.text);} ;
A : ~('a'|B) ;
B : 'b' ;
