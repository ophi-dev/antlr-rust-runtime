grammar ExpandedBlockTerminalState;
r @after {System.out.println($x.text);} : (B y=C? | ) x=A | x='a';
A : 'a';
B : 'b';
C : 'c';
