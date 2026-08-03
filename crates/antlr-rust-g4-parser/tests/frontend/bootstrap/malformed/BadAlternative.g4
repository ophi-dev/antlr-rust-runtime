grammar BadAlternative;

entry
    : left=A+ # Items
    | B -> skip
    ;

A: 'a';
B: 'b';
