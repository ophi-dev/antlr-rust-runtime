grammar ListLabelWithoutIterator;
r @after { let _: Vec<_> = $xs.collect(); } : xs+='a' | B xs+='a';
A : 'a';
B : 'b';
