grammar A;
x : 'x'
  | x '+'<assoc=right> x   // warning 157
  |<assoc=right> x '*' x   // ok
  ;
