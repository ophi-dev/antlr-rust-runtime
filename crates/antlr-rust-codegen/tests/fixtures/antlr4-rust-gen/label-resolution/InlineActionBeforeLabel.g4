grammar InlineActionBeforeLabel;
r : { println!("{}", $x.text); } A* x=A EOF ;
A:'a';
