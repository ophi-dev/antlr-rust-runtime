grammar InnerChoiceArityInAction;
r : (((y=A | z=A | B) x=A {System.out.println($x.text);}) | C) EOF;
A : 'a';
B : 'b';
C : 'c';
