grammar A;
x : ;
y  : x?;
z1 : ('foo' | 'bar'? 'bar2'?)?;
z2 : ('foo' | 'bar' 'bar2'? | 'bar2')?;
