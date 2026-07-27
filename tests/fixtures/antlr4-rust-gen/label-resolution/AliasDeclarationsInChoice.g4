grammar AliasDeclarationsInChoice;
r
@after { println!("{}", $x.text); }
  : (x=A | x='a') EOF ;
A : 'a';
