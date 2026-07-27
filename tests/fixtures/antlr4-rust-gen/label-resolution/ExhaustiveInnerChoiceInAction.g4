grammar ExhaustiveInnerChoiceInAction;
r : (((y=A | z=A) x=A {System.out.println($x.text);}) | C) EOF;
A : 'a';
C : 'c';
