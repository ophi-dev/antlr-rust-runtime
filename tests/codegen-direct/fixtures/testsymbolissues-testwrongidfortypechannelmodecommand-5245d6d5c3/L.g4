lexer grammar L;
tokens { TOKEN1 }
channels { CHANNEL1 }
TOKEN: 'asdf' -> type(CHANNEL1), channel(MODE1), mode(TOKEN1);
mode MODE1;
MODE1_TOKEN: 'qwer';