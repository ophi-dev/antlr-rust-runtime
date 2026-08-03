grammar A;
tokens{Foo}
b : Foo ;
X : 'foo1' -> popmode;
Y : 'foo2' -> token(Foo);