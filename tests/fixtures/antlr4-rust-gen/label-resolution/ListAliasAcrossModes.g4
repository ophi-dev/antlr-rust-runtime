grammar ListAliasAcrossModes;
r @after { let _: Vec<_> = $xs.collect(); } : xs+=A | xs+='a';
A : 'a';
