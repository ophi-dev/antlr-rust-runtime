const render = (name: string): string =>
    `outer ${name ? `inner ${name.toUpperCase()}` : `${/x+/.test(name)}`}`;
