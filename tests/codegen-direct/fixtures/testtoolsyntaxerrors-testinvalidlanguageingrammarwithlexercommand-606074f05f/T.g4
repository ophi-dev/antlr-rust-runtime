grammar T;
options { language=Foo; }
start : 'T' EOF;
Something : 'something' -> channel(CUSTOM);