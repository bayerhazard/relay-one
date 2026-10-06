// Stand-in for SvelteKit's `$app/state` in vitest (aliased in vitest.config.ts,
// like `$app/navigation`). The shell (Huelle, Profil) reads only the path to
// close its sheet and menu on a page change.
export const page = { url: new URL("http://localhost/") };
