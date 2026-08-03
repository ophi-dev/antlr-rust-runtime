grammar PrecedingSiblingBranch;
r : (A | x=A) {System.out.println($x.text);} EOF;
A : 'a';
