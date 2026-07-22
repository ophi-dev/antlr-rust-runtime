grammar S;

s locals [boolean seen=false]
    : { $seen = true; } {$seen}?<fail='not seen'> A
    ;

A
    : 'a'
    ;
