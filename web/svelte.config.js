import adapter from "@sveltejs/adapter-static";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  compilerOptions: {
    immutable: true,
  },
  kit: {
    adapter: adapter({
      pages: "build",
      assets: "build",
      fallback: "index.html",
      precompress: true,
      strict: true,
    }),
    alias: {
      $lib: "src/lib",
    },
  },
};

export default config;
