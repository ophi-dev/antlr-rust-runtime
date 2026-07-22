lexer grammar L;
channels { CHANNEL1 }
tokens { TYPE1 }
// Incompatible
T00: 'a00' -> skip, more;
T01: 'a01' -> skip, type(TYPE1);
T02: 'a02' -> skip, channel(CHANNEL1);
T03: 'a03' -> more, type(TYPE1);
T04: 'a04' -> more, channel(CHANNEL1);
T05: 'a05' -> more, skip;
T06: 'a06' -> type(TYPE1), skip;
T07: 'a07' -> type(TYPE1), more;
T08: 'a08' -> channel(CHANNEL1), skip;
T09: 'a09' -> channel(CHANNEL1), more;
// Allowed
T10: 'a10' -> type(TYPE1), channel(CHANNEL1);
T11: 'a11' -> channel(CHANNEL1), type(TYPE1);